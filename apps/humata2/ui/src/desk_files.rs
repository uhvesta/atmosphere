//! A Desk session's Files panel (DK-3, frame 38): what is in the session's
//! folder. Files and folders show as the daemon reports them; each repo
//! added to the session is a folder with the repo's name and its branch
//! beside it; under the list is what adding and removing a repo does.
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::layout::FullScreen;
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Session, SessionFile};
use humata2_session::{Panel, SessionAction};
use std::rc::Rc;

/// What adding a repo to the session does, and what removing one doesn't.
const NOTE: &str =
    "Repos you add are cloned into this folder. Removing one deletes its clone, not your branch.";

/// Text a flow test and a snapshot can read.
fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}

/// The frames' quiet text.
fn dim(look: Look, value: impl Into<SharedString>) -> Div {
    text(value)
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
}

/// The folder a repo is cloned into: its name without the owner.
fn clone_folder(repo: &str) -> &str {
    repo.rsplit('/').next().unwrap_or(repo)
}

/// One entry of the session's folder, with what is inside it.
struct Node {
    name: String,
    folder: bool,
    /// What the session's file actions call it: the clone ("" for the
    /// folder itself) and the path inside it.
    key: (String, String),
    /// For a repo's clone: "owner/repo · branch".
    repo_note: Option<String>,
    /// In the order the daemon reported them.
    children: Vec<Node>,
}

/// The entries from the folder down to `file`: each one's name and key.
fn steps(file: &SessionFile) -> Vec<(String, (String, String))> {
    let mut steps = Vec::new();
    // A file reported inside a clone sits under that clone's folder.
    if !file.repo.is_empty() {
        steps.push((
            clone_folder(&file.repo).to_owned(),
            (file.repo.clone(), String::new()),
        ));
    }
    let mut path = String::new();
    for part in file.path.split('/').filter(|part| !part.is_empty()) {
        if !path.is_empty() {
            path.push('/');
        }
        path.push_str(part);
        steps.push((part.to_owned(), (file.repo.clone(), path.clone())));
    }
    steps
}

/// Puts the entry at the end of `steps` into the tree, making the folders
/// above it that are not there yet.
fn insert(nodes: &mut Vec<Node>, steps: &[(String, (String, String))], folder: bool) {
    let Some(((name, key), rest)) = steps.split_first() else {
        return;
    };
    let at = nodes
        .iter()
        .position(|node| node.name == *name)
        .unwrap_or_else(|| {
            nodes.push(Node {
                name: name.clone(),
                folder: false,
                key: key.clone(),
                repo_note: None,
                children: Vec::new(),
            });
            nodes.len() - 1
        });
    let Some(node) = nodes.get_mut(at) else {
        return;
    };
    if rest.is_empty() {
        node.folder |= folder;
    } else {
        node.folder = true;
        insert(&mut node.children, rest, folder);
    }
}

/// The session's folder as a tree: what the daemon reported, in its order,
/// and a folder for every repo of the session that it did not report. A
/// repo's clone is a folder of the session's folder itself, and carries
/// its note.
fn tree(state: &AppState, session: &Session, repos: &[String]) -> Vec<Node> {
    let mut nodes = Vec::new();
    for file in state
        .workspace
        .session_files
        .get(&session.id)
        .into_iter()
        .flatten()
    {
        insert(&mut nodes, &steps(file), file.folder);
    }
    for repo in repos {
        insert(
            &mut nodes,
            &[(
                clone_folder(repo).to_owned(),
                (repo.clone(), String::new()),
            )],
            true,
        );
    }
    for node in &mut nodes {
        if node.folder
            && let Some(repo) = repos
                .iter()
                .find(|repo| clone_folder(repo.as_str()) == node.name)
        {
            node.repo_note = Some(clone_note(state, session, repo));
        }
    }
    nodes
}

/// "owner/repo · branch" for a clone, or the repo alone while the daemon
/// has not said which branch the clone is on. A Desk session has no branch
/// of its own (DK-3), so a clone sits on its repository's default branch.
fn clone_note(state: &AppState, session: &Session, repo: &str) -> String {
    let branch = state
        .workspace
        .desk
        .clone_branches
        .get(&(session.id.clone(), repo.to_owned()))
        .map(String::as_str)
        .filter(|branch| !branch.is_empty());
    match branch {
        Some(branch) => format!("{repo} · {branch}"),
        None => repo.to_owned(),
    }
}

