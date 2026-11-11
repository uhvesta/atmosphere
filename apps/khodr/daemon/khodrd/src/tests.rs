use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use khodr_wire::{Client, ClientError, Service, pb};

use crate::daemon::{Daemon, HostConfig, Hosts};
use crate::host::Host;

const SCRIPT: &str = r#"{"rules": [
    {"when": "hello", "steps": [{"step": "say", "text": "hi there"}]},
    {"when": "", "steps": [{"step": "say", "text": "something else"}]}
]}"#;

async fn client_of<S: Service>(service: Arc<S>) -> Client {
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    let (reader, writer) = tokio::io::split(theirs);
    tokio::spawn(khodr_wire::serve(
        reader,
        writer,
        pb::Role::Host,
        "test",
        service,
    ));
    let (reader, writer) = tokio::io::split(ours);
    match Client::connect(reader, writer, pb::Role::Daemon, "test").await {
        Ok(client) => client,
        Err(error) => panic!("couldn't connect: {error}"),
    }
}

async fn host(state: &Path) -> Client {
    match Host::open(state).await {
        Ok(host) => client_of(Arc::new(host)).await,
        Err(error) => panic!("couldn't open the host: {error}"),
    }
}

fn start(directory: &Path, host: &str) -> pb::request::Kind {
    pb::request::Kind::StartSession(pb::StartSession {
        backend: pb::Backend::Scripted.into(),
        working_directory: directory.display().to_string(),
        name: "test".to_owned(),
        backend_config: SCRIPT.to_owned(),
        host: host.to_owned(),
        ..Default::default()
    })
}

async fn started(client: &Client, request: pb::request::Kind) -> pb::SessionInfo {
    match client.call(request).await {
        Ok(pb::response::Kind::SessionStarted(pb::SessionStarted {
            session: Some(session),
        })) => session,
        other => panic!("not started: {other:?}"),
    }
}

async fn send(client: &Client, session: &str, text: &str) {
    let request = pb::request::Kind::SendMessage(pb::SendMessage {
        session_id: session.to_owned(),
        text: text.to_owned(),
        mode: pb::SendMode::Enqueue.into(),
    });
    let sent = client.call(request).await;
    assert!(
        matches!(sent, Ok(pb::response::Kind::MessageSent(_))),
        "{sent:?}"
    );
}

/// Watches `session` until `turns` turns have ended and it is idle, and
/// returns its events.
async fn after_turns(client: &Client, session: &str, turns: usize) -> Vec<pb::SessionEvent> {
    let request = pb::request::Kind::WatchSession(pb::WatchSession {
        session_id: session.to_owned(),
    });
    let Ok((_, mut stream)) = client.stream(request).await else {
        panic!("couldn't watch {session}")
    };
    let mut events = Vec::new();
    let done = |events: &[pb::SessionEvent]| {
        let ended = events
            .iter()
            .filter(|event| matches!(event.kind, Some(pb::session_event::Kind::TurnEnded(_))))
            .count();
        ended >= turns
            && matches!(
                events.last().and_then(|event| event.kind.as_ref()),
                Some(pb::session_event::Kind::Idle(_))
            )
    };
    while !done(&events) {
        match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
            Ok(Some(Ok(pb::StreamItem {
                kind: Some(pb::stream_item::Kind::SessionEvent(event)),
            }))) => events.push(event),
            other => panic!("the watch ended with {other:?}; events so far: {events:#?}"),
        }
    }
    events
}

fn said(events: &[pb::SessionEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            Some(pb::session_event::Kind::UserMessage(message)) => {
                Some(format!("user: {}", message.text))
            }
            Some(pb::session_event::Kind::AssistantMessage(message)) => {
                Some(format!("agent: {}", message.text))
            }
            _ => None,
        })
        .collect()
}

fn user_message(events: &[pb::SessionEvent], text: &str) -> String {
    events
        .iter()
        .find(|event| matches!(&event.kind, Some(pb::session_event::Kind::UserMessage(message)) if message.text == text))
        .map(|event| event.id.clone())
        .unwrap_or_else(|| panic!("no message {text:?} in {events:#?}"))
}

#[tokio::test]
async fn a_session_plays_its_script_on_the_host() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = host(state.path()).await;
    let session = started(&client, start(state.path(), "")).await;
    assert_eq!(session.state(), pb::SessionState::Idle);
    send(&client, &session.id, "hello").await;
    let events = after_turns(&client, &session.id, 1).await;
    assert_eq!(said(&events), ["user: hello", "agent: hi there"]);
}

