//! End-to-end app reducer tests for desktop notifications (NOTIFY, D889).

use std::sync::Arc;
use std::time::Duration;

use humata2_app::{App, AppAction as A, AppState, Load, Route, channel};
use humata2_clients::model::{
    Answer, NotBroughtBack, Request, RequestId, RequestKind, Restart, ResumeFailure, Session,
    SessionId, Status,
};
use humata2_clients::{Event, FakeNotifier, NotifyTarget, fixture};
use humata2_session::{Panel, SessionAction};
use humata2_settings::SettingsAction;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<App>, Arc<FakeNotifier>, Arc<TestClock>) {
    let workspace = Arc::new(fixture::design());
    let clock = Arc::new(TestClock::new());
    let (clients, fakes) = humata2_clients::fake_clients((*workspace).clone(), clock.clone());
    let notifier = fakes.notifier.clone();
    let mut state = AppState::launch(&clients);
    state.load = Load::Ready;
    state.workspace = workspace.clone();
    state.inbox.workspace = workspace.clone();
    state.session.workspace = workspace;
    state.route = Route::Home;
    (
        TestStore::new(state, App::new(clients)).non_exhaustive(),
        notifier,
        clock,
    )
}

fn enable_notifications(store: &mut TestStore<App>) {
    store.send(
        A::Settings(SettingsAction::ToggleDesktopNotifications),
        |s| {
            s.settings.preferences.notify.enabled = true;
        },
    );
    store.skip_received();
}

fn make_request(store: &TestStore<App>, session: &SessionId, id: &str, kind: RequestKind) -> Request {
    let mut request = store.state().workspace.requests[0].clone();
    request.id = RequestId::new(id);
    request.session = session.clone();
    request.kind = kind;
    request.operation = "shell.exec".to_owned();
    request.title = format!("Allow {id}");
    request.answer = None;
    request
}

fn two_sessions_in_same_channel(store: &TestStore<App>) -> (Session, Session, String) {
    let workspace = &store.state().workspace;
    for channel in &workspace.channels {
        let sessions: Vec<Session> = workspace
            .sessions
            .iter()
            .filter(|s| !s.archived && s.channel == channel.id)
            .cloned()
            .collect();
        if sessions.len() >= 2 {
            return (sessions[0].clone(), sessions[1].clone(), channel.name.clone());
        }
    }
    panic!("fixture::design() should contain a channel with at least 2 active sessions");
}

#[test]
fn default_off_never_posts_notifications() {
    let (mut store, notifier, _) = setup();
    assert!(!store.state().settings.preferences.notify.enabled);

    store.send(A::WindowActive(false), |_| {});
    store.skip_received();

    let session = store.state().workspace.sessions[0].clone();
    let req = make_request(&store, &session.id, "req-default-off", RequestKind::Command);
    store.send(A::Event(Event::Request(req)), |_| {});
    store.skip_received();

    let mut failed = session.clone();
    failed.resume_failed = Some(ResumeFailure::ConversationMissing);
    store.send(A::Event(Event::Session(failed)), |_| {});
    store.skip_received();

    let machine = store.state().workspace.machines[0].id.clone();
    store.send(
        A::Event(Event::Restart(Restart {
            machine,
            at_unix_ms: 1_000,
            report: None,
            resumed: Vec::new(),
            not_brought_back: vec![NotBroughtBack {
                session: Some(session.id),
                tab: None,
                problem: "couldn't resume".to_owned(),
            }],
        })),
        |_| {},
    );
    store.skip_received();

    assert!(notifier.posted().is_empty());
}

