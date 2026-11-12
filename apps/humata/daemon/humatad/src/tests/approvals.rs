//! Admin actions from clients other than the app wait for the person, or
//! run under a grant they gave.

use std::sync::Arc;

use humata_wire::{Client, ClientError, pb};

use super::{board, board_log, client_of, daemon_itself, git_repository, item, project};
use crate::daemon::{Connection, Daemon};
use crate::hosts::Hosts;
use crate::peer::Peer;

/// The app and an agent's `humata mcp`, on one daemon, and a project.
struct Clients {
    app: Client,
    agent: Client,
    project: String,
    _state: tempfile::TempDir,
    _repository: tempfile::TempDir,
}

async fn clients() -> Clients {
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
    Clients {
        app,
        agent,
        project,
        _state: state,
        _repository: repository,
    }
}

fn add(project: &str, title: &str) -> pb::request::Kind {
    pb::request::Kind::CreateItem(pb::CreateItem {
        project_id: project.to_owned(),
        title: title.to_owned(),
        // An agent can't pass for the app by saying it is.
        surface: pb::ChangeSurface::App.into(),
        ..Default::default()
    })
}

fn delete(item: &str) -> pb::request::Kind {
    pb::request::Kind::DeleteItem(pb::DeleteItem {
        item_id: item.to_owned(),
        ..Default::default()
    })
}

/// The approval `request` waits on, asked for by `client`.
async fn asked(client: &Client, request: pb::request::Kind) -> pb::Approval {
    match client.call(request).await {
        Ok(pb::response::Kind::NeedsApproval(needs)) => {
            *needs.approval.unwrap_or_else(|| panic!("no approval"))
        }
        other => panic!("ran without approval: {other:?}"),
    }
}

async fn decide(
    app: &Client,
    ids: &[&str],
    approve: bool,
    grant: &[pb::ApprovalScope],
    seconds: u32,
) -> Result<pb::ApprovalsDecided, ClientError> {
    let request = pb::request::Kind::DecideApprovals(pb::DecideApprovals {
        ids: ids.iter().map(|id| (*id).to_owned()).collect(),
        approve,
        grant_scopes: grant.iter().map(|scope| (*scope).into()).collect(),
        grant_seconds: seconds,
    });
    match app.call(request).await? {
        pb::response::Kind::ApprovalsDecided(decided) => Ok(decided),
        other => panic!("{other:?}"),
    }
}

async fn pending(client: &Client) -> Vec<pb::Approval> {
    match client
        .call(pb::request::Kind::ListInbox(pb::ListInbox {}))
        .await
    {
        Ok(pb::response::Kind::Inbox(inbox)) => inbox.approvals,
        other => panic!("{other:?}"),
    }
}

fn titles(board: &[(String, pb::ItemStatus)]) -> Vec<&str> {
    board.iter().map(|(title, _)| title.as_str()).collect()
}

#[tokio::test]
async fn the_apps_admin_actions_run_at_once() {
    let clients = clients().await;
    item(&clients.app, &clients.project, "", "Ship it").await;
    assert_eq!(
        titles(&board(&clients.app, &clients.project).await),
        ["Ship it"]
    );
    assert!(pending(&clients.app).await.is_empty());
}

#[tokio::test]
async fn an_agents_admin_action_waits_and_runs_once_approved() {
    let clients = clients().await;
    let approval = asked(&clients.agent, add(&clients.project, "Ship it")).await;
    assert!(board(&clients.app, &clients.project).await.is_empty());
    let waiting = pending(&clients.app).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].client, "humata mcp");
    assert_eq!(waiting[0].project_id, clients.project);

    let decided = decide(&clients.app, &[&approval.id], true, &[], 0)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(decided.approvals.len(), 1);
    assert!(matches!(
        decided.approvals[0]
            .result
            .as_ref()
            .and_then(|result| result.kind.as_ref()),
        Some(pb::response::Kind::ItemCreated(_))
    ));
    assert_eq!(
        titles(&board(&clients.app, &clients.project).await),
        ["Ship it"]
    );
    assert!(pending(&clients.app).await.is_empty());

    // The journal says it came from an agent, under this approval.
    let log = board_log(&clients.app, &clients.project).await;
    let change = log
        .iter()
        .find(|change| change.approval_id == approval.id)
        .unwrap_or_else(|| panic!("no change under the approval: {log:?}"));
    assert_eq!(change.surface(), pb::ChangeSurface::Mcp);

    // Deciding it again runs nothing twice.
    let again = decide(&clients.app, &[&approval.id], true, &[], 0)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(again.approvals.is_empty());
    assert_eq!(board(&clients.app, &clients.project).await.len(), 1);
}

#[tokio::test]
async fn a_denied_action_changes_nothing_and_the_waiting_client_hears_so() {
    let clients = clients().await;
    let approval = asked(&clients.agent, add(&clients.project, "Ship it")).await;
    let waiting = {
        let id = approval.id.clone();
        let agent = &clients.agent;
        async move {
            agent
                .call(pb::request::Kind::WaitForApproval(pb::WaitForApproval {
                    id,
                    wait_seconds: 30,
                }))
                .await
        }
    };
    let ids = [approval.id.as_str()];
    let (waited, decided) = tokio::join!(waiting, decide(&clients.app, &ids, false, &[], 0));
    decided.unwrap_or_else(|error| panic!("{error}"));
    let Ok(pb::response::Kind::ApprovalStatus(status)) = waited else {
        panic!("{waited:?}")
    };
    let status = status.approval.unwrap_or_else(|| panic!("no approval"));
    assert_eq!(status.state(), pb::ApprovalState::Denied);
    assert!(board(&clients.app, &clients.project).await.is_empty());
}

