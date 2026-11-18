//! FileService through `humatad2`'s gRPC API: files in a session's clone
//! read and written as chunks, compressed when both ends can (TR-11, D65).

mod common;

use std::path::PathBuf;

use common::{Daemon, folder, state_folder};
use humata2_chunk::{CHUNK_MAX, Codec, Decoder, Encoder, Header};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use sha2::{Digest, Sha256};
use tokio_stream::StreamExt;
use tonic::Code;
use tonic::transport::Channel;

const SESSION: &str = "se_files";
const REPO: &str = "api";

/// A daemon that knows one session, `SESSION`, and the folder of its
/// clone of `REPO`.
async fn daemon_with_clone() -> (Daemon, PathBuf) {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_session(&SessionRecord {
            id: SESSION.to_owned(),
            channel: "ch_test".to_owned(),
            machine: String::new(),
            title: "files".to_owned(),
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

/// About `len` bytes of source-like text: compresses well.
fn text(len: usize) -> Vec<u8> {
    let mut text = Vec::with_capacity(len);
    let mut line = 0u64;
    while text.len() < len {
        text.extend_from_slice(format!("    let value_{line} = compute(&input, {});\n", line % 97).as_bytes());
        line += 1;
    }
    text.truncate(len);
    text
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn header(chunk: &pb::FileChunk) -> Header {
    let codec = Codec::from_wire(chunk.codec).expect("a codec this build knows");
    let raw_len = match (codec, chunk.raw_size) {
        (Codec::None, 0) => chunk.data.len() as u32,
        (_, raw_size) => raw_size,
    };
    Header {
        codec,
        raw_len,
        crc32: chunk.crc32,
    }
}

/// Reads `path` whole, decoding each chunk, and returns its bytes and the
/// codecs its chunks came in.
async fn read(daemon: &Daemon, path: &str, accept: &[Codec]) -> (Vec<u8>, Vec<Codec>) {
    let mut chunks = files(daemon)
        .await
        .read_file(pb::ReadFileRequest {
            location: Some(location(path)),
            accept_codecs: Codec::wire_list(accept),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let mut decoder = Decoder::new();
    let mut data = Vec::new();
    let mut codecs = Vec::new();
    let mut sha = String::new();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.unwrap();
        assert_eq!(chunk.offset, data.len() as u64, "a chunk skipped or repeated bytes");
        let header = header(&chunk);
        codecs.push(header.codec);
        let raw = decoder.decode(header, chunk.data).unwrap();
        assert!(raw.len() <= CHUNK_MAX);
        data.extend_from_slice(&raw);
        if chunk.last {
            sha = chunk.sha256;
            break;
        }
    }
    assert_eq!(sha, sha256(&data));
    (data, codecs)
}

/// The messages that upload `data` to `path`: a header, then chunks each
/// encoded with the next of `codecs`, round and round.
fn upload(path: &str, data: &[u8], codecs: &[Codec], key: Option<&str>) -> Vec<pb::WriteFileMessage> {
    use pb::write_file_message::Message as Part;

    let mut messages = vec![pb::WriteFileMessage {
        message: Some(Part::Header(pb::WriteFileHeader {
            idempotency_key: key.map(|key| pb::IdempotencyKey { value: key.to_owned() }),
            location: Some(location(path)),
            size: data.len() as u64,
            sha256: sha256(data),
            replace: false,
            expected_sha256: None,
        })),
    }];
    let mut encoder = Encoder::new(codecs[0]);
    let pieces: Vec<&[u8]> = data.chunks(CHUNK_MAX).collect();
    for (index, piece) in pieces.iter().enumerate() {
        encoder.set_codec(codecs[index % codecs.len()]);
        let chunk = encoder.encode(piece.to_vec()).unwrap();
        messages.push(pb::WriteFileMessage {
            message: Some(Part::Chunk(pb::FileChunk {
                offset: (index * CHUNK_MAX) as u64,
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

async fn write(daemon: &Daemon, messages: Vec<pb::WriteFileMessage>) -> Result<pb::FileEntry, tonic::Status> {
    files(daemon)
        .await
        .write_file(tokio_stream::iter(messages))
        .await
        .map(tonic::Response::into_inner)
}

/// `messages` as a replace of a file last read with contents `based_on`.
fn replacing(mut messages: Vec<pb::WriteFileMessage>, based_on: &[u8]) -> Vec<pb::WriteFileMessage> {
    if let Some(pb::WriteFileMessage {
        message: Some(pb::write_file_message::Message::Header(header)),
    }) = messages.first_mut()
    {
        header.replace = true;
        header.expected_sha256 = Some(sha256(based_on));
    }
    messages
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replace_made_from_a_stale_read_is_refused_and_writes_nothing() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("note.rs"), b"base\n").unwrap();

    // The first writer read "base" and replaces it.
    write(&daemon, replacing(upload("note.rs", b"first\n", &[Codec::None], None), b"base\n"))
        .await
        .unwrap();
    // The second writer read "base" too: the file has moved on since.
    let refused = write(&daemon, replacing(upload("note.rs", b"second\n", &[Codec::None], None), b"base\n"))
        .await
        .unwrap_err();

    assert_eq!(refused.code(), Code::Aborted);
    assert_eq!(std::fs::read(clone.join("note.rs")).unwrap(), b"first\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replace_with_no_base_named_still_overwrites() {
    let (daemon, clone) = daemon_with_clone().await;
    std::fs::write(clone.join("note.rs"), b"base\n").unwrap();
    let mut messages = upload("note.rs", b"new\n", &[Codec::None], None);
    if let Some(pb::WriteFileMessage {
        message: Some(pb::write_file_message::Message::Header(header)),
    }) = messages.first_mut()
    {
        header.replace = true;
    }

    write(&daemon, messages).await.unwrap();

    assert_eq!(std::fs::read(clone.join("note.rs")).unwrap(), b"new\n");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_read_in_compressed_chunks() {
    let (daemon, clone) = daemon_with_clone().await;
    let data = text(2 * CHUNK_MAX + 12_345);
    std::fs::write(clone.join("big.rs"), &data).unwrap();

    let (read, codecs) = read(&daemon, "big.rs", &Codec::ALL).await;

    assert_eq!(read, data);
    assert_eq!(codecs.len(), 3);
    assert!(codecs.iter().any(|codec| *codec != Codec::None));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reader_that_offers_no_codecs_gets_raw_chunks() {
    let (daemon, clone) = daemon_with_clone().await;
    let data = text(CHUNK_MAX + 1);
    std::fs::write(clone.join("big.rs"), &data).unwrap();

    let (read, codecs) = read(&daemon, "big.rs", &[]).await;

    assert_eq!(read, data);
    assert!(codecs.iter().all(|codec| *codec == Codec::None));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_written_in_compressed_chunks_lands_whole() {
    let (daemon, clone) = daemon_with_clone().await;
    let data = text(3 * CHUNK_MAX + 7);

    let entry = write(&daemon, upload("new.rs", &data, &[Codec::Zstd, Codec::Lz4, Codec::None], None))
        .await
        .unwrap();

    assert_eq!(entry.size, data.len() as u64);
    assert_eq!(std::fs::read(clone.join("new.rs")).unwrap(), data);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_chunk_is_refused_and_nothing_is_written() {
    let (daemon, clone) = daemon_with_clone().await;
    let data = text(CHUNK_MAX + 100);
    let mut messages = upload("damaged.rs", &data, &[Codec::Lz4], None);
    let Some(pb::write_file_message::Message::Chunk(chunk)) = &mut messages[1].message else {
        panic!("the second message is a chunk");
    };
    assert_ne!(chunk.codec, Codec::None.wire());
    let middle = chunk.data.len() / 2;
    chunk.data[middle] ^= 0xff;

    let refused = write(&daemon, messages).await.unwrap_err();

    assert_eq!(refused.code(), Code::DataLoss);
    assert!(!clone.join("damaged.rs").exists());
    let leftovers: Vec<_> = std::fs::read_dir(&clone).unwrap().collect();
    assert!(leftovers.is_empty(), "the refused upload left {leftovers:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_retried_with_the_same_key_writes_once() {
    let (daemon, clone) = daemon_with_clone().await;
    let first = text(CHUNK_MAX + 10);
    let entry = write(&daemon, upload("keyed.rs", &first, &[Codec::Lz4], Some("k1")))
        .await
        .unwrap();

    // The retry carries other bytes: had it been written, they'd show.
    let other = text(100);
    let again = write(&daemon, upload("keyed.rs", &other, &[Codec::None], Some("k1")))
        .await
        .unwrap();

    assert_eq!(again, entry);
    assert_eq!(std::fs::read(clone.join("keyed.rs")).unwrap(), first);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_key_for_a_file_that_exists_is_refused_without_replace() {
    let (daemon, clone) = daemon_with_clone().await;
    let first = text(100);
    write(&daemon, upload("keyed.rs", &first, &[Codec::None], Some("k1")))
        .await
        .unwrap();

    let refused = write(&daemon, upload("keyed.rs", &text(200), &[Codec::None], Some("k2")))
        .await
        .unwrap_err();

    assert_eq!(refused.code(), Code::AlreadyExists);
    assert_eq!(std::fs::read(clone.join("keyed.rs")).unwrap(), first);
}
