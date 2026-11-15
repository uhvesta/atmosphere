//! `humata files …` (CLI.md §5.5): locations, reading and writing files.

mod support;

use humata2_chunk::{Codec, Decoder, Encoder, Header};
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
        ..Default::default()
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
async fn put_of_a_big_file_sends_it_in_order_with_one_last_chunk() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let local = folder.path().join("big.bin");
    let bytes: Vec<u8> = (0..(5 << 19) + 7).map(|index: u32| (index % 251) as u8).collect();
    std::fs::write(&local, &bytes).expect("the local file");
    let local = local.to_str().expect("a UTF-8 path");
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "put", local, "ma_1:channel/big.bin"],
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
    assert_eq!(header.size, bytes.len() as u64);
    let chunks: Vec<&pb::FileChunk> = parts[1..]
        .iter()
        .map(|part| match part {
            Some(Part::Chunk(chunk)) => chunk,
            other => panic!("only chunks follow the header: {other:?}"),
        })
        .collect();
    let mut sent = Vec::new();
    for chunk in &chunks {
        assert_eq!(chunk.offset, sent.len() as u64);
        sent.extend_from_slice(&chunk.data);
    }
    assert_eq!(sent, bytes);
    assert_eq!(chunks.iter().filter(|chunk| chunk.last).count(), 1);
    assert!(chunks.last().is_some_and(|chunk| chunk.last));
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

/// The bytes a `files put` sent, decoded, and the codecs its chunks used.
fn received(daemon: &FakeDaemon) -> (Vec<u8>, Vec<Codec>) {
    let mut decoder = Decoder::new();
    let mut bytes = Vec::new();
    let mut codecs = Vec::new();
    for message in daemon.sent::<pb::WriteFileMessage>(rpc::files::WRITE_FILE) {
        let Some(Part::Chunk(chunk)) = message.message else { continue };
        assert_eq!(chunk.offset, bytes.len() as u64, "chunks arrive in order");
        let codec = Codec::from_wire(chunk.codec).expect("a known codec");
        codecs.push(codec);
        let header = Header {
            codec,
            raw_len: chunk.raw_size,
            crc32: chunk.crc32,
        };
        bytes.extend(decoder.decode(header, chunk.data).expect("the chunk decodes"));
    }
    (bytes, codecs)
}

fn source_like(len: usize) -> Vec<u8> {
    let line = b"pub fn put(local: &Path, location: FileLocation) -> Result<Done, CliError> {}\n";
    line.iter().copied().cycle().take(len).collect()
}

async fn put(daemon: &FakeDaemon, bytes: &[u8]) {
    let folder = tempfile::tempdir().expect("a temporary folder");
    let local = folder.path().join("src.rs");
    std::fs::write(&local, bytes).expect("the local file");
    let console = FakeConsole::pipe();
    let local = local.to_str().expect("a UTF-8 path");
    let exit = run(
        daemon,
        &console,
        &["--json", "--channel", "ch_1", "files", "put", local, "ma_1:channel/src.rs"],
    )
    .await;
    assert_eq!(exit, exit::OK);
}

#[tokio::test]
async fn put_compresses_with_the_first_codec_the_daemon_offers() {
    let bytes = source_like((5 << 19) + 7);
    for (offered, expected) in [
        (vec![Codec::Lz4, Codec::Zstd, Codec::None], Codec::Lz4),
        (vec![Codec::Zstd, Codec::Lz4, Codec::None], Codec::Zstd),
        (vec![Codec::None], Codec::None),
    ] {
        let daemon = FakeDaemon::new();
        daemon.reply(
            rpc::daemon::GET_SERVER_INFO,
            &pb::ServerInfo {
                chunk_codecs: Codec::wire_list(&offered),
                ..Default::default()
            },
        );
        put(&daemon, &bytes).await;
        let (sent, codecs) = received(&daemon);
        assert_eq!(sent, bytes, "offered {offered:?}");
        assert!(codecs.contains(&expected), "offered {offered:?}, sent {codecs:?}");
        assert!(codecs.iter().all(|codec| [expected, Codec::None].contains(codec)));
    }
}

#[tokio::test]
async fn put_to_a_daemon_that_names_no_codec_sends_raw_chunks() {
    let bytes = source_like(3 << 19);
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::daemon::GET_SERVER_INFO,
        &pb::ServerInfo {
            chunk_codecs: vec![9],
            ..Default::default()
        },
    );
    put(&daemon, &bytes).await;
    let (sent, codecs) = received(&daemon);
    assert_eq!(sent, bytes);
    assert!(codecs.iter().all(|codec| *codec == Codec::None));
}

fn encoded(codec: Codec, raw: &[u8], offset: u64, last: bool) -> pb::FileChunk {
    let encoded = Encoder::new(codec).encode(raw.to_vec()).expect("encodes");
    pb::FileChunk {
        offset,
        data: encoded.data,
        sha256: String::new(),
        last,
        codec: encoded.header.codec.wire(),
        raw_size: encoded.header.raw_len,
        crc32: encoded.header.crc32,
    }
}

#[tokio::test]
async fn cat_decodes_chunks_whatever_their_codec() {
    let first = source_like(1 << 20);
    let second = source_like(4000);
    let daemon = FakeDaemon::new();
    daemon.stream(
        rpc::files::READ_FILE,
        &[
            &encoded(Codec::Zstd, &first, 0, false),
            &encoded(Codec::Lz4, &second, 1 << 20, true),
        ],
    );
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--channel", "ch_1", "files", "cat", "ma_1:shared/src.rs"],
    )
    .await;
    assert_eq!(exit, exit::OK);
    assert_eq!(console.written(), [first, second].concat());
    let sent: pb::ReadFileRequest = daemon.only(rpc::files::READ_FILE);
    assert!(Codec::known(&sent.accept_codecs).contains(&Codec::Lz4));
}

#[tokio::test]
async fn cat_of_a_damaged_chunk_fails_rather_than_printing_it() {
    let mut chunk = encoded(Codec::Lz4, &source_like(64 << 10), 0, true);
    let middle = chunk.data.len() / 2;
    chunk.data[middle] ^= 0x5a;
    let daemon = FakeDaemon::new();
    daemon.stream(rpc::files::READ_FILE, &[&chunk]);
    let console = FakeConsole::pipe();
    let exit = run(
        &daemon,
        &console,
        &["--channel", "ch_1", "files", "cat", "ma_1:shared/src.rs"],
    )
    .await;
    assert_ne!(exit, exit::OK);
    assert!(console.written().is_empty());
}
