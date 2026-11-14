//! Agents start sessions of their own and write to other sessions only as
//! their project lets them, and within its limits.

use std::sync::Arc;

use humata_wire::{Client, ClientError, pb};

use super::{
    after_turns, client_of, daemon, daemon_itself, git_repository, history_of, item, project,
    refused_with, send, sent_to, start_on, started, waiting,
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
    write_text(client, from, to, "hello").await
}

async fn write_text(
    client: &Client,
    from: &str,
    to: &str,
    text: &str,
) -> Result<pb::AgentActed, ClientError> {
    let asked = client
        .call(pb::request::Kind::AgentSendMessage(pb::AgentSendMessage {
            session_id: from.to_owned(),
            to_session_id: to.to_owned(),
            text: text.to_owned(),
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

#[tokio::test]
async fn a_side_chat_starts_no_session_and_writes_to_none_whatever_its_project_allows() {
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
            message_others: true,
            ..allowing()
        },
    )
    .await;
    send(client, &session.id, "hello").await;
    after_turns(client, &session.id, 1).await;
    let side = match client
        .call(pb::request::Kind::ForkSession(pb::ForkSession {
            session_id: session.id.clone(),
            read_only: true,
            ..Default::default()
        }))
        .await
    {
        Ok(pb::response::Kind::SessionForked(pb::SessionForked {
            session: Some(side),
        })) => side,
        other => panic!("not forked: {other:?}"),
    };
    assert!(side.read_only);

    let start = start_child(client, &side.id, "Tests").await;
    assert!(start.is_err(), "{start:?}");
    let sent = write(client, &side.id, &session.id).await;
    assert!(sent.is_err(), "{sent:?}");
    // Nothing started, nothing waits, and the session it is beside was
    // sent only what the person sent it.
    assert_eq!(sessions(client).await.len(), 2);
    assert!(requests(client).await.is_empty());
    assert_eq!(sent_to(client, &session.id).await, ["hello"]);

    // Nor where each would wait for the person.
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
    assert!(start_child(client, &side.id, "Tests").await.is_err());
    assert!(write(client, &side.id, &session.id).await.is_err());
    assert!(requests(client).await.is_empty());
}

/// The messages waiting in a session's queue as they last stood, in the
/// order they were queued.
fn queued(events: &[pb::SessionEvent]) -> Vec<String> {
    let mut waiting: Vec<(String, String)> = Vec::new();
    for event in events {
        if let Some(pb::session_event::Kind::MessageQueued(queued)) = &event.kind {
            match waiting.iter_mut().find(|(id, _)| *id == queued.queue_id) {
                Some((_, text)) => text.clone_from(&queued.text),
                None => waiting.push((queued.queue_id.clone(), queued.text.clone())),
            }
        }
    }
    waiting.into_iter().map(|(_, text)| text).collect()
}

#[tokio::test]
async fn an_agents_message_to_a_working_session_waits_apart_from_what_its_person_queued() {
    let Setup {
        client,
        project,
        task,
        session,
        ..
    } = &setup().await;
    let_agents(
        client,
        project,
        pb::AgentSessions {
            message_others: true,
            ..allowing()
        },
    )
    .await;
    // A session held mid-turn, with a message of the person's waiting.
    let busy = {
        let mut request = start_on(task);
        if let pb::request::Kind::StartSession(start) = &mut request {
            start.backend_config = r#"{"rules": [
                {"when": "write", "steps": [{"step": "write_file", "path": "a.txt", "contents": "a"}]},
                {"when": "", "steps": [{"step": "say", "text": "ok"}]}
            ]}"#
            .to_owned();
        }
        started(client, request).await
    };
    send(client, &busy.id, "write").await;
    waiting(client, &busy.id).await;
    send(client, &busy.id, "then the tests").await;

    let sent = write(client, &session.id, &busy.id).await;
    assert!(
        matches!(&sent, Ok(acted) if acted.session.is_some()),
        "{sent:?}"
    );
    // And what the person writes after doesn't join the agent's.
    send(client, &busy.id, "and the docs").await;

    let waits = queued(&history_of(client, &busy.id).await);
    let [persons, agents, after] = waits.as_slice() else {
        panic!("{waits:?}")
    };
    // The person's is as they wrote it; the agent's is its own, named as
    // the agent's.
    assert_eq!(persons, "then the tests");
    let from = crate::sender::sender(agents).unwrap_or_else(|| panic!("no sender in {agents:?}"));
    assert_eq!((from.id, from.text), (session.id.as_str(), "hello"));
    assert_eq!(after, "and the docs");
}

