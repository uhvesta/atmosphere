//! TransferService through `humatad2`'s gRPC API: copy and move between a
//! session's clone and a channel's shared folder, uploads, and what a
//! restart does to a transfer under way (FM-2, FM-4, FM-6, FM-7, FM-8,
//! FM-14, PER-27).

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{Daemon, error_of, folder, invalid_field, not_found, state_folder};
use humata2_chunk::{CHUNK_MAX, Codec, Encoder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, ChannelRepo, MainClone as MainState, SessionRecord, Store};
use sha2::{Digest, Sha256};
use tonic::transport::Channel;
use tonic::{Code, Status, Streaming};

const SESSION: &str = "se_tr";
const CHANNEL: &str = "ch_tr";
const CHANNEL_FOLDER: &str = "tr-channel";
const REPO: &str = "api";

/// The most a test waits for a transfer to end.
const LONG: Duration = Duration::from_secs(90);

struct World {
    daemon: Daemon,
    /// The session's clone of `REPO`.
    clone: PathBuf,
    shared: PathBuf,
}

/// A daemon that knows one session and one channel, with their folders.
async fn world() -> World {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: CHANNEL.to_owned(),
            machine: String::new(),
            title: "transfers".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    store
        .put_channel(&ChannelRecord {
            id: CHANNEL.to_owned(),
            name: "transfers".to_owned(),
            folder: CHANNEL_FOLDER.to_owned(),
            branch_prefix: None,
            default_agent: "zsh".to_owned(),
            repos: vec![ChannelRepo {
                owner: "acme".to_owned(),
                name: REPO.to_owned(),
                main: MainState::Ready,
            }],
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let clone = dir.path().join(REPO);
    let channel = dir.path().join("Humata").join(CHANNEL_FOLDER);
    let shared = channel.join("shared");
    for path in [&clone, &shared, &channel.join("main").join(REPO), &channel.join("sessions")] {
        std::fs::create_dir_all(path).unwrap();
    }
    World {
        daemon: Daemon::start_in(dir).await,
        clone,
        shared,
    }
}

async fn transfers(daemon: &Daemon) -> pb::transfer_service_client::TransferServiceClient<Channel> {
    pb::transfer_service_client::TransferServiceClient::new(daemon.channel().await)
}

async fn files(daemon: &Daemon) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
}

async fn bin(daemon: &Daemon) -> pb::trash_service_client::TrashServiceClient<Channel> {
    pb::trash_service_client::TrashServiceClient::new(daemon.channel().await)
}

fn at(root: pb::file_location::Root, path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(root),
        path: path.to_owned(),
    }
}

fn channel_id() -> Option<pb::ChannelId> {
    Some(pb::ChannelId {
        value: CHANNEL.to_owned(),
    })
}

fn in_clone(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::SessionClone(pb::SessionClone {
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: REPO.to_owned(),
            }),
        }),
        path,
    )
}

fn in_shared(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::ChannelShared(pb::ChannelShared {
            channel: channel_id(),
        }),
        path,
    )
}

fn in_main(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::MainClone(pb::MainClone {
            channel: channel_id(),
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: REPO.to_owned(),
            }),
        }),
        path,
    )
}

/// A path under the user's home folder, which holds Humata's folders.
fn in_home(path: &str) -> pb::FileLocation {
    at(pb::file_location::Root::Home(pb::MachineHome {}), path)
}

fn start_request(
    sources: &[pb::FileLocation],
    destination: pb::FileLocation,
    mode: pb::TransferMode,
    on_existing: pb::OnExisting,
    key: Option<&str>,
) -> pb::StartTransferRequest {
    pb::StartTransferRequest {
        idempotency_key: key.map(|key| pb::IdempotencyKey {
            value: key.to_owned(),
        }),
        sources: sources.to_vec(),
        destination: Some(destination),
        mode: mode as i32,
        on_existing: on_existing as i32,
        reason: String::new(),
        name: String::new(),
    }
}

async fn start(daemon: &Daemon, request: pb::StartTransferRequest) -> Result<pb::Transfer, Status> {
    transfers(daemon)
        .await
        .start_transfer(request)
        .await
        .map(tonic::Response::into_inner)
}

