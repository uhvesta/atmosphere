//! TransferEndService through `humatad2`'s gRPC API: the steps of a
//! transfer the host drives, one by one (FM-6, FM-8, FM-9, D142).

use humatad_test_common as common;

use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::PathBuf;

use common::{Daemon, error_of, folder, invalid_field, state_folder};
use humata2_chunk::{CHUNK_MAX, Codec, Encoder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, ChannelRepo, MainClone as MainState, SessionRecord, Store};
use sha2::{Digest, Sha256};
use tonic::transport::Channel;
use tonic::{Code, Status};

const SESSION: &str = "se_te";
const CHANNEL: &str = "ch_te";
const CHANNEL_FOLDER: &str = "te-channel";
const REPO: &str = "api";
const STAGING: &str = ".humata-transfer-t1";
const MIB: usize = 1 << 20;

struct World {
    daemon: Daemon,
    shared: PathBuf,
}

async fn world() -> World {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: CHANNEL.to_owned(),
            machine: String::new(),
            title: "transfer end".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    store
        .put_channel(&ChannelRecord {
            id: CHANNEL.to_owned(),
            name: "transfer end".to_owned(),
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
        shared,
    }
}

async fn end(daemon: &Daemon) -> pb::transfer_end_service_client::TransferEndServiceClient<Channel> {
    pb::transfer_end_service_client::TransferEndServiceClient::new(daemon.channel().await)
}

async fn files(daemon: &Daemon) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
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

fn in_shared(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::ChannelShared(pb::ChannelShared {
            channel: channel_id(),
        }),
        path,
    )
}

