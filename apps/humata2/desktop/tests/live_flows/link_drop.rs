//! Typing over a link that drops (micro case 16) in the app against a
//! real host `humatad2` linked to a second real `humatad2`, devbox-1
//! (`World::start_with_machine`). The session and its shell tab live on
//! devbox-1, so every key goes app → host → link → devbox-1's holder →
//! `/bin/sh -i`. The link is cut as a dropped connection is
//! (`Devbox::cut_link`: the link's process dies and the test's stand-in
//! for `ssh` hangs every redial) and let through again with no help from
//! the app.
//!
//! Checks read answers the shell computes (`| tr a-z A-Z` capitals), never
//! the echo of the typed line. Words start with `xq` so no app label can
//! match them.
use humata2_app::AppAction;
use humata2_clients::Clients;
use humata2_clients::model::{Agent, MachineId, SessionId, SessionOptions};
use humata2_session::SessionAction;

use super::typing::{UPPER, answered, key, ready, shells_of, shows, wait_for, wait_gone};
use super::world::{DEVBOX, LONG, World};
use super::{App, runtime};

/// What the tab says while its machine is out of reach: the pane's banner
/// and the session's own line both start so.
const LOST: &str = "Lost the connection to";

/// A session on devbox-1 whose agent has answered its first message.
async fn session_on_devbox(world: &World, clients: &Clients) -> SessionId {
    let daemon = clients.daemon.clone();
    let workspace = daemon.load().await.expect("load");
    let machine = |pick: &dyn Fn(&str) -> bool| -> MachineId {
        workspace
            .machines
            .iter()
            .find(|machine| pick(&machine.name))
            .map(|machine| machine.id.clone())
            .expect("the host lists the machine")
    };
    let devbox = machine(&|name| name == DEVBOX);
    let mac = machine(&|name| name != DEVBOX);
    let repo = world.repo("owner", "ledger").expect("make a repo");
    let channel = super::create_channel_ready(
        &daemon,
        "ledger",
        vec![repo.clone()],
        vec![mac, devbox.clone()],
    )
    .await;
    daemon
        .create_session(
            &channel,
            Agent::Claude,
            &devbox,
            "Fix the ledger",
            SessionOptions {
                repos: vec![repo],
                first_message: "hello box".to_owned(),
                ..SessionOptions::default()
            },
        )
        .await
        .expect("start a session on devbox-1")
}

fn toasts_saying(app: &App<'_>, what: &str) -> usize {
    app.ui
        .read(|state| state.toasts.iter().filter(|toast| toast.text.contains(what)).count())
}

/// Case 16 steps 3, 4, 5, 7 and 8: while the link is down the tab says so,
/// what is typed is refused once with a toast and never arrives later,
/// output printed meanwhile shows once the link is back by itself, and
/// typing then reaches the same shell, whose half-typed line is intact.
#[test]
fn a_dropped_link_refuses_typing_and_the_same_shell_takes_it_again_once_back() {
    let runtime = runtime();
    let world = runtime
        .block_on(World::start_with_machine())
        .expect("start the host with devbox-1");
    let clients = runtime.block_on(world.clients()).expect("connect");
    let id = runtime.block_on(session_on_devbox(&world, &clients));
    let mut app = App::open(&runtime, clients);
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("the window stays open");
    app.settle();
    app.show_session(&id);
    app.wait_within("the agent's answer on devbox-1", LONG, |app| {
        app.view().contains("got: hello box")
    });
    let before = shells_of(&app, &id).expect("the session is listed");
    app.cx.update(|cx| {
        app.ui.send(
            AppAction::Session(SessionAction::OpenTerminal {
                agent: Agent::Shell,
                split: false,
            }),
            cx,
        )
    });
    app.settle();
    app.wait_within("a shell tab on devbox-1", LONG, |app| {
        shells_of(app, &id) == Some(before + 1)
    });
    // The tab is listed before its screen arrives from devbox-1, and keys
    // typed before then are refused, never queued: wait for the prompt.
    ready(&mut app, "$");
    answered(&mut app, "xqup");

    // A job that prints while the link is down, and a half-typed line.
    let late = format!("(sleep 6; echo xqwhile{UPPER}) &");
    app.type_text(&late);
    key(&mut app, "enter");
    app.type_text("echo xqhalf");
    // The shell's echo of the line: every key so far has reached it.
    wait_for(&mut app, "echo xqhalf");
    world.devbox().cut_link();
    assert!(!shows(&mut app, "XQWHILE"), "the job has not printed yet");

    // Step 3: the tab says the link is down.
    wait_for(&mut app, LOST);

    // Step 4: keys typed now are refused, told once, and never sent.
    let told = toasts_saying(&app, "not sent");
    app.type_text("xqlost");
    app.settle();
    assert_eq!(toasts_saying(&app, "not sent"), told + 1, "one toast for the refused keys");

    // Step 5: the link comes back with no help from the app.
    world.devbox().restore_link();
    wait_gone(&mut app, LOST);

    // Step 7: what the job printed meanwhile shows.
    wait_for(&mut app, "XQWHILE");

    // Step 8: typing reaches the same shell; its line holds `echo xqhalf`
    // and none of what was refused.
    app.type_text(UPPER);
    key(&mut app, "enter");
    wait_for(&mut app, "XQHALF");
    assert!(!shows(&mut app, "XQLOST"), "refused keys never arrive late");
    answered(&mut app, "xqafter");
}