/// A copy of `source` into the folder `destination`.
async fn copy(daemon: &Daemon, source: pb::FileLocation, destination: pb::FileLocation) -> pb::Transfer {
    start(
        daemon,
        start_request(
            &[source],
            destination,
            pb::TransferMode::Copy,
            pb::OnExisting::Fail,
            None,
        ),
    )
    .await
    .unwrap()
}

fn id(transfer: &pb::Transfer) -> pb::TransferId {
    transfer.id.clone().unwrap()
}

fn state(transfer: &pb::Transfer) -> pb::TransferState {
    transfer.state()
}

async fn watch(daemon: &Daemon, id: &pb::TransferId, from_version: u64) -> Result<Streaming<pb::Transfer>, Status> {
    transfers(daemon)
        .await
        .watch_transfer(pb::WatchTransferRequest {
            id: Some(id.clone()),
            from_version,
        })
        .await
        .map(tonic::Response::into_inner)
}

/// The first change `done` accepts, or the last one before the stream ends.
async fn until(
    daemon: &Daemon,
    id: &pb::TransferId,
    done: impl Fn(&pb::Transfer) -> bool,
) -> pb::Transfer {
    let mut changes = watch(daemon, id, 0).await.unwrap();
    let mut last = None;
    let waited = tokio::time::timeout(LONG, async {
        while let Some(change) = changes.message().await.unwrap() {
            let stop = done(&change);
            last = Some(change);
            if stop {
                break;
            }
        }
    })
    .await;
    assert!(waited.is_ok(), "the transfer didn't get there in time: {last:?}");
    last.expect("the watch sent the transfer's state")
}

/// The transfer's last state: the watch ends once it is final.
async fn settled(daemon: &Daemon, id: &pb::TransferId) -> pb::Transfer {
    until(daemon, id, |_| false).await
}

async fn control(
    daemon: &Daemon,
    id: &pb::TransferId,
    call: &str,
) -> Result<pb::Transfer, Status> {
    let mut client = transfers(daemon).await;
    let request = pb::TransferControlRequest {
        idempotency_key: None,
        id: Some(id.clone()),
    };
    match call {
        "pause" => client.pause_transfer(request).await,
        "resume" => client.resume_transfer(request).await,
        _ => client.cancel_transfer(request).await,
    }
    .map(tonic::Response::into_inner)
}

async fn all(daemon: &Daemon, active_only: bool) -> Vec<pb::Transfer> {
    transfers(daemon)
        .await
        .list_transfers(pb::ListTransfersRequest {
            active_only,
            page: None,
        })
        .await
        .unwrap()
        .into_inner()
        .transfers
}

/// What ListDirectory shows in `location`, by name.
async fn listed(daemon: &Daemon, location: pb::FileLocation) -> Vec<String> {
    files(daemon)
        .await
        .list_directory(pb::ListDirectoryRequest {
            location: Some(location),
            page: None,
        })
        .await
        .unwrap()
        .into_inner()
        .entries
        .iter()
        .map(|entry| {
            let path = &entry.location.as_ref().unwrap().path;
            path.rsplit('/').next().unwrap().to_owned()
        })
        .collect()
}

fn no_staging(folder: &Path) {
    for entry in std::fs::read_dir(folder).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with(".humata-transfer-"),
            "a staging folder is left in {}: {name}",
            folder.display()
        );
    }
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Bytes that don't compress, the same for the same seed.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut data = Vec::with_capacity(len + 8);
    while data.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        data.extend_from_slice(&state.to_le_bytes());
    }
    data.truncate(len);
    data
}

#[derive(Debug, PartialEq)]
enum Node {
    Folder,
    File { len: u64, sha256: String },
    Link(PathBuf),
}

/// Everything under `root`, by path relative to it.
fn tree(root: &Path) -> BTreeMap<String, Node> {
    fn walk(root: &Path, at: &Path, into: &mut BTreeMap<String, Node>) {
        for entry in std::fs::read_dir(at).unwrap().flatten() {
            let path = entry.path();
            let name = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            let kind = std::fs::symlink_metadata(&path).unwrap().file_type();
            if kind.is_symlink() {
                into.insert(name, Node::Link(std::fs::read_link(&path).unwrap()));
            } else if kind.is_dir() {
                into.insert(name, Node::Folder);
                walk(root, &path, into);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                into.insert(
                    name,
                    Node::File {
                        len: bytes.len() as u64,
                        sha256: sha256(&bytes),
                    },
                );
            }
        }
    }
    let mut nodes = BTreeMap::new();
    walk(root, root, &mut nodes);
    nodes
}

