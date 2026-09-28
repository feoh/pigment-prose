//! Opening and saving recipes (task 12): native file dialogs, the
//! unsaved-changes prompt and the order of operations between them, kept
//! free of egui so every path can be tested with a scripted
//! [`Dialogs`] stand-in.
//!
//! - **Dialogs never block the UI.** [`NativeDialogs`] runs the platform
//!   dialog (XDG desktop portal on Linux, `IFileDialog` on Windows,
//!   `NSOpenPanel`/`NSSavePanel` on macOS, via rfd) on a helper thread, and
//!   the UI polls for the answer. Overwrite confirmation is the save
//!   dialog's own, following each platform's convention.
//! - **Nothing is lost silently.** Opening a recipe or quitting with unsaved
//!   work asks first: save, discard, or cancel. Cancelling any dialog, or a
//!   failed save, stops the whole operation.
//! - **A failed open changes nothing** ([`Document::replace_from_file`]
//!   semantics), and a failed save keeps the path and the unsaved state.
//!
//! Messages name the file (its file name only) and never contain recipe
//! contents or prose.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};

use pigment_core::recipe::VersionNotice;
use pigment_io::{Document, RecipeFileError, SaveError};

/// Suggested name for a recipe that has never been saved. Deliberately not
/// derived from the prose, which must not leak into file names.
pub const DEFAULT_FILE_NAME: &str = "painting.recipe.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogRequest {
    Open,
    Save {
        suggested: String,
    },
    /// Where to write an exported PNG (task 13).
    ExportPng {
        suggested: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogAnswer {
    Picked(PathBuf),
    /// The user closed the dialog without choosing a file.
    Cancelled,
}

/// A source of file dialog answers. One dialog at a time.
pub trait Dialogs {
    /// Shows a dialog. `parent` makes it modal to the window where the
    /// platform supports that.
    fn start(&mut self, request: DialogRequest, parent: Option<&dyn Parent>);
    /// The answer, once the user has made one. Never blocks.
    fn poll(&mut self) -> Option<DialogAnswer>;
}

/// A window a dialog can be attached to.
pub trait Parent: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle {}
impl<T: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle> Parent for T {}

/// The platform's dialogs, run on a helper thread.
pub struct NativeDialogs {
    pending: Option<Receiver<DialogAnswer>>,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for NativeDialogs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeDialogs")
            .field("pending", &self.pending.is_some())
            .finish()
    }
}

impl NativeDialogs {
    /// `wake` is called when an answer is ready (request a repaint).
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> NativeDialogs {
        NativeDialogs {
            pending: None,
            wake: Arc::new(wake),
        }
    }
}

impl Dialogs for NativeDialogs {
    fn start(&mut self, request: DialogRequest, parent: Option<&dyn Parent>) {
        let filter = match request {
            DialogRequest::ExportPng { .. } => ("PNG image", "png"),
            _ => ("Pigment Prose recipe", "json"),
        };
        let mut d = rfd::FileDialog::new()
            .add_filter(filter.0, &[filter.1])
            .set_can_create_directories(true);
        if let Some(p) = parent {
            d = d.set_parent(p);
        }
        let (tx, rx) = mpsc::channel();
        self.pending = Some(rx);
        let wake = self.wake.clone();
        let answer = move |picked: Option<PathBuf>| {
            let _ = tx.send(picked.map_or(DialogAnswer::Cancelled, DialogAnswer::Picked));
            wake();
        };
        match request {
            DialogRequest::Open => {
                let d = d.set_title("Open recipe");
                std::thread::spawn(move || answer(d.pick_file()));
            }
            DialogRequest::Save { suggested } => {
                let d = d.set_title("Save recipe").set_file_name(suggested);
                std::thread::spawn(move || answer(d.save_file()));
            }
            DialogRequest::ExportPng { suggested } => {
                let d = d.set_title("Export PNG").set_file_name(suggested);
                std::thread::spawn(move || answer(d.save_file()));
            }
        }
    }

    fn poll(&mut self) -> Option<DialogAnswer> {
        let rx = self.pending.as_ref()?;
        match rx.try_recv() {
            Ok(a) => {
                self.pending = None;
                Some(a)
            }
            Err(mpsc::TryRecvError::Empty) => None,
            // The dialog thread died (for example no portal is running).
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                Some(DialogAnswer::Cancelled)
            }
        }
    }
}

