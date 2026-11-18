//! CV-8 on a real daemon: a Claude session in # payments goes on in
//! # billing from before its turn 2, through the session menu's "Continue in
//! another channel…". The new session is Billing's, its agent's copy of the
//! conversation stops before turn 2, and the Payments session is as it was.
use gpui_kit::Role;
use humata2_app::Route;
use humata2_clients::model::Session;

use super::world::World;
use super::{App, DEADLINE, running_session, runtime, session_now};

const FIRST: &str = "write the ledger path";
const SECOND: &str = "add retries to the ledger";
const TITLE: &str = "Write path";

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

#[test]
fn a_claude_session_continues_in_another_channel_from_before_turn_2() {
    let runtime = runtime();
    let (world, clients, source, billing) = runtime.block_on(async {
        let mut world = World::start().await.expect("start the daemon");
        // The agent declares Claude's conversation format, so the daemon can
        // list turns and carry a conversation cut at one. The daemon reads
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
        world.reboot().await.expect("start the daemon again");

        let clients = world.clients().await.expect("connect");
        // Channel A, payments, and its running session with two turns.
        let source = running_session(&world, &clients, TITLE, &prompt(FIRST)).await;
        clients
            .terminals
            .write(&source, 0, &prompt(SECOND))
            .await
            .expect("type the second turn");
        // Woken by the agent's write, not by a timer.
        let until = tokio::time::Instant::now() + DEADLINE;
        while !world.conversation_text().contains(SECOND) {
            let woke = tokio::time::timeout_at(until, world.wrote()).await;
            assert!(woke.is_ok(), "the agent never got the second turn");
        }
        // Channel B, billing, on a repository of its own.
        let mac = World::this_mac(&clients)
            .await
            .expect("the workspace has This Mac");
        let web = world.repo("owner", "web").expect("make billing's repo");
        let billing = clients
            .daemon
            .create_channel("billing", vec![web], vec![mac])
            .await
            .expect("create billing");
        (world, clients, source, billing)
    });
    let before: Session = runtime
        .block_on(session_now(&clients.daemon, &source))
        .expect("the daemon lists the payments session");

    let mut app = App::open(&runtime, clients.clone());
    app.wait_until("the session and billing to load", |app| {
        app.ui.read(|state| {
            state.workspace.session(&source).is_some() && state.workspace.channel(&billing).is_some()
        })
    });
    app.show_session(&source);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(source.clone()))
    });

    app.click(Role::Button, "Session actions");
    app.click(Role::Button, "Continue in another channel…");
    assert!(
        app.ui.read(|state| state.session.continuing.is_some()),
        "the menu item opens the dialog"
    );
    app.click(Role::RadioButton, &format!("continue-channel-{billing}"));
    app.wait_until("billing to be picked", |app| {
        app.ui.read(|state| {
            state
                .session
                .continuing
                .as_ref()
                .is_some_and(|draft| draft.channel.as_ref() == Some(&billing))
        })
    });
    // "Continue from" shows Now until another point is picked; opening it
    // asks the daemon for the turns.
    app.click(Role::Button, "Now");
    let turn = format!("Before 2: {SECOND}");
    app.wait_until("the picker to list the turns", |app| {
        app.view().contains(&format!("Button {turn:?}"))
    });
    app.click(Role::Button, &turn);
    app.wait_until("before turn 2 to be picked", |app| {
        app.ui.read(|state| {
            state
                .session
                .continuing
                .as_ref()
                .is_some_and(|draft| draft.before_turn == Some(2))
        })
    });
    app.click(Role::Button, "Continue in # billing");

    app.wait_until("the new session's page to open in billing", |app| {
        app.ui.read(|state| match &state.route {
            Route::Session(open) => {
                open != &source
                    && state
                        .workspace
                        .session(open)
                        .is_some_and(|session| session.channel == billing)
            }
            _ => false,
        })
    });
    let new = app
        .ui
        .read(|state| match &state.route {
            Route::Session(open) => state.workspace.session(open).cloned(),
            _ => None,
        })
        .expect("the new session is in the app");
    assert!(
        app.ui.read(|state| state.session.continuing.is_none()),
        "the dialog closes once the session starts"
    );
    assert!(new.parent.is_none(), "it is a top-level session");
    assert!(app.view().contains(&new.title), "the page shows the new session");

    let held = runtime
        .block_on(session_now(&clients.daemon, &new.id))
        .expect("the daemon lists the new session");
    assert_eq!(held.channel, billing);
    assert!(held.parent.is_none());
    assert_eq!(
        conversations_holding(&world, FIRST),
        2,
        "the new session's agent has a copy of the conversation"
    );
    assert_eq!(
        conversations_holding(&world, SECOND),
        1,
        "the copy stops before turn 2"
    );

    let after = runtime
        .block_on(session_now(&clients.daemon, &source))
        .expect("the payments session is still there");
    assert_eq!(after.channel, before.channel, "it stays in payments");
    assert_eq!(after.title, before.title);
    assert_eq!(after.branch, before.branch);
    assert_eq!(after.parent, before.parent);
    assert!(!after.archived);
    assert!(
        app.ui
            .read(|state| state.workspace.session(&source).is_some_and(|s| s.channel == before.channel)),
        "the app still shows it in payments"
    );
    drop(app);
    drop(world);
}
