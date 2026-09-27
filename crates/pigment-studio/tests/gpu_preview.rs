//! Hardware preview-lifecycle suite (task 11). Ignored by default; run with
//! `scripts/gpu-tests.sh` or
//! `cargo test --release -p pigment-studio --test gpu_preview -- --ignored --test-threads=1 --nocapture`.

#![allow(clippy::print_stderr)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use pigment_core::capability::AdapterPolicy;
use pigment_core::frame::AspectRatio;
use pigment_core::request::RequestId;
use pigment_core::seed::{SeedBundle, TextDigest, Variation};
use pigment_core::settings::{Appearance, FormSettings};
use pigment_gpu::{GpuContext, PaintRenderer};
use pigment_studio::preview::PreviewView;
use pigment_studio::worker::{PreviewJob, PreviewWorker, WorkerOptions};

fn rss_bytes() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib: u64 = s
        .lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(kib * 1024)
}

fn job(text: &str, w: u32, h: u32, haze: f64) -> PreviewJob {
    let mut appearance = Appearance::default();
    appearance.atmosphere.haze = haze;
    PreviewJob {
        seeds: SeedBundle::derive(TextDigest::from_source(text).unwrap(), Variation(0)),
        form: FormSettings::default(),
        aspect: AspectRatio::of(16, 9),
        appearance,
        width: w,
        height: h,
        submitted_at: Instant::now(),
    }
}

/// A storm of requests (resize drags, slider-like paint changes, prose
/// edits) at a rate the GPU cannot keep up with: queued work stays at one
/// running + one pending, every result is drained, the newest request is
/// the one shown, and process memory does not grow with the number of
/// requests.
#[test]
#[ignore = "needs a hardware GPU; run scripts/gpu-tests.sh"]
fn a_request_storm_stays_bounded_and_ends_on_the_newest() {
    let ctx = Arc::new(GpuContext::new(&AdapterPolicy::default()).expect("hardware GPU"));
    assert!(!ctx.capabilities.adapter.software);
    eprintln!("preview suite on {}", ctx.capabilities.label());
    let renderer = PaintRenderer::new(ctx).unwrap();
    let worker = PreviewWorker::spawn(renderer, WorkerOptions::default(), || {});
    let mut view = PreviewView::default();
    let mut id = 0u64;
    let mut shown = 0u32;
    let mut round = |n: u32, view: &mut PreviewView, shown: &mut u32| {
        for i in 0..n {
            id += 1;
            // Sizes wander like a window being dragged; every 50th request
            // edits the prose (new scene); the rest change paint only.
            let w = 640 + (i * 37 % 1280);
            let h = w * 9 / 16;
            let text = format!("storm {}", i / 50);
            let j = job(&text, w, h, (i % 100) as f64 / 100.0);
            view.submitted(RequestId(id));
            worker.submit(RequestId(id), j);
            assert!(worker.in_flight() <= 2);
            for r in worker.drain() {
                *shown += view.receive(r, Instant::now()).is_some() as u32;
            }
            std::thread::sleep(Duration::from_micros(500));
        }
        // Let it settle.
        let deadline = Instant::now() + Duration::from_secs(10);
        while view.is_pending() && Instant::now() < deadline {
            for r in worker.drain() {
                *shown += view.receive(r, Instant::now()).is_some() as u32;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!view.is_pending(), "settled");
        assert_eq!(view.shown.unwrap().id, RequestId(id), "newest shown");
    };
    round(200, &mut view, &mut shown); // warm-up: allocator and driver pools
    let before = rss_bytes();
    let t = Instant::now();
    round(1000, &mut view, &mut shown);
    let elapsed = t.elapsed();
    let after = rss_bytes();
    let stats = worker.stats();
    eprintln!(
        "1000 requests in {:.2} s: {} jobs started, {} painted, {} cancelled; {} shown, {} stale dropped; \
         scenes built {} / reused {}",
        elapsed.as_secs_f64(),
        stats.started,
        stats.rendered,
        stats.cancelled,
        shown,
        view.stale_dropped,
        stats.scenes_built,
        stats.scenes_reused
    );
    if let (Some(b), Some(a)) = (before, after) {
        let growth = a as i64 - b as i64;
        eprintln!(
            "resident memory: {:.1} MiB before, {:.1} MiB after (growth {:.1} MiB)",
            b as f64 / (1 << 20) as f64,
            a as f64 / (1 << 20) as f64,
            growth as f64 / (1 << 20) as f64
        );
        assert!(growth < 64 << 20, "memory grew by {growth} bytes");
    }
    assert!(
        stats.started < 1200,
        "most requests were superseded, not rendered"
    );
    assert!(
        stats.scenes_reused > stats.scenes_built,
        "paint-only changes reuse the scene"
    );
}
