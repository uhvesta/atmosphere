//! A folder of files with a preview pane and a rename sheet. The preview
//! is plain optional state run with `if_let`: it's there while a file is
//! selected. The rename form is presented in a `PresentationState` and run
//! with `if_presented`, which cancels its work when it goes and lets it
//! close itself with `Effect::dismiss`. When the rename is saved, the form
//! tells the folder through a delegate action, and the folder renames the
//! file.
//!
//! Patterns: optional child state with `if_let`; a presented child with
//! `PresentationState`, `if_presented` and `Effect::dismiss`; the parent
//! reacting to the child's save.

use std::sync::Arc;
use std::time::Duration;

use tca::{
    Clock, Effect, PresentationAction, PresentationState, Reducer, TestClock, TestStore, if_let,
    if_presented,
};

// The preview pane.

#[derive(Clone, Debug, PartialEq)]
struct Preview {
    name: String,
    /// None while loading.
    text: Option<String>,
}

#[derive(Debug, PartialEq)]
enum PreviewAction {
    Load,
    Loaded(String),
}

/// One preview loads at a time, so it's cancelled by a plain id.
const PREVIEW: &str = "preview";

struct Previewing {
    clock: Arc<dyn Clock>,
}

impl Reducer for Previewing {
    type State = Preview;
    type Action = PreviewAction;

    fn reduce(&self, preview: &mut Preview, action: PreviewAction) -> Effect<PreviewAction> {
        match action {
            PreviewAction::Load => {
                preview.text = None;
                let name = preview.name.clone();
                let clock = self.clock.clone();
                // Cancelling in flight: a newer selection replaces the
                // load under way rather than racing it.
                Effect::future(async move {
                    clock.sleep(Duration::from_millis(500)).await;
                    PreviewAction::Loaded(format!("first lines of {name}"))
                })
                .cancellable(PREVIEW, true)
            }
            PreviewAction::Loaded(text) => {
                preview.text = Some(text);
                Effect::none()
            }
        }
    }
}

// The rename form.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NameProblem {
    Empty,
    Taken,
}

#[derive(Clone, Debug, PartialEq)]
struct RenameForm {
    original: String,
    name: String,
    /// The other names in the folder, copied in when the form opens, so it
    /// can check a name without being handed the folder.
    taken: Vec<String>,
    saving: bool,
    problem: Option<NameProblem>,
}

#[derive(Clone, Debug, PartialEq)]
enum RenameAction {
    Typed(String),
    Save,
    Saved,
    Cancel,
    Delegate(RenameDelegate),
}

#[derive(Clone, Debug, PartialEq)]
enum RenameDelegate {
    Renamed { from: String, to: String },
}

struct Renaming {
    clock: Arc<dyn Clock>,
}

impl Reducer for Renaming {
    type State = RenameForm;
    type Action = RenameAction;

    fn reduce(&self, form: &mut RenameForm, action: RenameAction) -> Effect<RenameAction> {
        match action {
            RenameAction::Typed(name) => {
                form.name = name;
                form.problem = None;
                Effect::none()
            }
            RenameAction::Save => {
                let name = form.name.trim();
                form.problem = if name.is_empty() {
                    Some(NameProblem::Empty)
                } else if form.taken.iter().any(|taken| taken == name) {
                    Some(NameProblem::Taken)
                } else {
                    None
                };
                if form.problem.is_some() {
                    return Effect::none();
                }
                form.saving = true;
                let clock = self.clock.clone();
                Effect::future(async move {
                    clock.sleep(Duration::from_secs(1)).await;
                    RenameAction::Saved
                })
            }
            RenameAction::Saved => {
                form.saving = false;
                let renamed = RenameDelegate::Renamed {
                    from: form.original.clone(),
                    to: form.name.trim().to_owned(),
                };
                // Tell the folder first, then close: `then` keeps the
                // order, so the folder has the new name before the form
                // goes.
                Effect::send(RenameAction::Delegate(renamed)).then(Effect::dismiss())
            }
            RenameAction::Cancel => Effect::dismiss(),
            RenameAction::Delegate(_) => Effect::none(),
        }
    }
}

// The folder.

