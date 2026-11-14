use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use humata_core::{
    Daemon, DaemonError, LiveDaemon, Session, SessionState, Sessions, SessionsAction,
    SessionsFeature,
};
use humata_wire::{Reply, Service, ServiceFuture, pb};
use tca::{BoxFuture, TestClock, TestStore};

const EVERY: Duration = Duration::from_secs(2);

/// Answers each ask with the next scripted answer; once they run out, it
/// never answers.
#[derive(Default)]
struct FakeDaemon {
    answers: Mutex<VecDeque<Result<Vec<Session>, DaemonError>>>,
}

impl FakeDaemon {
    fn answering(
        answers: impl IntoIterator<Item = Result<Vec<Session>, DaemonError>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().collect()),
        })
    }
}

impl Daemon for FakeDaemon {
    fn sessions(&self) -> BoxFuture<Result<Vec<Session>, DaemonError>> {
        let next = self.answers.lock().unwrap().pop_front();
        Box::pin(async move {
            match next {
                Some(answer) => answer,
                None => std::future::pending().await,
            }
        })
    }
}

fn session(id: &str, last_active_unix_ms: i64) -> Session {
    Session {
        id: id.into(),
        name: id.into(),
        state: SessionState::Idle,
        project_id: String::new(),
        host: String::new(),
        last_active_unix_ms,
        archived: false,
        unread: false,
    }
}

fn store(daemon: Arc<FakeDaemon>, clock: &TestClock) -> TestStore<SessionsFeature> {
    TestStore::new(
        Sessions::default(),
        SessionsFeature::new(daemon, Arc::new(clock.clone()), EVERY),
    )
}

#[test]
fn while_it_shows_the_list_is_asked_for_again_newest_first_without_archived_ones() {
    let old = session("old", 1);
    let new = session("new", 2);
    let archived = Session {
        archived: true,
        ..session("gone", 3)
    };
    let newer = Session {
        unread: true,
        ..session("old", 5)
    };
    let first = vec![old.clone(), archived, new.clone()];
    let daemon = FakeDaemon::answering([Ok(first.clone()), Ok(vec![newer.clone(), new.clone()])]);
    let clock = TestClock::new();
    let mut store = store(daemon, &clock);

    store.send(SessionsAction::Appeared, |s| s.loading = true);
    store.receive(SessionsAction::Loaded(first), |s| {
        s.loading = false;
        s.list = [new.clone(), old].into_iter().collect();
    });
    clock.advance(EVERY);
    store.receive(
        SessionsAction::Loaded(vec![newer.clone(), new.clone()]),
        |s| {
            s.list = [newer, new].into_iter().collect();
        },
    );
    store.send(SessionsAction::Disappeared, |_| {});
    store.finish();
}

#[test]
fn a_failure_keeps_the_list_and_says_why_until_the_next_answer() {
    let one = session("one", 1);
    let daemon = FakeDaemon::answering([
        Ok(vec![one.clone()]),
        Err(DaemonError::Unexpected),
        Ok(vec![one.clone()]),
    ]);
    let clock = TestClock::new();
    let mut store = store(daemon, &clock);

    store.send(SessionsAction::Appeared, |s| s.loading = true);
    store.receive(SessionsAction::Loaded(vec![one.clone()]), |s| {
        s.loading = false;
        s.list = [one.clone()].into_iter().collect();
    });
    clock.advance(EVERY);
    store.receive(
        SessionsAction::Failed(DaemonError::Unexpected.to_string()),
        |s| {
            s.error = Some(DaemonError::Unexpected.to_string());
        },
    );
    clock.advance(EVERY);
    store.receive(SessionsAction::Loaded(vec![one]), |s| s.error = None);
    store.send(SessionsAction::Disappeared, |_| {});
    store.finish();
}

#[test]
fn a_selection_whose_session_has_gone_is_cleared() {
    let one = session("one", 1);
    let two = session("two", 2);
    let daemon = FakeDaemon::answering([]);
    let clock = TestClock::new();
    let mut store = store(daemon, &clock);

    store.send(
        SessionsAction::Loaded(vec![one.clone(), two.clone()]),
        |s| {
            s.list = [two.clone(), one.clone()].into_iter().collect();
        },
    );
    store.send(SessionsAction::Select(Some("one".into())), |s| {
        s.selected = Some("one".into());
    });
    store.send(SessionsAction::Loaded(vec![two.clone()]), |s| {
        s.list = [two].into_iter().collect();
        s.selected = None;
    });
    store.finish();
}

#[test]
fn refreshing_twice_asks_once_more_and_leaving_stops_it() {
    let one = session("one", 1);
    let daemon = FakeDaemon::answering([Ok(vec![one.clone()])]);
    let clock = TestClock::new();
    let mut store = store(daemon, &clock);

    store.send(SessionsAction::Refresh, |_| {});
    store.receive(SessionsAction::Loaded(vec![one.clone()]), |s| {
        s.list = [one].into_iter().collect();
    });
    // The daemon doesn't answer this one; leaving cancels it.
    store.send(SessionsAction::Refresh, |_| {});
    store.send(SessionsAction::Disappeared, |_| {});
    store.finish();
}

/// A daemon that lists two sessions, the way `humatad` answers.
struct Listing;

impl Service for Listing {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
        Box::pin(async move {
            let kind = match request {
                pb::request::Kind::ListSessions(_) => pb::response::Kind::Sessions(pb::Sessions {
                    sessions: vec![
                        pb::SessionInfo {
                            id: "a".into(),
                            name: "First".into(),
                            state: pb::SessionState::Working.into(),
                            last_active_unix_ms: 10,
                            turns: 3,
                            seen_turns: 2,
                            ..Default::default()
                        },
                        pb::SessionInfo {
                            id: "b".into(),
                            name: "Second".into(),
                            host: "builder".into(),
                            archived: true,
                            ..Default::default()
                        },
                    ],
                }),
                _ => humata_wire::error(pb::ErrorCode::Unspecified, "not in this test"),
            };
            Reply::Response(kind)
        })
    }
}

#[tokio::test]
async fn the_live_daemon_lists_what_humatad_answers_on_its_socket() {
    let folder = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let socket = folder.path().join("humatad.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (reader, writer) = stream.into_split();
            tokio::spawn(humata_wire::serve(
                reader,
                writer,
                pb::Role::Daemon,
                "test",
                Arc::new(Listing),
            ));
        }
    });

    let daemon = LiveDaemon::new(Some(socket));
    let sessions = daemon.sessions().await.unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].state, SessionState::Working);
    assert!(sessions[0].unread);
    assert_eq!(sessions[1].host, "builder");
    assert!(sessions[1].archived);
    // Asked again, it reuses the connection and still answers.
    assert_eq!(daemon.sessions().await.unwrap().len(), 2);
}

#[tokio::test]
async fn the_live_daemon_says_it_couldnt_connect_when_nothing_listens() {
    let folder = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let daemon = LiveDaemon::new(Some(folder.path().join("nobody.sock")));
    assert!(matches!(
        daemon.sessions().await,
        Err(DaemonError::Connect { .. })
    ));
}
