//! ChannelService.GetGrid through a real `humatad2` (CH-9, TP-1): the Repos
//! and machines grid has a cell per repo and machine of the channel, each
//! counting the sessions with a clone of that repo there, and `humata
//! channel grid` answers it.

mod common;

use common::{create_channel, humata, start_session, start_world};
use humata2_grpc::humata::v1 as pb;

async fn grid(daemon: &common::Daemon, channel: &pb::Channel) -> Result<pb::ChannelGrid, tonic::Status> {
    daemon
        .channel_service()
        .await
        .get_grid(pb::GetGridRequest {
            channel: channel.id.clone(),
        })
        .await
        .map(tonic::Response::into_inner)
}

/// (repo, machine, state, sessions) of each cell, in order.
fn cells(grid: &pb::ChannelGrid) -> Vec<(String, String, pb::grid_cell::State, u32)> {
    grid.cells
        .iter()
        .map(|cell| {
            let repo = cell.repo.clone().unwrap();
            (
                format!("{}/{}", repo.owner, repo.name),
                cell.machine.clone().unwrap().value,
                cell.state(),
                cell.sessions,
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_grid_counts_each_repos_session_clones_on_each_machine() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    world.remote("acme", "web");
    let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api"), ("acme", "web")]).await;
    assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
    let machine = channel.machines[0].value.clone();

    let before = grid(&daemon, &channel).await.unwrap();
    assert_eq!(before.channel, channel.id);
    use pb::grid_cell::State::Ready;
    assert_eq!(
        cells(&before),
        [
            ("acme/api".to_owned(), machine.clone(), Ready, 0),
            ("acme/web".to_owned(), machine.clone(), Ready, 0),
        ]
    );

    let (_, started) = start_session(&daemon, &channel, "Fix the bug").await;
    assert_eq!(started.state, pb::OperationState::Succeeded as i32, "{:?}", started.error);

    let after = grid(&daemon, &channel).await.unwrap();
    assert_eq!(
        cells(&after),
        [
            ("acme/api".to_owned(), machine.clone(), Ready, 1),
            ("acme/web".to_owned(), machine, Ready, 1),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_grid_of_a_channel_that_is_not_there_is_not_found() {
    let (daemon, _world) = start_world().await;
    let missing = pb::Channel {
        id: Some(pb::ChannelId {
            value: "ch_missing".to_owned(),
        }),
        ..pb::Channel::default()
    };

    let refused = grid(&daemon, &missing).await.unwrap_err();

    assert_eq!(refused.code(), tonic::Code::NotFound, "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_cli_answers_the_grid() {
    let (daemon, world) = start_world().await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;

    let (code, json) = humata(&daemon, &["channel", "grid", &channel.id.unwrap().value]);

    assert_eq!(code, 0, "{json}");
    assert_eq!(json["ok"], true, "{json}");
}
