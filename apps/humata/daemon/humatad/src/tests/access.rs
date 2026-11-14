//! Agent Access: what the command line and agents' MCP servers may ask
//! goes by the person's switches, and the app's requests by none.

use std::sync::Arc;

use humata_wire::{Client, ClientError, pb};

use super::{client_of, daemon_itself, git_repository, project};
use crate::daemon::{Connection, Daemon};
use crate::hosts::Hosts;
use crate::peer::Peer;

/// The app, an agent's `humata mcp` and the person's `humata`, on one
/// daemon.
struct Clients {
    app: Client,
    agent: Client,
    terminal: Client,
    daemon: Arc<Daemon>,
    repository: tempfile::TempDir,
    _state: tempfile::TempDir,
}

fn other(program: &str, surface: pb::ChangeSurface) -> Peer {
    Peer::Client {
        program: program.to_owned(),
        parent: "zsh".to_owned(),
        surface,
    }
}

async fn clients() -> Clients {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let daemon: Arc<Daemon> = daemon_itself(state.path(), Hosts::default()).await;
    let app = client_of(Connection::new(daemon.clone(), Peer::App)).await;
    let agent = client_of(Connection::new(
        daemon.clone(),
        other("humata mcp", pb::ChangeSurface::Mcp),
    ))
    .await;
    let terminal = client_of(Connection::new(
        daemon.clone(),
        other("humata", pb::ChangeSurface::CommandLine),
    ))
    .await;
    Clients {
        app,
        agent,
        terminal,
        daemon,
        repository: git_repository(),
        _state: state,
    }
}

/// A client that isn't the app, saying in its hello that it is `kind`.
async fn declared(daemon: &Arc<Daemon>, kind: pb::ClientKind) -> Client {
    let connection = Connection::new(
        daemon.clone(),
        other("some-program", pb::ChangeSurface::CommandLine),
    );
    let (ours, theirs) = tokio::io::duplex(64 * 1024);
    let (reader, writer) = tokio::io::split(theirs);
    tokio::spawn(humata_wire::serve(
        reader,
        writer,
        pb::Role::Daemon,
        "test",
        connection,
    ));
    let (reader, writer) = tokio::io::split(ours);
    Client::connect_as(reader, writer, pb::Role::Client, "test", kind)
        .await
        .unwrap_or_else(|error| panic!("couldn't connect: {error}"))
}

async fn access(client: &Client) -> pb::AgentAccess {
    match client
        .call(pb::request::Kind::GetAgentAccess(pb::GetAgentAccess {}))
        .await
    {
        Ok(pb::response::Kind::AgentAccess(access)) => access,
        other => panic!("{other:?}"),
    }
}

async fn set(client: &Client, access: pb::AgentAccess) -> Result<pb::AgentAccess, ClientError> {
    let request = pb::request::Kind::SetAgentAccess(pb::SetAgentAccess {
        access: Some(access),
    });
    match client.call(request).await? {
        pb::response::Kind::AgentAccess(access) => Ok(access),
        other => panic!("{other:?}"),
    }
}

/// Changes the agents' switches as the person would in Settings.
async fn set_agents(app: &Client, change: impl FnOnce(&mut pb::ClientAccess)) {
    let mut agents = access(app).await.agents.unwrap_or_default();
    change(&mut agents);
    let asked = pb::AgentAccess {
        agents: Some(agents),
        ..Default::default()
    };
    set(app, asked)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
}

fn list_projects() -> pb::request::Kind {
    pb::request::Kind::ListProjects(pb::ListProjects {})
}

fn list_inbox() -> pb::request::Kind {
    pb::request::Kind::ListInbox(pb::ListInbox {})
}

fn delete_project(id: &str) -> pb::request::Kind {
    pb::request::Kind::DeleteProject(pb::DeleteProject {
        project_id: id.to_owned(),
    })
}

fn switched_off(answer: &Result<pb::response::Kind, ClientError>) -> bool {
    matches!(
        answer,
        Err(ClientError::Refused {
            code: pb::ErrorCode::AccessOff,
            ..
        })
    )
}

