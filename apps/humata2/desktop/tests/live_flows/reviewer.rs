//! REVIEWER exit (D849) in the app against a real `humatad2`: a reviewer is
//! a tab in the session's own strip. Picking an agent from the full-screen
//! Diff's New agent tab opens it and leaves full screen; selecting another
//! tab hides it without ending it; selecting it again shows the same live
//! shell; a reboot lists it again; its × ends it and nothing else.
use humata2_app::layout::FullScreen;
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Agent, SessionId};
use humata2_review::ReviewAction;
use humata2_session::{CloseAction, CloseChoice, Panel, SessionAction, Tab};

use super::world::World;
use super::{App, running_session, runtime, typing};

fn activate(app: &mut App<'_>) {
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .expect("the window stays open");
    app.settle();
}

fn send(app: &mut App<'_>, action: AppAction) {
    app.cx.update(|cx| app.ui.send(action, cx));
    app.settle();
}

fn full(app: &App<'_>) -> bool {
    app.ui
        .read(|state| state.full_screen() == Some(FullScreen::Panel))
}

/// The tabs of the session's first row.
fn strip(state: &AppState) -> Vec<Tab> {
    state
        .session
        .view()
        .map_or_else(Vec::new, |view| view.groups[0].tabs.clone())
}

fn tab_count(app: &App<'_>) -> usize {
    app.ui.read(|state| strip(state).len())
}

/// The selected tab of the first row, by index.
fn selected(app: &App<'_>) -> Option<usize> {
    app.ui
        .read(|state| state.session.view().and_then(|view| view.groups.first().map(|g| g.selected)))
}

/// What the daemon lists as the session's shell tabs.
fn daemon_shells(app: &App<'_>, id: &SessionId) -> usize {
    let daemon = app.daemon.clone();
    let workspace = app.runtime.block_on(daemon.load()).expect("load");
    workspace
        .sessions
        .iter()
        .find(|session| &session.id == id)
        .map_or(0, |session| session.shells.len())
}

fn select(app: &mut App<'_>, index: usize) {
    send(
        app,
        AppAction::Session(SessionAction::SelectGroupTab { group: 0, index }),
    );
    app.wait_until("the tab to be selected", |app| selected(app) == Some(index));
}

/// Types a line to the test agent and waits for its echo to show.
fn prove_live(app: &mut App<'_>) {
    app.type_text("ping reviewer");
    typing::key(app, "enter");
    typing::wait_for(app, "got: ping reviewer");
}

#[test]
fn a_reviewer_tab_opens_hides_shows_comes_back_after_a_reboot_and_ends_on_its_cross() {
    let runtime = runtime();
    let (mut world, clients, id) = runtime.block_on(async {
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
    let before = tab_count(&app);
    assert!(before >= 1);

    // 1. Open: the Diff panel full screen, then the pick the row sends.
    send(
        &mut app,
        AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
    );
    send(&mut app, AppAction::Session(SessionAction::ToggleWiden));
    app.wait_until("the Diff panel full screen", |app| full(app));
    send(
        &mut app,
        AppAction::Review(ReviewAction::AgentTabPicked(Agent::Claude)),
    );
    send(&mut app, AppAction::Session(SessionAction::ToggleWiden));
    send(
        &mut app,
        AppAction::Session(SessionAction::OpenAgentTab {
            agent: Agent::Claude,
            prompt: "Look at the diff".to_owned(),
        }),
    );
    app.wait_until("one more tab, selected", |app| {
        tab_count(app) == before + 1 && selected(app) == Some(before)
    });
    assert!(!full(&app), "full screen gives way to the new tab");
    let reviewer = before;

    // D858: OpenAgentTab launches the test agent directly and delivers the
    // opening prompt in the background.
    typing::wait_for(&mut app, "got: Look at the diff");

    // 2. Hide: the agent's tab is shown; the reviewer is listed and running.
    select(&mut app, 0);
    assert_eq!(tab_count(&app), before + 1);
    let listed = daemon_shells(&app, &id);
    assert!(listed >= 1, "the daemon still lists the reviewer's shell");
    let agents = app.ui.read(|state| {
        state
            .session
            .workspace
            .session(&id)
            .map(|session| state.session.agent_count(session))
    });
    assert_eq!(agents, Some(2), "the session's agent and the reviewer's");

    // 3. Show: the same live shell answers a typed command.
    select(&mut app, reviewer);
    prove_live(&mut app);
    let shells = typing::shells_of(&app, &id).expect("listed");
    assert_eq!(daemon_shells(&app, &id), shells);

    // 4. Restart: the tab is listed again after a reboot.
    drop(app);
    let clients = runtime.block_on(async {
        world.reboot().await.expect("reboot");
        world.clients().await.expect("connect again")
    });
    let mut app = App::open(&runtime, clients);
    activate(&mut app);
    app.show_session(&id);
    app.wait_until("the reviewer tab to be listed again", |app| {
        tab_count(app) == before + 1
    });
    assert_eq!(typing::shells_of(&app, &id), Some(shells));
    let reboot_agent = app.ui.read(|state| {
        state
            .session
            .workspace
            .session(&id)
            .and_then(|session| session.terminal_agents.get(&reviewer).copied())
    });
    assert_eq!(
        reboot_agent,
        Some(Agent::Claude),
        "D858: reviewer tab agent persists across daemon reboot"
    );

    // 5. End: the × closes the tab; the agent's tab stays.
    // The × asks first; End for good ends this tab's shell.
    send(
        &mut app,
        AppAction::Session(SessionAction::Close(CloseAction::Ask {
            group: 0,
            index: reviewer,
        })),
    );
    app.wait_until("the close dialog", |app| {
        app.ui.read(|state| state.session.closing.is_some())
    });
    send(
        &mut app,
        AppAction::Session(SessionAction::Close(CloseAction::Choose(
            CloseChoice::EndForGood,
        ))),
    );
    app.wait_until("the tab count to drop by one", |app| {
        tab_count(app) == before
    });
    app.wait_until("the daemon to list one shell less", |app| {
        daemon_shells(app, &id) == shells - 1
    });
    // The daemon tells the app too, so its copy of the session drops it.
    app.wait_until("the app to list one shell less", |app| {
        typing::shells_of(app, &id) == Some(shells - 1)
    });
    assert!(
        app.ui.read(|state| matches!(
            state.session.view().and_then(|view| view.selected(0)),
            Some(Tab::Terminal(_))
        )),
        "the agent's tab is still there to show"
    );
    drop(app);
    drop(world);
}
