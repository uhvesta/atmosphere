//! A session clone's file list is told on the feed (bug 23): the user's
//! writes, renames and deletes through the file service each come out as a
//! FileChanged event on the session's topic, so the palette's Files group
//! stays current without asking again.

use humatad_test_common as common;

use std::path::PathBuf;
use std::time::Duration;

use common::{Daemon, folder, state_folder};
use humata2_chunk::{Codec, Encoder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store, TabKind, TabRecord};
use sha2::{Digest, Sha256};
use tonic::Streaming;
use tonic::transport::Channel;

const SESSION: &str = "se_changes";
const REPO: &str = "api";
const WAIT: Duration = Duration::from_secs(60);

async fn daemon_with_clone() -> (Daemon, PathBuf) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_test".to_owned(),
            machine: String::new(),
            title: "changes".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);
    let clone = dir.path().join(REPO);
    std::fs::create_dir(&clone).unwrap();
    (Daemon::start_in(dir).await, clone)
}

async fn files(daemon: &Daemon) -> pb::file_service_client::FileServiceClient<Channel> {
    pb::file_service_client::FileServiceClient::new(daemon.channel().await)
}

fn location(path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: REPO.to_owned(),
            }),
        })),
        path: path.to_owned(),
    }
}

/// The messages that upload `data` to `path`, replacing what is there.
fn upload(path: &str, data: &[u8], replace: bool) -> Vec<pb::WriteFileMessage> {
    use pb::write_file_message::Message as Part;
    let sha: String = Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect();
    let chunk = Encoder::new(Codec::None).encode(data.to_vec()).unwrap();
    vec![
        pb::WriteFileMessage {
            message: Some(Part::Header(pb::WriteFileHeader {
                idempotency_key: None,
                location: Some(location(path)),
                size: data.len() as u64,
                sha256: sha,
                replace,
                expected_sha256: None,
            })),
        },
        pb::WriteFileMessage {
            message: Some(Part::Chunk(pb::FileChunk {
                offset: 0,
                data: chunk.data,
                last: true,
                codec: chunk.header.codec.wire(),
                raw_size: chunk.header.raw_len,
                crc32: chunk.header.crc32,
                ..Default::default()
            })),
        },
    ]
}

async fn write(daemon: &Daemon, path: &str, data: &[u8], replace: bool) {
    files(daemon)
        .await
        .write_file(tokio_stream::iter(upload(path, data, replace)))
        .await
        .unwrap_or_else(|status| panic!("the write of {path} failed: {status}"));
}

async fn session_events(daemon: &Daemon) -> Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            sessions: vec![pb::SessionId {
                value: SESSION.to_owned(),
            }],
            topics: vec![pb::Topic::Sessions as i32],
            ..pb::SubscribeRequest::default()
        })
        .await
        .unwrap()
        .into_inner()
}

/// The next FileChanged on the stream.
async fn next_change(events: &mut Streaming<pb::Event>) -> pb::FileChanged {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let event = tokio::time::timeout_at(deadline, events.message())
            .await
            .expect("no FileChanged came on the feed")
            .unwrap()
            .expect("the feed ended");
        if let Some(pb::event::Payload::FileChanged(changed)) = event.payload {
            return changed;
        }
    }
}

