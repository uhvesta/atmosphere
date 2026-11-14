//! Agents start sessions of their own and write to other sessions only as
//! their project lets them, and within its limits.

use std::sync::Arc;

use humata_wire::{Client, ClientError, pb};

use super::{
    after_turns, client_of, daemon, daemon_itself, git_repository, item, project, refused_with,
    start_on, started,
};
use crate::daemon::{Connection, Daemon};
use crate::hosts::Hosts;
use crate::peer::Peer;

/// A daemon with a project, a task in it and a session on the task.
struct Setup {
    client: Client,
    project: String,
    task: String,
    session: pb::SessionInfo,
    _state: tempfile::TempDir,
    _repository: tempfile::TempDir,
}

async fn setup() -> Setup {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await.id;
    let task = item(&client, &project, "", "login").await.id;
    let session = started(&client, start_on(&task)).await;
    Setup {
        client,
        project,
        task,
        session,
        _state: state,
        _repository: repository,
    }
}

/// Sets what `project`'s agents may do to other sessions.
async fn let_agents(client: &Client, project: &str, allowed: pb::AgentSessions) {
    let updated = client
        .call(pb::request::Kind::UpdateProject(pb::UpdateProject {
            project_id: project.to_owned(),
            agent_sessions: Some(allowed),
            ..Default::default()
        }))
        .await;
    assert!(updated.is_ok(), "{updated:?}");
}

fn allowing() -> pb::AgentSessions {
    pb::AgentSessions {
        mode: pb::AgentSessionsMode::Allow.into(),
        ..Default::default()
    }
}

async fn start_child(
    client: &Client,
    from: &str,
    name: &str,
) -> Result<pb::AgentActed, ClientError> {
    let asked = client
        .call(pb::request::Kind::AgentStartSession(
            pb::AgentStartSession {
                session_id: from.to_owned(),
                name: name.to_owned(),
                text: "hello".to_owned(),
            },
        ))
        .await?;
    match asked {
        pb::response::Kind::AgentActed(acted) => Ok(acted),
        other => panic!("{other:?}"),
    }
}

async fn write(client: &Client, from: &str, to: &str) -> Result<pb::AgentActed, ClientError> {
    let asked = client
        .call(pb::request::Kind::AgentSendMessage(pb::AgentSendMessage {
            session_id: from.to_owned(),
            to_session_id: to.to_owned(),
            text: "hello".to_owned(),
        }))
        .await?;
    match asked {
        pb::response::Kind::AgentActed(acted) => Ok(acted),
        other => panic!("{other:?}"),
    }
}

async fn sessions(client: &Client) -> Vec<pb::SessionInfo> {
    match client
        .call(pb::request::Kind::ListSessions(pb::ListSessions::default()))
        .await
    {
        Ok(pb::response::Kind::Sessions(sessions)) => sessions.sessions,
        other => panic!("{other:?}"),
    }
}

async fn requests(client: &Client) -> Vec<pb::AgentRequest> {
    match client
        .call(pb::request::Kind::ListInbox(pb::ListInbox {}))
        .await
    {
        Ok(pb::response::Kind::Inbox(inbox)) => inbox
            .entries
            .into_iter()
            .filter_map(|entry| entry.request)
            .collect(),
        other => panic!("{other:?}"),
    }
}

fn answer(request: &str, answer: pb::answer_request::Answer) -> pb::request::Kind {
    pb::request::Kind::AnswerRequest(pb::AnswerRequest {
        request_id: request.to_owned(),
        answer: Some(answer),
    })
}

