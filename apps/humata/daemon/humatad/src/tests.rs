use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use humata_wire::{Client, ClientError, Service, pb};

use crate::daemon::Daemon;
use crate::host::Host;
use crate::hosts::{HostConfig, Hosts};
use crate::store::Store;

const SCRIPT: &str = r#"{"rules": [
    {"when": "hello", "steps": [{"step": "say", "text": "hi there"}]},
    {"when": "", "steps": [{"step": "say", "text": "something else"}]}
]}"#;

async fn client_of<S: Service>(service: Arc<S>) -> Client {
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    let (reader, writer) = tokio::io::split(theirs);
    tokio::spawn(humata_wire::serve(
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
    match Host::open(state, &state.join("worktrees")).await {
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
        ..Default::default()
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
        .call(pb::request::Kind::ListSessions(pb::ListSessions::default()))
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

    let mut unset = start(state.path(), "");
    if let pb::request::Kind::StartSession(start) = &mut unset {
        start.backend = pb::Backend::Unspecified.into();
    }
    let unset = client.call(unset).await;
    assert!(
        matches!(
            unset,
            Err(ClientError::Refused {
                code: pb::ErrorCode::UnknownBackend,
                ..
            })
        ),
        "{unset:?}"
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
    let store = match Store::open(&state.join("humata.sqlite3")) {
        Ok(store) => store,
        Err(error) => panic!("couldn't open the store: {error}"),
    };
    match Host::open(&state.join("host"), &state.join("worktrees")).await {
        Ok(local) => {
            client_of(Arc::new(Daemon::new(
                local,
                store,
                hosts,
                state.join("hosts.json"),
                "test".to_owned(),
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
                code: pb::ErrorCode::UnknownHost,
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
            ..HostConfig::default()
        },
    );
    hosts.hosts.insert(
        "missing".to_owned(),
        HostConfig {
            command: vec!["/nonexistent/humatad".to_owned()],
            ..HostConfig::default()
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
        .call(pb::request::Kind::ListSessions(pb::ListSessions::default()))
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

fn create_project(repository: &Path) -> pb::request::Kind {
    pb::request::Kind::CreateProject(pb::CreateProject {
        name: "humata".to_owned(),
        repositories: vec![pb::Repository {
            name: String::new(),
            path: repository.display().to_string(),
        }],
        instructions: "be brief".to_owned(),
    })
}

/// A git repository with one commit, in a new temporary folder.
fn git_repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    std::fs::write(dir.path().join("README.md"), "hello\n")
        .unwrap_or_else(|error| panic!("{error}"));
    for arguments in [
        &["init", "--quiet", "--initial-branch=main"][..],
        &["add", "README.md"],
        &["commit", "--quiet", "-m", "first"],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(arguments)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@localhost")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@localhost")
            .status()
            .unwrap_or_else(|error| panic!("{error}"));
        assert!(status.success(), "git {arguments:?}");
    }
    dir
}

async fn project(client: &Client, repository: &Path) -> pb::Project {
    match client.call(create_project(repository)).await {
        Ok(pb::response::Kind::ProjectCreated(pb::ProjectCreated {
            project: Some(project),
        })) => project,
        other => panic!("not created: {other:?}"),
    }
}

fn start_in(project: &str) -> pb::request::Kind {
    pb::request::Kind::StartSession(pb::StartSession {
        backend: pb::Backend::Scripted.into(),
        project_id: project.to_owned(),
        backend_config: SCRIPT.to_owned(),
        ..Default::default()
    })
}

async fn sessions_in(client: &Client, project: &str) -> Vec<String> {
    let listed = client
        .call(pb::request::Kind::ListSessions(pb::ListSessions {
            project_id: project.to_owned(),
        }))
        .await;
    let Ok(pb::response::Kind::Sessions(sessions)) = listed else {
        panic!("{listed:?}")
    };
    let mut ids: Vec<_> = sessions
        .sessions
        .into_iter()
        .map(|session| session.id)
        .collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn a_session_in_a_project_works_in_its_own_copy_and_lists_under_it() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let inside = started(&client, start_in(&project.id)).await;
    let outside = started(&client, start(state.path(), "")).await;
    assert_eq!(inside.project_id, project.id);
    assert_ne!(
        inside.working_directory,
        repository.path().display().to_string()
    );
    send(&client, &inside.id, "hello").await;
    after_turns(&client, &inside.id, 1).await;
    let copy = PathBuf::from(&inside.working_directory);
    assert!(copy.join("README.md").is_file(), "{copy:?}");
    assert_eq!(outside.project_id, "");
    let fork = client
        .call(pb::request::Kind::ForkSession(pb::ForkSession {
            session_id: inside.id.clone(),
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::SessionForked(pb::SessionForked {
        session: Some(fork),
    })) = fork
    else {
        panic!("not forked: {fork:?}")
    };
    assert_eq!(fork.project_id, project.id);
    let mut expected = vec![inside.id.clone(), fork.id.clone()];
    expected.sort();
    assert_eq!(sessions_in(&client, &project.id).await, expected);
    assert_eq!(sessions_in(&client, "").await.len(), 3);
}

#[tokio::test]
async fn projects_and_their_sessions_survive_a_restart() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let (project, session) = {
        let client = daemon(state.path(), Hosts::default()).await;
        let project = project(&client, repository.path()).await;
        let session = started(&client, start_in(&project.id)).await;
        (project, session)
    };
    let client = daemon(state.path(), Hosts::default()).await;
    let listed = client
        .call(pb::request::Kind::ListProjects(pb::ListProjects {}))
        .await;
    let Ok(pb::response::Kind::Projects(projects)) = listed else {
        panic!("{listed:?}")
    };
    assert_eq!(projects.projects, vec![project.clone()]);
    assert_eq!(sessions_in(&client, &project.id).await, vec![session.id]);
}

#[tokio::test]
async fn a_project_needs_a_folder_on_this_mac() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), Hosts::default()).await;
    let file = state.path().join("file");
    std::fs::write(&file, "").unwrap_or_else(|error| panic!("{error}"));
    for repository in [
        state.path().join("missing"),
        file,
        PathBuf::from("relative"),
    ] {
        let created = client.call(create_project(&repository)).await;
        assert!(
            matches!(
                created,
                Err(ClientError::Refused {
                    code: pb::ErrorCode::InvalidRequest,
                    ..
                })
            ),
            "{repository:?}: {created:?}"
        );
    }
}

async fn update(
    client: &Client,
    update: pb::UpdateSession,
) -> Result<pb::SessionInfo, ClientError> {
    match client
        .call(pb::request::Kind::UpdateSession(update))
        .await?
    {
        pb::response::Kind::SessionUpdated(pb::SessionUpdated {
            session: Some(session),
        }) => Ok(session),
        other => panic!("not updated: {other:?}"),
    }
}

#[tokio::test]
async fn an_archived_session_is_put_away_and_refuses_messages() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let session = started(&client, start_in(&project.id)).await;
    send(&client, &session.id, "hello").await;
    after_turns(&client, &session.id, 1).await;

    let archived = update(
        &client,
        pb::UpdateSession {
            session_id: session.id.clone(),
            name: Some("put away".to_owned()),
            archived: Some(true),
            keeper: None,
            item_id: None,
        },
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));
    assert!(archived.archived);
    assert_eq!(archived.name, "put away");
    assert_eq!(archived.project_id, project.id);
    assert!(archived.checkouts.is_empty());
    assert!(!PathBuf::from(&session.working_directory).exists());
    let sent = client
        .call(pb::request::Kind::SendMessage(pb::SendMessage {
            session_id: session.id.clone(),
            text: "hello".to_owned(),
            ..Default::default()
        }))
        .await;
    assert!(
        matches!(
            sent,
            Err(ClientError::Refused {
                code: pb::ErrorCode::Archived,
                ..
            })
        ),
        "{sent:?}"
    );
    let changes = client
        .call(pb::request::Kind::ReadChanges(pb::ReadChanges {
            session_id: session.id.clone(),
        }))
        .await;
    let Ok(pb::response::Kind::Changes(changes)) = changes else {
        panic!("{changes:?}")
    };
    assert_eq!(changes.repositories.len(), 1);
}

#[tokio::test]
async fn a_project_of_several_repositories_names_each_folder_apart() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let (one, two) = (git_repository(), git_repository());
    let client = daemon(state.path(), Hosts::default()).await;
    let repository = |name: &str, path: &Path| pb::Repository {
        name: name.to_owned(),
        path: path.display().to_string(),
    };
    let created = client
        .call(pb::request::Kind::CreateProject(pb::CreateProject {
            name: "both".to_owned(),
            repositories: vec![repository("app", one.path()), repository("app", two.path())],
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::ProjectCreated(pb::ProjectCreated {
        project: Some(project),
    })) = created
    else {
        panic!("{created:?}")
    };
    let names: Vec<_> = project
        .repositories
        .iter()
        .map(|repository| repository.name.as_str())
        .collect();
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1]);

    let session = started(&client, start_in(&project.id)).await;
    send(&client, &session.id, "hello").await;
    after_turns(&client, &session.id, 1).await;
    let folder = PathBuf::from(&session.working_directory);
    for name in names {
        assert!(folder.join(name).join("README.md").is_file(), "{name}");
    }
}

#[tokio::test]
async fn a_project_needs_a_repository() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), Hosts::default()).await;
    let created = client
        .call(pb::request::Kind::CreateProject(pb::CreateProject {
            name: "empty".to_owned(),
            ..Default::default()
        }))
        .await;
    assert!(
        matches!(
            created,
            Err(ClientError::Refused {
                code: pb::ErrorCode::InvalidRequest,
                ..
            })
        ),
        "{created:?}"
    );
}