#[tokio::test]
async fn only_so_many_messages_wait_for_the_person_and_none_over_the_size() {
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

    // A message far over the size waits nowhere, and neither does a
    // session asked to start with one.
    let long = "word ".repeat(20_000);
    let refused = write_text(client, &session.id, &other.id, &long).await;
    assert!(refused.is_err(), "{refused:?}");
    let refused = client
        .call(pb::request::Kind::AgentStartSession(
            pb::AgentStartSession {
                session_id: session.id.clone(),
                name: "Tests".to_owned(),
                text: long,
            },
        ))
        .await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(requests(client).await.is_empty());

    // Messages wait until one more is refused.
    let mut kept = 0usize;
    while write(client, &session.id, &other.id).await.is_ok() {
        kept += 1;
        assert!(kept < 1000, "nothing limits the messages waiting");
    }
    assert!(kept > 0);
    assert_eq!(requests(client).await.len(), kept);

    // Answering one makes room for one.
    let request = requests(client).await.remove(0);
    let declined = client
        .call(answer(
            &request.id,
            pb::answer_request::Answer::Dismiss(true),
        ))
        .await;
    assert!(declined.is_ok(), "{declined:?}");
    assert!(write(client, &session.id, &other.id).await.is_ok());
    assert!(write(client, &session.id, &other.id).await.is_err());
    assert_eq!(requests(client).await.len(), kept);
}

#[tokio::test]
async fn what_became_of_a_request_reaches_its_agent_as_humatas_notice() {
    let Setup {
        client, session, ..
    } = &setup().await;
    let acted = start_child(client, &session.id, "Tests")
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let request = acted.request.unwrap_or_else(|| panic!("nothing waits"));
    let declined = client
        .call(answer(
            &request.id,
            pb::answer_request::Answer::Dismiss(true),
        ))
        .await;
    assert!(declined.is_ok(), "{declined:?}");

    let events = after_turns(client, &session.id, 1).await;
    let told = sent_to(client, &session.id).await;
    let [told] = told.as_slice() else {
        panic!("{told:?}")
    };
    // It is marked as Humata's: neither the person's own words nor
    // another agent's.
    assert!(crate::sender::notice(told).is_some(), "{told:?}");
    assert!(crate::sender::sender(told).is_none(), "{told:?}");
    assert_eq!(received(&events).len(), 1);
}

#[tokio::test]
async fn allowing_a_start_past_the_projects_limit_is_refused_and_it_stays_in_the_inbox() {
    let Setup {
        client,
        project,
        session,
        ..
    } = &setup().await;
    let asking = |max_children| pb::AgentSessions {
        mode: pb::AgentSessionsMode::Ask.into(),
        message_others: false,
        max_children,
        max_depth: 0,
    };
    let_agents(client, project, asking(2)).await;
    for name in ["First", "Second"] {
        let asked = start_child(client, &session.id, name).await;
        assert!(
            matches!(&asked, Ok(acted) if acted.request.is_some()),
            "{asked:?}"
        );
    }
    // The person lowers the limit before answering either.
    let_agents(client, project, asking(1)).await;

    let request = requests(client).await.remove(0);
    let allowed = client
        .call(answer(
            &request.id,
            pb::answer_request::Answer::Approve(true),
        ))
        .await;
    assert!(allowed.is_err(), "{allowed:?}");
    assert_eq!(sessions(client).await.len(), 1);
    assert_eq!(requests(client).await.len(), 2);

    // Declining the other makes room, and it can be allowed after all.
    let other = requests(client).await.remove(1);
    let declined = client
        .call(answer(&other.id, pb::answer_request::Answer::Dismiss(true)))
        .await;
    assert!(declined.is_ok(), "{declined:?}");
    let allowed = client
        .call(answer(
            &request.id,
            pb::answer_request::Answer::Approve(true),
        ))
        .await;
    assert!(allowed.is_ok(), "{allowed:?}");
    assert_eq!(sessions(client).await.len(), 2);
}

/// What `count` messages from `from`'s agent to `to`, sent together, came
/// to.
async fn written_together(
    client: &Arc<Client>,
    from: &str,
    to: &str,
    count: usize,
) -> Vec<Result<pb::AgentActed, ClientError>> {
    let mut sending = tokio::task::JoinSet::new();
    for _ in 0..count {
        let (client, from, to) = (Arc::clone(client), from.to_owned(), to.to_owned());
        sending.spawn(async move { write(&client, &from, &to).await });
    }
    let mut results = Vec::new();
    while let Some(done) = sending.join_next().await {
        results.push(done.unwrap_or_else(|error| panic!("{error}")));
    }
    results
}

