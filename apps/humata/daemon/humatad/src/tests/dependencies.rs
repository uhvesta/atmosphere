//! Dependencies between a board's items, through the daemon: what a
//! listing says of blocked items and the plan's order, links made and
//! refused, and an agent's proposed link applied.

use humata_wire::{Client, ClientError, pb};

use super::{
    after_turns, daemon, git_repository, item, project, proposals_of, send, start_in, started,
};
use crate::hosts::Hosts;

async fn items(client: &Client, project: &str) -> Vec<pb::Item> {
    let listed = client
        .call(pb::request::Kind::ListItems(pb::ListItems {
            project_id: project.to_owned(),
        }))
        .await;
    match listed {
        Ok(pb::response::Kind::Items(items)) => items.items,
        other => panic!("{other:?}"),
    }
}

async fn update(client: &Client, update: pb::UpdateItem) -> Result<pb::Item, ClientError> {
    match client.call(pb::request::Kind::UpdateItem(update)).await? {
        pb::response::Kind::ItemUpdated(pb::ItemUpdated { item: Some(item) }) => Ok(item),
        other => panic!("not updated: {other:?}"),
    }
}

async fn link(client: &Client, item: &pb::Item, on: &pb::Item) -> Result<pb::Item, ClientError> {
    let linking = pb::UpdateItem {
        item_id: item.id.clone(),
        add_depends_on: vec![on.id.clone()],
        surface: pb::ChangeSurface::CommandLine.into(),
        ..Default::default()
    };
    update(client, linking).await
}

/// The titles of `project`'s items in the plan's order.
async fn plan(client: &Client, project: &str) -> Vec<String> {
    let mut items = items(client, project).await;
    items.sort_by_key(|item| item.plan_order);
    items.into_iter().map(|item| item.title).collect()
}

#[tokio::test]
async fn a_listing_says_what_is_blocked_by_what_and_the_order_to_take_items_in() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let release = item(&client, &project.id, "", "release").await;
    let notes = item(&client, &project.id, &release.id, "notes").await;
    let build = item(&client, &project.id, "", "build").await;
    let design = item(&client, &project.id, "", "design").await;

    let linked = link(&client, &release, &build)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(linked.depends_on, [build.id.clone()]);
    assert!(linked.blocked);
    assert_eq!(linked.waiting_on, [build.id.clone()]);
    assert_eq!(linked.kind(), pb::ItemKind::Milestone);
    link(&client, &build, &design)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    assert_eq!(
        plan(&client, &project.id).await,
        ["design", "build", "release", "notes"]
    );
    let listed = items(&client, &project.id).await;
    let of = |id: &str| -> pb::Item {
        listed
            .iter()
            .find(|item| item.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} isn't listed"))
    };
    // The milestone's task waits with it; a task on its own is a task.
    assert!(of(&notes.id).blocked);
    assert_eq!(of(&notes.id).waiting_on, [build.id.clone()]);
    assert_eq!(of(&notes.id).kind(), pb::ItemKind::Task);
    assert_eq!(of(&design.id).kind(), pb::ItemKind::Task);
    assert!(!of(&design.id).blocked);

    // Finishing what they wait on frees them.
    for done in [&design, &build] {
        let moved = pb::UpdateItem {
            item_id: done.id.clone(),
            status: Some(pb::ItemStatus::Done.into()),
            ..Default::default()
        };
        update(&client, moved)
            .await
            .unwrap_or_else(|error| panic!("{error}"));
    }
    assert!(
        items(&client, &project.id)
            .await
            .iter()
            .all(|item| !item.blocked)
    );
}

