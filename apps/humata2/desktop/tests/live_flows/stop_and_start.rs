//! Macro case 09 ("End of day: stop and start") in the app against a real
//! `humatad2`: This Mac's page offers Stop all sessions… under More, the
//! confirm sends nothing until it is confirmed, the confirmed stop is the
//! daemon's, the page then refuses Open a shell, and Start sessions brings
//! the sessions back.
//!
//! What the stop ends, withdraws and keeps across a daemon restart is the
//! daemon's own tests' (`daemon/humatad/tests/stop_sessions.rs`).
use gpui_kit::Role;
use humata2_app::{AppAction, Route};
use humata2_clients::machines::Machines;
use humata2_clients::model::{MachineId, SessionId, Status};
use humata2_hosts::page::MachinePageAction;
use std::sync::Arc;

use super::world::{World, live_clients};
use super::{App, running_session, runtime, session_now};

fn shows(app: &mut App<'_>, role: Role, label: &str) -> bool {
    app.view().contains(&format!("{role:?} {label:?}"))
}

/// Whether the daemon holds a stop for the machine.
fn stopped_on_daemon(app: &App<'_>, machines: &Arc<dyn Machines>, mac: &MachineId) -> bool {
    app.runtime
        .block_on(machines.detail(mac))
        .expect("the machine's page loads")
        .stop
        .is_some()
}

fn status_on_daemon(app: &App<'_>, id: &SessionId) -> Option<Status> {
    let daemon = app.daemon.clone();
    app.runtime
        .block_on(session_now(&daemon, id))
        .map(|session| session.status)
}

/// Steps 1–3, the app side of step 8, and step 11: More then Stop all
/// sessions… opens the confirm and Cancel stops nothing; confirming makes
/// the daemon's stop and the page's "Sessions stopped" banner, and the
/// running agent reads as stopped; Open a shell is then refused with
/// "Start sessions first"; Start sessions asks first, and confirming it
/// clears the stop and the agent no longer reads as stopped.
#[test]
fn this_macs_page_stops_its_sessions_only_when_confirmed_and_starts_them_again() {
    let runtime = runtime();
    let (world, clients, mac, session) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        // This Mac's page and its stop are the daemon's: the real machines
        // client, which `World::clients` keeps for worlds with a devbox.
        let clients = live_clients(world.socket(), true).await.expect("connect");
        let session = running_session(&world, &clients, "Cache warmup", "warm the cache").await;
        let mac = World::this_mac(&clients).await.expect("This Mac");
        (world, clients, mac, session)
    });
    let machines = clients.machines.clone();
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&session).is_some())
    });
    let page = mac.clone();
    app.cx
        .update(|cx| app.ui.send(AppAction::Navigate(Route::Machine(page)), cx));
    app.settle();
    app.wait_until("the machine's page to load", |app| {
        app.ui.read(|state| state.machine_page.detail.is_some())
    });

    // Step 1 and 2: the menu, then the confirm; Cancel sends nothing.
    app.click(Role::Button, "More");
    app.wait_until("the menu to offer the stop", |app| {
        shows(app, Role::Button, "Stop all sessions\u{2026}")
    });
    app.click(Role::Button, "Stop all sessions\u{2026}");
    app.wait_until("the stop's confirm", |app| {
        shows(app, Role::Dialog, "Stop all sessions")
    });
    assert!(!stopped_on_daemon(&app, &machines, &mac), "opening the confirm stops nothing");
    app.click(Role::Button, "Cancel");
    app.wait_until("the confirm to close", |app| {
        !shows(app, Role::Dialog, "Stop all sessions")
    });
    assert!(!stopped_on_daemon(&app, &machines, &mac), "Cancel stops nothing");
    assert_ne!(status_on_daemon(&app, &session), Some(Status::Stopped));

    // Step 3: confirmed, the stop is the daemon's and the page shows it.
    app.click(Role::Button, "More");
    app.wait_until("the menu to offer the stop", |app| {
        shows(app, Role::Button, "Stop all sessions\u{2026}")
    });
    app.click(Role::Button, "Stop all sessions\u{2026}");
    app.wait_until("the stop's confirm", |app| {
        shows(app, Role::Dialog, "Stop all sessions")
    });
    app.click(Role::Button, "Stop all sessions");
    app.wait_until("the page to show the stop", |app| {
        app.ui.read(|state| {
            state
                .machine_page
                .detail
                .as_ref()
                .and_then(|detail| detail.stop.as_ref())
                .is_some_and(|stop| stop.delivered)
        }) && shows(app, Role::Status, "Sessions stopped")
    });
    assert!(!shows(&mut app, Role::Dialog, "Stop all sessions"), "the confirm closes");
    assert!(stopped_on_daemon(&app, &machines, &mac));
    assert_eq!(status_on_daemon(&app, &session), Some(Status::Stopped));

    // Stopped, the menu offers no second stop.
    app.click(Role::Button, "More");
    app.settle();
    assert!(!shows(&mut app, Role::Button, "Stop all sessions\u{2026}"));
    app.cx.update(|cx| {
        app.ui
            .send(AppAction::MachinePage(MachinePageAction::CloseMenu), cx)
    });
    app.settle();

    // Step 8, the app's side: no shell while stopped.
    app.click(Role::Button, "Open a shell");
    app.wait_until("the refusal note", |app| {
        shows(app, Role::Status, "Start sessions first")
    });
    assert!(app.ui.read(|state| state.overlay().is_none()), "no session sheet opens");

    // Step 11: Start sessions asks first, then starts them.
    app.click(Role::Button, "Start sessions");
    app.wait_until("the start's confirm", |app| {
        shows(app, Role::Dialog, "Start sessions")
    });
    assert!(stopped_on_daemon(&app, &machines, &mac), "opening the confirm starts nothing");
    // The sheet's confirm names how many sessions it brings back, so it
    // never reads the same as the banner's Start sessions.
    let count = app.ui.read(|state| {
        state.machine_page.detail.as_ref().map_or(0, |detail| detail.sessions.len())
    });
    let confirm = if count == 1 {
        "Start 1 session".to_owned()
    } else {
        format!("Start {count} sessions")
    };
    app.click(Role::Button, &confirm);
    app.wait_until("the page to drop the stop", |app| {
        app.ui.read(|state| {
            state
                .machine_page
                .detail
                .as_ref()
                .is_some_and(|detail| detail.stop.is_none())
        }) && !shows(app, Role::Status, "Sessions stopped")
    });
    assert!(!stopped_on_daemon(&app, &machines, &mac));
    assert_ne!(status_on_daemon(&app, &session), Some(Status::Stopped));
    drop(app);
    drop(world);
}