#[test]
fn background_posts_for_live_events_and_skips_excluded() {
    let (mut store, notifier, _) = setup();
    enable_notifications(&mut store);
    store.send(A::WindowActive(false), |_| {});
    store.skip_received();

    let (s1, s2, _) = two_sessions_in_same_channel(&store);

    // Excluded: board.* and Migrate and already-answered requests.
    let mut board_req = make_request(&store, &s1.id, "req-board", RequestKind::Command);
    board_req.operation = "board.item.move".to_owned();
    store.send(A::Event(Event::Request(board_req)), |_| {});
    store.skip_received();

    let migrate_req = make_request(&store, &s1.id, "req-migrate", RequestKind::Migrate);
    store.send(A::Event(Event::Request(migrate_req)), |_| {});
    store.skip_received();

    let mut answered_req = make_request(&store, &s1.id, "req-answered", RequestKind::Command);
    answered_req.answer = Some(Answer::Approved);
    store.send(A::Event(Event::Request(answered_req)), |_| {});
    store.skip_received();

    assert!(notifier.posted().is_empty());

    // Live pending Agent-inbox request posts.
    let req = make_request(&store, &s1.id, "req-live-1", RequestKind::Command);
    store.send(A::Event(Event::Request(req)), |_| {});
    store.skip_received();

    let posted = notifier.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].id, format!("humata.session.{}", s1.id.as_str()));
    assert_eq!(
        posted[0].target,
        NotifyTarget::Request {
            session: s1.id.clone(),
            request: RequestId::new("req-live-1"),
        }
    );

    // Terminal-only NeedsYou on a session with no pending inbox request posts.
    notifier.clear();
    let mut s2_needs = s2.clone();
    s2_needs.status = Status::Working;
    store.send(A::Event(Event::Session(s2_needs.clone())), |_| {});
    store.skip_received();
    // Ensure s2 has no pending requests in the fixture before testing TerminalNeedsYou.
    let s2_pending: Vec<RequestId> = store
        .state()
        .workspace
        .waiting()
        .filter(|r| r.session == s2.id)
        .map(|r| r.id.clone())
        .collect();
    for id in s2_pending {
        store.send(A::Event(Event::RequestGone(id)), |_| {});
        store.skip_received();
    }
    notifier.clear();
    s2_needs.status = Status::NeedsYou;
    store.send(A::Event(Event::Session(s2_needs)), |_| {});
    store.skip_received();
    assert_eq!(notifier.posted().len(), 1);

    // Multi-session Restart failure posts under humata.restart.<machine_id>.
    notifier.clear();
    let machine = store.state().workspace.machines[0].id.clone();
    store.send(
        A::Event(Event::Restart(Restart {
            machine: machine.clone(),
            at_unix_ms: 2_000,
            report: None,
            resumed: Vec::new(),
            not_brought_back: vec![
                NotBroughtBack {
                    session: Some(s1.id),
                    tab: None,
                    problem: "missing".to_owned(),
                },
                NotBroughtBack {
                    session: Some(s2.id),
                    tab: None,
                    problem: "missing".to_owned(),
                },
            ],
        })),
        |_| {},
    );
    store.skip_received();
    let posted = notifier.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].id, format!("humata.restart.{}", machine.as_str()));
    assert_eq!(posted[0].target, NotifyTarget::Home);
}

