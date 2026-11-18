//! Flow 5 step 8 and macro case 02 step 14, on a real daemon: the Fork
//! dialog's other kinds (Code only, Context only), its "Fork from" point
//! (before an earlier turn), and a fork the daemon refuses, which keeps the
//! dialog for another try. The Code + context from Now case is in
//! `lifecycle.rs`.
use std::path::{Path, PathBuf};

use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_clients::model::{ForkKind, Session, SessionId};
use humata2_session::SessionAction;
use tokio::runtime::Runtime;

use super::world::World;
use super::{App, DEADLINE, running_session, runtime};

const FIRST: &str = "write the ledger path";
const SECOND: &str = "add retries to the ledger";
const PARENT: &str = "Write path";

/// A prompt as Claude writes it to its conversation file.
fn prompt(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "isSidechain": false,
        "message": {"role": "user", "content": text},
        "timestamp": "2026-11-15T09:00:00.000Z",
    })
    .to_string()
}

/// A running session "Write path" with two turns in its conversation and
/// an untracked file in its clone.
struct Parent {
    world: World,
    clients: humata2_clients::Clients,
    id: SessionId,
    clone: PathBuf,
}

fn setup(runtime: &Runtime) -> Parent {
    runtime.block_on(async {
        let mut world = World::start().await.expect("start the daemon");
        // The agent declares Claude's conversation format, so the daemon can
        // list turns and cut a conversation at one. The daemon reads
        // agents.json at start, so it starts again before any session exists.
        let root = world
            .conversations()
            .parent()
            .expect("the conversations folder has a parent")
            .to_path_buf();
        let agents_file = root.join("agents.json");
        let mut agents: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&agents_file).expect("read agents.json"),
        )
        .expect("agents.json is JSON");
        agents[0]["conversation_format"] = "claude-jsonl".into();
        agents[0]["version_args"] = serde_json::json!([]);
        std::fs::write(&agents_file, agents.to_string()).expect("write agents.json");
        // Its first message waits for Claude's SessionStart hook (D797).
        world.agent_runs_claude_hooks().expect("write the agent");
        world.reboot().await.expect("start the daemon again");

        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, PARENT, &prompt(FIRST)).await;
        clients
            .terminals
            .write(&id, 0, &prompt(SECOND))
            .await
            .expect("type the second turn");
        // D435: woken by the agent's write, not by a timer.
        let until = tokio::time::Instant::now() + DEADLINE;
        while !world.conversation_text().contains(SECOND) {
            let woke = tokio::time::timeout_at(until, world.wrote()).await;
            assert!(woke.is_ok(), "the agent never got the second turn");
        }
        let clones = session_clones(&world);
        assert_eq!(clones.len(), 1, "the parent has one clone: {clones:?}");
        let clone = clones[0].clone();
        std::fs::write(clone.join("notes.txt"), "untracked\n").expect("write an untracked file");
        Parent {
            world,
            clients,
            id,
            clone,
        }
    })
}

/// Every session clone under the channels folder (a folder under a
/// `sessions` folder that holds the repo's README), sorted.
fn session_clones(world: &World) -> Vec<PathBuf> {
    fn walk(folder: &Path, found: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(folder) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name() == ".git" || !path.is_dir() {
                continue;
            }
            if path.join("README.md").is_file()
                && path.components().any(|part| part.as_os_str() == "sessions")
            {
                found.push(path);
            } else {
                walk(&path, found);
            }
        }
    }
    let mut found = Vec::new();
    walk(&world.channels_folder(), &mut found);
    found.sort();
    found
}

/// How many conversation files hold `text`.
fn conversations_holding(world: &World, text: &str) -> usize {
    std::fs::read_dir(world.conversations())
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
                .filter(|content| content.contains(text))
                .count()
        })
        .unwrap_or(0)
}

fn send(app: &mut App, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

/// Opens the app on the parent's page.
fn open_parent<'r>(runtime: &'r Runtime, parent: &Parent) -> App<'r> {
    let mut app = App::open(runtime, parent.clients.clone());
    let id = parent.id.clone();
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });
    app
}

/// Presses the page's Fork and checks the dialog opened.
fn open_dialog(app: &mut App) {
    app.click(Role::Button, "Fork");
    assert!(
        app.ui.read(|state| state.session.fork.is_some()),
        "Fork opens the dialog"
    );
    // The heading's words aren't in the accessibility tree; the dialog is,
    // and it forks the session the page has open.
    assert!(app.view().contains("Dialog \"Fork session\""), "the fork dialog shows");
    let parent = app.ui.read(|state| {
        state
            .session
            .open
            .as_ref()
            .and_then(|id| state.workspace.session(id))
            .map(|session| session.title.clone())
    });
    assert_eq!(parent.as_deref(), Some(PARENT), "the dialog forks the open session");
}