fn in_channel(path: &str) -> pb::FileLocation {
    at(
        pb::file_location::Root::ChannelFolder(pb::ChannelFolder {
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

fn data(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8 ^ seed).collect()
}

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn chunk(offset: usize, bytes: &[u8], last: bool) -> pb::WriteAtMessage {
    let encoded = Encoder::new(Codec::None).encode(bytes.to_vec()).unwrap();
    pb::WriteAtMessage {
        message: Some(pb::write_at_message::Message::Chunk(pb::FileChunk {
            offset: offset as u64,
            data: encoded.data,
            last,
            codec: encoded.header.codec.wire(),
            raw_size: encoded.header.raw_len,
            crc32: encoded.header.crc32,
            ..Default::default()
        })),
    }
}

fn header(location: pb::FileLocation, offset: usize) -> pb::WriteAtMessage {
    pb::WriteAtMessage {
        message: Some(pb::write_at_message::Message::Header(pb::WriteAtHeader {
            location: Some(location),
            offset: offset as u64,
        })),
    }
}

/// Writes `bytes` to `location` from `offset` in chunks; the confirmed
/// offset.
async fn write_at(
    daemon: &Daemon,
    location: pb::FileLocation,
    offset: usize,
    bytes: &[u8],
) -> Result<u64, Status> {
    let mut messages = vec![header(location, offset)];
    let pieces: Vec<&[u8]> = bytes.chunks(CHUNK_MAX).collect();
    for (index, piece) in pieces.iter().enumerate() {
        messages.push(chunk(offset + index * CHUNK_MAX, piece, index + 1 == pieces.len()));
    }
    end(daemon)
        .await
        .write_at(tokio_stream::iter(messages))
        .await
        .map(|response| response.into_inner().confirmed_offset)
}

async fn make_folder(daemon: &Daemon, location: pb::FileLocation) -> Result<(), Status> {
    end(daemon)
        .await
        .make_folder(pb::MakeFolderRequest {
            location: Some(location),
            mode: 0o755,
        })
        .await
        .map(|_| ())
}

async fn move_entry(daemon: &Daemon, from: pb::FileLocation, to: pb::FileLocation) -> Result<(), Status> {
    end(daemon)
        .await
        .move_entry(pb::MoveEntryRequest {
            from: Some(from),
            to: Some(to),
        })
        .await
        .map(|_| ())
}

async fn hash(daemon: &Daemon, location: pb::FileLocation, bytes: usize) -> Result<String, Status> {
    end(daemon)
        .await
        .hash_prefix(pb::HashPrefixRequest {
            location: Some(location),
            bytes: bytes as u64,
        })
        .await
        .map(|response| response.into_inner().sha256)
}

async fn remove_staged(daemon: &Daemon, location: pb::FileLocation) -> Result<(), Status> {
    end(daemon)
        .await
        .remove_staged(pb::EndPathRequest {
            location: Some(location),
        })
        .await
        .map(|_| ())
}

async fn exists(daemon: &Daemon, location: pb::FileLocation) -> bool {
    end(daemon)
        .await
        .exists(pb::EndPathRequest {
            location: Some(location),
        })
        .await
        .unwrap()
        .into_inner()
        .exists
}

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
        .filter_map(|entry| entry.location.as_ref())
        .map(|location| location.path.rsplit('/').next().unwrap_or_default().to_owned())
        .collect()
}

fn outside(status: &Status) {
    assert_eq!(status.code(), Code::PermissionDenied, "{status:?}");
    assert!(
        matches!(error_of(status).kind, Some(pb::error::Kind::OutsideHumataFolders(_))),
        "{status:?}"
    );
}

#[tokio::test]
async fn the_steps_of_one_small_transfer_end_with_the_file_in_place() {
    let world = world().await;
    let bytes = data(3 * MIB, 7);
    make_folder(&world.daemon, in_shared(STAGING)).await.unwrap();
    let staged = format!("{STAGING}/data.bin");
    let first = write_at(&world.daemon, in_shared(&staged), 0, &bytes[..2 * MIB])
        .await
        .unwrap();
    assert_eq!(first, 2 * MIB as u64);
    let second = write_at(&world.daemon, in_shared(&staged), first as usize, &bytes[2 * MIB..])
        .await
        .unwrap();
    assert_eq!(second, 3 * MIB as u64);
    assert_eq!(
        hash(&world.daemon, in_shared(&staged), bytes.len()).await.unwrap(),
        sha(&bytes)
    );
    end(&world.daemon)
        .await
        .finish_file(pb::FinishFileRequest {
            location: Some(in_shared(&staged)),
            mode: 0o755,
            modified_at: Some(humata2_grpc::Timestamp {
                seconds: 1_700_000_000,
                nanos: 0,
            }),
        })
        .await
        .unwrap();
    // The staging folder is never shown while the transfer builds in it.
    assert!(!listed(&world.daemon, in_shared("")).await.iter().any(|n| n == STAGING));
    end(&world.daemon)
        .await
        .sync_folder(pb::EndPathRequest {
            location: Some(in_shared(STAGING)),
        })
        .await
        .unwrap();
    move_entry(&world.daemon, in_shared(&staged), in_shared("data.bin"))
        .await
        .unwrap();
    remove_staged(&world.daemon, in_shared(STAGING)).await.unwrap();

    let placed = world.shared.join("data.bin");
    assert_eq!(std::fs::read(&placed).unwrap(), bytes);
    let meta = std::fs::metadata(&placed).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o755);
    assert_eq!(meta.mtime(), 1_700_000_000);
    assert!(!world.shared.join(STAGING).exists());
}

#[tokio::test]
async fn writing_below_what_is_there_cuts_the_file_first() {
    let world = world().await;
    let old = data(2 * MIB, 1);
    let new = data(MIB, 9);
    write_at(&world.daemon, in_shared("f.bin"), 0, &old).await.unwrap();
    let confirmed = write_at(&world.daemon, in_shared("f.bin"), MIB, &new).await.unwrap();
    assert_eq!(confirmed, 2 * MIB as u64);
    let mut expected = old[..MIB].to_vec();
    expected.extend_from_slice(&new);
    assert_eq!(std::fs::read(world.shared.join("f.bin")).unwrap(), expected);
}

#[tokio::test]
async fn a_chunk_at_the_wrong_offset_is_refused_and_what_was_confirmed_stays() {
    let world = world().await;
    let old = data(MIB, 3);
    write_at(&world.daemon, in_shared("f.bin"), 0, &old).await.unwrap();
    let messages = vec![
        header(in_shared("f.bin"), MIB),
        chunk(MIB + 5, &data(100, 4), true),
    ];
    let status = end(&world.daemon)
        .await
        .write_at(tokio_stream::iter(messages))
        .await
        .unwrap_err();
    let (field, problem) = invalid_field(&status);
    assert_eq!(field, "chunk.offset");
    assert_eq!(problem, pb::invalid_field::Problem::OutOfRange);
    assert_eq!(hash(&world.daemon, in_shared("f.bin"), MIB).await.unwrap(), sha(&old));
}

#[tokio::test]
async fn plan_tree_lists_folders_before_what_they_hold_and_never_follows_links() {
    let world = world().await;
    let tree = world.shared.join("src");
    std::fs::create_dir_all(tree.join("nested")).unwrap();
    std::fs::write(tree.join("nested").join("empty"), b"").unwrap();
    std::fs::write(tree.join("run.sh"), b"#!/bin/sh\n").unwrap();
    std::fs::set_permissions(tree.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    symlink("nested/empty", tree.join("link")).unwrap();

    let mut stream = end(&world.daemon)
        .await
        .plan_tree(pb::PlanTreeRequest {
            source: Some(in_shared("src")),
        })
        .await
        .unwrap()
        .into_inner();
    let mut entries = Vec::new();
    while let Some(entry) = stream.message().await.unwrap() {
        entries.push(entry);
    }
    let rels: Vec<&str> = entries.iter().map(|e| e.rel.as_str()).collect();
    assert_eq!(rels, ["", "link", "nested", "nested/empty", "run.sh"]);
    let by = |rel: &str| entries.iter().find(|e| e.rel == rel).unwrap();
    assert_eq!(by("").kind, pb::file_entry::Kind::Folder as i32);
    assert_eq!(by("link").kind, pb::file_entry::Kind::Symlink as i32);
    assert_eq!(by("link").link_target, "nested/empty");
    assert_eq!(by("nested").kind, pb::file_entry::Kind::Folder as i32);
    assert_eq!(by("nested/empty").size, 0);
    assert_eq!(by("run.sh").size, 10);
    assert_eq!(by("run.sh").mode & 0o777, 0o755);
}

#[tokio::test]
async fn exists_is_true_for_a_dangling_link_and_false_for_nothing() {
    let world = world().await;
    symlink("nowhere", world.shared.join("dangling")).unwrap();
    assert!(exists(&world.daemon, in_shared("dangling")).await);
    assert!(!exists(&world.daemon, in_shared("nothing")).await);
}

#[tokio::test]
async fn moving_onto_an_existing_name_moves_nothing() {
    let world = world().await;
    std::fs::write(world.shared.join("a"), b"a").unwrap();
    std::fs::write(world.shared.join("b"), b"b").unwrap();
    let status = move_entry(&world.daemon, in_shared("a"), in_shared("b")).await.unwrap_err();
    assert_eq!(status.code(), Code::AlreadyExists);
    assert!(matches!(error_of(&status).kind, Some(pb::error::Kind::NameTaken(_))));
    assert_eq!(std::fs::read(world.shared.join("a")).unwrap(), b"a");
    assert_eq!(std::fs::read(world.shared.join("b")).unwrap(), b"b");
}

#[tokio::test]
async fn moving_across_two_roots_is_refused() {
    let world = world().await;
    std::fs::write(world.shared.join("a"), b"a").unwrap();
    let status = move_entry(&world.daemon, in_shared("a"), in_channel("a")).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert!(world.shared.join("a").exists());
    assert!(!world.shared.parent().unwrap().join("a").exists());
}

#[tokio::test]
async fn remove_staged_refuses_what_is_not_in_a_staging_folder() {
    let world = world().await;
    std::fs::create_dir_all(world.shared.join("keep")).unwrap();
    std::fs::write(world.shared.join("keep").join("x"), b"x").unwrap();
    let status = remove_staged(&world.daemon, in_shared("keep")).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert!(world.shared.join("keep").join("x").exists());
    // A staging folder that is already gone is fine.
    remove_staged(&world.daemon, in_shared(".humata-transfer-gone")).await.unwrap();
}

#[tokio::test]
async fn nothing_leaves_its_root() {
    let world = world().await;
    let elsewhere = tempfile::tempdir().unwrap();
    symlink(elsewhere.path(), world.shared.join("out")).unwrap();
    std::fs::write(world.shared.join("a"), b"a").unwrap();

    outside(&make_folder(&world.daemon, in_shared("../x")).await.unwrap_err());
    outside(&make_folder(&world.daemon, in_shared("out/x")).await.unwrap_err());
    outside(&write_at(&world.daemon, in_shared("../x"), 0, b"x").await.unwrap_err());
    outside(&write_at(&world.daemon, in_shared("out/x"), 0, b"x").await.unwrap_err());
    outside(&move_entry(&world.daemon, in_shared("a"), in_shared("../a")).await.unwrap_err());
    outside(&move_entry(&world.daemon, in_shared("a"), in_shared("out/a")).await.unwrap_err());
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    assert!(world.shared.join("a").exists());
}

/// FC-2: the refusal every change to a main clone gets, here as from
/// FileService: the read-only-main-clone kind.
fn main_clone_refused(status: &Status) {
    assert!(
        matches!(error_of(status).kind, Some(pb::error::Kind::MainCloneReadOnly(_))),
        "{status:?}"
    );
}

#[tokio::test]
async fn a_main_clone_refuses_every_change() {
    let world = world().await;
    main_clone_refused(&make_folder(&world.daemon, in_main(STAGING)).await.unwrap_err());
    main_clone_refused(&write_at(&world.daemon, in_main("f"), 0, b"x").await.unwrap_err());
    main_clone_refused(&remove_staged(&world.daemon, in_main(STAGING)).await.unwrap_err());
    main_clone_refused(&move_entry(&world.daemon, in_main("a"), in_main("b")).await.unwrap_err());
}

#[tokio::test]
async fn free_space_of_the_shared_folder_is_positive() {
    let world = world().await;
    let free = end(&world.daemon)
        .await
        .free_space(pb::FreeSpaceRequest {
            folder: Some(in_shared("")),
        })
        .await
        .unwrap()
        .into_inner()
        .free_bytes;
    assert!(free > 0);
}