#[tokio::test]
async fn starting_in_an_unknown_project_is_refused() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = daemon(state.path(), Hosts::default()).await;
    let started = client.call(start_in("nope")).await;
    assert!(
        matches!(
            started,
            Err(ClientError::Refused {
                code: pb::ErrorCode::UnknownProject,
                ..
            })
        ),
        "{started:?}"
    );
}

#[tokio::test]
async fn a_host_refuses_project_requests() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let client = host(state.path()).await;
    let created = client.call(create_project(state.path())).await;
    assert!(
        matches!(
            created,
            Err(ClientError::Refused {
                code: pb::ErrorCode::InvalidRequest,
                ..
            })
        ),
        "{created:?}"
    );
}

async fn create_item(
    client: &Client,
    project: &str,
    parent: &str,
    title: &str,
) -> Result<pb::Item, ClientError> {
    let request = pb::request::Kind::CreateItem(pb::CreateItem {
        project_id: project.to_owned(),
        parent_id: parent.to_owned(),
        title: title.to_owned(),
        spec: format!("All about {title}."),
        ..Default::default()
    });
    match client.call(request).await? {
        pb::response::Kind::ItemCreated(pb::ItemCreated { item: Some(item) }) => Ok(item),
        other => panic!("not created: {other:?}"),
    }
}