#[tokio::test]
async fn a_grant_lets_actions_in_its_scopes_run_until_it_is_revoked() {
    let clients = clients().await;
    let first = asked(&clients.agent, add(&clients.project, "One")).await;
    let decided = decide(
        &clients.app,
        &[&first.id],
        true,
        &[pb::ApprovalScope::Board],
        3600,
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));
    let grant = decided.grant.unwrap_or_else(|| panic!("no grant"));

    // Under the grant, the next runs at once.
    let created = clients.agent.call(add(&clients.project, "Two")).await;
    assert!(
        matches!(created, Ok(pb::response::Kind::ItemCreated(_))),
        "{created:?}"
    );
    let log = board_log(&clients.app, &clients.project).await;
    assert!(log.iter().any(|change| change.grant_id == grant.id));

    // Deleting is destructive, which a board grant doesn't cover.
    let shown = board(&clients.app, &clients.project).await;
    assert_eq!(shown.len(), 2);
    let items = match clients
        .app
        .call(pb::request::Kind::ListItems(pb::ListItems {
            project_id: clients.project.clone(),
        }))
        .await
    {
        Ok(pb::response::Kind::Items(items)) => items.items,
        other => panic!("{other:?}"),
    };
    asked(&clients.agent, delete(&items[0].id)).await;

    clients
        .app
        .call(pb::request::Kind::RevokeGrant(pb::RevokeGrant {
            id: grant.id.clone(),
        }))
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    asked(&clients.agent, add(&clients.project, "Three")).await;
    assert_eq!(board(&clients.app, &clients.project).await.len(), 2);
}

#[tokio::test]
async fn an_expired_grant_covers_nothing() {
    let clients = clients().await;
    let first = asked(&clients.agent, add(&clients.project, "One")).await;
    decide(
        &clients.app,
        &[&first.id],
        true,
        &[pb::ApprovalScope::Board],
        1,
    )
    .await
    .unwrap_or_else(|error| panic!("{error}"));
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    asked(&clients.agent, add(&clients.project, "Two")).await;
}

#[tokio::test]
async fn batch_approval_runs_each_action() {
    let clients = clients().await;
    let one = asked(&clients.agent, add(&clients.project, "One")).await;
    let two = asked(&clients.agent, add(&clients.project, "Two")).await;
    decide(&clients.app, &[&one.id, &two.id], true, &[], 0)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let mut shown = titles(&board(&clients.app, &clients.project).await)
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    shown.sort();
    assert_eq!(shown, ["One", "Two"]);
}

#[tokio::test]
async fn only_the_app_decides_approvals_and_revokes_grants() {
    let clients = clients().await;
    let approval = asked(&clients.agent, add(&clients.project, "Ship it")).await;
    let decided = decide(&clients.agent, &[&approval.id], true, &[], 0).await;
    assert!(
        matches!(
            decided,
            Err(ClientError::Refused {
                code: pb::ErrorCode::AppOnly,
                ..
            })
        ),
        "{decided:?}"
    );
    let revoked = clients
        .agent
        .call(pb::request::Kind::RevokeGrant(pb::RevokeGrant {
            id: "any".to_owned(),
        }))
        .await;
    assert!(
        matches!(
            revoked,
            Err(ClientError::Refused {
                code: pb::ErrorCode::AppOnly,
                ..
            })
        ),
        "{revoked:?}"
    );
    assert!(board(&clients.app, &clients.project).await.is_empty());
}

#[tokio::test]
async fn reading_needs_no_approval() {
    let clients = clients().await;
    item(&clients.app, &clients.project, "", "Ship it").await;
    assert_eq!(
        titles(&board(&clients.agent, &clients.project).await),
        ["Ship it"]
    );
}

#[tokio::test]
async fn an_action_cut_short_by_a_restart_is_reported_so_and_not_run_again() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    // Approved, then the daemon stopped before it answered.
    let approval = pb::Approval {
        id: "a1".to_owned(),
        state: pb::ApprovalState::Approved.into(),
        ..Default::default()
    };
    {
        let store = crate::store::Store::open(&state.path().join("humata.sqlite3"))
            .unwrap_or_else(|error| panic!("{error}"));
        let request = pb::Request {
            kind: Some(add("p1", "Ship it")),
        };
        store
            .add_approval(&approval, &request)
            .unwrap_or_else(|error| panic!("{error}"));
    }

    let daemon: Arc<Daemon> = daemon_itself(state.path(), Hosts::default()).await;
    let app = client_of(Connection::new(daemon, Peer::App)).await;
    let waited = app
        .call(pb::request::Kind::WaitForApproval(pb::WaitForApproval {
            id: "a1".to_owned(),
            wait_seconds: 5,
        }))
        .await;
    let Ok(pb::response::Kind::ApprovalStatus(status)) = waited else {
        panic!("{waited:?}")
    };
    let status = status.approval.unwrap_or_else(|| panic!("no approval"));
    assert_eq!(status.state(), pb::ApprovalState::Approved);
    assert!(matches!(
        status.result.and_then(|result| result.kind),
        Some(pb::response::Kind::Error(_))
    ));
}
