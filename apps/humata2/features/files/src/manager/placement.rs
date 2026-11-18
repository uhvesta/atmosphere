//! Where files sent to a machine land (PL-3, PL-4), what those places are
//! called (PL-5) and the breadcrumbs of names a pane shows (PL-11). Pure:
//! the answers come from the workspace the page already holds.

use humata2_clients::model::{Channel, ChannelId, MachineId, SessionId};
use humata2_clients::{FileRoot, Folder, Workspace};

use super::{FilesState, Side};

/// A machine the Copy to and Move to menus offer, with where a send lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendTarget {
    /// The machine.
    pub machine: MachineId,
    /// Where the files land, by name: `gpu-box · ml shared`.
    pub label: String,
    /// Whether it answers; an offline machine is listed but can't be picked.
    pub online: bool,
}

fn channel_name(workspace: &Workspace, id: &ChannelId) -> String {
    workspace
        .channel(id)
        .map_or_else(|| id.to_string(), |channel| channel.name.clone())
}

fn machine_name(workspace: &Workspace, id: &MachineId) -> String {
    workspace
        .machine(id)
        .map_or_else(|| id.to_string(), |machine| machine.name.clone())
}

fn session_channel<'a>(workspace: &'a Workspace, session: &SessionId) -> Option<&'a Channel> {
    workspace
        .session(session)
        .and_then(|session| workspace.channel(&session.channel))
}