#[tokio::test]
async fn a_fork_keeps_the_history_before_its_event_and_goes_its_own_way() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = host(state.path()).await;
    let session = started(&client, start(state.path(), "")).await;
    send(&client, &session.id, "hello").await;
    send(&client, &session.id, "and then").await;
    let events = after_turns(&client, &session.id, 2).await;
    let second = user_message(&events, "and then");

    let fork = pb::request::Kind::ForkSession(pb::ForkSession {
        session_id: session.id.clone(),
        before_event_id: second.clone(),
        name: "fork".to_owned(),
    });
    let fork = match client.call(fork).await {
        Ok(pb::response::Kind::SessionForked(pb::SessionForked {
            session: Some(fork),
        })) => fork,
        other => panic!("not forked: {other:?}"),
    };
    assert_eq!(fork.parent_id, session.id);
    assert_eq!(fork.forked_before_event_id, second);

    send(&client, &fork.id, "hello again").await;
    let forked = after_turns(&client, &fork.id, 2).await;
    assert_eq!(
        said(&forked),
        [
            "user: hello",
            "agent: hi there",
            "user: hello again",
            "agent: hi there"
        ]
    );
    // The original is untouched.
    let original = after_turns(&client, &session.id, 2).await;
    assert_eq!(
        said(&original),
        [
            "user: hello",
            "agent: hi there",
            "user: and then",
            "agent: something else"
        ]
    );
}

#[tokio::test]
async fn sessions_survive_the_host_restarting() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let id = {
        let client = host(state.path()).await;
        let session = started(&client, start(state.path(), "")).await;
        send(&client, &session.id, "hello").await;
        after_turns(&client, &session.id, 1).await;
        session.id
    };
    let client = host(state.path()).await;
    let listed = client
        .call(pb::request::Kind::ListSessions(pb::ListSessions {}))
        .await;
    let Ok(pb::response::Kind::Sessions(sessions)) = listed else {
        panic!("{listed:?}")
    };
    assert_eq!(
        sessions
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        [id.as_str()]
    );
    send(&client, &id, "more").await;
    assert_eq!(said(&after_turns(&client, &id, 2).await).len(), 4);
}

#[tokio::test]
async fn the_host_refuses_what_it_does_not_have() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = host(state.path()).await;
    let unknown = client
        .call(pb::request::Kind::ReadHistory(pb::ReadHistory {
            session_id: "nope".to_owned(),
        }))
        .await;
    assert!(
        matches!(
            unknown,
            Err(ClientError::Refused {
                code: pb::ErrorCode::UnknownSession,
                ..
            })
        ),
        "{unknown:?}"
    );

    let mut copilot = start(state.path(), "");
    if let pb::request::Kind::StartSession(start) = &mut copilot {
        start.backend = pb::Backend::Copilot.into();
    }
    let copilot = client.call(copilot).await;
    assert!(
        matches!(
            copilot,
            Err(ClientError::Refused {
                code: pb::ErrorCode::UnknownBackend,
                ..
            })
        ),
        "{copilot:?}"
    );

    let mut nowhere = start(state.path(), "");
    if let pb::request::Kind::StartSession(start) = &mut nowhere {
        start.working_directory.clear();
    }
    let nowhere = client.call(nowhere).await;
    assert!(
        matches!(
            nowhere,
            Err(ClientError::Refused {
                code: pb::ErrorCode::InvalidRequest,
                ..
            })
        ),
        "{nowhere:?}"
    );
}

async fn daemon(state: &Path, hosts: Hosts) -> Client {
    match Host::open(&state.join("host")).await {
        Ok(local) => {
            client_of(Arc::new(Daemon::new(
                local,
                hosts,
                state.join("hosts.json"),
            )))
            .await
        }
        Err(error) => panic!("couldn't open the host: {error}"),
    }
}

#[tokio::test]
async fn the_daemon_runs_sessions_on_this_mac_when_no_host_is_named() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), Hosts::default()).await;
    let session = started(&client, start(state.path(), "")).await;
    assert_eq!(session.host, "");
    send(&client, &session.id, "hello").await;
    assert_eq!(
        said(&after_turns(&client, &session.id, 1).await),
        ["user: hello", "agent: hi there"]
    );
}

#[tokio::test]
async fn the_daemon_refuses_a_host_it_does_not_know() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), Hosts::default()).await;
    let started = client.call(start(state.path(), "elsewhere")).await;
    assert!(
        matches!(
            started,
            Err(ClientError::Refused {
                code: pb::ErrorCode::InvalidRequest,
                ..
            })
        ),
        "{started:?}"
    );
}

#[tokio::test]
async fn a_host_whose_command_fails_is_unavailable_and_the_rest_still_list() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let mut hosts = Hosts::default();
    hosts.hosts.insert(
        "broken".to_owned(),
        HostConfig {
            command: vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "echo no route >&2; exit 1".to_owned(),
            ],
        },
    );
    hosts.hosts.insert(
        "missing".to_owned(),
        HostConfig {
            command: vec!["/nonexistent/khodrd".to_owned()],
        },
    );
    let client = daemon(state.path(), hosts).await;

    for host in ["broken", "missing"] {
        let started = client.call(start(state.path(), host)).await;
        assert!(
            matches!(
                started,
                Err(ClientError::Refused {
                    code: pb::ErrorCode::HostUnavailable,
                    ..
                })
            ),
            "{started:?}"
        );
    }
    let local = started(&client, start(state.path(), "")).await;
    let listed = client
        .call(pb::request::Kind::ListSessions(pb::ListSessions {}))
        .await;
    let Ok(pb::response::Kind::Sessions(sessions)) = listed else {
        panic!("{listed:?}")
    };
    assert_eq!(
        sessions
            .sessions
            .iter()
            .map(|session| session.id.as_str())
            .collect::<Vec<_>>(),
        [local.id.as_str()]
    );
}
