//! Cancellation, progress and the preview job model.
//!
//! Preview policy (docs/architecture.md, "Preview lifecycle"):
//! - The UI submits into a [`Mailbox`] with **one pending slot**. A newer
//!   submission replaces the pending one and cancels the in-flight job, so
//!   queued work is bounded at one running + one pending however fast the
//!   user types or drags.
//! - Results pass through [`PreviewState::accept`]: a result is displayed
//!   only if it is newer than what is on screen. An old result arriving late
//!   is dropped.
//! - Exports use their own single-job slot and are never superseded.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use crate::request::RequestId;

/// Cooperative cancellation, checked by renderers between tiles (and by
/// sinks between bands). Cloning shares the flag.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> CancelToken {
        CancelToken::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Scene,
    Setup,
    /// Tiles rendered and read back; `done` of `total`.
    Tiles,
    /// Encoder flush and atomic rename (exports).
    Finalize,
}

/// Measured progress. Counts only; no ETA or percentage guesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub id: RequestId,
    pub phase: Phase,
    pub done: u32,
    pub total: u32,
}

pub trait ProgressSink {
    fn report(&mut self, progress: Progress);
}

impl<F: FnMut(Progress)> ProgressSink for F {
    fn report(&mut self, progress: Progress) {
        self(progress)
    }
}

/// Discards progress.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn report(&mut self, _: Progress) {}
}

/// A submitted job and its cancellation handle.
#[derive(Debug)]
pub struct Job<T> {
    pub id: RequestId,
    pub payload: T,
    pub cancel: CancelToken,
}

#[derive(Debug)]
struct Slots<T> {
    pending: Option<Job<T>>,
    running: Option<(RequestId, CancelToken)>,
    closed: bool,
}

/// Latest-wins, one-pending-slot mailbox between the UI and a worker thread.
#[derive(Debug)]
pub struct Mailbox<T> {
    slots: Mutex<Slots<T>>,
    ready: Condvar,
}

/// What happened to older work when a job was submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Submitted {
    /// A pending job that was dropped without starting.
    pub superseded: Option<RequestId>,
    /// A running job that was asked to cancel.
    pub cancelled: Option<RequestId>,
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Mailbox {
            slots: Mutex::new(Slots {
                pending: None,
                running: None,
                closed: false,
            }),
            ready: Condvar::new(),
        }
    }
}

impl<T> Mailbox<T> {
    pub fn new() -> Mailbox<T> {
        Mailbox::default()
    }

    /// UI side. Never blocks on rendering.
    pub fn submit(&self, id: RequestId, payload: T) -> Submitted {
        let mut s = self.slots.lock().expect("mailbox poisoned");
        let superseded = s.pending.take().map(|j| j.id);
        let cancelled = s.running.as_ref().map(|(rid, tok)| {
            tok.cancel();
            *rid
        });
        s.pending = Some(Job {
            id,
            payload,
            cancel: CancelToken::new(),
        });
        self.ready.notify_one();
        Submitted {
            superseded,
            cancelled,
        }
    }

    /// Worker side. Blocks until a job is pending; `None` after `close`.
    /// The returned job is marked running until [`Mailbox::done`].
    pub fn next(&self) -> Option<Job<T>> {
        let mut s = self.slots.lock().expect("mailbox poisoned");
        loop {
            if s.closed {
                return None;
            }
            if let Some(job) = s.pending.take() {
                s.running = Some((job.id, job.cancel.clone()));
                return Some(job);
            }
            s = self.ready.wait(s).expect("mailbox poisoned");
        }
    }

    /// Worker side: the running job has ended (any outcome).
    pub fn done(&self, id: RequestId) {
        let mut s = self.slots.lock().expect("mailbox poisoned");
        if s.running.as_ref().is_some_and(|(rid, _)| *rid == id) {
            s.running = None;
        }
    }

    /// Shutdown: drops pending work, cancels running work, wakes the worker.
    pub fn close(&self) {
        let mut s = self.slots.lock().expect("mailbox poisoned");
        s.closed = true;
        s.pending = None;
        if let Some((_, tok)) = &s.running {
            tok.cancel();
        }
        self.ready.notify_all();
    }

    /// Jobs pending or running (0, 1 or 2).
    pub fn in_flight(&self) -> usize {
        let s = self.slots.lock().expect("mailbox poisoned");
        s.pending.is_some() as usize + s.running.is_some() as usize
    }
}

/// UI-side stale-result filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PreviewState {
    /// Newest request submitted.
    pub requested: Option<RequestId>,
    /// Request whose result is on screen.
    pub displayed: Option<RequestId>,
}

impl PreviewState {
    pub fn submitted(&mut self, id: RequestId) {
        self.requested = Some(self.requested.map_or(id, |r| r.max(id)));
    }

    /// Whether a finished result may replace what is on screen. Updates the
    /// displayed id when it returns `true`.
    pub fn accept(&mut self, id: RequestId) -> bool {
        if self.displayed.is_some_and(|d| id <= d) {
            return false;
        }
        self.displayed = Some(id);
        true
    }

    /// The screen shows an older recipe than the newest request (show a
    /// "rendering…" indicator; the controls already reflect the new values).
    pub fn is_pending(&self) -> bool {
        self.requested.is_some() && self.requested != self.displayed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn newer_submission_supersedes_pending_and_cancels_running() {
        let m = Mailbox::new();
        m.submit(RequestId(1), "a");
        let job1 = m.next().unwrap();
        assert_eq!(job1.id, RequestId(1));
        let s = m.submit(RequestId(2), "b");
        assert_eq!(
            s,
            Submitted {
                superseded: None,
                cancelled: Some(RequestId(1))
            }
        );
        assert!(job1.cancel.is_cancelled());
        let s = m.submit(RequestId(3), "c");
        assert_eq!(s.superseded, Some(RequestId(2)));
        assert_eq!(m.in_flight(), 2, "bounded: one running + one pending");
        m.done(RequestId(1));
        let job3 = m.next().unwrap();
        assert_eq!((job3.id, job3.payload), (RequestId(3), "c"));
        assert!(!job3.cancel.is_cancelled());
    }

    #[test]
    fn rapid_submissions_stay_bounded() {
        let m = Mailbox::new();
        for i in 1..=10_000 {
            m.submit(RequestId(i), vec![0u8; 16]);
            assert!(m.in_flight() <= 2);
        }
        assert_eq!(m.next().unwrap().id, RequestId(10_000));
    }

    #[test]
    fn close_wakes_a_blocked_worker() {
        let m = Arc::new(Mailbox::<()>::new());
        let w = {
            let m = m.clone();
            thread::spawn(move || m.next().is_none())
        };
        thread::sleep(Duration::from_millis(20));
        m.close();
        assert!(w.join().unwrap());
    }

    #[test]
    fn late_old_results_never_replace_newer_ones() {
        let mut p = PreviewState::default();
        for i in 1..=3 {
            p.submitted(RequestId(i));
        }
        assert!(p.accept(RequestId(3)));
        assert!(!p.accept(RequestId(2)), "out-of-order old result");
        assert!(!p.accept(RequestId(3)), "duplicate");
        assert!(!p.is_pending());
        p.submitted(RequestId(4));
        assert!(p.is_pending());
    }

    #[test]
    fn closures_are_progress_sinks() {
        let mut seen = Vec::new();
        {
            let mut sink = |p: Progress| seen.push(p.done);
            let s: &mut dyn ProgressSink = &mut sink;
            s.report(Progress {
                id: RequestId(1),
                phase: Phase::Tiles,
                done: 1,
                total: 2,
            });
        }
        assert_eq!(seen, vec![1]);
    }
}