#[tokio::test]
async fn a_link_that_closes_a_loop_is_refused_as_one_and_the_journal_has_the_surface() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let first = item(&client, &project.id, "", "first").await;
    let second = item(&client, &project.id, "", "second").await;
    link(&client, &second, &first)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    let refused = link(&client, &first, &second).await;
    assert!(
        matches!(
            refused,
            Err(ClientError::Refused {
                code: pb::ErrorCode::DependencyCycle,
                ..
            })
        ),
        "{refused:?}"
    );
    let own = link(&client, &first, &first).await;
    assert!(
        matches!(
            own,
            Err(ClientError::Refused {
                code: pb::ErrorCode::InvalidDependency,
                ..
            })
        ),
        "{own:?}"
    );

    let log = client
        .call(pb::request::Kind::ReadBoardLog(pb::ReadBoardLog {
            project_id: project.id.clone(),
            limit: 1,
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::BoardLog(log)) = log else {
        panic!("{log:?}")
    };
    let change = &log.changes[0];
    assert_eq!(change.item_id, second.id);
    assert_eq!(change.surface(), pb::ChangeSurface::CommandLine);
    let depends_on = |fields: &Option<pb::ItemFields>| {
        fields
            .as_ref()
            .map(|fields| fields.depends_on.clone())
            .unwrap_or_default()
    };
    assert!(depends_on(&change.before).is_empty());
    assert_eq!(depends_on(&change.after), [first.id.clone()]);

    // Reverting the change takes the link off again.
    let reverted = client
        .call(pb::request::Kind::RevertBoardChange(
            pb::RevertBoardChange {
                change_id: change.id,
                ..Default::default()
            },
        ))
        .await;
    let Ok(pb::response::Kind::BoardChangeReverted(reverted)) = reverted else {
        panic!("{reverted:?}")
    };
    assert!(reverted.item.is_some_and(|item| item.depends_on.is_empty()));
}

#[tokio::test]
async fn an_agents_proposed_link_and_milestone_reach_the_board_only_when_applied() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let design = item(&client, &project.id, "", "design").await;
    let build = item(&client, &project.id, "", "build").await;
    let mut request = start_in(&project.id);
    if let pb::request::Kind::StartSession(start) = &mut request {
        start.backend_config = serde_json::json!({"rules": [{"when": "", "steps": [
            {"step": "tool", "tool": "mcp__humata__propose_dependency",
             "arguments": {"item_id": build.id, "depends_on": [design.id], "note": "Needs the design."}},
            {"step": "tool", "tool": "propose_task",
             "arguments": {"title": "Release", "spec": "Ship it.", "kind": "milestone",
                           "depends_on": [build.id]}},
        ]}]})
        .to_string();
    }
    let session = started(&client, request).await;
    send(&client, &session.id, "plan").await;
    after_turns(&client, &session.id, 1).await;

    let proposals = proposals_of(&client, &session.id).await;
    let [dependency, milestone] = proposals.as_slice() else {
        panic!("two proposals: {proposals:?}")
    };
    assert_eq!(dependency.kind(), pb::ProposalKind::Dependency);
    assert_eq!(dependency.item_id, build.id);
    assert_eq!(dependency.depends_on, [design.id.clone()]);
    assert_eq!(dependency.depends_on_titles, ["design"]);
    assert_eq!(milestone.item_kind(), pb::ItemKind::Milestone);
    // Nothing is linked until the person applies it.
    assert!(
        items(&client, &project.id)
            .await
            .iter()
            .all(|item| item.depends_on.is_empty())
    );

    for proposal in [dependency, milestone] {
        let applied = client
            .call(pb::request::Kind::ApplyProposal(pb::ApplyProposal {
                session_id: session.id.clone(),
                proposal_id: proposal.id.clone(),
                ..Default::default()
            }))
            .await;
        assert!(applied.is_ok(), "{applied:?}");
    }
    assert_eq!(
        plan(&client, &project.id).await,
        ["design", "build", "Release"]
    );
    let listed = items(&client, &project.id).await;
    let release = listed
        .iter()
        .find(|item| item.title == "Release")
        .unwrap_or_else(|| panic!("no Release in {listed:?}"));
    assert_eq!(release.kind(), pb::ItemKind::Milestone);
    assert_eq!(release.waiting_on, [build.id.clone()]);
    // The journal says the link is the session's.
    let log = client
        .call(pb::request::Kind::ReadBoardLog(pb::ReadBoardLog {
            project_id: project.id.clone(),
            item_id: build.id.clone(),
            limit: 1,
            ..Default::default()
        }))
        .await;
    let Ok(pb::response::Kind::BoardLog(log)) = log else {
        panic!("{log:?}")
    };
    assert_eq!(log.changes[0].session_id, session.id);
    // Applied once: a second time is refused.
    let again = client
        .call(pb::request::Kind::ApplyProposal(pb::ApplyProposal {
            session_id: session.id.clone(),
            proposal_id: dependency.id.clone(),
            ..Default::default()
        }))
        .await;
    assert!(again.is_err(), "{again:?}");
}