/// `tree/` with nested folders, an empty file, a 3 MiB file and a symlink.
fn small_tree(clone: &Path) {
    let root = clone.join("tree");
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::create_dir_all(root.join("empty-folder")).unwrap();
    std::fs::write(root.join("empty.txt"), b"").unwrap();
    std::fs::write(root.join("a/hello.txt"), b"hello").unwrap();
    std::fs::write(root.join("a/b/big.bin"), noise(1, 3 << 20)).unwrap();
    std::os::unix::fs::symlink("a/hello.txt", root.join("link")).unwrap();
}

/// `tree/` with `files` files of 8 MiB.
fn large_tree(clone: &Path, files: u64) {
    let root = clone.join("tree");
    std::fs::create_dir_all(root.join("part")).unwrap();
    for index in 0..files {
        let folder = if index % 2 == 0 { root.clone() } else { root.join("part") };
        std::fs::write(folder.join(format!("f{index}.bin")), noise(index + 7, 8 << 20)).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn copying_a_tree_ends_done_with_identical_bytes_and_no_staging() {
    let world = world().await;
    small_tree(&world.clone);

    let started = copy(&world.daemon, in_clone("tree"), in_shared("")).await;
    // The staging folder is never shown, while the copy runs or after.
    let during = listed(&world.daemon, in_shared("")).await;
    assert!(during.iter().all(|name| !name.starts_with(".humata-transfer-")));
    let end = settled(&world.daemon, &id(&started)).await;

    assert_eq!(state(&end), pb::TransferState::Done, "{end:?}");
    assert_eq!(end.done_bytes, end.total_bytes);
    assert!(end.total_bytes >= 3 << 20);
    assert_eq!(end.done_files, end.total_files);
    assert!(end.problem.is_none());
    assert_eq!(tree(&world.shared.join("tree")), tree(&world.clone.join("tree")));
    no_staging(&world.shared);
    let after = listed(&world.daemon, in_shared("")).await;
    assert_eq!(after, vec!["tree".to_owned()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_leaves_the_source_gone_and_the_destination_whole() {
    let world = world().await;
    small_tree(&world.clone);
    let before = tree(&world.clone.join("tree"));

    let started = start(
        &world.daemon,
        start_request(
            &[in_clone("tree")],
            in_shared(""),
            pb::TransferMode::Move,
            pb::OnExisting::Fail,
            None,
        ),
    )
    .await
    .unwrap();
    let end = settled(&world.daemon, &id(&started)).await;

    assert_eq!(state(&end), pb::TransferState::Done, "{end:?}");
    assert!(!world.clone.join("tree").exists());
    assert_eq!(tree(&world.shared.join("tree")), before);
    no_staging(&world.shared);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_taken_name_fails_without_writing_when_the_choice_is_fail() {
    let world = world().await;
    std::fs::write(world.clone.join("note.txt"), "new").unwrap();
    std::fs::write(world.shared.join("note.txt"), "old").unwrap();

    let started = copy(&world.daemon, in_clone("note.txt"), in_shared("")).await;
    let end = settled(&world.daemon, &started.id.clone().unwrap()).await;

    assert_eq!(state(&end), pb::TransferState::Failed, "{end:?}");
    let problem = end.problem.expect("a failed transfer says why");
    assert!(matches!(problem.kind, Some(pb::error::Kind::NameTaken(_))), "{problem:?}");
    assert!(!problem.fix.is_empty());
    assert_eq!(std::fs::read_to_string(world.shared.join("note.txt")).unwrap(), "old");
    assert_eq!(std::fs::read_to_string(world.clone.join("note.txt")).unwrap(), "new");
    no_staging(&world.shared);
}

#[tokio::test(flavor = "multi_thread")]
async fn replacing_puts_the_old_one_in_the_trash() {
    let world = world().await;
    std::fs::write(world.clone.join("note.txt"), "new").unwrap();
    std::fs::write(world.shared.join("note.txt"), "old").unwrap();

    let started = start(
        &world.daemon,
        start_request(
            &[in_clone("note.txt")],
            in_shared(""),
            pb::TransferMode::Copy,
            pb::OnExisting::Replace,
            None,
        ),
    )
    .await
    .unwrap();
    let end = settled(&world.daemon, &id(&started)).await;

    assert_eq!(state(&end), pb::TransferState::Done, "{end:?}");
    assert_eq!(std::fs::read_to_string(world.shared.join("note.txt")).unwrap(), "new");
    let trash = bin(&world.daemon)
        .await
        .list_trash(pb::ListTrashRequest::default())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(trash.items.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn keeping_both_keeps_the_old_one_and_numbers_the_new_one() {
    let world = world().await;
    std::fs::write(world.clone.join("note.txt"), "new").unwrap();
    std::fs::write(world.shared.join("note.txt"), "old").unwrap();

    let started = start(
        &world.daemon,
        start_request(
            &[in_clone("note.txt")],
            in_shared(""),
            pb::TransferMode::Copy,
            pb::OnExisting::KeepBoth,
            None,
        ),
    )
    .await
    .unwrap();
    let end = settled(&world.daemon, &id(&started)).await;

    assert_eq!(state(&end), pb::TransferState::Done, "{end:?}");
    assert_eq!(std::fs::read_to_string(world.shared.join("note.txt")).unwrap(), "old");
    assert_eq!(std::fs::read_to_string(world.shared.join("note-2.txt")).unwrap(), "new");
    assert_eq!(
        end.outcomes,
        vec![pb::TransferOutcome {
            landing: pb::TransferLanding::SavedAs as i32,
            saved_as: "note-2.txt".to_owned(),
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn keeping_both_a_file_with_the_same_bytes_is_not_copied_again() {
    let world = world().await;
    std::fs::write(world.clone.join("note.txt"), "same").unwrap();
    std::fs::write(world.shared.join("note.txt"), "same").unwrap();

    let started = start(
        &world.daemon,
        start_request(
            &[in_clone("note.txt")],
            in_shared(""),
            pb::TransferMode::Copy,
            pb::OnExisting::KeepBoth,
            None,
        ),
    )
    .await
    .unwrap();
    let end = settled(&world.daemon, &id(&started)).await;

    assert_eq!(state(&end), pb::TransferState::Done, "{end:?}");
    assert!(!world.shared.join("note-2.txt").exists());
    assert_eq!(
        end.outcomes,
        vec![pb::TransferOutcome {
            landing: pb::TransferLanding::AlreadyThere as i32,
            saved_as: String::new(),
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_that_leaves_out_what_to_do_is_refused() {
    let world = world().await;
    std::fs::write(world.clone.join("note.txt"), "x").unwrap();

    let no_mode = start(
        &world.daemon,
        start_request(
            &[in_clone("note.txt")],
            in_shared(""),
            pb::TransferMode::Unspecified,
            pb::OnExisting::Fail,
            None,
        ),
    )
    .await
    .unwrap_err();
    let no_choice = start(
        &world.daemon,
        start_request(
            &[in_clone("note.txt")],
            in_shared(""),
            pb::TransferMode::Copy,
            pb::OnExisting::Unspecified,
            None,
        ),
    )
    .await
    .unwrap_err();
    let no_source = start(
        &world.daemon,
        start_request(&[], in_shared(""), pb::TransferMode::Copy, pb::OnExisting::Fail, None),
    )
    .await
    .unwrap_err();

    assert_eq!(invalid_field(&no_mode).0, "mode");
    assert_eq!(invalid_field(&no_choice).0, "on_existing");
    assert_eq!(invalid_field(&no_source).0, "sources");
    assert!(all(&world.daemon, false).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_source_outside_its_root_and_a_main_clone_destination_are_refused() {
    let world = world().await;
    std::fs::write(world.clone.join("note.txt"), "x").unwrap();
    let outside = world.clone.parent().unwrap().join("elsewhere");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(&outside, world.clone.join("way-out")).unwrap();

    let dots = start(
        &world.daemon,
        start_request(
            &[in_clone("../elsewhere/secret.txt")],
            in_shared(""),
            pb::TransferMode::Copy,
            pb::OnExisting::Fail,
            None,
        ),
    )
    .await
    .unwrap_err();
    let through_link = start(
        &world.daemon,
        start_request(
            &[in_clone("way-out/secret.txt")],
            in_shared(""),
            pb::TransferMode::Copy,
            pb::OnExisting::Fail,
            None,
        ),
    )
    .await
    .unwrap_err();
    let into_main = start(
        &world.daemon,
        start_request(
            &[in_clone("note.txt")],
            in_main(""),
            pb::TransferMode::Copy,
            pb::OnExisting::Fail,
            None,
        ),
    )
    .await
    .unwrap_err();

    for refused in [&dots, &through_link] {
        assert!(
            matches!(
                error_of(refused).kind,
                Some(pb::error::Kind::OutsideHumataFolders(_))
            ),
            "{refused:?}"
        );
    }
    assert_eq!(invalid_field(&into_main).0, "destination");
    assert!(std::fs::read_dir(&world.shared).unwrap().next().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn through_the_home_folder_a_main_clone_is_copied_out_of_but_never_into_or_away() {
    let world = world().await;
    let main = world.shared.parent().unwrap().join("main").join(REPO);
    std::fs::write(main.join("README.md"), "reference").unwrap();
    std::fs::write(world.clone.join("note.txt"), "x").unwrap();
    let main_at_home = format!("Humata/{CHANNEL_FOLDER}/main/{REPO}");
    let readme = format!("{main_at_home}/README.md");

    // Copy out is allowed and leaves the main clone as it was.
    let out = copy(&world.daemon, in_home(&readme), in_shared("")).await;
    let done = settled(&world.daemon, &id(&out)).await;
    assert_eq!(state(&done), pb::TransferState::Done, "{done:?}");
    assert_eq!(std::fs::read_to_string(world.shared.join("README.md")).unwrap(), "reference");

    // Copy into it, move out of it, move a folder that holds it: refused.
    let request = |sources: &[pb::FileLocation], destination, mode| {
        start_request(sources, destination, mode, pb::OnExisting::KeepBoth, None)
    };
    let into = start(
        &world.daemon,
        request(&[in_clone("note.txt")], in_home(&main_at_home), pb::TransferMode::Copy),
    )
    .await
    .unwrap_err();
    let away = start(
        &world.daemon,
        request(&[in_home(&readme)], in_shared(""), pb::TransferMode::Move),
    )
    .await
    .unwrap_err();
    let holder = format!("Humata/{CHANNEL_FOLDER}/main");
    let above = start(
        &world.daemon,
        request(&[in_home(&holder)], in_shared(""), pb::TransferMode::Move),
    )
    .await
    .unwrap_err();

    for refused in [&into, &away, &above] {
        assert!(
            matches!(error_of(refused).kind, Some(pb::error::Kind::MainCloneReadOnly(_))),
            "{refused:?}"
        );
    }
    assert_eq!(std::fs::read_to_string(main.join("README.md")).unwrap(), "reference");
    assert!(!main.join("note.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_key_starts_one_transfer_also_after_a_restart() {
    let mut world = world().await;
    std::fs::write(world.clone.join("note.txt"), "x").unwrap();
    let request = || {
        start_request(
            &[in_clone("note.txt")],
            in_shared(""),
            pb::TransferMode::Move,
            pb::OnExisting::Fail,
            Some("same-key"),
        )
    };

    let first = start(&world.daemon, request()).await.unwrap();
    let again = start(&world.daemon, request()).await.unwrap();
    assert_eq!(first.id, again.id);
    settled(&world.daemon, &id(&first)).await;
    assert_eq!(all(&world.daemon, false).await.len(), 1);

    world.daemon.restart().await;
    // The move finished and its source is gone, yet the key still answers.
    let after = start(&world.daemon, request()).await.unwrap();

    assert_eq!(after.id, first.id);
    assert_eq!(state(&after), pb::TransferState::Done);
    assert_eq!(all(&world.daemon, false).await.len(), 1);
    let other = start(
        &world.daemon,
        start_request(
            &[in_shared("note.txt")],
            in_clone(""),
            pb::TransferMode::Copy,
            pb::OnExisting::Fail,
            Some("another-key"),
        ),
    )
    .await
    .unwrap();
    assert_ne!(other.id, first.id);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_paused_copy_removes_what_arrived_and_leaves_the_source() {
    let world = world().await;
    large_tree(&world.clone, 12);
    let before = tree(&world.clone.join("tree"));

    let started = copy(&world.daemon, in_clone("tree"), in_shared("")).await;
    let paused = control(&world.daemon, &id(&started), "pause").await.unwrap();
    // A copy fast enough to be done before it could be paused can't be cancelled.
    if state(&paused) == pb::TransferState::Paused {
        let cancelled = control(&world.daemon, &id(&started), "cancel").await.unwrap();
        assert_eq!(state(&cancelled), pb::TransferState::Cancelled);
        assert!(!world.shared.join("tree").exists());
        no_staging(&world.shared);
    } else {
        assert_eq!(state(&paused), pb::TransferState::Done);
    }
    assert_eq!(tree(&world.clone.join("tree")), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_transfer_cannot_be_cancelled() {
    let world = world().await;
    std::fs::write(world.clone.join("note.txt"), "x").unwrap();
    let started = copy(&world.daemon, in_clone("note.txt"), in_shared("")).await;
    let end = settled(&world.daemon, &id(&started)).await;
    assert_eq!(state(&end), pb::TransferState::Done);

    let refused = control(&world.daemon, &id(&started), "cancel").await.unwrap_err();

    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::NotCancellable(_))),
        "{refused:?}"
    );
    assert!(world.shared.join("note.txt").exists());
}

/// A copy cut by a SIGKILL continues on its own after the restart (PER-27).
async fn killed_copy_finishes_after_restart(kill_after_a_third: bool) {
    let mut world = world().await;
    large_tree(&world.clone, 8);
    let before = tree(&world.clone.join("tree"));

    let started = copy(&world.daemon, in_clone("tree"), in_shared("")).await;
    if kill_after_a_third {
        until(&world.daemon, &id(&started), |change| {
            change.total_bytes > 0 && change.done_bytes * 3 > change.total_bytes
        })
        .await;
    }
    world.daemon.kill();
    world.daemon.restart().await;
    let end = settled(&world.daemon, &id(&started)).await;

    assert_eq!(state(&end), pb::TransferState::Done, "{end:?}");
    assert_eq!(end.done_bytes, end.total_bytes);
    assert_eq!(tree(&world.shared.join("tree")), before);
    no_staging(&world.shared);
    assert_eq!(all(&world.daemon, true).await.len(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copy_killed_a_third_of_the_way_finishes_by_itself_after_a_restart() {
    killed_copy_finishes_after_restart(true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_copy_killed_as_it_starts_finishes_by_itself_after_a_restart() {
    killed_copy_finishes_after_restart(false).await;
}

// ---------------------------------------------------------------------------
// Uploads.

/// The messages that upload `data` into the shared folder as `name`: a
/// header, then the chunks from byte `from`, each with the next of `codecs`.
/// `only` limits how many chunks follow the header.
fn upload_messages(
    name: &str,
    data: &[u8],
    sha256: &str,
    from: usize,
    resume: Option<&pb::TransferId>,
    codecs: &[Codec],
    only: Option<usize>,
) -> Vec<pb::UploadMessage> {
    use pb::upload_message::Message as Part;

    let mut messages = vec![pb::UploadMessage {
        message: Some(Part::Header(pb::UploadHeader {
            idempotency_key: None,
            destination: Some(in_shared("")),
            name: name.to_owned(),
            size: data.len() as u64,
            sha256: sha256.to_owned(),
            on_existing: pb::OnExisting::Fail as i32,
            resume: resume.cloned(),
        })),
    }];
    let mut encoder = Encoder::new(codecs[0]);
    let pieces: Vec<&[u8]> = data[from..].chunks(CHUNK_MAX).collect();
    for (index, piece) in pieces.iter().enumerate().take(only.unwrap_or(usize::MAX)) {
        encoder.set_codec(codecs[index % codecs.len()]);
        let chunk = encoder.encode(piece.to_vec()).unwrap();
        messages.push(pb::UploadMessage {
            message: Some(Part::Chunk(pb::FileChunk {
                offset: (from + index * CHUNK_MAX) as u64,
                data: chunk.data,
                last: index + 1 == pieces.len(),
                codec: chunk.header.codec.wire(),
                raw_size: chunk.header.raw_len,
                crc32: chunk.header.crc32,
                ..Default::default()
            })),
        });
    }
    messages
}

async fn send(daemon: &Daemon, messages: Vec<pb::UploadMessage>) -> Result<pb::Transfer, Status> {
    transfers(daemon)
        .await
        .upload(tokio_stream::iter(messages))
        .await
        .map(tonic::Response::into_inner)
}

const MIX: &[Codec] = &[Codec::Zstd, Codec::Lz4, Codec::None];

fn in_pastes(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::SessionPastes(pb::SessionPastes {
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
        }),
        path,
    )
}

/// The messages of a whole upload of `data` as `name` into `into`.
fn upload_into(into: pb::FileLocation, name: &str, data: &[u8]) -> Vec<pb::UploadMessage> {
    let mut messages = upload_messages(name, data, &sha256(data), 0, None, MIX, None);
    if let Some(pb::upload_message::Message::Header(header)) = messages[0].message.as_mut() {
        header.destination = Some(into);
    }
    messages
}

/// D335: a file uploaded into a session's pastes says where it is on the
/// machine's disk, which is what the app types; two pastes of one name land
/// apart. An upload anywhere else tells no path.
#[tokio::test(flavor = "multi_thread")]
async fn an_upload_into_a_sessions_pastes_says_where_it_is_on_disk() {
    let world = world().await;
    for folder in ["p1", "p2"] {
        files(&world.daemon)
            .await
            .create_folder(pb::CreateFolderRequest {
                idempotency_key: None,
                location: Some(in_pastes(folder)),
            })
            .await
            .expect("a paste's folder is made");
    }
    let (first, second) = (noise(11, 4096), noise(12, 4096));

    let one = send(&world.daemon, upload_into(in_pastes("p1"), "notes.txt", &first))
        .await
        .unwrap();
    let two = send(&world.daemon, upload_into(in_pastes("p2"), "notes.txt", &second))
        .await
        .unwrap();

    assert_eq!(state(&one), pb::TransferState::Done, "{one:?}");
    assert_eq!(state(&two), pb::TransferState::Done, "{two:?}");
    assert!(Path::new(&one.disk_path).is_absolute(), "{one:?}");
    assert_ne!(one.disk_path, two.disk_path);
    assert_eq!(std::fs::read(&one.disk_path).unwrap(), first);
    assert_eq!(std::fs::read(&two.disk_path).unwrap(), second);

    let shared = send(&world.daemon, upload_into(in_shared(""), "notes.txt", &first))
        .await
        .unwrap();
    assert_eq!(state(&shared), pb::TransferState::Done, "{shared:?}");
    assert!(shared.disk_path.is_empty(), "only pastes tell their path: {shared:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_in_chunks_arrives_whole_and_the_transfer_is_done() {
    let world = world().await;
    let data = noise(3, 3 << 20);

    let done = send(
        &world.daemon,
        upload_messages("up.bin", &data, &sha256(&data), 0, None, MIX, None),
    )
    .await
    .unwrap();

    assert_eq!(state(&done), pb::TransferState::Done, "{done:?}");
    assert_eq!(done.done_bytes, data.len() as u64);
    assert_eq!(std::fs::read(world.shared.join("up.bin")).unwrap(), data);
    no_staging(&world.shared);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_cut_short_is_paused_and_resumes_from_where_it_got() {
    let world = world().await;
    let data = noise(4, 3 << 20);
    let sum = sha256(&data);

    let cut = send(
        &world.daemon,
        upload_messages("up.bin", &data, &sum, 0, None, MIX, Some(1)),
    )
    .await
    .unwrap_err();
    assert_eq!(cut.code(), Code::Aborted, "{cut:?}");
    let paused = all(&world.daemon, true).await;
    assert_eq!(paused.len(), 1);
    let paused = &paused[0];
    assert_eq!(state(paused), pb::TransferState::Paused);
    assert_eq!(paused.done_bytes, CHUNK_MAX as u64);
    assert!(paused.problem.is_some());
    assert!(listed(&world.daemon, in_shared("")).await.is_empty());

    let done = send(
        &world.daemon,
        upload_messages(
            "up.bin",
            &data,
            &sum,
            paused.done_bytes as usize,
            paused.id.as_ref(),
            MIX,
            None,
        ),
    )
    .await
    .unwrap();

    assert_eq!(state(&done), pb::TransferState::Done, "{done:?}");
    assert_eq!(done.id, paused.id);
    assert_eq!(std::fs::read(world.shared.join("up.bin")).unwrap(), data);
    no_staging(&world.shared);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_that_names_another_file_is_refused() {
    let world = world().await;
    let data = noise(5, 2 << 20);
    send(
        &world.daemon,
        upload_messages("up.bin", &data, &sha256(&data), 0, None, MIX, Some(1)),
    )
    .await
    .unwrap_err();
    let paused = all(&world.daemon, true).await.remove(0);
    let other = noise(6, 2 << 20);

    let refused = send(
        &world.daemon,
        upload_messages(
            "other.bin",
            &other,
            &sha256(&other),
            CHUNK_MAX,
            paused.id.as_ref(),
            MIX,
            None,
        ),
    )
    .await
    .unwrap_err();

    assert_eq!(invalid_field(&refused).0, "header.resume");
    assert_eq!(state(&all(&world.daemon, false).await[0]), pb::TransferState::Paused);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chunk_at_the_wrong_offset_is_refused() {
    let world = world().await;
    let data = noise(7, 2 << 20);
    let mut messages = upload_messages("up.bin", &data, &sha256(&data), 0, None, MIX, None);
    let Some(pb::upload_message::Message::Chunk(chunk)) = &mut messages[1].message else {
        panic!("the second message is a chunk");
    };
    chunk.offset = 4096;

    let refused = send(&world.daemon, messages).await.unwrap_err();

    let (field, problem) = invalid_field(&refused);
    assert_eq!(field, "chunk.offset");
    assert_eq!(problem, pb::invalid_field::Problem::OutOfRange);
    assert!(listed(&world.daemon, in_shared("")).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_wrong_sha256_is_refused_and_nothing_appears() {
    let world = world().await;
    let data = noise(8, 2 << 20);

    let refused = send(
        &world.daemon,
        upload_messages("up.bin", &data, &sha256(b"something else"), 0, None, MIX, None),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::ChecksumMismatch(_))),
        "{refused:?}"
    );
    assert!(listed(&world.daemon, in_shared("")).await.is_empty());
    no_staging(&world.shared);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_interrupted_upload_survives_a_restart_and_can_still_be_resumed() {
    let mut world = world().await;
    let data = noise(9, 3 << 20);
    let sum = sha256(&data);
    send(
        &world.daemon,
        upload_messages("up.bin", &data, &sum, 0, None, MIX, Some(2)),
    )
    .await
    .unwrap_err();

    world.daemon.restart().await;
    let paused = all(&world.daemon, true).await.remove(0);

    assert_eq!(state(&paused), pb::TransferState::Paused);
    assert_eq!(paused.done_bytes, 2 * CHUNK_MAX as u64);
    let done = send(
        &world.daemon,
        upload_messages(
            "up.bin",
            &data,
            &sum,
            paused.done_bytes as usize,
            paused.id.as_ref(),
            MIX,
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(state(&done), pb::TransferState::Done, "{done:?}");
    assert_eq!(std::fs::read(world.shared.join("up.bin")).unwrap(), data);
}

// ---------------------------------------------------------------------------
// Watching.

#[tokio::test(flavor = "multi_thread")]
async fn a_watch_from_the_current_version_waits_for_the_next_change() {
    let world = world().await;
    let data = noise(10, 2 << 20);
    send(
        &world.daemon,
        upload_messages("up.bin", &data, &sha256(&data), 0, None, MIX, Some(1)),
    )
    .await
    .unwrap_err();
    let paused = all(&world.daemon, true).await.remove(0);
    let transfer = id(&paused);

    let mut changes = watch(&world.daemon, &transfer, paused.version).await.unwrap();
    // Nothing happens to a paused upload; the wait is for that quiet.
    let quiet = tokio::time::timeout(Duration::from_millis(400), changes.message()).await;
    assert!(quiet.is_err(), "the watch sent {quiet:?} with nothing changed");
    control(&world.daemon, &transfer, "cancel").await.unwrap();

    let next = tokio::time::timeout(LONG, changes.message())
        .await
        .unwrap()
        .unwrap()
        .expect("the cancel is sent to the watcher");
    assert_eq!(state(&next), pb::TransferState::Cancelled);
    assert!(next.version > paused.version);
    assert!(changes.message().await.unwrap().is_none(), "the watch ends after a final state");
}

#[tokio::test(flavor = "multi_thread")]
async fn watching_a_transfer_that_does_not_exist_is_not_found() {
    let world = world().await;

    let refused = watch(
        &world.daemon,
        &pb::TransferId {
            value: "tr_nothing".to_owned(),
        },
        0,
    )
    .await
    .unwrap_err();

    assert_eq!(not_found(&refused), pb::EntityKind::Transfer);
}
