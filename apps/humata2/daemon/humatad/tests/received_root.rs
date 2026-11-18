//! PL-3, PL-4: files sent from a place that is not a channel's land in the
//! target machine's received place, `<channels>/received/<source machine>/`.
//! The user names it; an agent that names it is refused, like the home
//! folder; a machine id that is not one folder name is refused.

mod common;

use std::time::Duration;

use common::{Build, Daemon, World, error_of, folder, invalid_field, shell_token, wait_status};
use humata2_grpc::humata::v1 as pb;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Status};

const LONG: Duration = Duration::from_secs(60);

async fn daemon() -> (Daemon, World) {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    (Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await, world)
}

async fn transfers(daemon: &Daemon) -> pb::transfer_service_client::TransferServiceClient<Channel> {
    pb::transfer_service_client::TransferServiceClient::new(daemon.channel().await)
}

async fn files(daemon: &Daemon) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
}

fn in_home(path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(pb::file_location::Root::Home(pb::MachineHome {})),
        path: path.to_owned(),
    }
}

fn in_received(from: &str, path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(pb::file_location::Root::Received(pb::MachineReceived {
            from: Some(pb::MachineId { value: from.to_owned() }),
        })),
        path: path.to_owned(),
    }
}

fn copy_request(source: pb::FileLocation, destination: pb::FileLocation) -> pb::StartTransferRequest {
    pb::StartTransferRequest {
        idempotency_key: None,
        sources: vec![source],
        destination: Some(destination),
        mode: pb::TransferMode::Copy as i32,
        on_existing: pb::OnExisting::Fail as i32,
        reason: String::new(),
        name: String::new(),
    }
}

async fn start(
    daemon: &Daemon,
    request: pb::StartTransferRequest,
    token: Option<&str>,
) -> Result<pb::Transfer, Status> {
    let mut request = tonic::Request::new(request);
    if let Some(token) = token {
        request.metadata_mut().insert(
            "humata-session-token",
            MetadataValue::try_from(token).expect("a token is ASCII"),
        );
    }
    transfers(daemon)
        .await
        .start_transfer(request)
        .await
        .map(tonic::Response::into_inner)
}

/// The transfer's last state: the watch ends once it is final.
async fn settled(daemon: &Daemon, transfer: &pb::Transfer) -> pb::Transfer {
    let mut changes = transfers(daemon)
        .await
        .watch_transfer(pb::WatchTransferRequest {
            id: transfer.id.clone(),
            from_version: 0,
        })
        .await
        .expect("watch the transfer")
        .into_inner();
    let mut last = None;
    let waited = tokio::time::timeout(LONG, async {
        while let Some(change) = changes.message().await.expect("the watch goes on") {
            last = Some(change);
        }
    })
    .await;
    assert!(waited.is_ok(), "the transfer didn't finish in time: {last:?}");
    last.expect("the watch sent the transfer's state")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copy_from_home_lands_in_the_received_place_of_the_source_machine() {
    let (daemon, world) = daemon().await;
    std::fs::write(daemon.dir.path().join("name.txt"), "from the desk").expect("write the file");

    let started = start(&daemon, copy_request(in_home("name.txt"), in_received("gpu-box", "")), None)
        .await
        .expect("the user copies into a received place");
    let done = settled(&daemon, &started).await;

    assert_eq!(done.state(), pb::TransferState::Done, "{done:?}");
    let landed = world.channels.join("received").join("gpu-box").join("name.txt");
    assert_eq!(std::fs::read_to_string(landed).expect("the copy landed"), "from the desk");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_received_place_nothing_landed_in_lists_empty() {
    let (daemon, _world) = daemon().await;

    let listed = files(&daemon)
        .await
        .list_directory(pb::ListDirectoryRequest {
            location: Some(in_received("never-sent", "")),
            page: None,
        })
        .await
        .expect("a missing received place lists as empty")
        .into_inner();

    assert!(listed.entries.is_empty(), "{listed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_id_that_is_not_one_folder_name_is_refused() {
    let (daemon, world) = daemon().await;
    std::fs::write(daemon.dir.path().join("name.txt"), "x").expect("write the file");

    for bad in ["", "..", ".", "a/b", "../x"] {
        let refused = start(&daemon, copy_request(in_home("name.txt"), in_received(bad, "")), None)
            .await
            .expect_err("a bad machine id isn't a destination");
        assert_eq!(refused.code(), Code::InvalidArgument, "{bad:?}: {refused:?}");
        let (field, _) = invalid_field(&refused);
        assert!(field.contains("received"), "{bad:?}: {field}");
    }
    assert!(!world.channels.join("x").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_naming_the_received_place_is_refused_as_outside_humata_folders() {
    let (daemon, _world) = daemon().await;
    let started = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            title: "Notes".to_owned(),
            access: Some(pb::DeskAccess {
                boards: true,
                channels_and_machines: false,
            }),
            ..Default::default()
        })
        .await
        .expect("start a desk session")
        .into_inner();
    let session = started.session.expect("the session id");
    wait_status(&daemon, &session, pb::SessionStatus::Working).await;
    let (token, _shell) = shell_token(&daemon, &session).await;

    let refused = files(&daemon)
        .await
        .list_directory({
            let mut request = tonic::Request::new(pb::ListDirectoryRequest {
                location: Some(in_received("gpu-box", "")),
                page: None,
            });
            request.metadata_mut().insert(
                "humata-session-token",
                MetadataValue::try_from(token.as_str()).expect("a token is ASCII"),
            );
            request
        })
        .await
        .expect_err("an agent doesn't reach the received place");

    assert_eq!(refused.code(), Code::PermissionDenied, "{refused:?}");
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::OutsideHumataFolders(_))),
        "{refused:?}"
    );
}
