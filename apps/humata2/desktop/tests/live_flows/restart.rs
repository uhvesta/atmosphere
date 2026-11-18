//! SPEC 26.8 and PER-15/PER-29 (frames 46 and 47) in the app against a real
//! `humatad2`: after a reboot the app says what came back, Home groups the
//! session that couldn't resume apart from the one that came back, and
//! the Couldn't resume card's Try again and Start fresh work.
//!
//! The daemon is left to finish bringing its sessions back before the app
//! opens, as when humatad starts at login and Humata is opened later: the
//! restart is already known when the app first loads.

use gpui_kit::Role;
use humata2_app::{AppAction, Route, restart_groups};
use humata2_clients::model::{Agent, Session, SessionId, SessionOptions, Status};
use humata2_clients::{Clients, Event};

use super::world::World;
use super::{App, DEADLINE, running_session, runtime};

/// Starts a second session in the channel the first one made, and waits
/// until its agent has its first message.
async fn another_session(world: &World, clients: &Clients, title: &str, first: &str) -> SessionId {
    let workspace = clients.daemon.load().await.expect("load");
    let made = workspace
        .channels
        .iter()
        .find(|channel| channel.name == "payments")
        .expect("the first session made the payments channel");
    let channel = made.id.clone();
    let repo = made.repos.first().cloned().expect("the channel has its repo");
    let mac = World::this_mac(clients).await.expect("This Mac");
    let id = clients
        .daemon
        .create_session(
            &channel,
            Agent::Claude,
            &mac,
            title,
            SessionOptions {
                repos: vec![repo],
                first_message: first.to_owned(),
                ..SessionOptions::default()
            },
        )
        .await
        .expect("create the second session");
    // D435: woken by the agent's write, not by a timer.
    let until = tokio::time::Instant::now() + DEADLINE;
    while !world.conversation_text().contains(first) {
        let woke = tokio::time::timeout_at(until, world.wrote()).await;
        assert!(woke.is_ok(), "the second agent never got its first message");
    }
    id
}

/// The conversation file holding `line`, with what it held.
fn conversation_with(world: &World, line: &str) -> (std::path::PathBuf, String) {
    std::fs::read_dir(world.conversations())
        .expect("read conversations")
        .flatten()
        .find_map(|entry| {
            let text = std::fs::read_to_string(entry.path()).ok()?;
            text.contains(line).then(|| (entry.path(), text))
        })
        .expect("a conversation holds the line")
}

/// Reboots, then waits until the daemon has brought its sessions back and
/// says so, before any app is open.
fn reboot_and_settle(runtime: &tokio::runtime::Runtime, world: &mut World) -> Clients {
    runtime.block_on(async {
        world.reboot().await.expect("reboot");
        let clients = world.clients().await.expect("connect again");
        // D435: follow first, then look once, so a report made in between
        // is still heard.
        let mut events = clients.daemon.events().await.expect("follow the daemon");
        let known = clients
            .daemon
            .load()
            .await
            .is_ok_and(|workspace| !workspace.restarts.is_empty());
        if !known {
            let told = tokio::time::timeout(DEADLINE, async {
                while let Some(event) = events.recv().await {
                    if let Ok(Event::Restart(_)) = event {
                        return true;
                    }
                }
                false
            })
            .await;
            assert!(matches!(told, Ok(true)), "the daemon never reported its restart");
        }
        clients
    })
}

fn failed(session: Option<Session>) -> bool {
    session.is_some_and(|session| session.resume_failed.is_some())
}

/// Flow 7 steps 3–5: the toast, Home's two groups, then Try again once the
/// lost conversation is back.
#[test]
fn after_a_reboot_the_app_says_what_came_back_and_try_again_resumes_the_one_that_couldnt() {
    let runtime = runtime();
    let (mut world, kept, lost) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let kept = running_session(&world, &clients, "Read path", "kept-line").await;
        let lost = another_session(&world, &clients, "Old spike", "lost-line").await;
        (world, kept, lost)
    });
    let (file, text) = conversation_with(&world, "lost-line");
    std::fs::remove_file(&file).expect("lose the second conversation");

    let clients = reboot_and_settle(&runtime, &mut world);
    let mut app = App::open(&runtime, clients);

    // Step 3: the restart is told, though it was known before the app opened.
    app.wait_until("the restart to be told", |app| {
        app.ui.read(|state| !state.toasts.is_empty() && !state.workspace.restarts.is_empty())
    });
    let resumed = app.ui.read(|state| state.workspace.restarts[0].resumed.clone());
    assert!(resumed.contains(&kept), "the kept session came back: {resumed:?}");

    // Step 4: Home groups the one that couldn't resume, and only that one.
    // (The test agent reports no turn end, so the Idle group isn't checked
    // here; features/app tests cover the split.)
    app.cx
        .update(|cx| app.ui.send(AppAction::Navigate(Route::Home), cx));
    app.settle();
    app.wait_until("Home to group the session that couldn't resume", |app| {
        app.ui.read(|state| {
            restart_groups(&state.workspace)
                .couldnt_resume
                .iter()
                .any(|session| session.id == lost)
        })
    });
    app.ui.read(|state| {
        let groups = restart_groups(&state.workspace);
        assert!(groups.couldnt_resume.iter().all(|session| session.id != kept));
    });

    // Step 5: what it needed is back, so Try again resumes it in its
    // conversation.
    std::fs::write(&file, text).expect("bring the conversation back");
    app.show_session(&lost);
    app.wait_until("the Couldn't resume card", |app| {
        failed(app.session_titled("Old spike"))
    });
    app.click(Role::Button, "Try again");
    app.wait_until("the session to resume", |app| {
        app.session_titled("Old spike").is_some_and(|session| {
            session.resume_failed.is_none() && !matches!(session.status, Status::Exited { .. })
        })
    });
    app.wait_until("the agent resumed in its conversation", |app| {
        app.view().contains("agent up --resume")
    });
    let after = std::fs::read_to_string(&file).unwrap_or_default();
    assert!(after.contains("lost-line"), "the same conversation was resumed");
    drop(app);
    drop(world);
}

