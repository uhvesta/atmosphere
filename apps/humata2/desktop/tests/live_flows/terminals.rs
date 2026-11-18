//! Flow 9 steps 1, 3 and 4 in the app against a real `humatad2`: a new
//! terminal tab on a running session, a Terminal session from the New
//! session sheet, and a machine page's Open a shell. Each types a command
//! whose answer (`WIRE42`) can only come from the shell running it, and
//! does so without clicking the new tab first.
use gpui_kit::Role;
use humata2_app::{AppAction, Overlay, Route};
use humata2_clients::model::{Agent, SessionId, Status};
use humata2_flow::Action;
use humata2_session::SessionAction;

use super::world::World;
use super::{App, app_key, channel_with_repo, running_session, runtime};

/// The command typed into the new terminal. The shell's answer, `WIRE42`,
/// differs from the command line the terminal echoes back.
const COMMAND: &str = "echo WIRE$((40+2))";
const ANSWER: &str = "WIRE42";

/// Headless windows start inactive; focus needs activation.
fn activate(app: &mut App<'_>) {
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("the window stays open");
    app.settle();
}

/// Once the shown terminal has focus at its shell's prompt, with no click,
/// types the command and Enter, then waits for the shell's answer to show.
/// `watch` is polled each round (to record status).
fn run_and_wait(app: &mut App<'_>, mut watch: impl FnMut(&mut App<'_>)) {
    app.wait_until("the terminal focused at its shell's prompt", |app| {
        watch(app);
        super::typing::at_prompt(app, "$")
    });
    app.type_text(COMMAND);
    app.play(&Action::Key("enter".to_owned()));
    app.wait_until("the shell's answer to the typed command", |app| {
        watch(app);
        app.view().contains(ANSWER)
    });
}

fn shells_of(app: &App<'_>, id: &SessionId) -> Option<usize> {
    app.ui.read(|state| {
        state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map(|session| session.shells.len())
    })
}

/// Waits for This Mac to be online in the app and returns its id.
fn mac_online(app: &mut App<'_>) -> humata2_clients::model::MachineId {
    app.wait_until("This Mac to load online", |app| {
        app.ui.read(|state| {
            state.workspace.machines.first().is_some_and(|m| m.online)
                && !state.workspace.channels.is_empty()
        })
    });
    app.ui
        .read(|state| state.workspace.machines.first().map(|m| m.id.clone()))
        .expect("the workspace has This Mac")
}

/// Waits for a Terminal (Shell) session to be listed and returns its id.
fn shell_session(app: &mut App<'_>) -> SessionId {
    app.wait_until("the Terminal session to be listed", |app| {
        app.ui.read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .any(|session| session.agent == Agent::Shell)
        })
    });
    app.ui
        .read(|state| {
            state
                .workspace
                .sessions
                .iter()
                .find(|session| session.agent == Agent::Shell)
                .map(|session| session.id.clone())
        })
        .expect("listed after the wait")
}

/// Step 1: New terminal on a running session adds a shell tab that takes
/// typing at once and runs the command.
#[test]
fn a_new_terminal_tab_takes_typing_without_a_click_and_runs_the_command() {
    let runtime = runtime();
    let (world, clients, id) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        let id = running_session(&world, &clients, "Fix the ledger", "hello agent").await;
        (world, clients, id)
    });
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    app.show_session(&id);
    app.wait_until("the agent's answer in its terminal", |app| {
        app.view().contains("got: hello agent")
    });
    let before = shells_of(&app, &id).expect("the session is listed");

    // The plus button's menu row carries the same label as the button, so
    // it can't be clicked by label; its action is sent, as the fakes test
    // does (`desktop/tests/native/terminal.rs`).
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
    app.wait_until("the session to have one more tab", |app| {
        shells_of(app, &id) == Some(before + 1)
    });

    run_and_wait(&mut app, |_| {});
    assert_eq!(shells_of(&app, &id), Some(before + 1));
    drop(app);
    drop(world);
}

/// Step 3: New session with Terminal picked starts a shell session that
/// never reads Working and answers a typed command.
#[test]
fn a_terminal_started_from_new_session_runs_a_shell_that_answers_and_never_reads_working() {
    let runtime = runtime();
    let (world, clients) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        channel_with_repo(&world, &clients, "payments").await;
        (world, clients)
    });
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    mac_online(&mut app);

    app.play(&Action::Key(app_key("n")));
    assert!(
        app.ui
            .read(|state| matches!(state.overlays.last(), Some(Overlay::NewSession(_)))),
        "New session opens the sheet"
    );
    app.click(Role::Tab, "Terminal");
    app.wait_until("Terminal to be picked", |app| {
        app.sheet_holds(|sheet| sheet.agent == Agent::Shell)
    });
    app.click(Role::TextInput, "Title");
    app.type_text("Scratch shell");
    app.wait_until("the typed title to reach the sheet", |app| {
        app.sheet_holds(|sheet| sheet.title == "Scratch shell")
    });
    app.click(Role::Button, "Start session");

    let id = shell_session(&mut app);
    assert_eq!(
        app.session_titled("Scratch shell").map(|session| session.id),
        Some(id.clone()),
        "the session carries the typed title"
    );
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });

    let mut worked = false;
    run_and_wait(&mut app, |app| {
        worked |= app
            .session_titled("Scratch shell")
            .is_some_and(|session| session.status == Status::Working);
    });
    assert!(!worked, "a terminal session never reads Working");
    app.wait_until("the terminal session to settle Idle", |app| {
        app.session_titled("Scratch shell")
            .is_some_and(|session| session.status == Status::Idle)
    });
    drop(app);
    drop(world);
}

/// Step 4: a machine page's Open a shell opens the sheet on that machine
/// with Terminal picked; starting it gives a shell that answers.
#[test]
fn open_a_shell_on_a_machine_page_starts_a_terminal_there() {
    let runtime = runtime();
    let (world, clients, mac) = runtime.block_on(async {
        let world = World::start().await.expect("start the daemon");
        let clients = world.clients().await.expect("connect");
        channel_with_repo(&world, &clients, "payments").await;
        let mac = World::this_mac(&clients).await.expect("This Mac");
        (world, clients, mac)
    });
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    mac_online(&mut app);

    app.cx
        .update(|cx| app.ui.send(AppAction::Navigate(Route::Machine(mac.clone())), cx));
    app.settle();
    app.click(Role::Button, "Open a shell");
    app.wait_until("the sheet to open on this machine as a Terminal", |app| {
        app.sheet_holds(|sheet| {
            sheet.agent == Agent::Shell && sheet.machine.as_ref() == Some(&mac)
        })
    });
    app.click(Role::Button, "Start session");

    let id = shell_session(&mut app);
    app.wait_until("the session page to show", |app| {
        app.ui.read(|state| state.route == Route::Session(id.clone()))
    });
    run_and_wait(&mut app, |_| {});
    let machine = app.ui.read(|state| {
        state
            .workspace
            .sessions
            .iter()
            .find(|session| session.id == id)
            .map(|session| session.machine.clone())
    });
    assert_eq!(machine.as_ref(), Some(&mac), "the shell runs on This Mac");
    drop(app);
    drop(world);
}
