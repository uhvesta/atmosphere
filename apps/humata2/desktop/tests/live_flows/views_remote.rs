//! SPEC flow 26.12 (prepared views) for a session that runs on a box, with
//! the app on the host. The agent prepares and marks its view ready on the
//! box's own daemon with the session token its tab was given there; the box
//! forwards the view to the host (VW-12), so the offer reaches the app over
//! the host's views feed. What the person chooses is answered on the host.
//! The daemon side of forwarding is `daemon/humatad/tests/views_remote.rs`.

use humata2_app::{AppAction, Route};
use humata2_clients::model::{SessionId, ViewId, ViewState};
use humata2_views::ViewsAction;

use super::remote::RemoteWorld;
use super::views::{
    AgentCalls, app_state, daemon_state, in_journal, offered, seen, send, showing, view_tab, wait_for_daemon,
    wait_for_offer,
};
use super::{App, runtime};

const BOX: &str = "box-1";
const TITLE: &str = "PR review: ledger schema";

/// A host with box-1, a running session on box-1, and its agent as the box
/// sees it.
fn started(runtime: &tokio::runtime::Runtime) -> (RemoteWorld, humata2_clients::Clients, SessionId, AgentCalls) {
    runtime.block_on(async {
        let world = RemoteWorld::start(&[BOX], &[("owner", "ledger")]).await;
        let clients = world.clients().await.expect("the app connects to the host");
        let session = world.running_session_on(BOX, "Ledger schema migration").await;
        let agent = AgentCalls {
            socket: world.box_socket(BOX),
            token: world.agent_token(BOX, &session).await,
        };
        (world, clients, session, agent)
    })
}

/// The app on the box session's page, which has loaded.
fn on_the_page(app: &mut App, session: &SessionId) {
    app.wait_until("the box session to load", |app| {
        app.ui.read(|state| state.workspace.session(session).is_some())
    });
    send(app, AppAction::Navigate(Route::Session(session.clone())));
}

/// 26.12 step 1 for a box session: the view its agent marks ready on the
/// box is offered in the app, waiting, and nothing else changes; its draft
/// is not offered.
#[test]
fn a_view_a_box_agent_marks_ready_reaches_the_app_over_the_host() {
    let runtime = runtime();
    let (_world, clients, session, agent) = started(&runtime);
    let mut app = App::open(&runtime, clients);
    on_the_page(&mut app, &session);
    let before = seen(&app, &session);

    let draft = ViewId::new(runtime.block_on(agent.prepare("Still drafting")).value);
    let id = runtime.block_on(agent.offer(TITLE));
    wait_for_offer(&mut app, &session, &id);

    app.ui.read(|state| {
        assert_eq!(app_state(state, &id), Some(ViewState::Waiting));
        assert_eq!(
            state.views.record(&id).map(|record| record.view.session.clone()),
            Some(session.clone()),
            "the offer is the box session's"
        );
        assert!(!offered(state, &session).contains(&draft), "a draft stays on the box");
        assert!(in_journal(state, &session, &draft).is_empty());
        assert_eq!(showing(state), None, "nothing opens until the person chooses to");
        assert_eq!(view_tab(state, &session, &id), (false, false));
    });
    assert_eq!(seen(&app, &session), before, "an offer leaves the page as it was");
}

/// 26.12 steps 2 to 5 for a box session: Open it, step to the second step,
/// Revert puts the page back and the host agrees; it opens again.
#[test]
fn reverting_an_opened_box_walkthrough_puts_the_page_back() {
    let runtime = runtime();
    let (_world, clients, session, agent) = started(&runtime);
    let views = clients.views.clone();
    let mut app = App::open(&runtime, clients);
    on_the_page(&mut app, &session);
    let id = runtime.block_on(agent.offer(TITLE));
    wait_for_offer(&mut app, &session, &id);
    let before = seen(&app, &session);

    send(&mut app, AppAction::Views(ViewsAction::Open(id.clone())));
    app.ui.read(|state| {
        assert_eq!(state.route, Route::Session(session.clone()));
        assert_eq!(showing(state), Some(id.clone()));
        assert_eq!(view_tab(state, &session, &id), (true, true));
        assert_eq!(app_state(state, &id), Some(ViewState::Opened));
    });
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Opened);

    send(&mut app, AppAction::Views(ViewsAction::Step { id: id.clone(), to: 1 }));
    app.ui.read(|state| assert_eq!(state.view_step.get(&id), Some(&1)));

    send(&mut app, AppAction::Views(ViewsAction::Revert));
    assert_eq!(seen(&app, &session), before, "Revert puts back exactly what was there");
    app.ui.read(|state| {
        assert!(!view_tab(state, &session, &id).0);
        assert_eq!(showing(state), None);
        assert_eq!(app_state(state, &id), Some(ViewState::Reverted));
        assert_eq!(in_journal(state, &session, &id), vec![ViewState::Reverted]);
    });
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Reverted);

    send(&mut app, AppAction::Views(ViewsAction::Open(id.clone())));
    app.ui.read(|state| {
        assert_eq!(showing(state), Some(id.clone()), "the Journal's view opens again");
        assert_eq!(app_state(state, &id), Some(ViewState::Opened));
    });
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Opened);
    assert_eq!(daemon_state(&app, &views, &session, &id), Some(ViewState::Opened));
}

/// Keep for a box session ends the showing, leaves the view's tab, and the
/// host and the Journal say Kept.
#[test]
fn keeping_a_box_view_is_recorded_on_the_host() {
    let runtime = runtime();
    let (_world, clients, session, agent) = started(&runtime);
    let views = clients.views.clone();
    let mut app = App::open(&runtime, clients);
    on_the_page(&mut app, &session);
    let id = runtime.block_on(agent.offer(TITLE));
    wait_for_offer(&mut app, &session, &id);
    send(&mut app, AppAction::Views(ViewsAction::Open(id.clone())));
    let opened = seen(&app, &session);

    send(&mut app, AppAction::Views(ViewsAction::Keep));
    assert_eq!(seen(&app, &session), opened, "Keep changes nothing on the page");
    app.ui.read(|state| {
        assert_eq!(showing(state), None);
        assert!(view_tab(state, &session, &id).0, "the view's tab stays");
        assert_eq!(app_state(state, &id), Some(ViewState::Kept));
        assert_eq!(in_journal(state, &session, &id), vec![ViewState::Kept]);
    });
    wait_for_daemon(&mut app, &views, &session, &id, ViewState::Kept);
}
