//! Flow 05 step 6 (the queue) on a real `humatad2`: messages typed while the
//! agent is in a turn wait in the app, can be shown and removed, and the
//! one the person sends reaches the real agent.
//!
//! The rig's agent has no hook, so the daemon judges its turn from its
//! output: Working while it prints, Probably idle once it has been quiet,
//! and never "turn ended". That is a turn that is not over, so nothing the
//! person queues is sent on its own, with no timing in the test: the queue
//! holds until Send now.
//!
//! The app has no composer control; the queue is fed by the session's
//! draft and submit actions (the same ones the native fake test sends), so
//! those two go through `AppAction`. Show, Remove and Send now are clicked.
use gpui_kit::SharedString;
use gpui_kit::test::TestWindowExt as _;
use humata2_app::AppAction;
use humata2_clients::model::{SessionId, Status};
use humata2_session::SessionAction;

use super::world::World;
use super::{App, running_session, runtime};

/// Types `text` in the composer and submits it, as the native queue test does.
fn queue_message(app: &mut App, text: &str) {
    app.cx.update(|cx| {
        app.ui
            .send(AppAction::Session(SessionAction::Draft(text.to_owned())), cx);
        app.ui.send(AppAction::Session(SessionAction::Submit), cx);
    });
    app.settle();
}

/// The messages waiting to send in the open session, in order.
fn queued(app: &App) -> Vec<String> {
    app.ui.read(|state| {
        state
            .session
            .view()
            .map(|view| view.queue.iter().map(|message| message.text()).collect())
            .unwrap_or_default()
    })
}

/// Whether the window draws an element with this id.
fn shows(app: &mut App, id: &str) -> bool {
    app.settle();
    app.cx
        .update_window(app.window, |_, window, _| {
            window.try_find(SharedString::from(id.to_owned())).is_some()
        })
        .expect("the window stays open")
}

/// Clicks the control with this element id. Several queued messages each
/// have a Show/Remove/Send now button of the same name, so the driver, which
/// finds controls by role and label, can't pick one; the id can.
fn click_id(app: &mut App, id: &str) {
    app.settle();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.click(SharedString::from(id.to_owned()), cx)
        })
        .expect("the control is drawn");
    app.settle();
}

fn status_of(app: &App, id: &SessionId) -> Option<Status> {
    app.ui
        .read(|state| state.workspace.session(id).map(|session| session.status))
}

/// Flow 05 step 6: two messages typed mid-turn wait, are shown, one is
/// removed, and the other, sent by hand, reaches the real agent. The removed
/// one never does.
#[test]
fn messages_typed_mid_turn_wait_can_be_shown_and_removed_and_the_rest_reaches_the_agent() {
    const KEPT: &str = "Then check the read path before dropping the old index.";
    const REMOVED: &str = "Keep both indexes until the backfill ends.";
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "start the work").await;
        (world, clients, id)
    });
    let mut app = App::open(&runtime, clients);
    app.wait_until("the session to load", |app| {
        app.ui.read(|state| state.workspace.session(&id).is_some())
    });
    app.show_session(&id);
    app.wait_until("the agent's first answer", |app| {
        app.view().contains("got: start the work")
    });

    // Typed while the agent is not at a turn's end: both wait.
    queue_message(&mut app, REMOVED);
    queue_message(&mut app, KEPT);
    assert_eq!(queued(&app), [REMOVED, KEPT]);
    assert!(shows(&mut app, "session-queue"), "the queue bar shows");
    assert!(
        !shows(&mut app, "queued-message-0"),
        "the messages are folded until Show"
    );

    // Even once the agent has gone quiet (its turn end is only a guess),
    // nothing is sent on its own.
    app.wait_until("the agent to go quiet", |app| {
        status_of(app, &id) == Some(Status::ProbablyIdle)
    });
    assert_eq!(queued(&app), [REMOVED, KEPT], "still waiting");
    assert!(
        !world.conversation_text().contains(REMOVED) && !world.conversation_text().contains(KEPT),
        "the agent has had neither"
    );

    // Show lists them; Remove drops the first.
    // The queue bar's Show (the page has other buttons of that name).
    click_id(&mut app, "show-queued");
    assert!(shows(&mut app, "queued-message-1"));
    click_id(&mut app, "remove-queued-0");
    assert_eq!(queued(&app), [KEPT]);

    // Send now types the rest into the real agent, which answers it.
    click_id(&mut app, "send-queued-0");
    app.wait_until("the agent to get the message", |_| {
        world.conversation_text().contains(KEPT)
    });
    app.wait_until("the agent's answer on screen", |app| {
        app.view().contains(&format!("got: {KEPT}"))
    });
    app.wait_until("the queue to empty", |app| queued(app).is_empty());
    assert!(!shows(&mut app, "session-queue"), "the queue bar goes");
    assert!(
        !world.conversation_text().contains(REMOVED),
        "the removed message never reached the agent"
    );
    drop(app);
    drop(world);
}