#[derive(Clone, Debug, Default, PartialEq)]
struct Folder {
    files: Vec<String>,
    preview: Option<Preview>,
    rename: PresentationState<RenameForm>,
}

#[derive(Debug, PartialEq)]
enum FolderAction {
    Select(String),
    ClosePreview,
    Preview(PreviewAction),
    RenameTapped(String),
    Rename(PresentationAction<RenameAction>),
}

struct Browsing {
    previewing: Previewing,
    renaming: Renaming,
}

impl Reducer for Browsing {
    type State = Folder;
    type Action = FolderAction;

    fn reduce(&self, folder: &mut Folder, action: FolderAction) -> Effect<FolderAction> {
        match action {
            FolderAction::Select(name) => {
                folder.preview = Some(Preview { name, text: None });
                // Calls the child's own code for its first action rather
                // than sending one: no round trip through the store.
                if_let(&self.previewing, &mut folder.preview, PreviewAction::Load)
                    .map(FolderAction::Preview)
            }
            FolderAction::ClosePreview => {
                folder.preview = None;
                // `if_let` only drops actions for a child that's gone; it
                // doesn't stop the child's work. The parent does that.
                Effect::cancel(PREVIEW)
            }
            FolderAction::Preview(action) => {
                if_let(&self.previewing, &mut folder.preview, action).map(FolderAction::Preview)
            }
            FolderAction::RenameTapped(name) => {
                let taken = folder
                    .files
                    .iter()
                    .filter(|file| **file != name)
                    .cloned()
                    .collect();
                folder.rename.present(RenameForm {
                    original: name.clone(),
                    name,
                    taken,
                    saving: false,
                    problem: None,
                });
                Effect::none()
            }
            FolderAction::Rename(action) => {
                let renamed = match &action {
                    PresentationAction::Presented(RenameAction::Delegate(
                        RenameDelegate::Renamed { from, to },
                    )) => Some((from.clone(), to.clone())),
                    _ => None,
                };
                // The child first, then what the folder wants of it.
                let effect = if_presented(&self.renaming, &mut folder.rename, action)
                    .map(FolderAction::Rename);
                if let Some((from, to)) = renamed {
                    if let Some(file) = folder.files.iter_mut().find(|file| **file == from) {
                        file.clone_from(&to);
                    }
                    if let Some(preview) = folder.preview.as_mut().filter(|p| p.name == from) {
                        preview.name = to;
                    }
                }
                effect
            }
        }
    }
}

fn folder() -> (TestStore<Browsing>, TestClock) {
    let clock = TestClock::new();
    let state = Folder {
        files: vec!["notes.txt".into(), "plan.md".into()],
        ..Folder::default()
    };
    let reducer = Browsing {
        previewing: Previewing {
            clock: Arc::new(clock.clone()),
        },
        renaming: Renaming {
            clock: Arc::new(clock.clone()),
        },
    };
    (TestStore::new(state, reducer), clock)
}

fn form(name: &str, taken: &str) -> RenameForm {
    RenameForm {
        original: name.into(),
        name: name.into(),
        taken: vec![taken.into()],
        saving: false,
        problem: None,
    }
}

fn rename(action: RenameAction) -> FolderAction {
    FolderAction::Rename(PresentationAction::Presented(action))
}

/// Changes the presented form in an expected state.
fn with_form(folder: &mut Folder, change: impl FnOnce(&mut RenameForm)) {
    if let Some(form) = folder.rename.get_mut() {
        change(form);
    }
}

#[test]
fn selecting_a_file_previews_it_and_a_newer_selection_replaces_the_load() {
    let (mut store, clock) = folder();
    store.send(FolderAction::Select("notes.txt".into()), |s| {
        s.preview = Some(Preview {
            name: "notes.txt".into(),
            text: None,
        });
    });
    store.send(FolderAction::Select("plan.md".into()), |s| {
        s.preview = Some(Preview {
            name: "plan.md".into(),
            text: None,
        });
    });
    clock.advance(Duration::from_millis(500));
    store.receive(
        FolderAction::Preview(PreviewAction::Loaded("first lines of plan.md".into())),
        |s| {
            s.preview = Some(Preview {
                name: "plan.md".into(),
                text: Some("first lines of plan.md".into()),
            });
        },
    );
}