/// What the user asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Open,
    Save,
    SaveAs,
    Quit,
}

/// Answer to "save changes first?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Save,
    Discard,
    Cancel,
}

/// Where the flow is waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    /// Unsaved work would be replaced by `then` (Open or Quit).
    Confirm {
        then: Intent,
    },
    /// An open dialog is showing.
    Opening,
    /// A save dialog is showing; `then` runs after a successful save.
    Saving {
        then: Option<Intent>,
    },
}

/// Something the app must do or show.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// A recipe was opened into the document.
    Opened {
        name: String,
        notices: Vec<VersionNotice>,
    },
    Saved {
        name: String,
    },
    /// Close the window now (unsaved work was saved or discarded).
    Close,
    /// An operation failed; the document is unchanged.
    Failed {
        action: &'static str,
        name: String,
        error: String,
    },
}

/// The open/save state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileFlow {
    pub step: Step,
}

impl Default for FileFlow {
    fn default() -> Self {
        FileFlow { step: Step::Idle }
    }
}

/// The file name to show for `path`.
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the recipe".to_string())
}

impl FileFlow {
    /// A dialog or prompt is open: the rest of the window should not start
    /// another file operation.
    pub fn busy(&self) -> bool {
        self.step != Step::Idle
    }

    /// Starts `intent`. `unsaved` is whether replacing the document would
    /// lose work.
    pub fn request(
        &mut self,
        intent: Intent,
        doc: &mut Document,
        unsaved: bool,
        dialogs: &mut dyn Dialogs,
        parent: Option<&dyn Parent>,
    ) -> Vec<Effect> {
        if self.busy() {
            return Vec::new();
        }
        match intent {
            Intent::Open | Intent::Quit if unsaved => {
                self.step = Step::Confirm { then: intent };
                Vec::new()
            }
            Intent::Open => self.proceed(Intent::Open, dialogs, parent),
            Intent::Quit => vec![Effect::Close],
            Intent::Save | Intent::SaveAs => self.save(intent, None, doc, dialogs, parent),
        }
    }

    /// The answer to the unsaved-changes prompt.
    pub fn choose(
        &mut self,
        choice: Choice,
        doc: &mut Document,
        dialogs: &mut dyn Dialogs,
        parent: Option<&dyn Parent>,
    ) -> Vec<Effect> {
        let Step::Confirm { then } = self.step else {
            return Vec::new();
        };
        self.step = Step::Idle;
        match choice {
            Choice::Cancel => Vec::new(),
            Choice::Discard => self.proceed(then, dialogs, parent),
            Choice::Save => self.save(Intent::Save, Some(then), doc, dialogs, parent),
        }
    }

    /// Checks for a dialog answer. Call every frame while [`busy`](Self::busy).
    pub fn poll(
        &mut self,
        doc: &mut Document,
        dialogs: &mut dyn Dialogs,
        parent: Option<&dyn Parent>,
    ) -> Vec<Effect> {
        let (Step::Opening | Step::Saving { .. }) = self.step else {
            return Vec::new();
        };
        let Some(answer) = dialogs.poll() else {
            return Vec::new();
        };
        let step = std::mem::replace(&mut self.step, Step::Idle);
        let DialogAnswer::Picked(path) = answer else {
            return Vec::new(); // cancelled: nothing happens
        };
        match step {
            Step::Opening => vec![open(doc, &path)],
            Step::Saving { then } => match doc.save_as(&path) {
                Ok(()) => {
                    let mut out = vec![Effect::Saved {
                        name: display_name(&path),
                    }];
                    if let Some(t) = then {
                        out.extend(self.proceed(t, dialogs, parent));
                    }
                    out
                }
                Err(e) => vec![save_failed(&path, &e)],
            },
            _ => unreachable!(),
        }
    }