#[tokio::test]
async fn a_fresh_daemon_has_reads_and_changes_on_and_destroys_off() {
    let clients = clients().await;
    let access = access(&clients.app).await;
    for client in [access.agents, access.terminal] {
        let client = client.unwrap_or_else(|| panic!("a client's switches are missing"));
        assert!(client.enabled && client.read && client.change, "{client:?}");
        assert!(!client.destroy, "{client:?}");
        assert!(client.actions.iter().all(|action| action.enabled));
        for group in [
            pb::AccessGroup::Read,
            pb::AccessGroup::Change,
            pb::AccessGroup::Destroy,
        ] {
            assert!(
                client.actions.iter().any(|action| action.group() == group),
                "no action in {group:?}"
            );
        }
    }
    // Nothing is refused for the app; what destroys is for the others.
    assert!(access.refused.is_empty(), "{:?}", access.refused);
    for client in [&clients.agent, &clients.terminal] {
        let refused = self::access(client).await.refused;
        assert!(
            refused.contains(&"delete_project".to_owned()),
            "{refused:?}"
        );
        assert!(
            !refused.contains(&"list_projects".to_owned()),
            "{refused:?}"
        );
        assert!(!refused.contains(&"send_message".to_owned()), "{refused:?}");
    }
}

#[tokio::test]
async fn an_action_that_is_off_is_refused_for_the_client_asking_and_runs_for_the_app() {
    let clients = clients().await;
    let project = project(&clients.app, clients.repository.path()).await.id;

    // Deleting is off for the agent and the command line from the start.
    for client in [&clients.agent, &clients.terminal] {
        let refused = client.call(delete_project(&project)).await;
        assert!(switched_off(&refused), "{refused:?}");
    }

    // One action off for agents is off for them alone.
    set_agents(&clients.app, |agents| {
        for action in &mut agents.actions {
            if action.id == "list_projects" {
                action.enabled = false;
            }
        }
    })
    .await;
    let refused = clients.agent.call(list_projects()).await;
    assert!(switched_off(&refused), "{refused:?}");
    let inbox = clients.agent.call(list_inbox()).await;
    assert!(
        matches!(inbox, Ok(pb::response::Kind::Inbox(_))),
        "{inbox:?}"
    );
    for client in [&clients.terminal, &clients.app] {
        let listed = client.call(list_projects()).await;
        assert!(
            matches!(listed, Ok(pb::response::Kind::Projects(_))),
            "{listed:?}"
        );
    }

    // The app deletes whatever the switches say.
    let deleted = clients.app.call(delete_project(&project)).await;
    assert!(
        matches!(deleted, Ok(pb::response::Kind::ProjectDeleted(_))),
        "{deleted:?}"
    );
}

#[tokio::test]
async fn turning_a_group_off_turns_its_actions_off() {
    let clients = clients().await;
    set_agents(&clients.app, |agents| agents.read = false).await;
    for read in [list_projects(), list_inbox()] {
        let refused = clients.agent.call(read).await;
        assert!(switched_off(&refused), "{refused:?}");
    }
    // Its actions keep their own switches, and another group's still run.
    let shown = access(&clients.agent).await;
    let agents = shown.agents.unwrap_or_default();
    assert!(agents.actions.iter().all(|action| action.enabled));
    assert!(shown.refused.contains(&"list_projects".to_owned()));
    let made = clients
        .agent
        .call(super::create_project(clients.repository.path()))
        .await;
    assert!(
        matches!(made, Ok(pb::response::Kind::ProjectCreated(_))),
        "{made:?}"
    );

    // On again, with the group that destroys: its actions run.
    set_agents(&clients.app, |agents| {
        agents.read = true;
        agents.destroy = true;
    })
    .await;
    let listed = match clients.agent.call(list_projects()).await {
        Ok(pb::response::Kind::Projects(projects)) => projects.projects,
        other => panic!("{other:?}"),
    };
    let deleted = clients.agent.call(delete_project(&listed[0].id)).await;
    assert!(
        matches!(deleted, Ok(pb::response::Kind::ProjectDeleted(_))),
        "{deleted:?}"
    );
}