/// Confirms the dialog. Its Fork button shares its label with the page's, so
/// the driver can't pick one; the confirm is sent as the button sends it.
fn confirm(app: &mut App) {
    send(app, AppAction::Session(SessionAction::ConfirmFork));
}

/// Waits for the app to open a session forked from `parent`, and returns it.
fn wait_for_child(app: &mut App, parent: &SessionId) -> Session {
    app.wait_until("the child session's page to open", |app| {
        // A refused fork keeps the dialog with why: no child is coming.
        let refused = app.ui.read(|state| {
            (!state.session.forking && state.session.fork.is_some())
                .then(|| state.session.error.clone())
                .flatten()
        });
        if let Some(why) = refused {
            panic!("the fork was refused: {why}");
        }
        app.ui.read(|state| match &state.route {
            Route::Session(open) => {
                open != parent
                    && state
                        .workspace
                        .session(open)
                        .is_some_and(|child| child.parent.as_ref() == Some(parent))
            }
            _ => false,
        })
    });
    let child = app.ui.read(|state| match &state.route {
        Route::Session(open) => state.workspace.session(open).cloned(),
        _ => None,
    });
    child.expect("the child is in the app")
}

/// The session as the daemon lists it now.
fn daemon_session(runtime: &Runtime, app: &App, id: &SessionId) -> Session {
    let daemon = app.daemon.clone();
    runtime
        .block_on(daemon.load())
        .expect("the daemon loads")
        .sessions
        .into_iter()
        .find(|session| &session.id == id)
        .expect("the daemon lists the session")
}

/// Flow 5 step 8; macro case 02 step 14 and 16 (Fork again with Code only):
/// Code only hides "Fork from"; Fork starts a child with its own copy of the
/// files, untracked ones included, and a conversation of its own that holds
/// nothing of the parent's.
#[test]
fn a_code_only_fork_copies_the_files_into_a_fresh_conversation() {
    let runtime = runtime();
    let parent = setup(&runtime);
    let mut app = open_parent(&runtime, &parent);

    open_dialog(&mut app);
    assert!(app.view().contains("Fork from"), "Code + context offers Fork from");
    app.click(Role::RadioButton, "fork-kind-2");
    app.wait_until("Code only to be picked", |app| {
        app.ui.read(|state| {
            state
                .session
                .fork
                .as_ref()
                .is_some_and(|fork| fork.kind == ForkKind::Shared)
        })
    });
    assert!(
        !app.view().contains("Fork from"),
        "a fresh agent has no turns to cut at, so Code only has no Fork from"
    );
    confirm(&mut app);

    let child = wait_for_child(&mut app, &parent.id);
    assert_eq!(child.fork, Some(ForkKind::Shared));
    assert!(
        app.ui.read(|state| state.session.fork.is_none()),
        "the dialog closes once the fork starts"
    );
    assert!(app.view().contains(&child.title), "the page shows the child");

    let held = daemon_session(&runtime, &app, &child.id);
    assert_eq!(held.parent.as_ref(), Some(&parent.id));
    assert_eq!(held.fork, Some(ForkKind::Shared));

    let clones = session_clones(&parent.world);
    assert_eq!(clones.len(), 2, "the fork has its own copy: {clones:?}");
    let copy = clones
        .iter()
        .find(|clone| **clone != parent.clone)
        .expect("a second clone");
    assert_eq!(
        std::fs::read_to_string(copy.join("notes.txt")).expect("the untracked file was copied"),
        "untracked\n"
    );
    assert_eq!(
        conversations_holding(&parent.world, SECOND),
        1,
        "only the parent's conversation holds its turns"
    );
    drop(app);
    drop(parent);
}

/// Flow 5 step 8; macro case 02 step 17 (Fork again with Context only): the
/// child knows the conversation and shares the parent's clone and branch, with
/// no copies.
#[test]
fn a_context_only_fork_carries_the_conversation_and_shares_the_parents_files() {
    let runtime = runtime();
    let parent = setup(&runtime);
    let mut app = open_parent(&runtime, &parent);
    let parent_branch = app
        .ui
        .read(|state| state.workspace.session(&parent.id).map(|s| s.branch.clone()))
        .expect("the parent is in the app");

    open_dialog(&mut app);
    app.click(Role::RadioButton, "fork-kind-1");
    app.wait_until("Context only to be picked", |app| {
        app.ui.read(|state| {
            state
                .session
                .fork
                .as_ref()
                .is_some_and(|fork| fork.kind == ForkKind::Conversation)
        })
    });
    confirm(&mut app);

    let child = wait_for_child(&mut app, &parent.id);
    assert_eq!(child.fork, Some(ForkKind::Conversation));
    assert_eq!(child.branch, parent_branch, "it works on the parent's branch");
    assert!(app.view().contains(&child.title), "the page shows the child");

    let held = daemon_session(&runtime, &app, &child.id);
    assert_eq!(held.parent.as_ref(), Some(&parent.id));
    assert_eq!(held.fork, Some(ForkKind::Conversation));
    assert_eq!(held.branch, parent_branch);

    assert_eq!(
        session_clones(&parent.world),
        vec![parent.clone.clone()],
        "no copy of the files was made"
    );
    assert_eq!(
        conversations_holding(&parent.world, FIRST),
        2,
        "the fork holds a copy of the conversation"
    );
    assert_eq!(conversations_holding(&parent.world, SECOND), 2);
    drop(app);
    drop(parent);
}