    /// Saves to the document's path, or asks for one.
    fn save(
        &mut self,
        intent: Intent,
        then: Option<Intent>,
        doc: &mut Document,
        dialogs: &mut dyn Dialogs,
        parent: Option<&dyn Parent>,
    ) -> Vec<Effect> {
        if intent == Intent::Save {
            match doc.save() {
                Ok(()) => {
                    let name = doc.path().map(display_name).unwrap_or_default();
                    let mut out = vec![Effect::Saved { name }];
                    if let Some(t) = then {
                        out.extend(self.proceed(t, dialogs, parent));
                    }
                    return out;
                }
                Err(SaveError::File(e)) => {
                    let path = doc.path().map(Path::to_path_buf).unwrap_or_default();
                    return vec![save_failed(&path, &e)];
                }
                Err(SaveError::NoPath) => {}
            }
        }
        let suggested = doc
            .path()
            .map(display_name)
            .unwrap_or_else(|| DEFAULT_FILE_NAME.to_string());
        dialogs.start(DialogRequest::Save { suggested }, parent);
        self.step = Step::Saving { then };
        Vec::new()
    }

    /// Runs Open or Quit once unsaved work has been dealt with.
    fn proceed(
        &mut self,
        intent: Intent,
        dialogs: &mut dyn Dialogs,
        parent: Option<&dyn Parent>,
    ) -> Vec<Effect> {
        match intent {
            Intent::Open => {
                dialogs.start(DialogRequest::Open, parent);
                self.step = Step::Opening;
                Vec::new()
            }
            Intent::Quit => vec![Effect::Close],
            Intent::Save | Intent::SaveAs => Vec::new(),
        }
    }
}

fn open(doc: &mut Document, path: &Path) -> Effect {
    match doc.replace_from_file(path) {
        Ok(notices) => Effect::Opened {
            name: display_name(path),
            notices,
        },
        Err(e) => Effect::Failed {
            action: "open",
            name: display_name(path),
            error: e.to_string(),
        },
    }
}

