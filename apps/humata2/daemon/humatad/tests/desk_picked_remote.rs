//! A picked Desk folder on a box is never migrated (D458, D460): Migrate
//! makes the scratch folders Humata made on that box repositories, with no
//! commit, and leaves a folder the person picked as it is, with its files in
//! place and no repository. A second Migrate on the box changes nothing.
//! The box is asked over its own daemon, which runs as the user.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use humata2_store::{DeskFolderKind, DeskRow, SessionDetail, SessionRecord, StartStep};
use remote_rig::Rig;
use tonic::transport::Channel;

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");

async fn desks(daemon: &common::Daemon) -> pb::desk_service_client::DeskServiceClient<Channel> {
    pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
}

/// A plain Desk session on the box, in a scratch folder (a new start never
/// takes a picked one, D451). Returns the folder it works in.
async fn start_on_box(daemon: &common::Daemon, title: &str) -> PathBuf {
    let started = desks(daemon)
        .await
        .start_desk_session(pb::StartDeskSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: format!("desk-{title}") }),
            title: title.to_owned(),
            access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
            ..Default::default()
        })
        .await
        .expect("the Desk session starts on the box")
        .into_inner();
    if let Some(operation) = &started.operation {
        let ended = common::finished(daemon, operation).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{:?}", ended.error);
    }
    PathBuf::from(started.desk.and_then(|desk| desk.folder).expect("a start says where it works").path)
}

/// A Desk session in a folder the person picked, as an older release
/// recorded it on the box: its session, finished start and Desk row written
/// to the box's store. No start can make one now.
async fn old_picked_on_box(rig: &Rig, title: &str, folder: &Path) {
    let machine = common::machine_id(rig.the(BOX).daemon()).await.value;
    let id = format!("se_old_{}", title.to_lowercase().replace(' ', "_"));
    let store = rig.the(BOX).store();
    store
        .put_session_with_detail(
            &SessionRecord {
                id: id.clone(),
                channel: "desk".to_owned(),
                machine,
                title: title.to_owned(),
                agent: "zsh".to_owned(),
                folder: folder.canonicalize().unwrap(),
                created_unix_ms: 1,
            },
            &SessionDetail {
                session: id.clone(),
                repos: Vec::new(),
                model: None,
                first_message: None,
                start: StartStep::Done,
                operation: None,
                revision: 1,
            },
        )
        .unwrap();
    store
        .put_desk(&DeskRow {
            session_id: id,
            kind: DeskFolderKind::Picked,
            boards: true,
            channels_and_machines: false,
            last_used_unix_ms: 1,
            closed_unix_ms: None,
            deleted_unix_ms: None,
            held_work: String::new(),
        })
        .unwrap();
}

fn has_commit(folder: &Path) -> bool {
    std::process::Command::new("git")
        .args(["-C", &folder.display().to_string(), "rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .expect("git runs")
        .status
        .success()
}

/// A folder as an older release made it: no repository.
fn without_repo(folder: &Path) {
    std::fs::remove_dir_all(folder.join(".git")).expect("the folder has a repository to remove");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_picked_desk_folder_on_a_box_is_never_migrated_and_a_rerun_changes_nothing() {
    let rig = Rig::start_seen(&[BOX], &[REPO]).await;
    let daemon = rig.the(BOX).daemon();

    // A folder the person picked on the box, outside its Humata root.
    let picked = rig.the(BOX).home().join("Notes");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::write(picked.join("plan.md"), "keep me\n").unwrap();
    old_picked_on_box(&rig, "Picked notes", &picked).await;

    // A scratch folder Humata made before versions: no repository yet.
    let scratch = start_on_box(daemon, "Old notes").await;
    without_repo(&scratch);

    let checked = desks(daemon)
        .await
        .check_desk_folders(pb::CheckDeskFoldersRequest::default())
        .await
        .expect("the box checks its Desk folders")
        .into_inner();
    assert_eq!(checked.without_versions, 1, "only the scratch folder lacks versions: a picked one is never counted");

    let migrated = desks(daemon)
        .await
        .migrate_desk_folders(pb::MigrateDeskFoldersRequest::default())
        .await
        .expect("the box migrates its Desk folders")
        .into_inner();
    assert_eq!(migrated.migrated, 1, "the scratch folder is made a repository");
    assert!(scratch.join(".git").is_dir(), "the scratch folder is a repository now");
    assert!(!has_commit(&scratch), "migrating saves no version");
    assert!(!picked.join(".git").exists(), "the picked folder is never made a repository");
    assert_eq!(std::fs::read_to_string(picked.join("plan.md")).unwrap(), "keep me\n", "the picked folder's files stay where they were");

    // A second Migrate on the box, and a check, change nothing.
    let again = desks(daemon)
        .await
        .migrate_desk_folders(pb::MigrateDeskFoldersRequest::default())
        .await
        .expect("the box migrates its Desk folders again")
        .into_inner();
    assert_eq!(again.migrated, 0, "a rerun migrates nothing");
    let rechecked = desks(daemon)
        .await
        .check_desk_folders(pb::CheckDeskFoldersRequest::default())
        .await
        .expect("the box checks its Desk folders again")
        .into_inner();
    assert_eq!(rechecked.without_versions, 0, "every Humata-made folder now has its repository");
    assert!(!picked.join(".git").exists(), "the rerun leaves the picked folder alone");
    assert_eq!(std::fs::read_to_string(picked.join("plan.md")).unwrap(), "keep me\n", "the rerun leaves its files alone");
}
