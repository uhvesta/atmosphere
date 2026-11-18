//! Two viewers on one terminal (micro case 20) against a real `humatad2`
//! and holder: two headless apps, each on its own connection to the host,
//! show the same shell tab. Both type into it, both see both answers, and
//! while the session works each one's bar says the other can type.
//!
//! Checks read answers the shell computes (`| tr a-z A-Z` capitals), never
//! the echo of the typed line. Words start with `xq` so no app label can
//! match them.
use std::process::{Command, Stdio};

use humata2_app::{AppAction, Route};
use humata2_clients::model::SessionId;
use humata2_session::SessionAction;

use super::typing::{answered, shells_of, zsh};
use super::world::{LONG, World};
use super::{App, runtime};

/// What a viewer's bar says while one other viewer can type.
const ONE_OTHER: &str = "1 other can type here";

/// Reports the session's turn as an agent's hook does (`humata turn`).
fn turn(world: &World, state: &str) {
    let humata = std::env::var_os("HUMATA").expect("the test is given humata's path");
    let humata = std::fs::canonicalize(humata).expect("humata's path resolves");
    let status = Command::new(humata)
        .args(["turn", state])
        .env("HUMATA_SOCKET", world.socket())
        .env("HUMATA_SESSION_TOKEN", world.agent_token())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .expect("humata runs");
    assert!(status.success(), "humata turn {state} is accepted");
}

fn shown_session(app: &App<'_>) -> SessionId {
    app.ui.read(|state| match &state.route {
        Route::Session(id) => id.clone(),
        other => panic!("the app shows a session, not {other:?}"),
    })
}

/// Shows the session's newest shell tab, as a click on its tab does.
fn show_newest_shell(app: &mut App<'_>, id: &SessionId) {
    let tab = shells_of(app, id).expect("the session is listed");
    app.cx
        .update(|cx| app.ui.send(AppAction::Session(SessionAction::SelectTab(tab)), cx));
    app.settle();
}

/// Case 20 steps 1, 2 and 3 and branch B2: the second viewer shows the
/// shell the first opened, what either types reaches that one shell and
/// shows in both, and while the session works each bar says one other can
/// type; once the second viewer closes, the first one's bar stops saying so
/// and its shell still answers.
#[test]
fn two_viewers_type_into_one_shell_and_each_hears_the_other() {
    let runtime = runtime();
    let (mut a, world) = zsh(&runtime);
    let id = shown_session(&a);
    answered(&mut a, "xqfroma");

    // Step 1: viewer B, a second app on its own connection, shows the
    // same tab and what A's shell printed.
    let clients = runtime.block_on(world.clients()).expect("connect viewer B");
    let mut b = App::open(&runtime, clients);
    b.cx
        .update_window(b.window, |_, window, _| window.activate_window())
        .expect("the window stays open");
    b.settle();
    b.show_session(&id);
    show_newest_shell(&mut b, &id);
    b.wait_within("viewer B to show A's answer", LONG, |b| b.view().contains("XQFROMA"));

    // Step 2: B types into the same shell; both viewers show its answer,
    // then A types again and both show that too.
    answered(&mut b, "xqfromb");
    a.wait_within("viewer A to show B's answer", LONG, |a| a.view().contains("XQFROMB"));
    answered(&mut a, "xqagaina");
    b.wait_within("viewer B to show A's next answer", LONG, |b| {
        b.view().contains("XQAGAINA")
    });

    // Step 3: while the session works, each bar names the other writer.
    turn(&world, "working");
    a.wait_within("viewer A's bar to name B", LONG, |a| a.view().contains(ONE_OTHER));
    b.wait_within("viewer B's bar to name A", LONG, |b| b.view().contains(ONE_OTHER));

    // B closes; A's bar stops naming another writer.
    drop(b);
    a.wait_within("viewer A's bar to drop B", LONG, |a| !a.view().contains(ONE_OTHER));
    answered(&mut a, "xqalone");
    turn(&world, "idle");
}