/// Flow 7 step 5, the other button: Start fresh starts a new conversation
/// that is told what the old one printed.
#[test]
fn start_fresh_on_the_couldnt_resume_card_starts_a_new_conversation_that_hears_the_summary() {
    let runtime = runtime();
    let (mut world, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Old spike", "remember-the-blue-door").await;
        (world, id)
    });
    world.lose_conversations().expect("delete conversations");
    let clients = reboot_and_settle(&runtime, &mut world);
    let mut app = App::open(&runtime, clients);
    app.show_session(&id);
    app.wait_until("the Couldn't resume card", |app| {
        failed(app.session_titled("Old spike"))
    });
    // The terminal's saved output is left, so the card offers a summary.
    app.click(Role::Button, "Start fresh with a summary");
    app.wait_until("the session to start fresh", |app| {
        app.session_titled("Old spike").is_some_and(|session| {
            session.resume_failed.is_none() && !matches!(session.status, Status::Exited { .. })
        })
    });
    // D435: woken by the agent's write, not by a timer.
    let until = tokio::time::Instant::now() + DEADLINE;
    while !world.conversation_text().contains("got: remember-the-blue-door") {
        let woke = runtime.block_on(async { tokio::time::timeout_at(until, world.wrote()).await });
        assert!(
            woke.is_ok(),
            "the new conversation never heard what the old terminal printed"
        );
    }
    drop(app);
    drop(world);
}

/// Flow 7 step 5: opening a session that couldn't resume shows its card and
/// no error. Opening it marks it read on the daemon, which keeps the card
/// (D392).
#[test]
fn opening_a_session_that_couldnt_resume_shows_no_error_and_keeps_its_card() {
    let runtime = runtime();
    let (mut world, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Old spike", "before-the-reboot").await;
        (world, id)
    });
    world.lose_conversations().expect("delete conversations");
    let clients = reboot_and_settle(&runtime, &mut world);
    let mut app = App::open(&runtime, clients);
    // Known as needing recovery before it is opened, so the open sends the
    // read mark.
    app.wait_until("the session to need recovery", |app| {
        failed(app.session_titled("Old spike"))
    });
    app.show_session(&id);
    app.settle();
    app.ui.read(|state| {
        assert_eq!(state.session.error, None, "opening it reports no error");
    });
    assert!(
        failed(app.session_titled("Old spike")),
        "the card stays until it is resumed or started fresh"
    );
    drop(app);
    drop(world);
}

/// Flow 7 step 4, the Idle group: an agent that reports its turn through
/// its hooks (`humata turn`) comes back from the reboot, says it is idle,
/// and Home lists it under "Idle after the restart", while the session that
/// couldn't resume stays in its own group and is not listed as idle.
#[test]
fn after_a_reboot_an_agent_that_reports_idle_is_listed_under_idle_after_the_restart() {
    let runtime = runtime();
    let (mut world, kept, lost) = runtime.block_on(async {
        let world = World::start_with_turn_hooks().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let kept = running_session(&world, &clients, "Read path", "kept-line").await;
        let lost = another_session(&world, &clients, "Old spike", "lost-line").await;
        (world, kept, lost)
    });
    let (file, _) = conversation_with(&world, "lost-line");
    std::fs::remove_file(&file).expect("lose the second conversation");

    let clients = reboot_and_settle(&runtime, &mut world);
    let mut app = App::open(&runtime, clients);
    app.cx
        .update(|cx| app.ui.send(AppAction::Navigate(Route::Home), cx));
    app.settle();

    // The resumed agent's hook reports idle; the daemon pushes the status
    // and the app regroups.
    app.wait_until("Home to list the resumed agent as idle after the restart", |app| {
        app.ui.read(|state| {
            restart_groups(&state.workspace)
                .idle
                .iter()
                .any(|session| session.id == kept)
        })
    });
    app.ui.read(|state| {
        let groups = restart_groups(&state.workspace);
        assert!(
            groups.idle.iter().all(|session| session.id != lost),
            "a session that couldn't resume is not idle after the restart"
        );
        assert!(groups.couldnt_resume.iter().any(|session| session.id == lost));
        assert!(groups.couldnt_resume.iter().all(|session| session.id != kept));
    });
    drop(app);
    drop(world);
}