/// Flow 5 step 8, "Fork from" before an earlier turn (macro case 02 step
/// 14): the child carries the conversation up to that turn and the code as it
/// is now.
#[test]
fn a_fork_from_before_a_turn_carries_the_conversation_up_to_it() {
    let runtime = runtime();
    let parent = setup(&runtime);
    let mut app = open_parent(&runtime, &parent);

    open_dialog(&mut app);
    // The pop-up shows where the fork starts; it is "Now" until another is
    // picked. Opening it asks the daemon for the turns.
    app.click(Role::Button, "Now");
    let turn = format!("Before 2: {SECOND}");
    app.wait_until("the picker to list the turns", |app| {
        app.view().contains(&format!("Button {turn:?}"))
    });
    app.click(Role::Button, &turn);
    app.wait_until("the fork to start before turn 2", |app| {
        app.ui.read(|state| {
            state
                .session
                .fork
                .as_ref()
                .is_some_and(|fork| fork.before_turn == Some(2))
        })
    });
    assert!(app.view().contains("Before turn 2"), "the row names where it starts");
    confirm(&mut app);

    let child = wait_for_child(&mut app, &parent.id);
    assert_eq!(child.fork, Some(ForkKind::Worktree));
    let held = daemon_session(&runtime, &app, &child.id);
    assert_eq!(held.parent.as_ref(), Some(&parent.id));

    assert_eq!(
        conversations_holding(&parent.world, FIRST),
        2,
        "the fork carries the first turn"
    );
    assert_eq!(
        conversations_holding(&parent.world, SECOND),
        1,
        "the fork stops before the second turn"
    );
    let clones = session_clones(&parent.world);
    assert_eq!(clones.len(), 2, "the fork has its own copy: {clones:?}");
    let copy = clones
        .iter()
        .find(|clone| **clone != parent.clone)
        .expect("a second clone");
    assert!(copy.join("notes.txt").is_file(), "the code is as it is now");
    drop(app);
    drop(parent);
}

/// Macro case 02 step 14, a fork the daemon refuses (its status row names
/// `a_failed_fork_keeps_its_title_and_kind_for_retry`): a turn that isn't in
/// the conversation is refused; no child is made; the dialog stays with the
/// title and kind; a second try from Now starts the child.
#[test]
fn a_refused_fork_keeps_the_dialog_and_a_retry_makes_the_child() {
    let runtime = runtime();
    let parent = setup(&runtime);
    let mut app = open_parent(&runtime, &parent);

    open_dialog(&mut app);
    // The dialog's turn list only offers real turns, so a turn that is not
    // there can only be sent as an action; the daemon refuses it.
    send(&mut app, AppAction::Session(SessionAction::ForkTitle("Retry path".into())));
    send(&mut app, AppAction::Session(SessionAction::ForkBeforeTurn(Some(9))));
    confirm(&mut app);
    app.wait_until("the refusal to show", |app| {
        app.ui.read(|state| state.session.error.is_some())
    });
    app.wait_until("the dialog to be ready again", |app| {
        app.ui.read(|state| !state.session.forking)
    });
    let kept = app.ui.read(|state| state.session.fork.clone());
    let kept = kept.expect("the dialog stays open after a refusal");
    assert_eq!(kept.title, "Retry path");
    assert_eq!(kept.kind, ForkKind::Worktree);
    assert_eq!(
        app.ui.read(|state| state.route.clone()),
        Route::Session(parent.id.clone()),
        "the app stays on the parent"
    );
    let daemon = app.daemon.clone();
    let sessions = runtime
        .block_on(daemon.load())
        .expect("the daemon loads")
        .sessions;
    assert_eq!(sessions.len(), 1, "the refused fork made no session");
    assert_eq!(session_clones(&parent.world).len(), 1, "and no copy of the files");

    send(&mut app, AppAction::Session(SessionAction::ForkBeforeTurn(None)));
    confirm(&mut app);
    let child = wait_for_child(&mut app, &parent.id);
    assert_eq!(child.title, "Retry path");
    assert_eq!(child.fork, Some(ForkKind::Worktree));
    assert!(
        app.ui.read(|state| state.session.fork.is_none()),
        "the dialog closes once the fork starts"
    );
    drop(app);
    drop(parent);
}