/// The folder name a session's files get under a channel's shared folder:
/// the last part of the session's own folder when it is known, so a renamed
/// session keeps one folder. Otherwise its title in lower case, every run of
/// other characters one `-`; its id when the title has nothing to use.
#[must_use]
pub fn session_slug(workspace: &Workspace, session: &SessionId) -> String {
    if let Some(name) = workspace
        .session(session)
        .and_then(|known| known.folder.trim_end_matches('/').rsplit('/').next())
        .filter(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    let title = workspace.session(session).map_or("", |known| known.title.as_str());
    let slug = slugify(title);
    if slug.is_empty() {
        session.to_string()
    } else {
        slug
    }
}

/// `text` in lower case, every run of other characters one `-`, none at
/// either end. Empty when `text` has nothing to use.
pub(super) fn slugify(text: &str) -> String {
    let mut slug = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Where sending `from`'s files to `target` puts them, with no question
/// asked. A channel's shared folder goes to that channel's shared folder; a
/// session's clone or folder goes to a folder for the session in its
/// channel's shared folder; both only on a machine of that channel. Anything
/// else (Home, a Desk session, pastes, a machine outside the channel) goes
/// to the target's received place for the source machine.
#[must_use]
pub fn landing(from: &Folder, target: &MachineId, workspace: &Workspace) -> Folder {
    let received = || {
        Folder::root(
            target.clone(),
            FileRoot::Received {
                from: from.machine.clone(),
            },
        )
    };
    let in_channel = |channel: &ChannelId| workspace.channel_machines(channel).any(|machine| &machine.id == target);
    match &from.root {
        FileRoot::ChannelShared(channel) if in_channel(channel) => {
            Folder::root(target.clone(), FileRoot::ChannelShared(channel.clone()))
        }
        FileRoot::SessionClone { session, .. } | FileRoot::SessionFolder { session } => {
            match session_channel(workspace, session) {
                Some(channel) if in_channel(&channel.id) => {
                    Folder::root(target.clone(), FileRoot::ChannelShared(channel.id.clone()))
                        .child(&session_slug(workspace, session))
                }
                _ => received(),
            }
        }
        _ => received(),
    }
}

fn session_crumbs(workspace: &Workspace, id: &SessionId) -> Vec<String> {
    match workspace.session(id) {
        Some(session) if humata2_clients::desk::is_desk(session) => vec!["Desk".to_owned(), session.title.clone()],
        Some(session) => vec![
            format!("# {}", channel_name(workspace, &session.channel)),
            session.title.clone(),
        ],
        None => vec![id.to_string()],
    }
}

/// The names from the machine down to the folder, as a pane's header shows
/// them: `This Mac`, `# ml`, `shared`, `datasets`. Home is `Home`.
#[must_use]
pub fn crumbs(folder: &Folder, workspace: &Workspace) -> Vec<String> {
    let mut found = vec![machine_name(workspace, &folder.machine)];
    match &folder.root {
        FileRoot::Home => found.push("Home".to_owned()),
        FileRoot::ChannelFolder(channel) => found.push(format!("# {}", channel_name(workspace, channel))),
        FileRoot::ChannelShared(channel) => {
            found.push(format!("# {}", channel_name(workspace, channel)));
            found.push("shared".to_owned());
        }
        FileRoot::SessionClone { session, repo } => {
            found.extend(session_crumbs(workspace, session));
            found.push(repo.rsplit('/').next().unwrap_or(repo).to_owned());
        }
        FileRoot::SessionPastes(session) => {
            found.extend(session_crumbs(workspace, session));
            found.push("pastes".to_owned());
        }
        FileRoot::SessionFolder { session } => found.extend(session_crumbs(workspace, session)),
        FileRoot::Received { from } => found.push(format!("From {}", machine_name(workspace, from))),
    }
    found.extend(folder.path.split('/').filter(|part| !part.is_empty()).map(str::to_owned));
    found
}

/// A landing place by name for the Copy to menu: `gpu-box · ml shared`,
/// `laptop-2 · From This Mac`.
#[must_use]
pub fn landing_label(landing: &Folder, workspace: &Workspace) -> String {
    let place = match &landing.root {
        FileRoot::ChannelShared(channel) => {
            let mut place = format!("{} shared", channel_name(workspace, channel));
            for part in landing.path.split('/').filter(|part| !part.is_empty()) {
                place.push_str(" \u{203a} ");
                place.push_str(part);
            }
            place
        }
        _ => crumbs(landing, workspace)
            .into_iter()
            .skip(1)
            .collect::<Vec<_>>()
            .join(" \u{203a} "),
    };
    format!("{} \u{b7} {place}", machine_name(workspace, &landing.machine))
}

/// The channel a folder belongs to, if it is in one.
fn channel_of(folder: &Folder, workspace: &Workspace) -> Option<ChannelId> {
    match &folder.root {
        FileRoot::ChannelFolder(channel) | FileRoot::ChannelShared(channel) => Some(channel.clone()),
        FileRoot::SessionClone { session, .. }
        | FileRoot::SessionPastes(session)
        | FileRoot::SessionFolder { session } => session_channel(workspace, session).map(|channel| channel.id.clone()),
        FileRoot::Home | FileRoot::Received { .. } => None,
    }
}

impl FilesState {
    /// The names from the machine down to `side`'s folder; empty when the
    /// pane has no folder.
    #[must_use]
    pub fn breadcrumb(&self, side: Side) -> Vec<String> {
        self.pane(side)
            .folder
            .as_ref()
            .map_or_else(Vec::new, |folder| crumbs(folder, &self.workspace))
    }

    /// What Copy to and Move to offer for the active pane: the channel's
    /// machines first, then the others, each with where the files land. A
    /// machine is left out where the files would only come back to the place
    /// they are in.
    #[must_use]
    pub fn send_targets(&self) -> Vec<SendTarget> {
        let Some(from) = self.active_pane().folder.as_ref() else {
            return Vec::new();
        };
        let workspace: &Workspace = &self.workspace;
        let first: Vec<MachineId> = channel_of(from, workspace)
            .map(|channel| {
                workspace
                    .channel_machines(&channel)
                    .map(|machine| machine.id.clone())
                    .collect()
            })
            .unwrap_or_default();
        let (near, far): (Vec<_>, Vec<_>) = workspace
            .machines
            .iter()
            .partition(|machine| first.contains(&machine.id));
        near.into_iter()
            .chain(far)
            .filter_map(|machine| {
                let to = landing(from, &machine.id, workspace);
                let back_home = matches!(&to.root, FileRoot::Received { from: source } if source == &machine.id);
                if back_home || &to == from {
                    return None;
                }
                Some(SendTarget {
                    machine: machine.id.clone(),
                    label: landing_label(&to, workspace),
                    online: machine.online,
                })
            })
            .collect()
    }
}