#[test]
fn frontmost_on_item_suppresses_notification() {
    let (mut store, notifier, _) = setup();
    enable_notifications(&mut store);
    assert!(store.state().window_active);

    let (s1, s2, _) = two_sessions_in_same_channel(&store);

    // Viewing global Inbox suppresses Agent-inbox requests.
    store.send(A::Switch(Route::Inbox), |_| {});
    store.skip_received();
    let req1 = make_request(&store, &s1.id, "req-suppressed-inbox", RequestKind::Command);
    store.send(A::Event(Event::Request(req1)), |_| {});
    store.skip_received();
    assert!(notifier.posted().is_empty());

    // Viewing Session(s1) with Panel::Inbox suppresses requests for s1, but not s2.
    store.send(A::Switch(Route::Session(s1.id.clone())), |_| {});
    store.skip_received();
    store.send(A::Session(SessionAction::SetPanel(Some(Panel::Inbox))), |_| {});
    store.skip_received();
    let req2 = make_request(&store, &s1.id, "req-suppressed-panel", RequestKind::Command);
    store.send(A::Event(Event::Request(req2)), |_| {});
    store.skip_received();
    assert!(notifier.posted().is_empty());

    let req_other = make_request(&store, &s2.id, "req-other-session", RequestKind::Command);
    store.send(A::Event(Event::Request(req_other)), |_| {});
    store.skip_received();
    assert_eq!(notifier.posted().len(), 1);
    notifier.clear();

    // Viewing Channel(c) on Tab::Inbox suppresses requests for sessions in that channel.
    store.send(A::Switch(Route::Channel(s1.channel.clone())), |_| {});
    store.skip_received();
    store.send(A::Channel(channel::Action::Tab(channel::Tab::Inbox)), |_| {});
    store.skip_received();
    let req3 = make_request(&store, &s1.id, "req-suppressed-channel", RequestKind::Command);
    store.send(A::Event(Event::Request(req3)), |_| {});
    store.skip_received();
    assert!(notifier.posted().is_empty());

    // Backgrounding the window allows posting even on Route::Inbox.
    store.send(A::Switch(Route::Inbox), |_| {});
    store.skip_received();
    store.send(A::WindowActive(false), |_| {});
    store.skip_received();
    let req4 = make_request(&store, &s1.id, "req-background-inbox", RequestKind::Command);
    store.send(A::Event(Event::Request(req4)), |_| {});
    store.skip_received();
    assert_eq!(notifier.posted().len(), 1);
}

#[test]
fn thirty_second_burst_collapsing_per_session_and_per_channel() {
    let (mut store, notifier, clock) = setup();
    enable_notifications(&mut store);
    store.send(A::WindowActive(false), |_| {});
    store.skip_received();

    let (s1, s2, channel_name) = two_sessions_in_same_channel(&store);

    // 1st request in s1 posts immediately.
    let req1 = make_request(&store, &s1.id, "burst-1", RequestKind::Command);
    store.send(A::Event(Event::Request(req1)), |_| {});
    store.skip_received();

    // 2nd request in s1 within 30 s replaces in place with burst summary.
    clock.advance(Duration::from_secs(10));
    let req2 = make_request(&store, &s1.id, "burst-2", RequestKind::FileEdit);
    store.send(A::Event(Event::Request(req2)), |_| {});
    store.skip_received();

    let posted = notifier.posted();
    assert_eq!(posted.len(), 2);
    assert_eq!(posted[1].id, format!("humata.session.{}", s1.id.as_str()));
    assert!(
        posted[1].title.contains("2 items need you"),
        "expected burst collapse title, got: {}",
        posted[1].title
    );

    // Request in a 2nd distinct session in the same channel within 30 s collapses into channel notice.
    clock.advance(Duration::from_secs(5));
    let req3 = make_request(&store, &s2.id, "burst-3", RequestKind::Command);
    store.send(A::Event(Event::Request(req3)), |_| {});
    store.skip_received();

    let posted = notifier.posted();
    assert_eq!(posted.len(), 3);
    assert_eq!(posted[2].id, format!("humata.channel.{}", s1.channel.as_str()));
    assert!(
        posted[2].title.contains(&format!("2 sessions need you in #{channel_name}")),
        "expected channel collapse title, got: {}",
        posted[2].title
    );
    assert_eq!(posted[2].target, NotifyTarget::Inbox);
}