#[tokio::test]
async fn starts_sent_together_are_counted_one_after_another() {
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
            max_children: 2,
            max_depth: 1,
        },
    )
    .await;

    let (first, second, third, fourth) = tokio::join!(
        start_child(client, &session.id, "First"),
        start_child(client, &session.id, "Second"),
        start_child(client, &session.id, "Third"),
        start_child(client, &session.id, "Fourth"),
    );
    let asked = [first, second, third, fourth];
    let started = asked
        .iter()
        .filter(|asked| matches!(asked, Ok(acted) if acted.session.is_some()))
        .count();
    let refused = asked.iter().filter(|asked| asked.is_err()).count();
    assert_eq!((started, refused), (2, 2), "{asked:?}");
    // The asking session and the two it may have.
    assert_eq!(sessions(client).await.len(), 3);
}

#[tokio::test]
async fn a_start_allowed_twice_together_starts_one_session() {
    let Setup {
        client, session, ..
    } = &setup().await;
    let acted = start_child(client, &session.id, "Tests")
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let request = acted.request.unwrap_or_else(|| panic!("nothing waits"));

    let allow = || {
        client.call(answer(
            &request.id,
            pb::answer_request::Answer::Approve(true),
        ))
    };
    let (first, second) = tokio::join!(allow(), allow());
    assert!(
        first.is_ok() != second.is_ok(),
        "one allows it and the other finds it answered: {first:?} {second:?}"
    );
    assert_eq!(sessions(client).await.len(), 2);
    assert!(requests(client).await.is_empty());
}

#[tokio::test]
async fn a_start_declined_while_it_is_being_allowed_is_refused_and_started_once() {
    let Setup {
        client, session, ..
    } = &setup().await;
    let acted = start_child(client, &session.id, "Tests")
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let request = acted.request.unwrap_or_else(|| panic!("nothing waits"));

    // The Allow is sent first and takes as long as making the session's
    // copy, so the decline finds it under way. Should the decline get in
    // first all the same, it is the Allow that finds nothing to allow.
    let (allowed, declined) = tokio::join!(
        client.call(answer(
            &request.id,
            pb::answer_request::Answer::Approve(true),
        )),
        client.call(answer(
            &request.id,
            pb::answer_request::Answer::Dismiss(true),
        )),
    );

    assert!(
        allowed.is_ok() != declined.is_ok(),
        "one of them is refused: {allowed:?} {declined:?}"
    );
    // A session is there exactly when the Allow went through, never one
    // that was both started and declined.
    let expected = if allowed.is_ok() { 2 } else { 1 };
    assert_eq!(sessions(client).await.len(), expected);
    assert!(requests(client).await.is_empty());
}

#[tokio::test]
async fn messages_sent_together_wait_for_the_person_only_up_to_the_limit() {
    let setup = setup().await;
    let (client, session) = (Arc::new(setup.client), setup.session);
    let other = started(&client, start_on(&setup.task)).await;
    let_agents(
        &client,
        &setup.project,
        pb::AgentSessions {
            mode: pb::AgentSessionsMode::Ask.into(),
            message_others: true,
            ..Default::default()
        },
    )
    .await;
    // How many wait when they come one at a time.
    let mut limit = 0usize;
    while write(&client, &session.id, &other.id).await.is_ok() {
        limit += 1;
        assert!(limit < 1000, "nothing limits the messages waiting");
    }
    for request in requests(&client).await {
        let declined = client
            .call(answer(
                &request.id,
                pb::answer_request::Answer::Dismiss(true),
            ))
            .await;
        assert!(declined.is_ok(), "{declined:?}");
    }
    assert!(requests(&client).await.is_empty());

    // Sent together, no more of them wait.
    let asked = written_together(&client, &session.id, &other.id, limit + 4).await;
    let kept = asked.iter().filter(|asked| asked.is_ok()).count();
    assert_eq!(kept, limit, "{asked:?}");
    assert_eq!(requests(&client).await.len(), limit);
}