fn kind(changed: &pb::FileChanged) -> pb::file_changed::Kind {
    pb::file_changed::Kind::try_from(changed.kind).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_the_user_writes_into_a_clone_is_told_as_created() {
    let (daemon, _clone) = daemon_with_clone().await;
    let mut events = session_events(&daemon).await;

    write(&daemon, "brand_new.rs", b"fn main() {}\n", false).await;

    let changed = next_change(&mut events).await;
    assert_eq!(kind(&changed), pb::file_changed::Kind::Created);
    assert_eq!(changed.path, "brand_new.rs");
    assert_eq!(changed.session.as_ref().map(|id| id.value.as_str()), Some(SESSION));
    assert_eq!(changed.repo.as_ref().map(|repo| repo.name.as_str()), Some(REPO));
}

#[tokio::test(flavor = "multi_thread")]
async fn replacing_a_files_text_changes_no_list_and_tells_nothing() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("old.rs"), b"one\n").unwrap();
    let mut events = session_events(&daemon).await;

    write(&daemon, "old.rs", b"two\n", true).await;
    write(&daemon, "after.rs", b"x\n", false).await;

    // Events arrive in order: the replace told nothing before this one.
    let changed = next_change(&mut events).await;
    assert_eq!(kind(&changed), pb::file_changed::Kind::Created);
    assert_eq!(changed.path, "after.rs");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_the_user_renames_is_told_as_renamed() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("a.rs"), b"x\n").unwrap();
    let mut events = session_events(&daemon).await;

    files(&daemon)
        .await
        .rename(pb::RenameRequest {
            location: Some(location("a.rs")),
            new_name: "b.rs".to_owned(),
            ..Default::default()
        })
        .await
        .unwrap();

    let changed = next_change(&mut events).await;
    assert_eq!(kind(&changed), pb::file_changed::Kind::Renamed);
    assert_eq!((changed.old_path.as_str(), changed.path.as_str()), ("a.rs", "b.rs"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_the_user_deletes_is_told_as_deleted() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("gone.rs"), b"x\n").unwrap();
    let mut events = session_events(&daemon).await;

    files(&daemon)
        .await
        .move_to_trash(pb::MoveToTrashRequest {
            locations: vec![location("gone.rs")],
            ..Default::default()
        })
        .await
        .unwrap();

    let changed = next_change(&mut events).await;
    assert_eq!(kind(&changed), pb::file_changed::Kind::Deleted);
    assert_eq!(changed.path, "gone.rs");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_written_to_the_session_folder_is_told_without_a_repo() {
    use pb::write_file_message::Message as Part;
    let (daemon, _clone) = daemon_with_clone().await;
    let mut events = session_events(&daemon).await;

    let data = b"# Notes\n";
    let sha: String = Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let chunk = Encoder::new(Codec::None).encode(data.to_vec()).unwrap();
    let location = pb::FileLocation {
        machine: None,
        root: Some(pb::file_location::Root::SessionFolder(pb::SessionFolder {
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
        })),
        path: "notes.md".to_owned(),
    };
    let messages = vec![
        pb::WriteFileMessage {
            message: Some(Part::Header(pb::WriteFileHeader {
                idempotency_key: None,
                location: Some(location),
                size: data.len() as u64,
                sha256: sha,
                replace: false,
                expected_sha256: None,
            })),
        },
        pb::WriteFileMessage {
            message: Some(Part::Chunk(pb::FileChunk {
                offset: 0,
                data: chunk.data,
                last: true,
                codec: chunk.header.codec.wire(),
                raw_size: chunk.header.raw_len,
                crc32: chunk.header.crc32,
                ..Default::default()
            })),
        },
    ];
    files(&daemon)
        .await
        .write_file(tokio_stream::iter(messages))
        .await
        .unwrap();

    let changed = next_change(&mut events).await;
    assert_eq!(kind(&changed), pb::file_changed::Kind::Created);
    assert_eq!(changed.path, "notes.md");
    assert_eq!(changed.session.as_ref().map(|id| id.value.as_str()), Some(SESSION));
    assert!(changed.repo.is_none(), "session-folder changes have no repo: {changed:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn find_files_primes_empty_session_folder_and_shell_write_emits_created() {
    let dir = folder();
    let session_dir = dir.path().join("session_root");
    std::fs::create_dir(&session_dir).unwrap();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_test".to_owned(),
            machine: String::new(),
            title: "changes".to_owned(),
            agent: "zsh".to_owned(),
            folder: session_dir.clone(),
            created_unix_ms: 0,
        })
        .unwrap();
    // Live shell holder (no agent record) so the session folder watcher is active.
    store
        .put_tab(&TabRecord {
            id: "tb_shell".to_owned(),
            session: Some(SESSION.to_owned()),
            kind: TabKind::Shell,
            title: "zsh".to_owned(),
            holder_socket: dir.path().join("tb_shell.sock"),
            holder_pid: std::process::id(),
            created_unix_ms: 0,
            holder_generation: 0,
        })
        .unwrap();
    drop(store);

    let daemon = Daemon::start_in(dir).await;
    let mut events = session_events(&daemon).await;

    let listed = files(&daemon)
        .await
        .find_files(pb::FindFilesRequest {
            session: Some(pb::SessionId {
                value: SESSION.to_owned(),
            }),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert!(listed.entries.is_empty(), "fresh session root starts empty");

    // Write directly to disk as a shell tab or external tool would.
    std::fs::write(session_dir.join("CHANNEL_HOME_SPEC.md"), b"# Spec\n").unwrap();

    let changed = next_change(&mut events).await;
    assert_eq!(kind(&changed), pb::file_changed::Kind::Created);
    assert_eq!(changed.path, "CHANNEL_HOME_SPEC.md");
    assert_eq!(changed.session.as_ref().map(|id| id.value.as_str()), Some(SESSION));
    assert!(changed.repo.is_none(), "session-root files have no repo: {changed:?}");
}