#[tokio::test]
async fn the_master_switch_turns_everything_off_for_that_client_alone() {
    let clients = clients().await;
    set_agents(&clients.app, |agents| agents.enabled = false).await;
    let refused = clients.agent.call(list_projects()).await;
    assert!(switched_off(&refused), "{refused:?}");
    let listed = clients.terminal.call(list_projects()).await;
    assert!(
        matches!(listed, Ok(pb::response::Kind::Projects(_))),
        "{listed:?}"
    );
    // It can still read why.
    let shown = access(&clients.agent).await;
    assert!(!shown.agents.unwrap_or_default().enabled);
}

#[tokio::test]
async fn only_the_app_changes_the_switches() {
    let clients = clients().await;
    let mut wanted = access(&clients.agent).await;
    if let Some(agents) = wanted.agents.as_mut() {
        agents.destroy = true;
    }
    for client in [&clients.agent, &clients.terminal] {
        let refused = set(client, wanted.clone()).await;
        assert!(
            matches!(
                refused,
                Err(ClientError::Refused {
                    code: pb::ErrorCode::AppOnly,
                    ..
                })
            ),
            "{refused:?}"
        );
    }
    let kept = access(&clients.app).await.agents.unwrap_or_default();
    assert!(!kept.destroy);
}

#[tokio::test]
async fn a_client_goes_by_the_switches_of_the_kind_its_hello_says() {
    let clients = clients().await;
    let agent = declared(&clients.daemon, pb::ClientKind::Agent).await;
    let terminal = declared(&clients.daemon, pb::ClientKind::Terminal).await;
    set_agents(&clients.app, |agents| agents.enabled = false).await;
    let refused = agent.call(list_projects()).await;
    assert!(switched_off(&refused), "{refused:?}");
    let listed = terminal.call(list_projects()).await;
    assert!(
        matches!(listed, Ok(pb::response::Kind::Projects(_))),
        "{listed:?}"
    );
}

fn child(session: &str) -> pb::request::Kind {
    pb::request::Kind::AgentStartSession(pb::AgentStartSession {
        session_id: session.to_owned(),
        name: "Tests".to_owned(),
        text: "write the tests".to_owned(),
    })
}

#[tokio::test]
async fn a_sessions_agent_is_refused_a_tool_that_is_off_for_agents() {
    let clients = clients().await;
    // The switches are the agents' alone.
    let shown = access(&clients.app).await;
    let has = |client: &Option<pb::ClientAccess>| {
        client
            .iter()
            .flat_map(|client| &client.actions)
            .any(|action| action.id == "agent_start_session")
    };
    assert!(has(&shown.agents) && !has(&shown.terminal));

    // On, the call gets as far as whose session it is.
    let asked = clients.daemon.tool_call("", Some(child("no-such"))).await;
    assert!(
        !matches!(asked, Err(crate::error::DaemonError::Access(_))),
        "{asked:?}"
    );

    set_agents(&clients.app, |agents| {
        for action in &mut agents.actions {
            if action.id == "agent_start_session" {
                action.enabled = false;
            }
        }
    })
    .await;
    let refused = clients.daemon.tool_call("", Some(child("no-such"))).await;
    assert!(
        matches!(refused, Err(crate::error::DaemonError::Access(_))),
        "{refused:?}"
    );
    // The tools are told, to leave it out.
    let told = clients
        .daemon
        .tool_call(
            "",
            Some(pb::request::Kind::GetAgentAccess(pb::GetAgentAccess {})),
        )
        .await;
    match told {
        Ok(pb::response::Kind::AgentAccess(access)) => {
            assert!(access.refused.contains(&"agent_start_session".to_owned()));
            assert!(!access.refused.contains(&"agent_send_message".to_owned()));
        }
        other => panic!("{other:?}"),
    }
}