#[tokio::test]
async fn an_agents_message_keeps_its_senders_line_and_a_spec_note_waits_for_the_person() {
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
            message_others: true,
            ..allowing()
        },
    )
    .await;
    send(client, &other.id, "hello").await;
    after_turns(client, &other.id, 1).await;
    // The task's spec changes, which the other session is to hear of.
    let changed = client
        .call(pb::request::Kind::UpdateItem(pb::UpdateItem {
            item_id: task.clone(),
            spec: Some("Sign in with a passkey.".to_owned()),
            ..Default::default()
        }))
        .await;
    assert!(changed.is_ok(), "{changed:?}");

    // The agent's message comes first, and is still the agent's.
    let sent = write(client, &session.id, &other.id).await;
    assert!(sent.is_ok(), "{sent:?}");
    let events = after_turns(client, &other.id, 2).await;
    assert_eq!(
        received(&events).get(1),
        Some(&(Some(session.id.clone()), "hello".to_owned()))
    );

    // The note comes with what the person writes next.
    send(client, &other.id, "hello").await;
    let events = after_turns(client, &other.id, 3).await;
    let got = received(&events);
    let Some((None, persons)) = got.get(2) else {
        panic!("{got:?}")
    };
    assert!(
        persons.contains(task.as_str()) && persons.contains("Sign in with a passkey."),
        "{persons:?}"
    );
}

#[tokio::test]
async fn an_agent_writes_to_others_only_so_often_until_the_person_writes_to_it() {
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
            message_others: true,
            ..allowing()
        },
    )
    .await;

    let mut sent = 0usize;
    while write(client, &session.id, &other.id).await.is_ok() {
        sent += 1;
        assert!(sent < 1000, "nothing limits an agent's messages");
    }
    assert!(sent > 0);
    // The one refused never reached the other session.
    let events = after_turns(client, &other.id, sent).await;
    assert_eq!(received(&events).len(), sent);
    assert!(write(client, &session.id, &other.id).await.is_err());

    // The person writes to the asking session, and its agent may again.
    send(client, &session.id, "hello").await;
    let again = write(client, &session.id, &other.id).await;
    assert!(
        matches!(&again, Ok(acted) if acted.session.is_some()),
        "{again:?}"
    );
}

fn move_to_done(session: &str) -> pb::request::Kind {
    pb::request::Kind::RequestMove(pb::RequestMove {
        session_id: session.to_owned(),
        status: pb::ItemStatus::Done.into(),
        summary: "Login works.".to_owned(),
        ..Default::default()
    })
}

#[tokio::test]
async fn a_side_chat_asks_for_no_move_of_the_task_it_is_beside() {
    let Setup {
        client, session, ..
    } = &setup().await;
    send(client, &session.id, "hello").await;
    after_turns(client, &session.id, 1).await;
    let side = match client
        .call(pb::request::Kind::ForkSession(pb::ForkSession {
            session_id: session.id.clone(),
            read_only: true,
            ..Default::default()
        }))
        .await
    {
        Ok(pb::response::Kind::SessionForked(pb::SessionForked {
            session: Some(side),
        })) => side,
        other => panic!("not forked: {other:?}"),
    };

    let refused = client.call(move_to_done(&side.id)).await;
    assert!(refused.is_err(), "{refused:?}");
    assert!(requests(client).await.is_empty());

    // The session it is beside still asks.
    let asked = client.call(move_to_done(&session.id)).await;
    assert!(asked.is_ok(), "{asked:?}");
    assert_eq!(requests(client).await.len(), 1);
}

#[tokio::test]
async fn nothing_over_the_size_reaches_the_inbox() {
    let Setup {
        client, session, ..
    } = &setup().await;
    let long = "word ".repeat(20_000);

    let question = client
        .call(pb::request::Kind::AskPerson(pb::AskPerson {
            session_id: session.id.clone(),
            text: long.clone(),
            client_id: String::new(),
        }))
        .await;
    assert!(question.is_err(), "{question:?}");
    let moved = client
        .call(pb::request::Kind::RequestMove(pb::RequestMove {
            session_id: session.id.clone(),
            status: pb::ItemStatus::Done.into(),
            summary: long,
            ..Default::default()
        }))
        .await;
    assert!(moved.is_err(), "{moved:?}");
    // A session's name far longer than the sidebar shows, where a start
    // waits for the person.
    let named = start_child(client, &session.id, &"Tests ".repeat(100)).await;
    assert!(named.is_err(), "{named:?}");
    assert!(requests(client).await.is_empty());

    // What fits still waits there.
    let named = start_child(client, &session.id, "Tests").await;
    assert!(
        matches!(&named, Ok(acted) if acted.request.is_some()),
        "{named:?}"
    );
    assert_eq!(requests(client).await.len(), 1);
}
