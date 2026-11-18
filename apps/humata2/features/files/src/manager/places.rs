//! A row's real path and its menu (PL-10): **Reveal in Finder** on This Mac,
//! **Open terminal here** on another machine, and **Copy path** on both. The
//! path is only ever shown to the person through those choices.

use humata2_clients::model::SessionId;
use humata2_clients::{EntryKind, FileRoot, Folder, Workspace, is_this_mac};
use tca::Effect;

use super::placement::slugify;
use super::{Delegate, Files, FilesAction, FilesState, Side};

/// One choice in a row's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowChoice {
    /// Show it in Finder; only for a place on This Mac.
    RevealInFinder,
    /// Open a terminal tab on its machine, in its folder; only for a place
    /// on another machine.
    OpenTerminalHere,
    /// Copy its absolute path.
    CopyPath,
}

/// The row whose menu is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowMenu {
    /// Its pane.
    pub side: Side,
    /// Its name.
    pub name: String,
}

fn join(base: &str, name: &str) -> String {
    if name.is_empty() {
        base.to_owned()
    } else {
        format!("{}/{name}", base.trim_end_matches('/'))
    }
}

/// The absolute path of `folder` on its machine. A Home folder is under `~`,
/// the way the machine's own shell writes it. `None` where Humata keeps no
/// known place: the machine is unknown, or the session's folder is.
#[must_use]
pub fn folder_path(workspace: &Workspace, folder: &Folder) -> Option<String> {
    let machine_root = || workspace.machine(&folder.machine).map(|machine| machine.root.clone());
    let channel_dir = |channel| {
        let name = &workspace.channel(channel)?.name;
        Some(join(&machine_root()?, &slugify(name)))
    };
    let base = match &folder.root {
        FileRoot::Home => "~".to_owned(),
        FileRoot::ChannelFolder(channel) => channel_dir(channel)?,
        FileRoot::ChannelShared(channel) => join(&channel_dir(channel)?, "shared"),
        FileRoot::SessionClone { session, repo } => workspace
            .session(session)?
            .clones
            .iter()
            .find(|clone| &clone.repo == repo)?
            .path
            .clone(),
        FileRoot::SessionFolder { session } => {
            let known = &workspace.session(session)?.folder;
            if known.is_empty() {
                return None;
            }
            known.clone()
        }
        FileRoot::SessionPastes(_) => return None,
        FileRoot::Received { from } => join(&join(&machine_root()?, "received"), &from.to_string()),
    };
    Some(join(&base, &folder.path))
}

impl FilesState {
    /// The absolute path of the row called `name` in `side`'s pane.
    #[must_use]
    pub fn entry_path(&self, side: Side, name: &str) -> Option<String> {
        let pane = self.pane(side);
        pane.entries.iter().find(|entry| entry.name == name)?;
        let folder = pane.folder.as_ref()?;
        folder_path(&self.workspace, &folder.child(name))
    }

    /// What the row's menu offers, in order. Empty where the row has no known
    /// path. A pane whose machine is offline offers only Copy path.
    #[must_use]
    pub fn row_choices(&self, side: Side, name: &str) -> Vec<RowChoice> {
        if self.entry_path(side, name).is_none() {
            return Vec::new();
        }
        let pane = self.pane(side);
        let on_this_mac = pane
            .folder
            .as_ref()
            .and_then(|folder| self.workspace.machine(&folder.machine))
            .is_some_and(is_this_mac);
        let mut choices = Vec::new();
        if !pane.stale {
            choices.push(if on_this_mac {
                RowChoice::RevealInFinder
            } else {
                RowChoice::OpenTerminalHere
            });
        }
        choices.push(RowChoice::CopyPath);
        choices
    }

    /// The folder a terminal for the row starts in: the row itself when it
    /// is a folder, otherwise the folder it is in.
    fn terminal_folder(&self, side: Side, name: &str) -> Option<String> {
        let pane = self.pane(side);
        let folder = pane.folder.as_ref()?;
        let entry = pane.entries.iter().find(|entry| entry.name == name)?;
        if entry.kind == EntryKind::Folder {
            folder_path(&self.workspace, &folder.child(name))
        } else {
            folder_path(&self.workspace, folder)
        }
    }

    /// The session a new terminal tab on `folder`'s machine belongs to: the
    /// folder's own session, else one of its channel on that machine, else
    /// any session on that machine.
    fn terminal_session(&self, folder: &Folder) -> Option<SessionId> {
        let own = match &folder.root {
            FileRoot::SessionClone { session, .. }
            | FileRoot::SessionFolder { session }
            | FileRoot::SessionPastes(session) => Some(session.clone()),
            _ => None,
        };
        if own.is_some() {
            return own;
        }
        let channel = match &folder.root {
            FileRoot::ChannelFolder(channel) | FileRoot::ChannelShared(channel) => Some(channel),
            _ => None,
        };
        let sessions = &self.workspace.sessions;
        sessions
            .iter()
            .filter(|session| session.machine == folder.machine)
            .find(|session| channel.is_some_and(|channel| &session.channel == channel))
            .or_else(|| sessions.iter().find(|session| session.machine == folder.machine))
            .map(|session| session.id.clone())
    }
}

impl Files {
    pub(super) fn open_row_menu(&self, state: &mut FilesState, side: Side, name: String) -> Effect<FilesAction> {
        state.active = side;
        if !state.pane(side).stale {
            state.select(side, &name, false);
        }
        state.row_menu = (!state.row_choices(side, &name).is_empty()).then_some(RowMenu { side, name });
        Effect::none()
    }

    pub(super) fn copy_path(&self, state: &mut FilesState, side: Side, name: &str) -> Effect<FilesAction> {
        state.row_menu = None;
        let Some(path) = state.entry_path(side, name) else {
            return Effect::none();
        };
        self.clients.clipboard.copy(&path);
        Effect::send(FilesAction::Delegate(Delegate::Toast("Copied the path".to_owned())))
    }

    pub(super) fn reveal_row(&self, state: &mut FilesState, side: Side, name: &str) -> Effect<FilesAction> {
        state.row_menu = None;
        if !state.row_choices(side, name).contains(&RowChoice::RevealInFinder) {
            return Effect::none();
        }
        let Some(path) = state.entry_path(side, name) else {
            return Effect::none();
        };
        let reveal = self.clients.reveal.clone();
        Effect::run(move |send| async move {
            if let Err(error) = reveal.reveal(&path) {
                send.send(FilesAction::RevealFailed(error.to_string()));
            }
        })
    }

    pub(super) fn open_terminal_here(&self, state: &mut FilesState, side: Side, name: &str) -> Effect<FilesAction> {
        state.row_menu = None;
        if !state.row_choices(side, name).contains(&RowChoice::OpenTerminalHere) {
            return Effect::none();
        }
        let (Some(folder), Some(path)) = (state.pane(side).folder.clone(), state.terminal_folder(side, name)) else {
            return Effect::none();
        };
        let Some(session) = state.terminal_session(&folder) else {
            let machine = state.machine_name(&folder.machine);
            state.error = Some(format!(
                "No session runs on {machine} yet, so there is no terminal to open there. Start a session on {machine}, then choose Open terminal here again"
            ));
            return Effect::none();
        };
        Effect::send(FilesAction::Delegate(Delegate::OpenTerminal { session, folder: path }))
    }
}