#[test]
fn closing_the_preview_stops_its_load_and_late_actions_are_dropped() {
    let (mut store, clock) = folder();
    store.send(FolderAction::Select("notes.txt".into()), |s| {
        s.preview = Some(Preview {
            name: "notes.txt".into(),
            text: None,
        });
    });
    store.send(FolderAction::ClosePreview, |s| s.preview = None);
    clock.advance(Duration::from_secs(1));
    // One sent by hand finds no preview, and does nothing.
    store.send(FolderAction::Preview(PreviewAction::Loaded("late".into())), |_| {});
    store.finish();
}

#[test]
fn saving_a_rename_renames_the_file_and_closes_the_form() {
    let (mut store, clock) = folder();
    store.send(FolderAction::Select("notes.txt".into()), |s| {
        s.preview = Some(Preview {
            name: "notes.txt".into(),
            text: None,
        });
    });
    clock.advance(Duration::from_millis(500));
    store.receive(
        FolderAction::Preview(PreviewAction::Loaded("first lines of notes.txt".into())),
        |s| {
            if let Some(preview) = s.preview.as_mut() {
                preview.text = Some("first lines of notes.txt".into());
            }
        },
    );
    store.send(FolderAction::RenameTapped("notes.txt".into()), |s| {
        s.rename.present(form("notes.txt", "plan.md"));
    });
    store.send(rename(RenameAction::Typed("diary.txt".into())), |s| {
        with_form(s, |form| form.name = "diary.txt".into());
    });
    store.send(rename(RenameAction::Save), |s| {
        with_form(s, |form| form.saving = true);
    });
    clock.advance(Duration::from_secs(1));
    store.receive(rename(RenameAction::Saved), |s| {
        with_form(s, |form| form.saving = false);
    });
    store.receive(
        rename(RenameAction::Delegate(RenameDelegate::Renamed {
            from: "notes.txt".into(),
            to: "diary.txt".into(),
        })),
        |s| {
            s.files[0] = "diary.txt".into();
            if let Some(preview) = s.preview.as_mut() {
                preview.name = "diary.txt".into();
            }
        },
    );
    store.receive(FolderAction::Rename(PresentationAction::Dismiss), |s| {
        s.rename = PresentationState::none();
    });
    store.finish();
}

#[test]
fn a_name_already_taken_or_empty_is_refused_and_the_form_stays() {
    let (mut store, _clock) = folder();
    store.send(FolderAction::RenameTapped("notes.txt".into()), |s| {
        s.rename.present(form("notes.txt", "plan.md"));
    });
    store.send(rename(RenameAction::Typed("plan.md".into())), |s| {
        with_form(s, |form| form.name = "plan.md".into());
    });
    store.send(rename(RenameAction::Save), |s| {
        with_form(s, |form| form.problem = Some(NameProblem::Taken));
    });
    store.send(rename(RenameAction::Typed(" ".into())), |s| {
        with_form(s, |form| {
            form.name = " ".into();
            form.problem = None;
        });
    });
    store.send(rename(RenameAction::Save), |s| {
        with_form(s, |form| form.problem = Some(NameProblem::Empty));
    });
    assert!(store.state().rename.is_presented());
    store.finish();
}

#[test]
fn cancelling_while_saving_closes_the_form_and_drops_the_save() {
    let (mut store, clock) = folder();
    store.send(FolderAction::RenameTapped("plan.md".into()), |s| {
        s.rename.present(form("plan.md", "notes.txt"));
    });
    store.send(rename(RenameAction::Typed("roadmap.md".into())), |s| {
        with_form(s, |form| form.name = "roadmap.md".into());
    });
    store.send(rename(RenameAction::Save), |s| {
        with_form(s, |form| form.saving = true);
    });
    store.send(rename(RenameAction::Cancel), |_| {});
    store.receive(FolderAction::Rename(PresentationAction::Dismiss), |s| {
        s.rename = PresentationState::none();
    });
    // The save was the form's work, so it went with the form.
    clock.advance(Duration::from_secs(2));
    assert_eq!(store.state().files, ["notes.txt", "plan.md"]);
    store.finish();
}