async fn item(client: &Client, project: &str, parent: &str, title: &str) -> pb::Item {
    create_item(client, project, parent, title)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
}

/// Each item's title and shown status, in the board's order.
async fn board(client: &Client, project: &str) -> Vec<(String, pb::ItemStatus)> {
    let listed = client
        .call(pb::request::Kind::ListItems(pb::ListItems {
            project_id: project.to_owned(),
        }))
        .await;
    let Ok(pb::response::Kind::Items(items)) = listed else {
        panic!("{listed:?}")
    };
    items
        .items
        .iter()
        .map(|item| (item.title.clone(), item.status()))
        .collect()
}

async fn board_log(client: &Client, project: &str) -> Vec<pb::BoardChange> {
    let request = pb::request::Kind::ReadBoardLog(pb::ReadBoardLog {
        project_id: project.to_owned(),
    });
    match client.call(request).await {
        Ok(pb::response::Kind::BoardLog(log)) => log.changes,
        other => panic!("{other:?}"),
    }
}

fn start_on(item: &str) -> pb::request::Kind {
    pb::request::Kind::StartSession(pb::StartSession {
        backend: pb::Backend::Scripted.into(),
        item_id: item.to_owned(),
        backend_config: SCRIPT.to_owned(),
        ..Default::default()
    })
}

