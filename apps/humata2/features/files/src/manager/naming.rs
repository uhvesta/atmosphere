//! The name prompt behind New folder and Rename (FM-2): the name is checked
//! before any client call, and a bad one keeps the prompt open.

use tca::Effect;

use super::{Files, FilesAction, FilesState};

/// What the prompt is asking a name for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamingKind {
    /// A new folder in the active pane.
    NewFolder,
    /// The one selected name in the active pane.
    Rename,
}

/// An open name prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Naming {
    /// What it is for.
    pub kind: NamingKind,
    /// What the field starts with: empty, or the name being renamed.
    pub initial: String,
    /// Why the last name was refused, as a sentence; the prompt stays open.
    pub error: Option<String>,
}

/// Why `text` can't be a name, if it can't.
fn refusal(kind: NamingKind, initial: &str, text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return Some("The name is empty. Type a name, or cancel.".to_owned());
    }
    if text.contains('/') {
        return Some("A name can't contain \"/\". Leave out the slash; to put something in a folder, open the folder first.".to_owned());
    }
    if text == "." || text == ".." {
        return Some("\".\" and \"..\" are not names. Pick a name that is not made only of dots.".to_owned());
    }
    if kind == NamingKind::Rename && text == initial {
        return Some("That is the name it already has. Type a different name, or cancel.".to_owned());
    }
    None
}

impl FilesState {
    /// Opens the prompt; refused on a stale pane, with no folder, or for a
    /// rename without exactly one selected name.
    pub(super) fn begin_naming(&mut self, kind: NamingKind) {
        if self.usable(self.active).is_none() {
            return;
        }
        let initial = match kind {
            NamingKind::NewFolder => String::new(),
            NamingKind::Rename => match self.active_pane().selected.as_slice() {
                [only] => only.clone(),
                _ => return,
            },
        };
        self.picker = None;
        self.naming = Some(Naming {
            kind,
            initial,
            error: None,
        });
    }
}

impl Files {
    pub(super) fn commit_name(&self, state: &mut FilesState, text: String) -> Effect<FilesAction> {
        let Some(naming) = state.naming.as_mut() else {
            return Effect::none();
        };
        if let Some(error) = refusal(naming.kind, &naming.initial, &text) {
            naming.error = Some(error);
            return Effect::none();
        }
        let kind = naming.kind;
        state.naming = None;
        match kind {
            NamingKind::NewFolder => self.new_folder(state, text),
            NamingKind::Rename => self.rename(state, text),
        }
    }
}
