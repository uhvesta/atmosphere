//! D382: a Desk session has no repository; its documents sit in the
//! session's own folder, outside every clone. FileService reads them
//! through a `SessionFolder` location, and the policy judges that
//! location as the session's own: its agent reaches it, another session's
//! agent does not, and a path that leaves the folder is refused.

use humatad_test_common as common;

use std::path::PathBuf;

use common::{Build, Daemon, World, error_of, folder, shell_token, wait_status};
use humata2_grpc::humata::v1 as pb;
use tokio_stream::StreamExt;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Status};

async fn daemon() -> Daemon {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await
}

/// Starts a plain Desk session in a scratch folder and waits until its
/// agent is up; returns it and its folder.
async fn desk_session(daemon: &Daemon, title: &str) -> (pb::SessionId, PathBuf) {
    let started = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            title: title.to_owned(),
            access: Some(pb::DeskAccess {
                boards: true,
                channels_and_machines: false,
            }),
            ..Default::default()
        })
        .await
        .expect("start a desk session")
        .into_inner();
    let id = started.session.expect("the session id");
    wait_status(daemon, &id, pb::SessionStatus::Working).await;
    let path = started.desk.and_then(|desk| desk.folder).expect("its folder").path;
    (id, PathBuf::from(path))
}

async fn files(daemon: &Daemon) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
}

fn in_folder(session: &pb::SessionId, path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(pb::file_location::Root::SessionFolder(pb::SessionFolder {
            session: Some(session.clone()),
        })),
        path: path.to_owned(),
    }
}

/// Reads `location` whole as the agent holding `token`, or as the user
/// with none.
async fn read(daemon: &Daemon, location: pb::FileLocation, token: Option<&str>) -> Result<Vec<u8>, Status> {
    let mut request = tonic::Request::new(pb::ReadFileRequest {
        location: Some(location),
        ..Default::default()
    });
    if let Some(token) = token {
        request.metadata_mut().insert(
            "humata-session-token",
            MetadataValue::try_from(token).expect("a token is ASCII"),
        );
    }
    let mut chunks = files(daemon).await.read_file(request).await?.into_inner();
    let mut data = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk?;
        // Raw chunks: this reader offers no codecs.
        data.extend_from_slice(&chunk.data);
        if chunk.last {
            break;
        }
    }
    Ok(data)
}

fn is_denied(status: &Status) -> bool {
    status.code() == Code::PermissionDenied
}

fn is_outside(status: &Status) -> bool {
    is_denied(status)
        && matches!(
            error_of(status).kind,
            Some(pb::error::Kind::OutsideHumataFolders(_))
        )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_document_in_a_desk_sessions_folder_reads_back_through_its_session_folder() {
    let daemon = daemon().await;
    let (session, folder) = desk_session(&daemon, "Notes").await;
    std::fs::write(folder.join("rfc.md"), "the document").expect("write the document");
    std::fs::create_dir(folder.join("drafts")).expect("make a sub-folder");
    std::fs::write(folder.join("drafts").join("one.md"), "a draft").expect("write the draft");

    let top = read(&daemon, in_folder(&session, "rfc.md"), None).await.expect("the user reads it");
    let nested = read(&daemon, in_folder(&session, "drafts/one.md"), None)
        .await
        .expect("the user reads a nested file");

    assert_eq!(top, b"the document");
    assert_eq!(nested, b"a draft");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_path_that_leaves_the_session_folder_is_refused() {
    let daemon = daemon().await;
    let (session, folder) = desk_session(&daemon, "Notes").await;
    let beside = folder.with_file_name("not-yours.txt");
    std::fs::write(&beside, "secret").expect("write a file beside the folder");
    std::os::unix::fs::symlink(&beside, folder.join("link.txt")).expect("link out of the folder");

    for path in ["../not-yours.txt", "/etc/hosts", "link.txt"] {
        let refused = read(&daemon, in_folder(&session, path), None)
            .await
            .expect_err("a path outside the folder isn't read");
        assert!(is_outside(&refused), "{path}: {refused:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_with_no_folder_on_record_is_not_found() {
    let daemon = daemon().await;
    let refused = read(
        &daemon,
        in_folder(&pb::SessionId { value: "se_nobody".to_owned() }, "rfc.md"),
        None,
    )
    .await
    .expect_err("an unknown session has no folder");
    assert_eq!(refused.code(), Code::NotFound, "{refused:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_sessions_agent_reads_its_own_folder_and_another_desk_sessions_agent_is_refused() {
    let daemon = daemon().await;
    let (mine, folder) = desk_session(&daemon, "Mine").await;
    let (other, _) = desk_session(&daemon, "Other").await;
    std::fs::write(folder.join("rfc.md"), "the document").expect("write the document");
    let (mine_token, _mine_shell) = shell_token(&daemon, &mine).await;
    let (other_token, _other_shell) = shell_token(&daemon, &other).await;

    let own = read(&daemon, in_folder(&mine, "rfc.md"), Some(&mine_token))
        .await
        .expect("an agent reads its own session's folder");
    // Two Desk sessions are not channel-mates (D182.3), and no grant covers
    // the other's folder: the policy refuses, it does not ask.
    let refused = read(&daemon, in_folder(&mine, "rfc.md"), Some(&other_token))
        .await
        .expect_err("another session's agent is refused");

    assert_eq!(own, b"the document");
    assert!(is_denied(&refused), "{refused:?}");
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::Denied(_))),
        "{refused:?}"
    );
}