fn save_failed(path: &Path, e: &RecipeFileError) -> Effect {
    Effect::Failed {
        action: "save",
        name: display_name(path),
        error: e.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::VecDeque;
    use std::fs;

    use pigment_core::frame::UHD_4K;
    use pigment_core::seed::Variation;

    use super::*;

    /// Scripted dialogs: answers are handed out in order, one per dialog.
    #[derive(Debug, Default)]
    pub(crate) struct FakeDialogs {
        pub(crate) answers: VecDeque<DialogAnswer>,
        pub(crate) shown: Vec<DialogRequest>,
        open: bool,
    }

    impl FakeDialogs {
        pub(crate) fn answering(answers: impl IntoIterator<Item = DialogAnswer>) -> FakeDialogs {
            FakeDialogs {
                answers: answers.into_iter().collect(),
                ..Default::default()
            }
        }
    }

    impl Dialogs for FakeDialogs {
        fn start(&mut self, request: DialogRequest, _: Option<&dyn Parent>) {
            assert!(!self.open, "one dialog at a time");
            self.open = true;
            self.shown.push(request);
        }

        fn poll(&mut self) -> Option<DialogAnswer> {
            if !self.open {
                return None;
            }
            self.open = false;
            Some(
                self.answers
                    .pop_front()
                    .expect("an answer for every dialog"),
            )
        }
    }

    const PROSE: &str = "Grey heron over the shallows at dawn.";

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pigment-files-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn picked(p: &Path) -> DialogAnswer {
        DialogAnswer::Picked(p.to_path_buf())
    }

    /// Drives one request to completion.
    fn run(
        flow: &mut FileFlow,
        intent: Intent,
        doc: &mut Document,
        unsaved: bool,
        d: &mut FakeDialogs,
        choice: Option<Choice>,
    ) -> Vec<Effect> {
        let mut out = flow.request(intent, doc, unsaved, d, None);
        if let Some(c) = choice {
            assert!(matches!(flow.step, Step::Confirm { .. }), "{:?}", flow.step);
            out.extend(flow.choose(c, doc, d, None));
        }
        for _ in 0..4 {
            out.extend(flow.poll(doc, d, None));
        }
        out
    }

    #[test]
    fn save_asks_for_a_path_once_then_saves_in_place() {
        let dir = temp("save");
        let path = dir.join("lake.recipe.json");
        let mut doc = Document::from_prose(PROSE, UHD_4K).unwrap();
        let mut d = FakeDialogs::answering([picked(&path)]);
        let mut flow = FileFlow::default();
        let out = run(&mut flow, Intent::Save, &mut doc, true, &mut d, None);
        assert_eq!(
            out,
            [Effect::Saved {
                name: "lake.recipe.json".into()
            }]
        );
        assert_eq!(
            d.shown,
            [DialogRequest::Save {
                suggested: DEFAULT_FILE_NAME.into()
            }]
        );
        assert!(!doc.is_dirty() && path.exists());
        // Second save: no dialog.
        doc.next_variation();
        let out = run(&mut flow, Intent::Save, &mut doc, true, &mut d, None);
        assert_eq!(out.len(), 1);
        assert_eq!(d.shown.len(), 1);
        assert!(!doc.is_dirty());
        // Save As always asks, suggesting the current name.
        d.answers.push_back(DialogAnswer::Cancelled);
        let out = run(&mut flow, Intent::SaveAs, &mut doc, false, &mut d, None);
        assert!(out.is_empty());
        assert_eq!(
            d.shown[1],
            DialogRequest::Save {
                suggested: "lake.recipe.json".into()
            }
        );
        assert_eq!(flow.step, Step::Idle);
    }

    #[test]
    fn cancelled_dialogs_change_nothing() {
        let dir = temp("cancel");
        let mut doc = Document::from_prose(PROSE, UHD_4K).unwrap();
        let before = doc.clone();
        let mut d = FakeDialogs::answering([DialogAnswer::Cancelled, DialogAnswer::Cancelled]);
        let mut flow = FileFlow::default();
        assert!(run(&mut flow, Intent::SaveAs, &mut doc, true, &mut d, None).is_empty());
        assert!(run(&mut flow, Intent::Open, &mut doc, false, &mut d, None).is_empty());
        assert_eq!(doc, before);
        assert_eq!(flow.step, Step::Idle);
        assert!(
            fs::read_dir(&dir).unwrap().next().is_none(),
            "no file written"
        );
    }

    #[test]
    fn unsaved_work_is_never_replaced_without_asking() {
        let dir = temp("confirm");
        let other = dir.join("other.recipe.json");
        let mut saved = Document::from_prose("Another passage entirely.", UHD_4K).unwrap();
        saved.set_variation(Variation(7));
        saved.save_as(&other).unwrap();

        let mine = dir.join("mine.recipe.json");
        let mut doc = Document::from_prose(PROSE, UHD_4K).unwrap();
        let mut flow = FileFlow::default();

        // Cancel at the prompt: no dialog, nothing changes.
        let mut d = FakeDialogs::default();
        let before = doc.clone();
        assert!(
            run(
                &mut flow,
                Intent::Open,
                &mut doc,
                true,
                &mut d,
                Some(Choice::Cancel)
            )
            .is_empty()
        );
        assert!(d.shown.is_empty());
        assert_eq!(doc, before);

        // Save first, then open: two dialogs, the work is on disk.
        let mut d = FakeDialogs::answering([picked(&mine), picked(&other)]);
        let out = run(
            &mut flow,
            Intent::Open,
            &mut doc,
            true,
            &mut d,
            Some(Choice::Save),
        );
        assert!(matches!(&out[0], Effect::Saved { name } if name == "mine.recipe.json"));
        assert!(matches!(&out[1], Effect::Opened { name, .. } if name == "other.recipe.json"));
        assert_eq!(doc.recipe().seed.variation, Variation(7));
        assert_eq!(Document::open(&mine).unwrap().0.recipe(), before.recipe());

        // Untitled work: cancelling the save dialog cancels the open too.
        let mut d = FakeDialogs::answering([DialogAnswer::Cancelled]);
        doc = Document::from_prose(PROSE, UHD_4K).unwrap();
        let untitled = doc.clone();
        let out = run(
            &mut flow,
            Intent::Open,
            &mut doc,
            true,
            &mut d,
            Some(Choice::Save),
        );
        assert!(out.is_empty());
        assert_eq!(d.shown.len(), 1, "no open dialog after a cancelled save");
        assert_eq!(doc, untitled);

        // Discard: straight to the open dialog.
        let mut d = FakeDialogs::answering([picked(&other)]);
        let out = run(
            &mut flow,
            Intent::Open,
            &mut doc,
            true,
            &mut d,
            Some(Choice::Discard),
        );
        assert!(matches!(&out[0], Effect::Opened { .. }));
    }

    #[test]
    fn quitting_with_unsaved_work_asks_and_a_failed_save_keeps_the_window() {
        let dir = temp("quit");
        let mut doc = Document::from_prose(PROSE, UHD_4K).unwrap();
        let mut flow = FileFlow::default();
        let mut d = FakeDialogs::default();
        assert_eq!(
            run(&mut flow, Intent::Quit, &mut doc, false, &mut d, None),
            [Effect::Close]
        );
        assert!(
            run(
                &mut flow,
                Intent::Quit,
                &mut doc,
                true,
                &mut d,
                Some(Choice::Cancel)
            )
            .is_empty()
        );
        assert_eq!(
            run(
                &mut flow,
                Intent::Quit,
                &mut doc,
                true,
                &mut d,
                Some(Choice::Discard)
            ),
            [Effect::Close]
        );
        // Save to a directory that does not exist: the write fails, the
        // window stays open and the work is still unsaved.
        let bad = dir.join("missing/dir/x.recipe.json");
        let mut d = FakeDialogs::answering([picked(&bad)]);
        let out = run(
            &mut flow,
            Intent::Quit,
            &mut doc,
            true,
            &mut d,
            Some(Choice::Save),
        );
        assert_eq!(out.len(), 1, "{out:?}");
        match &out[0] {
            Effect::Failed {
                action,
                name,
                error,
            } => {
                assert_eq!((*action, name.as_str()), ("save", "x.recipe.json"));
                assert!(!error.contains(PROSE));
            }
            other => panic!("{other:?}"),
        }
        assert!(doc.is_dirty() && doc.path().is_none());
        let good = dir.join("ok.recipe.json");
        let mut d = FakeDialogs::answering([picked(&good)]);
        let out = run(
            &mut flow,
            Intent::Quit,
            &mut doc,
            true,
            &mut d,
            Some(Choice::Save),
        );
        assert_eq!(out.last(), Some(&Effect::Close));
    }

    #[test]
    fn a_bad_file_is_reported_and_leaves_the_document_alone() {
        let dir = temp("bad");
        let bad = dir.join("broken.recipe.json");
        fs::write(&bad, "{ not a recipe").unwrap();
        let mut doc = Document::from_prose(PROSE, UHD_4K).unwrap();
        let before = doc.clone();
        let mut flow = FileFlow::default();
        let mut d = FakeDialogs::answering([picked(&bad), picked(&dir.join("absent.json"))]);
        for _ in 0..2 {
            let out = run(&mut flow, Intent::Open, &mut doc, false, &mut d, None);
            assert!(
                matches!(&out[..], [Effect::Failed { action: "open", .. }]),
                "{out:?}"
            );
            assert_eq!(doc, before);
        }
        // Writing over an existing read-only file fails the same way.
        let ro = dir.join("ro.recipe.json");
        doc.save_as(&ro).unwrap();
        let mut perm = fs::metadata(&ro).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&ro, perm.clone()).unwrap();
        doc.next_variation();
        let out = run(&mut flow, Intent::Save, &mut doc, true, &mut d, None);
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(&ro, perm).unwrap();
        // Unix replaces the file by rename (allowed in a writable
        // directory); Windows refuses. Either way the state is consistent.
        match &out[..] {
            [Effect::Saved { .. }] => assert!(!doc.is_dirty()),
            [Effect::Failed { .. }] => assert!(doc.is_dirty()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn busy_flows_ignore_new_requests() {
        let mut doc = Document::from_prose(PROSE, UHD_4K).unwrap();
        let mut flow = FileFlow::default();
        let mut d = FakeDialogs::answering([DialogAnswer::Cancelled]);
        flow.request(Intent::SaveAs, &mut doc, true, &mut d, None);
        assert!(flow.busy());
        assert!(
            flow.request(Intent::Open, &mut doc, false, &mut d, None)
                .is_empty()
        );
        assert_eq!(d.shown.len(), 1);
        flow.poll(&mut doc, &mut d, None);
        assert!(!flow.busy());
    }
}