/// The messages sent to a session, each with the session that sent it if
/// another's agent did.
fn received(events: &[pb::SessionEvent]) -> Vec<(Option<String>, String)> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            Some(pb::session_event::Kind::UserMessage(message)) => {
                Some(match crate::sender::sender(&message.text) {
                    Some(sent) => (Some(sent.id.to_owned()), sent.text.to_owned()),
                    None => (None, message.text.clone()),
                })
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn an_agent_allowed_to_starts_a_child_where_it_is_and_writes_to_it_as_itself() {
    let Setup {
        client,
        project,
        task,
        session,
        ..
    } = &setup().await;
    let_agents(client, project, allowing()).await;

    let acted = start_child(client, &session.id, "Tests")
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let child = acted.session.unwrap_or_else(|| panic!("nothing started"));
    assert_eq!(child.started_by_session_id, session.id);
    assert_eq!(
        (child.project_id.as_str(), child.item_id.as_str()),
        (project.as_str(), task.as_str())
    );
    assert_eq!(child.name, "Tests");
    // It is no fork: it has none of its parent's history.
    assert!(child.parent_id.is_empty());

    let events = after_turns(client, &child.id, 1).await;
    assert_eq!(
        received(&events),
        [(Some(session.id.clone()), "hello".to_owned())]
    );
    let listed = sessions(client).await;
    assert!(
        listed
            .iter()
            .any(|listed| listed.id == child.id && listed.started_by_session_id == session.id),
        "{listed:?}"
    );

    // Its own child takes its messages without more ado.
    let sent = write(client, &session.id, &child.id).await;
    assert!(
        matches!(&sent, Ok(acted) if acted.session.is_some()),
        "{sent:?}"
    );
    let events = after_turns(client, &child.id, 2).await;
    assert_eq!(received(&events).len(), 2);
    assert!(requests(client).await.is_empty());
}

#[tokio::test]
async fn by_default_each_start_waits_for_the_person_and_declining_tells_the_agent() {
    let Setup {
        client, session, ..
    } = &setup().await;

    let acted = start_child(client, &session.id, "Tests")
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(acted.session.is_none());
    let asked = requests(client).await;
    let [request] = asked.as_slice() else {
        panic!("{asked:?}")
    };
    assert_eq!(request.kind(), pb::AgentRequestKind::StartSession);
    assert_eq!(
        (request.name.as_str(), request.text.as_str()),
        ("Tests", "hello")
    );
    assert_eq!(sessions(client).await.len(), 1);

    let declined = client
        .call(answer(
            &request.id,
            pb::answer_request::Answer::Dismiss(true),
        ))
        .await;
    assert!(declined.is_ok(), "{declined:?}");
    assert!(requests(client).await.is_empty());
    assert_eq!(sessions(client).await.len(), 1);
    // The agent hears of it, as its next message.
    let events = after_turns(client, &session.id, 1).await;
    assert_eq!(received(&events).len(), 1);
}

#[tokio::test]
async fn allowing_a_message_sends_it_as_the_agents_and_tells_the_agent() {
    let Setup {
        client,
        project,
        task,
        session,
        ..
    } = &setup().await;
    let other = started(client, start_on(task)).await;
    let_agents(
        client,
        project,
        pb::AgentSessions {
            mode: pb::AgentSessionsMode::Ask.into(),
            message_others: true,
            ..Default::default()
        },
    )
    .await;

    let acted = write(client, &session.id, &other.id)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let request = acted.request.unwrap_or_else(|| panic!("nothing waits"));
    assert_eq!(request.kind(), pb::AgentRequestKind::SendMessage);
    assert_eq!(request.target_session_id, other.id);

    let allowed = client
        .call(answer(
            &request.id,
            pb::answer_request::Answer::Approve(true),
        ))
        .await;
    assert!(allowed.is_ok(), "{allowed:?}");
    let events = after_turns(client, &other.id, 1).await;
    assert_eq!(
        received(&events),
        [(Some(session.id.clone()), "hello".to_owned())]
    );
    assert!(requests(client).await.is_empty());
    let told = after_turns(client, &session.id, 1).await;
    assert_eq!(received(&told).len(), 1);
    // Allowed once; again, it is no longer there.
    let again = client
        .call(answer(
            &request.id,
            pb::answer_request::Answer::Approve(true),
        ))
        .await;
    assert!(
        refused_with(&again, pb::ErrorCode::UnknownRequest),
        "{again:?}"
    );
}

#[tokio::test]
async fn with_agents_sessions_off_nothing_starts_is_sent_or_waits() {
    let Setup {
        client,
        project,
        task,
        session,
        ..
    } = &setup().await;
    let other = started(client, start_on(task)).await;
    let_agents(
        client,
        project,
        pb::AgentSessions {
            mode: pb::AgentSessionsMode::Off.into(),
            message_others: true,
            ..Default::default()
        },
    )
    .await;

    let start = start_child(client, &session.id, "Tests").await;
    assert!(matches!(
        start,
        Err(ClientError::Refused {
            code: pb::ErrorCode::InvalidRequest,
            ..
        })
    ));
    let sent = write(client, &session.id, &other.id).await;
    assert!(sent.is_err(), "{sent:?}");
    assert!(requests(client).await.is_empty());
    assert_eq!(sessions(client).await.len(), 2);
    // Its sessions are listed as having none of it, so their agents aren't
    // offered the tools.
    assert!(
        sessions(client)
            .await
            .iter()
            .all(|listed| listed.agent_sessions() == pb::AgentSessionsMode::Off)
    );
}

#[tokio::test]
async fn a_session_starts_no_more_children_than_its_project_allows_nor_deeper() {
    let Setup {
        client,
        project,
        session,
        ..
    } = &setup().await;
    let_agents(
        client,
        project,
        pb::AgentSessions {
            mode: pb::AgentSessionsMode::Allow.into(),
            message_others: false,
            max_children: 1,
            max_depth: 1,
        },
    )
    .await;

    let child = start_child(client, &session.id, "First")
        .await
        .unwrap_or_else(|error| panic!("{error}"))
        .session
        .unwrap_or_else(|| panic!("nothing started"));
    after_turns(client, &child.id, 1).await;

    // One child is all it may have.
    let second = start_child(client, &session.id, "Second").await;
    assert!(second.is_err(), "{second:?}");
    // And its child is as deep as agents' sessions go.
    let deeper = start_child(client, &child.id, "Deeper").await;
    assert!(deeper.is_err(), "{deeper:?}");
    assert_eq!(sessions(client).await.len(), 2);

    // An archived child makes room for another.
    let archived = client
        .call(pb::request::Kind::UpdateSession(pb::UpdateSession {
            session_id: child.id.clone(),
            archived: Some(true),
            ..Default::default()
        }))
        .await;
    assert!(archived.is_ok(), "{archived:?}");
    let third = start_child(client, &session.id, "Third").await;
    assert!(
        matches!(&third, Ok(acted) if acted.session.is_some()),
        "{third:?}"
    );
}

#[tokio::test]
async fn starts_that_wait_for_the_person_count_toward_the_limit() {
    let Setup {
        client,
        project,
        session,
        ..
    } = &setup().await;
    let_agents(
        client,
        project,
        pb::AgentSessions {
            mode: pb::AgentSessionsMode::Ask.into(),
            message_others: false,
            max_children: 1,
            max_depth: 0,
        },
    )
    .await;

    let first = start_child(client, &session.id, "First").await;
    assert!(
        matches!(&first, Ok(acted) if acted.request.is_some()),
        "{first:?}"
    );
    let second = start_child(client, &session.id, "Second").await;
    assert!(second.is_err(), "{second:?}");
    assert_eq!(requests(client).await.len(), 1);

    // The one waiting can still be allowed.
    let request = requests(client).await.remove(0);
    let allowed = client
        .call(answer(
            &request.id,
            pb::answer_request::Answer::Approve(true),
        ))
        .await;
    assert!(allowed.is_ok(), "{allowed:?}");
    assert_eq!(sessions(client).await.len(), 2);
}

#[tokio::test]
async fn an_agent_writes_only_to_its_own_children_unless_its_project_lets_it_write_to_others() {
    let Setup {
        client,
        project,
        task,
        session,
        ..
    } = &setup().await;
    let other = started(client, start_on(task)).await;
    let_agents(client, project, allowing()).await;

    let refused = write(client, &session.id, &other.id).await;
    assert!(refused.is_err(), "{refused:?}");
    let itself = write(client, &session.id, &session.id).await;
    assert!(itself.is_err(), "{itself:?}");
    let unknown = write(client, &session.id, "no-such-session").await;
    assert!(unknown.is_err(), "{unknown:?}");

    let_agents(
        client,
        project,
        pb::AgentSessions {
            message_others: true,
            ..allowing()
        },
    )
    .await;
    let sent = write(client, &session.id, &other.id).await;
    assert!(
        matches!(&sent, Ok(acted) if acted.session.is_some()),
        "{sent:?}"
    );
    let events = after_turns(client, &other.id, 1).await;
    assert_eq!(
        received(&events),
        [(Some(session.id.clone()), "hello".to_owned())]
    );
    // Still not to itself.
    assert!(write(client, &session.id, &session.id).await.is_err());
}

#[tokio::test]
async fn only_the_app_allows_what_an_agent_asked_or_changes_what_agents_may_do() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let daemon: Arc<Daemon> = daemon_itself(state.path(), Hosts::default()).await;
    let app = client_of(Connection::new(daemon.clone(), Peer::App)).await;
    let agent = client_of(Connection::new(
        daemon,
        Peer::Client {
            program: "humata mcp".to_owned(),
            parent: "claude".to_owned(),
            surface: pb::ChangeSurface::Mcp,
        },
    ))
    .await;
    let project = project(&app, repository.path()).await.id;
    let task = item(&app, &project, "", "login").await.id;
    let session = started(&app, start_on(&task)).await;

    let widened = agent
        .call(pb::request::Kind::UpdateProject(pb::UpdateProject {
            project_id: project.clone(),
            agent_sessions: Some(allowing()),
            ..Default::default()
        }))
        .await;
    assert!(
        refused_with(&widened, pb::ErrorCode::AppOnly),
        "{widened:?}"
    );

    // The agent asks, as its humata tools do, over a connection like this.
    let asked = start_child(&agent, &session.id, "Tests").await;
    assert!(
        matches!(&asked, Ok(acted) if acted.request.is_some()),
        "{asked:?}"
    );
    let request = requests(&app).await.remove(0);
    let allow = || answer(&request.id, pb::answer_request::Answer::Approve(true));
    let by_agent = agent.call(allow()).await;
    assert!(
        refused_with(&by_agent, pb::ErrorCode::AppOnly),
        "{by_agent:?}"
    );
    assert_eq!(sessions(&app).await.len(), 1);

    let by_app = app.call(allow()).await;
    assert!(by_app.is_ok(), "{by_app:?}");
    assert_eq!(sessions(&app).await.len(), 2);
}