#[tokio::test]
async fn a_session_on_a_subtask_moves_it_and_its_task() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let task = item(&client, &project.id, "", "login").await;
    let subtask = item(&client, &project.id, &task.id, "form").await;
    item(&client, &project.id, &task.id, "copy").await;
    use pb::ItemStatus::{InReview, Todo};
    let titles = |statuses: [pb::ItemStatus; 3]| {
        ["login", "form", "copy"]
            .into_iter()
            .map(str::to_owned)
            .zip(statuses)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        board(&client, &project.id).await,
        titles([Todo, Todo, Todo])
    );

    let session = started(&client, start_on(&subtask.id)).await;
    assert_eq!(session.project_id, project.id, "the item's project");
    assert_eq!(session.item_id, subtask.id);
    send(&client, &session.id, "hello").await;
    after_turns(&client, &session.id, 1).await;
    assert_eq!(
        board(&client, &project.id).await,
        titles([InReview, InReview, Todo])
    );

    // A fork stays on its parent's item.
    let fork = client
        .call(pb::request::Kind::ForkSession(pb::ForkSession {
            session_id: session.id.clone(),
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::SessionForked(pb::SessionForked {
        session: Some(fork),
    })) = fork
    else {
        panic!("{fork:?}")
    };
    assert_eq!(fork.item_id, subtask.id);

    // Moved to the project itself, and the fork archived, nothing moves
    // the board.
    for (session, archived) in [(&session.id, None), (&fork.id, Some(true))] {
        update(
            &client,
            pb::UpdateSession {
                session_id: session.clone(),
                item_id: Some(String::new()),
                archived,
                ..Default::default()
            },
        )
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    }
    assert_eq!(
        board(&client, &project.id).await,
        titles([Todo, Todo, Todo])
    );
}

#[tokio::test]
async fn done_stays_done_and_the_inbox_has_only_open_replies() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let open = item(&client, &project.id, "", "open").await;
    let finished = item(&client, &project.id, "", "finished").await;
    for item in [&open, &finished] {
        let session = started(&client, start_on(&item.id)).await;
        send(&client, &session.id, "hello").await;
        after_turns(&client, &session.id, 1).await;
    }
    let updated = client
        .call(pb::request::Kind::UpdateItem(pb::UpdateItem {
            item_id: finished.id.clone(),
            status: Some(pb::ItemStatus::Done.into()),
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::ItemUpdated(pb::ItemUpdated {
        item: Some(updated),
    })) = updated
    else {
        panic!("{updated:?}")
    };
    assert_eq!(updated.status(), pb::ItemStatus::Done);

    let inbox = client
        .call(pb::request::Kind::ListInbox(pb::ListInbox {}))
        .await;
    let Ok(pb::response::Kind::Inbox(inbox)) = inbox else {
        panic!("{inbox:?}")
    };
    let entries: Vec<(&str, &str, pb::InboxReason)> = inbox
        .entries
        .iter()
        .map(|entry| {
            (
                entry.project_name.as_str(),
                entry.item_title.as_str(),
                entry.reason(),
            )
        })
        .collect();
    assert_eq!(entries, [("humata", "open", pb::InboxReason::Reply)]);
}

#[tokio::test]
async fn every_board_change_can_be_reverted() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let task = item(&client, &project.id, "", "login").await;
    let renamed = client
        .call(pb::request::Kind::UpdateItem(pb::UpdateItem {
            item_id: task.id.clone(),
            title: Some("sign in".to_owned()),
            ..Default::default()
        }))
        .await;
    assert!(
        matches!(renamed, Ok(pb::response::Kind::ItemUpdated(_))),
        "{renamed:?}"
    );
    let revert = |change: u64| {
        client.call(pb::request::Kind::RevertBoardChange(
            pb::RevertBoardChange { change_id: change },
        ))
    };
    let changes = board_log(&client, &project.id).await;
    let [rename, add] = changes.as_slice() else {
        panic!("{changes:?}")
    };
    assert_eq!(
        rename.after.as_ref().map(|after| after.title.as_str()),
        Some("sign in")
    );
    assert!(add.before.is_none());

    revert(rename.id)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(board(&client, &project.id).await[0].0, "login");

    // Undoing the add removes it, unless something is on it.
    item(&client, &project.id, &task.id, "form").await;
    let refused = revert(add.id).await;
    assert!(
        matches!(
            refused,
            Err(ClientError::Refused {
                code: pb::ErrorCode::InvalidRequest,
                ..
            })
        ),
        "{refused:?}"
    );
    let added_subtask = board_log(&client, &project.id).await[0].id;
    revert(added_subtask)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let removed = revert(add.id).await;
    assert!(
        matches!(
            removed,
            Ok(pb::response::Kind::BoardChangeReverted(
                pb::BoardChangeReverted { item: None }
            ))
        ),
        "{removed:?}"
    );
    assert!(board(&client, &project.id).await.is_empty());

    // And undoing the removal brings it back.
    let removal = board_log(&client, &project.id).await[0].id;
    revert(removal)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(board(&client, &project.id).await.len(), 1);
}

#[tokio::test]
async fn a_subtask_of_a_subtask_and_an_unknown_item_are_refused() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let task = item(&client, &project.id, "", "login").await;
    let subtask = item(&client, &project.id, &task.id, "form").await;
    let nested = create_item(&client, &project.id, &subtask.id, "field").await;
    assert!(
        matches!(
            nested,
            Err(ClientError::Refused {
                code: pb::ErrorCode::InvalidRequest,
                ..
            })
        ),
        "{nested:?}"
    );
    let unknown = client.call(start_on("nope")).await;
    assert!(
        matches!(
            unknown,
            Err(ClientError::Refused {
                code: pb::ErrorCode::UnknownItem,
                ..
            })
        ),
        "{unknown:?}"
    );
}