/// The rows of `nodes` and of everything open under them. `parent` is the
/// path of the folder they are in, empty for the session's folder.
fn rows(
    ui: &Rc<Ui>,
    state: &AppState,
    nodes: &[Node],
    parent: &str,
    depth: usize,
    look: Look,
    out: &mut Vec<AnyElement>,
) {
    let view = state.session.view();
    for node in nodes {
        let path = if parent.is_empty() {
            node.name.clone()
        } else {
            format!("{parent}/{}", node.name)
        };
        let note = node.repo_note.clone();
        let folded = view.is_some_and(|view| view.folded_files.contains(&node.key));
        let selected = !node.folder
            && view
                .and_then(|view| view.file.as_ref())
                .is_some_and(|file| *file == node.key);
        let glyph = if note.is_some() {
            IconName::FolderGit2
        } else if !node.folder {
            IconName::FileText
        } else if folded || node.children.is_empty() {
            IconName::Folder
        } else {
            IconName::FolderOpen
        };
        let (repo, inside) = node.key.clone();
        // A folder opens and closes. A file is selected and opened in a
        // tab, as Open big does in a channel session's Files panel.
        let events = if node.folder {
            vec![SessionAction::ToggleFolder { repo, path: inside }]
        } else {
            vec![
                SessionAction::SelectFile { repo, path: inside },
                SessionAction::OpenFile,
            ]
        };
        let label = match &note {
            Some(note) => format!("{path}, {note}"),
            None => path.clone(),
        };
        let click_ui = ui.clone();
        out.push(
            row()
                .id(SharedString::from(format!("desk-files-entry-{path}")))
                .role(if node.folder {
                    Role::Button
                } else {
                    Role::ListItem
                })
                .aria_label(label)
                .test_support()
                .h(px(23.))
                .pl(px(8. + depth as f32 * 14.))
                .pr(px(8.))
                .gap(px(6.))
                .rounded(px(5.))
                .when(selected, |row| row.bg(rgb(look.fill)))
                .child(chrome::icon(glyph, look.secondary, 12.))
                .child(text(node.name.clone()).flex_1())
                .children(note.map(|note| dim(look, note)))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    for event in &events {
                        click_ui.send(AppAction::Session(event.clone()), cx);
                    }
                })
                .into_any_element(),
        );
        if node.folder && !folded {
            rows(ui, state, &node.children, &path, depth + 1, look, out);
        }
    }
}

/// The Files panel of `session`, or `None` when it isn't a Desk session:
/// the header with the machine the folder is on, the folder's entries, and
/// the note about repos under them.
pub(crate) fn panel(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Option<AnyElement> {
    if !humata2_clients::desk::is_desk(session) {
        return None;
    }
    let full = state.full_screen() == Some(FullScreen::Panel);
    let machine = state.workspace.machine(&session.machine).map_or_else(
        || session.machine.to_string(),
        |machine| machine.name.clone(),
    );
    let repos = state
        .desk
        .record(&session.id)
        .map(|record| record.repos.as_slice())
        .unwrap_or_default();
    let nodes = tree(state, session, repos);
    let mut entries = Vec::new();
    rows(ui, state, &nodes, "", 0, look, &mut entries);
    Some(
        col()
            .size_full()
            .child(crate::panels::panel_header(
                ui,
                state,
                Panel::Files,
                full,
                Some(dim(look, machine)),
                look,
            ))
            .child(
                col()
                    .id("desk-files")
                    .role(Role::List)
                    .aria_label("Session folder")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(6.))
                    .text_size(px(12.5))
                    .when(entries.is_empty(), |list| {
                        list.child(
                            div()
                                .px(px(8.))
                                .py(px(4.))
                                .text_color(rgb(look.tertiary))
                                .child(text("This folder is empty.")),
                        )
                    })
                    .children(entries),
            )
            .child(
                div()
                    .p(px(12.))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(look.line))
                    .child(dim(look, NOTE)),
            )
            .into_any_element(),
    )
}