/// Every change to `project`'s board, newest first.
async fn board_log(client: &Client, project: &str) -> Vec<pb::BoardChange> {
    let log = client
        .call(pb::request::Kind::ReadBoardLog(pb::ReadBoardLog {
            project_id: project.to_owned(),
            ..Default::default()
        }))
        .await;
    match log {
        Ok(pb::response::Kind::BoardLog(log)) => log.changes,
        other => panic!("{other:?}"),
    }
}

async fn delete(client: &Client, item: &pb::Item) {
    let deleted = client
        .call(pb::request::Kind::DeleteItem(pb::DeleteItem {
            item_id: item.id.clone(),
            ..Default::default()
        }))
        .await;
    assert!(deleted.is_ok(), "{deleted:?}");
}

async fn revert(client: &Client, change: u64) -> pb::BoardChangeReverted {
    let reverted = client
        .call(pb::request::Kind::RevertBoardChange(
            pb::RevertBoardChange {
                change_id: change,
                ..Default::default()
            },
        ))
        .await;
    match reverted {
        Ok(pb::response::Kind::BoardChangeReverted(reverted)) => reverted,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn undoing_a_deletion_others_waited_on_reverts_all_it_did_and_undo_moves_past_it() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let a = item(&client, &project.id, "", "a").await;
    let b = item(&client, &project.id, "", "b").await;
    link(&client, &b, &a)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let before_delete = board_log(&client, &project.id).await[0].id;

    delete(&client, &a).await;
    let deleting: Vec<u64> = board_log(&client, &project.id)
        .await
        .iter()
        .map(|change| change.id)
        .filter(|id| *id > before_delete)
        .collect();
    let removal = board_log(&client, &project.id)
        .await
        .into_iter()
        .find(|change| change.item_id == a.id && change.after.is_none())
        .unwrap_or_else(|| panic!("no removal of a"));
    revert(&client, removal.id).await;

    let log = board_log(&client, &project.id).await;
    let reverted: Vec<u64> = log
        .iter()
        .map(|change| change.reverts)
        .filter(|reverts| *reverts != 0)
        .collect();
    for change in &deleting {
        assert!(
            reverted.contains(change),
            "{change} isn't reverted: {log:?}"
        );
    }
    let listed = items(&client, &project.id).await;
    let b_now = listed
        .iter()
        .find(|item| item.id == b.id)
        .unwrap_or_else(|| panic!("no b in {listed:?}"));
    assert_eq!(b_now.depends_on, [a.id.clone()]);
    // What Undo takes next: the newest change that is no revert and isn't
    // reverted.
    let undoable = log
        .iter()
        .find(|change| change.reverts == 0 && !reverted.contains(&change.id))
        .map(|change| change.id);
    assert_eq!(undoable, Some(before_delete));
}

#[tokio::test]
async fn an_item_brought_back_keeps_its_place_in_the_plan() {
    let state = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
    let repository = git_repository();
    let client = daemon(state.path(), Hosts::default()).await;
    let project = project(&client, repository.path()).await;
    let first = item(&client, &project.id, "", "first").await;
    // Added a clear moment later, so the two aren't added in the same
    // millisecond.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    item(&client, &project.id, "", "second").await;

    delete(&client, &first).await;
    let removal = board_log(&client, &project.id).await[0].id;
    revert(&client, removal).await;

    assert_eq!(plan(&client, &project.id).await, ["first", "second"]);
}
