//! `humata files …` (CLI.md §5.5): locations, reading and writing files.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use pb::file_location::Root;
use pb::write_file_message::Message as Part;
use support::{FakeConsole, FakeDaemon, run};

// SHA-256 of "hello".
const HELLO_SHA256: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn chunk(data: &[u8], offset: u64, last: bool) -> pb::FileChunk {
    pb::FileChunk {
        offset,
        data: data.to_vec(),
        sha256: String::new(),
        last,
    }
}

#[tokio::test]
async fn a_session_location_names_machine_session_repo_and_path() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "files", "ls", "ma_1:se_1/acme/web/src/lib"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let sent: pb::ListDirectoryRequest = daemon.only(rpc::files::LIST_DIRECTORY);
    assert_eq!(
        sent.location,
        Some(pb::FileLocation {
            machine: support::machine("ma_1"),
            root: Some(Root::SessionClone(pb::SessionClone {
                session: support::session("se_1"),
                repo: Some(pb::RepoRef {
                    owner: "acme".into(),
                    name: "web".into(),
                }),
            })),
            path: "src/lib".into(),
        })
    );
}

#[tokio::test]
async fn a_location_without_a_repo_is_refused() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "files", "ls", "ma_1:se_1/acme"]).await;
    assert_eq!(exit, exit::USAGE);
    assert!(!daemon.called(rpc::files::LIST_DIRECTORY));
}

#[tokio::test]
async fn cat_writes_the_bytes_as_they_are() {
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::files::READ_FILE,
        &[&chunk(b"hel", 0, false), &chunk(b"lo", 3, true)],
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--channel", "ch_1", "files", "cat", "ma_1:shared/notes.txt"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert_eq!(console.written(), b"hello");
}

#[tokio::test]
async fn cat_with_json_gives_base64_size_and_digest() {
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::files::READ_FILE,
        &[&chunk(b"hel", 0, false), &chunk(b"lo", 3, true)],
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "cat", "ma_1:shared/notes.txt"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let result = &console.json()["result"];
    assert_eq!(result["data"], "aGVsbG8=");
    assert_eq!(result["bytes"], 5);
    assert_eq!(result["sha256"], HELLO_SHA256);
}

#[tokio::test]
async fn put_sends_a_header_then_the_file() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let local = folder.path().join("hello.txt");
    std::fs::write(&local, b"hello").expect("the local file");
    let local = local.to_str().expect("a UTF-8 path");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &[
            "--json", "--channel", "ch_1", "files", "put", local, "ma_1:channel/hello.txt",
            "--replace",
        ],
    )
    .await;
    assert_eq!(exit, exit::OK);
    let parts: Vec<Option<Part>> = daemon
        .sent::<pb::WriteFileMessage>(rpc::files::WRITE_FILE)
        .into_iter()
        .map(|message| message.message)
        .collect();
    let Some(Some(Part::Header(header))) = parts.first() else {
        panic!("the first message is the header: {parts:?}");
    };
    assert_eq!(header.size, 5);
    assert_eq!(header.sha256, HELLO_SHA256);
    assert!(header.replace);
    assert_eq!(header.location.as_ref().map(|location| location.path.as_str()), Some("hello.txt"));
    let data: Vec<u8> = parts[1..]
        .iter()
        .flat_map(|part| match part {
            Some(Part::Chunk(chunk)) => chunk.data.clone(),
            other => panic!("only chunks follow the header: {other:?}"),
        })
        .collect();
    assert_eq!(data, b"hello");
    assert!(matches!(parts.last(), Some(Some(Part::Chunk(chunk))) if chunk.last));
}

#[tokio::test]
async fn put_of_a_missing_local_file_sends_nothing() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let missing = folder.path().join("missing.txt");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &[
            "--json",
            "--channel",
            "ch_1",
            "files",
            "put",
            missing.to_str().expect("a UTF-8 path"),
            "ma_1:channel/missing.txt",
        ],
    )
    .await;
    assert_ne!(exit, exit::OK);
    assert!(!daemon.called(rpc::files::WRITE_FILE));
}

#[tokio::test]
async fn mv_renames_in_place_and_refuses_a_path() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let renamed = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "mv", "ma_1:shared/a.txt", "b.txt"],
    )
    .await;
    assert_eq!(renamed, exit::OK);
    let sent: pb::RenameRequest = daemon.only(rpc::files::RENAME);
    assert_eq!(sent.new_name, "b.txt");
    let console = FakeConsole::pipe();
    let moved = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "mv", "ma_1:shared/a.txt", "docs/b.txt"],
    )
    .await;
    assert_eq!(moved, exit::USAGE);
    assert_eq!(daemon.sent::<pb::RenameRequest>(rpc::files::RENAME).len(), 1);
}

#[tokio::test]
async fn cp_needs_a_source_and_a_destination() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "cp", "ma_1:shared/a.txt"]).await;
    assert_eq!(exit, exit::USAGE);
}