#[test]
fn withdrawal_on_request_gone_and_window_activation() {
    let (mut store, notifier, _) = setup();
    enable_notifications(&mut store);
    store.send(A::WindowActive(false), |_| {});
    store.skip_received();

    let (s1, _, _) = two_sessions_in_same_channel(&store);

    let req1 = make_request(&store, &s1.id, "withdraw-gone", RequestKind::Command);
    store.send(A::Event(Event::Request(req1)), |_| {});
    store.skip_received();
    assert_eq!(notifier.posted().len(), 1);

    // RequestGone withdraws the session notice.
    store.send(A::Event(Event::RequestGone(RequestId::new("withdraw-gone"))), |_| {});
    store.skip_received();
    assert_eq!(
        notifier.withdrawn(),
        vec![format!("humata.session.{}", s1.id.as_str())]
    );

    // Becoming active while on Session(s1) withdraws stale notice for s1.
    notifier.clear();
    store.send(A::Switch(Route::Session(s1.id.clone())), |_| {});
    store.skip_received();
    let req2 = make_request(&store, &s1.id, "withdraw-active", RequestKind::Command);
    store.send(A::Event(Event::Request(req2)), |_| {});
    store.skip_received();
    assert_eq!(notifier.posted().len(), 1);

    store.send(A::WindowActive(true), |_| {});
    store.skip_received();
    assert_eq!(
        notifier.withdrawn(),
        vec![format!("humata.session.{}", s1.id.as_str())]
    );
}

#[test]
fn notification_clicked_routes_to_target_and_falls_back_when_session_absent() {
    let (mut store, _, _) = setup();
    let (s1, _, _) = two_sessions_in_same_channel(&store);

    // 1. Request target with existing session -> Route::Session + Panel::Inbox + selected.
    let req_id = RequestId::new("clicked-req");
    store.send(
        A::NotificationClicked(NotifyTarget::Request {
            session: s1.id.clone(),
            request: req_id.clone(),
        }),
        |_| {},
    );
    store.skip_received();
    assert_eq!(store.state().route, Route::Session(s1.id.clone()));
    assert_eq!(store.state().panel(), Some(Panel::Inbox));
    assert_eq!(store.state().inbox.selected, Some(req_id));

    // 2. Request target with absent session -> fallback to Route::Inbox + selected.
    let missing_req = RequestId::new("missing-session-req");
    store.send(
        A::NotificationClicked(NotifyTarget::Request {
            session: SessionId::new("no-such-session"),
            request: missing_req.clone(),
        }),
        |_| {},
    );
    store.skip_received();
    assert_eq!(store.state().route, Route::Inbox);
    assert_eq!(store.state().inbox.selected, Some(missing_req));

    // 3. Session target -> Route::Session.
    store.send(A::NotificationClicked(NotifyTarget::Session(s1.id.clone())), |_| {});
    store.skip_received();
    assert_eq!(store.state().route, Route::Session(s1.id));

    // 4. Inbox target -> Route::Inbox.
    store.send(A::NotificationClicked(NotifyTarget::Inbox), |_| {});
    store.skip_received();
    assert_eq!(store.state().route, Route::Inbox);

    // 5. Home target -> Route::Home.
    store.send(A::NotificationClicked(NotifyTarget::Home), |_| {});
    store.skip_received();
    assert_eq!(store.state().route, Route::Home);
}

#[test]
fn clicking_a_todo_notification_lands_on_the_todo() {
    let (mut store, _, _) = setup();
    store.send(A::NotificationClicked(NotifyTarget::Todo { item: None }), |_| {});
    store.skip_received();
    assert_eq!(store.state().route, Route::Todo);

    store.send(A::NotificationClicked(NotifyTarget::Home), |_| {});
    store.skip_received();
    store.send(
        A::NotificationClicked(NotifyTarget::Todo {
            item: Some(humata2_clients::model::ItemId::new("H2-1")),
        }),
        |_| {},
    );
    store.skip_received();
    assert_eq!(store.state().route, Route::Todo);
}

#[test]
fn launch_requests_permission_once_when_notifications_were_saved_enabled() {
    let (mut store, notifier, _) = setup();
    assert_eq!(notifier.permission_requests(), 0);

    store.send(A::Settings(SettingsAction::ToggleDesktopNotifications), |_| {});
    store.skip_received();
    assert_eq!(notifier.permission_requests(), 1);

    // Simulating a launch with notifications already enabled requests permission again.
    store.send(A::Launch, |_| {});
    store.skip_received();
    store.skip_in_flight();
    assert_eq!(notifier.permission_requests(), 2);
}
