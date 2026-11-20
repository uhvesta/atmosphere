//! Transfers to, from and between other machines, driven by the host over
//! the machines' links (FM-5, FM-6, FM-8, FM-9, FM-13, FM-14; D142, D145,
//! D156; flows 26.7 and 26.15), through a real host daemon and machines
//! behind a fake `ssh` whose `humatad2` is the real binary.
//!
//! Enabling is a move to `tests/remote_transfers.rs`.
//!
//! Nothing here depends on speed. A link is lost "mid-way" by the link's
//! own transport: the gate lets a known number of bytes through and holds
//! the rest, the test waits on `WatchTransfer` until the transfer shows it
//! is past the point it needs, and only then ends the link. Every wait is
//! on a stream the host pushes (the transfer's watch, the machine events)
//! with [`LONG`] as a safety net; a link is ended through the process
//! group the rig recorded for it, and kept away with the fake machine's own
//! `hang` flag (see `mod rig` at the end of the file).

use humatad_test_common as common;

use std::collections::BTreeMap;
use std::path::Path;

use humata2_chunk::{CHUNK_MAX, Codec, Encoder};
use humata2_grpc::humata::v1 as pb;
use rig::{LONG, Rig, Way, pastes, shared};
use sha2::{Digest, Sha256};
use tonic::transport::Channel;
use tonic::{Status, Streaming};

const MIB: u64 = 1 << 20;

/// The engine has the far end save a file and confirm how much of it is on
/// disk after every 8 MiB (`SYNC_EVERY_BYTES` in `transfer_service.rs`). A
/// progress of this much or more is only ever shown after such a
/// confirmation.
const WINDOW: u64 = 8 * MIB;

/// The file of a copy that loses its link: one confirmed window and half of
/// the next.
const BIG: u64 = WINDOW + 4 * MIB;

/// Where the gate holds such a copy: all of the first window, with room for
/// what else the link carries (the calls before the file and the framing
/// of 8 MiB of chunks are well under 64 KiB, and the gate is exact to
/// 64 KiB), and 2 MiB short of the whole file, which is noise and doesn't
/// compress. So the first window is always confirmed, and the copy can
/// never finish, before the link is ended.
const HELD_AT: u64 = WINDOW + 2 * MIB;

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|byte| format!("{byte:02x}")).collect()
}

/// `len` bytes that don't compress, the same for the same seed.
fn noise(seed: u64, len: u64) -> Vec<u8> {
    let len = usize::try_from(len).unwrap();
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

/// A folder `payload` in `root`: a few small files, an empty one, a name
/// that isn't ASCII, and one file of a chunk and a bit.
fn make_payload(root: &Path) {
    let payload = root.join("payload");
    std::fs::create_dir_all(payload.join("docs/deeper")).unwrap();
    std::fs::write(payload.join("small.txt"), b"hello across machines\n").unwrap();
    std::fs::write(payload.join("empty"), b"").unwrap();
    std::fs::write(payload.join("docs/naïve ファイル.txt"), noise(2, 70_000)).unwrap();
    std::fs::write(payload.join("docs/deeper/mid.bin"), noise(3, MIB + 12_345)).unwrap();
}

/// A folder `payload` in `root` with one file, `big.bin`, of [`BIG`] bytes.
fn make_big_payload(root: &Path) {
    std::fs::create_dir_all(root.join("payload")).unwrap();
    std::fs::write(root.join("payload/big.bin"), noise(1, BIG)).unwrap();
}

/// Every file under `root` by relative path with its SHA-256, and every
/// folder by path with none. Nothing for a folder that isn't there.
fn tree(root: &Path) -> BTreeMap<String, Option<String>> {
    fn walk(root: &Path, at: &Path, into: &mut BTreeMap<String, Option<String>>) {
        for entry in std::fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                into.insert(rel, None);
                walk(root, &path, into);
            } else {
                into.insert(rel, Some(sha256(&std::fs::read(&path).unwrap())));
            }
        }
    }
    let mut into = BTreeMap::new();
    if root.is_dir() {
        walk(root, root, &mut into);
    }
    into
}

/// No hidden staging folder or partial file anywhere under `root`.
fn no_staging(root: &Path) {
    for rel in tree(root).keys() {
        assert!(
            !rel.contains(".humata-transfer-") && !rel.contains(".humata-partial-"),
            "{rel} is left over in {}",
            root.display()
        );
    }
}

/// Whether every file of `source` is somewhere under the trash folder
/// `trash` with the same bytes, at the same path inside one item.
fn in_trash(trash: &Path, source: &BTreeMap<String, Option<String>>) -> bool {
    let trashed = tree(trash);
    source
        .iter()
        .filter(|(_, digest)| digest.is_some())
        .all(|(path, digest)| {
            let tail = format!("/{path}");
            trashed.iter().any(|(found, bytes)| bytes == digest && found.ends_with(&tail))
        })
}

async fn transfers(rig: &Rig) -> pb::transfer_service_client::TransferServiceClient<Channel> {
    pb::transfer_service_client::TransferServiceClient::new(rig.channel().await)
}

async fn start(
    rig: &Rig,
    sources: &[pb::FileLocation],
    destination: pb::FileLocation,
    mode: pb::TransferMode,
) -> Result<pb::Transfer, Status> {
    transfers(rig)
        .await
        .start_transfer(pb::StartTransferRequest {
            idempotency_key: None,
            sources: sources.to_vec(),
            destination: Some(destination),
            mode: mode as i32,
            on_existing: pb::OnExisting::Fail as i32,
            reason: String::new(),
            name: String::new(),
        })
        .await
        .map(tonic::Response::into_inner)
}

/// A transfer in a few words, for a failed wait's message.
fn told(transfer: &pb::Transfer) -> String {
    format!(
        "{:?} at {} of {} bytes ({})",
        transfer.state(),
        transfer.done_bytes,
        transfer.total_bytes,
        transfer.problem.as_ref().map_or("no problem", |problem| problem.message.as_str())
    )
}

/// The machine a transfer says it waits for.
fn waits_for(transfer: &pb::Transfer) -> Option<&str> {
    match transfer.problem.as_ref()?.kind.as_ref()? {
        pb::error::Kind::MachineOffline(offline) => Some(offline.machine_name.as_str()),
        _ => None,
    }
}

/// One transfer as `WatchTransfer` tells it. While fewer changes wait than
/// the watch buffers (hundreds; a transfer here makes a few dozen), every
/// change is sent, so `seen` misses none.
struct Watch {
    changes: Streaming<pb::Transfer>,
    /// The start's answer, then every state the watch sent, oldest first.
    seen: Vec<pb::Transfer>,
}

impl Watch {
    async fn of(rig: &Rig, started: &pb::Transfer) -> Self {
        let changes = transfers(rig)
            .await
            .watch_transfer(pb::WatchTransferRequest {
                id: started.id.clone(),
                from_version: 0,
            })
            .await
            .expect("the transfer can be watched")
            .into_inner();
        Self {
            changes,
            seen: vec![started.clone()],
        }
    }

    /// The first state `reached` holds for, starting with the last one
    /// seen. Fails, saying `what` it waited for and what the transfer last
    /// was, if the watch ends first or nothing comes within [`LONG`].
    async fn until(
        &mut self,
        what: &str,
        reached: impl Fn(&pb::Transfer) -> bool,
    ) -> pb::Transfer {
        let deadline = tokio::time::Instant::now() + LONG;
        loop {
            let last = self.seen.last().expect("the start's answer is always there");
            if reached(last) {
                return last.clone();
            }
            let next = match tokio::time::timeout_at(deadline, self.changes.message()).await {
                Ok(Ok(Some(next))) => next,
                Ok(Ok(None)) => panic!("the transfer ended before it was {what}: {}", told(last)),
                Ok(Err(status)) => panic!(
                    "the transfer's watch failed ({status:?}) before it was {what}: {}",
                    told(last)
                ),
                Err(_) => panic!("the transfer wasn't {what} within {LONG:?}: {}", told(last)),
            };
            self.seen.push(next);
        }
    }

    async fn done(&mut self) -> pb::Transfer {
        self.until("done", |transfer| transfer.state() == pb::TransferState::Done).await
    }
}

/// Copies or moves and waits for it to be done.
async fn finish(
    rig: &Rig,
    sources: &[pb::FileLocation],
    destination: pb::FileLocation,
    mode: pb::TransferMode,
) -> pb::Transfer {
    let started = start(rig, sources, destination, mode).await.expect("the transfer starts");
    Watch::of(rig, &started).await.done().await
}

/// What a copy that lost its link mid-way showed.
struct Lost {
    /// The transfer once it waited for the machine.
    waiting: pb::Transfer,
    /// Every state from that one to the last, oldest first.
    after: Vec<pb::Transfer>,
    done: pb::Transfer,
}

/// Copies `source`, a folder holding one file of [`BIG`] bytes, into the
/// folder `in` on the gated machine `to`, whose link is lost once the first
/// window is confirmed there and before the file can be whole, then comes
/// back.
async fn copy_losing_the_link(rig: &Rig, source: pb::FileLocation, to: &str) -> Lost {
    let there = rig.the(to);
    there.hold_after(Way::ToMachine, HELD_AT);
    let started = start(rig, &[source], there.at("in"), pb::TransferMode::Copy)
        .await
        .expect("the copy starts");
    let mut watch = Watch::of(rig, &started).await;
    // Held by the gate, not by luck: the copy gets this far and no further.
    let held = watch
        .until("past its first confirmed window", |transfer| transfer.done_bytes >= WINDOW)
        .await;
    assert_ne!(
        held.state(),
        pb::TransferState::Done,
        "the copy finished through a link the gate holds at {HELD_AT} bytes: the gate let it by"
    );

    rig.link_lost(to).await;
    let waiting = watch
        .until("waiting for its machine", |transfer| {
            transfer.state() == pb::TransferState::WaitingForMachine
        })
        .await;
    let from = watch.seen.len() - 1;

    rig.link_back(to).await;
    let done = watch.done().await;
    Lost {
        waiting,
        after: watch.seen[from..].to_vec(),
        done,
    }
}

// Flow 26.7, steps 1 to 3 and 6. Fails if a folder copied from the host to a
// machine doesn't arrive there byte for byte (a file of more than one
// confirmed window included), or leaves its staging folder behind.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_copied_to_a_machine_arrives_with_identical_bytes_and_no_staging_folder() {
    let rig = Rig::start(&["box-1"], &[]).await;
    make_payload(&rig.host_shared());
    std::fs::write(rig.host_shared().join("payload/big.bin"), noise(1, WINDOW + MIB + 17)).unwrap();
    let there = rig.the("box-1");
    std::fs::create_dir_all(there.shared().join("in")).unwrap();

    let source = [shared(None, "payload")];
    let done = finish(&rig, &source, there.at("in"), pb::TransferMode::Copy).await;

    assert_eq!(done.done_bytes, done.total_bytes, "{}", told(&done));
    assert_eq!(
        tree(&there.shared().join("in/payload")),
        tree(&rig.host_shared().join("payload")),
        "what is on the machine's disk isn't what was copied"
    );
    no_staging(&there.shared());
    assert!(rig.host_shared().join("payload").is_dir(), "a copy leaves its source");
}

// Flow 26.7, steps 4 to 6 (FM-8, FM-13). Fails if a copy whose machine's
// link is lost part way fails for good or names no machine or the wrong one
// while it waits, doesn't go on by itself when the link is back, or ends
// with other bytes on the machine than were copied.
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_whose_link_is_lost_part_way_waits_for_the_machine_and_finishes_when_it_is_back() {
    let rig = Rig::start(&[], &["box-1"]).await;
    make_big_payload(&rig.host_shared());
    let there = rig.the("box-1");
    std::fs::create_dir_all(there.shared().join("in")).unwrap();

    let lost = copy_losing_the_link(&rig, shared(None, "payload"), "box-1").await;

    assert_eq!(waits_for(&lost.waiting), Some("box-1"), "{}", told(&lost.waiting));
    assert_eq!(lost.done.done_bytes, lost.done.total_bytes, "{}", told(&lost.done));
    assert_eq!(
        tree(&there.shared().join("in/payload")),
        tree(&rig.host_shared().join("payload")),
        "what is on the machine's disk isn't what was copied"
    );
    no_staging(&there.shared());
}

// FM-8, flow 26.7 step 5: "resumes from the last confirmed offset". Fails if
// a copy that had a window of its file confirmed on the machine before the
// link was lost starts that file over when the link is back, instead of
// going on from what was confirmed.
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_goes_on_from_what_the_machine_confirmed_when_its_link_is_back() {
    let rig = Rig::start(&[], &["box-1"]).await;
    make_big_payload(&rig.host_shared());
    std::fs::create_dir_all(rig.the("box-1").shared().join("in")).unwrap();

    let lost = copy_losing_the_link(&rig, shared(None, "payload"), "box-1").await;

    // The first window was confirmed before the link went, so from then on
    // the transfer never has less than that.
    let shown: Vec<u64> = lost.after.iter().map(|transfer| transfer.done_bytes).collect();
    assert!(
        shown.iter().all(|done| *done >= WINDOW),
        "the copy went back below the {WINDOW} bytes the machine had confirmed; after the \
         link was lost it showed {shown:?}"
    );
}

// Flow 26.15, steps 1 to 3 (FM-9). Fails if a folder copied from one machine
// to another through the host doesn't arrive byte for byte, leaves a staging
// folder on the destination, or takes the source away.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_copied_from_one_machine_to_another_arrives_whole() {
    let rig = Rig::start(&["box-a", "box-b"], &[]).await;
    let (from, to) = (rig.the("box-a"), rig.the("box-b"));
    make_payload(&from.shared());
    let source = tree(&from.shared().join("payload"));
    std::fs::create_dir_all(to.shared().join("in")).unwrap();

    let done = finish(&rig, &[from.at("payload")], to.at("in"), pb::TransferMode::Copy).await;

    assert_eq!(done.done_bytes, done.total_bytes, "{}", told(&done));
    assert_eq!(tree(&to.shared().join("in/payload")), source);
    no_staging(&to.shared());
    assert_eq!(tree(&from.shared().join("payload")), source, "a copy leaves its source as it was");
    assert!(
        !rig.host_shared().join("payload").exists() && !rig.host_shared().join("in").exists(),
        "nothing of it stays on the host it went through"
    );
}

/// The mode of each entry of a tree of folders at 0750, 0555 (one inside
/// another) and 0700, and a 0555 top, with a file in each read-only one.
const MODED: [(&str, u32); 8] = [
    ("tree", 0o750),
    ("tree/ro", 0o555),
    ("tree/ro/sub", 0o555),
    ("tree/ro/inside.txt", 0o644),
    ("tree/ro/sub/deep.txt", 0o600),
    ("tree/private", 0o700),
    ("locked", 0o555),
    ("locked/note.txt", 0o640),
];

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

/// Makes read-only folders under `root` writable again, so the rig's
/// temporary folder can go.
fn unlock(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(meta) = std::fs::symlink_metadata(root) else { return };
    if meta.is_dir() {
        let _ = std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700));
        for entry in std::fs::read_dir(root).unwrap().flatten() {
            unlock(&entry.path());
        }
    }
}

// VF-59 (FM-6). Fails if a folder copied to a machine arrives with another
// mode than its source's: a read-only one made writable to fill it and
// left so, at the top or deeper, or one inside it set before what it holds.
#[tokio::test(flavor = "multi_thread")]
async fn folders_copied_to_a_machine_keep_their_exact_modes() {
    use std::os::unix::fs::PermissionsExt;
    let rig = Rig::start(&["box-1"], &[]).await;
    let src = rig.host_shared();
    for (rel, _) in MODED {
        if rel.ends_with(".txt") {
            std::fs::write(src.join(rel), noise(7, 64)).unwrap();
        } else {
            std::fs::create_dir_all(src.join(rel)).unwrap();
        }
    }
    for (rel, mode) in MODED.iter().rev() {
        std::fs::set_permissions(src.join(rel), std::fs::Permissions::from_mode(*mode)).unwrap();
    }
    let there = rig.the("box-1");
    std::fs::create_dir_all(there.shared().join("in")).unwrap();

    let sources = [shared(None, "tree"), shared(None, "locked")];
    let done = finish(&rig, &sources, there.at("in"), pb::TransferMode::Copy).await;

    let arrived: Vec<(&str, u32)> =
        MODED.iter().map(|(rel, _)| (*rel, mode_of(&there.shared().join("in").join(rel)))).collect();
    no_staging(&there.shared());
    unlock(&there.shared());
    unlock(&src);
    assert_eq!(done.done_bytes, done.total_bytes, "{}", told(&done));
    assert_eq!(arrived, MODED.to_vec());
}

// Flow 26.15, steps 4 and 5 (FM-13). Fails if a machine to machine copy
// whose destination's link is lost part way doesn't wait naming the
// destination (and not the source, whose link stayed up), doesn't go on by
// itself when that link is back, or ends with other bytes than the source's.
#[tokio::test(flavor = "multi_thread")]
async fn a_machine_to_machine_copy_waits_for_the_destination_and_completes_when_it_is_back() {
    let rig = Rig::start(&["box-a"], &["box-b"]).await;
    let (from, to) = (rig.the("box-a"), rig.the("box-b"));
    make_big_payload(&from.shared());
    std::fs::create_dir_all(to.shared().join("in")).unwrap();

    let lost = copy_losing_the_link(&rig, from.at("payload"), "box-b").await;

    assert_eq!(waits_for(&lost.waiting), Some("box-b"), "{}", told(&lost.waiting));
    assert_eq!(lost.done.done_bytes, lost.done.total_bytes, "{}", told(&lost.done));
    assert_eq!(
        tree(&to.shared().join("in/payload")),
        tree(&from.shared().join("payload")),
        "what is on the destination's disk isn't what the source has"
    );
    no_staging(&to.shared());
}

// FM-14. Fails if a move from a machine to the host takes the source away,
// or puts anything in that machine's trash, while part of it has still to
// arrive; or if, once done, the source isn't in that machine's trash (it is
// still in place, was deleted outright, or went to the host's trash).
#[tokio::test(flavor = "multi_thread")]
async fn a_move_from_a_machine_puts_the_source_in_that_machines_trash_only_once_it_is_done() {
    let rig = Rig::start(&[], &["box-1"]).await;
    let there = rig.the("box-1");
    make_payload(&there.shared());
    std::fs::write(there.shared().join("payload/more.bin"), noise(4, 4 * MIB)).unwrap();
    let source = tree(&there.shared().join("payload"));
    let (its_trash, host_trash) = (tree(&there.trash()), tree(&rig.host_trash()));
    std::fs::create_dir_all(rig.host_shared().join("in")).unwrap();

    // The machine's link carries 2 MiB toward the host and holds the rest:
    // room for a whole chunk, and less than half of the 4 MiB file.
    there.hold_after(Way::ToHost, 2 * MIB);
    let started = start(&rig, &[there.at("payload")], shared(None, "in"), pb::TransferMode::Move)
        .await
        .expect("the move starts");
    let mut watch = Watch::of(&rig, &started).await;
    let part_way = watch.until("part way", |transfer| transfer.done_bytes > 0).await;
    assert_ne!(
        part_way.state(),
        pb::TransferState::Done,
        "the move finished through a link the gate holds at 2 MiB: the gate let it by"
    );
    // Some of it is across and the rest can't come yet.
    assert_eq!(
        tree(&there.shared().join("payload")),
        source,
        "the source changed before all of it had arrived"
    );
    assert_eq!(
        tree(&there.trash()),
        its_trash,
        "something went to the machine's trash before all of it had arrived"
    );

    there.release(Way::ToHost);
    let done = watch.done().await;

    assert_eq!(done.done_bytes, done.total_bytes, "{}", told(&done));
    assert_eq!(tree(&rig.host_shared().join("in/payload")), source);
    assert!(!there.shared().join("payload").exists(), "the source is still in its folder");
    assert!(in_trash(&there.trash(), &source), "the source isn't in the machine's trash");
    assert_eq!(tree(&rig.host_trash()), host_trash, "the host's trash changed");
    no_staging(&rig.host_shared());
}

// D156, FM-13. Fails if a transfer started toward a machine whose link is
// down is refused, or fails, instead of being accepted and waiting with the
// machine named; or if it doesn't run by itself once the link is back.
#[tokio::test(flavor = "multi_thread")]
async fn a_transfer_started_while_the_machine_is_offline_is_accepted_and_waits_for_it() {
    let rig = Rig::start(&["box-1"], &[]).await;
    make_payload(&rig.host_shared());
    let there = rig.the("box-1");
    std::fs::create_dir_all(there.shared().join("in")).unwrap();
    rig.link_lost("box-1").await;

    let started = start(&rig, &[shared(None, "payload")], there.at("in"), pb::TransferMode::Copy)
        .await
        .expect("a transfer to a machine that is offline is accepted");
    let mut watch = Watch::of(&rig, &started).await;
    let waiting = watch
        .until("waiting for its machine", |transfer| {
            transfer.state() == pb::TransferState::WaitingForMachine
        })
        .await;

    assert_eq!(waits_for(&waiting), Some("box-1"), "{}", told(&waiting));
    assert_eq!(tree(&there.shared().join("in")), BTreeMap::new(), "something reached the machine");

    rig.link_back("box-1").await;
    let done = watch.done().await;

    assert_eq!(done.done_bytes, done.total_bytes, "{}", told(&done));
    assert_eq!(
        tree(&there.shared().join("in/payload")),
        tree(&rig.host_shared().join("payload"))
    );
    no_staging(&there.shared());
}

// FM-4 with FM-9. Fails if a file the app streams up into a folder on a
// machine doesn't end as a done transfer with the file's bytes on that
// machine's disk, or leaves a partial file or staging folder there.
#[tokio::test(flavor = "multi_thread")]
async fn an_upload_to_a_machine_arrives_whole() {
    use pb::upload_message::Message as Part;

    let rig = Rig::start(&["box-1"], &[]).await;
    let there = rig.the("box-1");
    std::fs::create_dir_all(there.shared().join("in")).unwrap();
    let data = noise(5, 2 * MIB + 12_345);
    let mut messages = vec![pb::UploadMessage {
        message: Some(Part::Header(pb::UploadHeader {
            idempotency_key: None,
            destination: Some(there.at("in")),
            name: "up.bin".to_owned(),
            size: data.len() as u64,
            sha256: sha256(&data),
            on_existing: pb::OnExisting::Fail as i32,
            resume: None,
        })),
    }];
    let mut encoder = Encoder::new(Codec::None);
    let pieces: Vec<&[u8]> = data.chunks(CHUNK_MAX).collect();
    assert!(pieces.len() > 2, "the upload is several chunks");
    for (index, piece) in pieces.iter().enumerate() {
        let chunk = encoder.encode(piece.to_vec()).unwrap();
        messages.push(pb::UploadMessage {
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

    let done = transfers(&rig)
        .await
        .upload(tokio_stream::iter(messages))
        .await
        .expect("the upload is taken")
        .into_inner();

    assert_eq!(done.state(), pb::TransferState::Done, "{}", told(&done));
    assert_eq!(done.done_bytes, data.len() as u64);
    assert!(
        std::fs::read(there.shared().join("in/up.bin")).unwrap() == data,
        "the file on the machine's disk isn't what was uploaded"
    );
    no_staging(&there.shared());
    assert!(!rig.host_shared().join("in").exists(), "nothing of it is on the host");
}

/// D335: a file uploaded into the pastes of a session on another machine
/// lands on that machine, and the upload says where on its disk: what the
/// app types into the session's terminal there.
#[tokio::test(flavor = "multi_thread")]
async fn an_upload_into_a_remote_sessions_pastes_says_its_path_there() {
    use pb::upload_message::Message as Part;

    let rig = Rig::start(&["box-1"], &[]).await;
    pb::file_service_client::FileServiceClient::new(rig.channel().await)
        .create_folder(pb::CreateFolderRequest {
            idempotency_key: None,
            location: Some(pastes(Some("box-1"), "p1")),
        })
        .await
        .expect("the paste's folder is made on the machine");
    let data = noise(7, 12_288);
    let chunk = Encoder::new(Codec::None).encode(data.clone()).unwrap();
    let messages = vec![
        pb::UploadMessage {
            message: Some(Part::Header(pb::UploadHeader {
                idempotency_key: None,
                destination: Some(pastes(Some("box-1"), "p1")),
                name: "notes.txt".to_owned(),
                size: data.len() as u64,
                sha256: sha256(&data),
                on_existing: pb::OnExisting::Fail as i32,
                resume: None,
            })),
        },
        pb::UploadMessage {
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

    let done = transfers(&rig)
        .await
        .upload(tokio_stream::iter(messages))
        .await
        .expect("the upload is taken")
        .into_inner();

    assert_eq!(done.state(), pb::TransferState::Done, "{}", told(&done));
    assert!(!done.disk_path.is_empty(), "{}", told(&done));
    assert!(
        std::fs::read(&done.disk_path).unwrap() == data,
        "the file at the path told isn't what was uploaded"
    );
    assert!(
        !Path::new(&done.disk_path).starts_with(rig.host.dir.path()),
        "the path is the machine's, not the host's"
    );
}

/// The host daemon, the machines behind its `ssh`, and the test's hold on
/// their links. `remote_transfers.rs` and `remote_files.rs` carry the same
/// text, because each test file is a crate of its own; harness-needs.md
/// has the move to one file under `tests/common/`.
///
/// A machine here is three things the test owns: a fake machine (a home
/// folder and a fake `ssh` that runs commands in it), the machine's own
/// `humatad2` in the machine role (the release binary, started by the test,
/// since a link only relays to the machine's daemon and serves no files
/// itself), and a control folder for its link. The host is the admin
/// daemon; the only `ssh` on its PATH is this rig's, which hands each call
/// to the right machine.
mod rig {
    #![allow(dead_code)]

    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use humata2_fake_machine::FakeMachine;
    use humata2_grpc::humata::v1 as pb;
    use humata2_store::{ChannelRecord, SessionRecord, Store};
    use rustix::process::{Pid, Signal, kill_process_group};
    use sha2::{Digest, Sha256};
    use tokio::sync::Mutex;
    use tonic::Streaming;
    use tonic::transport::Channel;

    use crate::common::{Build, Daemon, binary, folder, state_folder};

    /// The one channel, known to the host and to every machine.
    pub const CHANNEL: &str = "ch_remote";
    /// The one session, in the channel on `box-1`: the host and the machine
    /// both know it.
    pub const SESSION: &str = "se_remote_paste";
    const CHANNEL_FOLDER: &str = "remote-channel";

    /// The most a test waits for something the host does on its own. It is a
    /// safety net that fails the test saying what it waited for, never a
    /// pace: every wait ends on a pushed change. The slowest thing waited
    /// for is a link coming back: the host dials again 5 s after a link
    /// ends, then 10, 20 and at most 30 s after each dial that fails
    /// (`peer_client.rs`), and a link's start is a daemon's start (15 s at
    /// most, `common::WAIT`). This is more than twice the two together.
    pub const LONG: Duration = Duration::from_secs(120);

    const FILES: [&str; 5] = ["humatad2", "humata2_holder", "humata", "stack", "humata2_fusefs"];

    /// The gate is a perl script until the harness has a helper of its own
    /// (harness-needs.md).
    const PERL: &str = "/usr/bin/perl";

    /// `humatad2` as a machine gets it from the tools bundle.
    const WRAPPER: &str = r##"#!/bin/sh
# humatad2 on one of this test's machines: the real one. A link is told the
# state folder and socket of the machine's own daemon, which the test
# started in the folder `~/.humata2` points at. The folder is named by its
# short, real path, because a socket's path must be short.
case " $* " in *" --stdio "*)
  at=$(cd -P "$HOME/.humata2" && pwd -P) || exit 70
  exec '__REAL__' "$@" --state "$at/state" --socket "$at/d.sock" ;;
esac
exec '__REAL__' "$@"
"##;

    /// The host's `ssh`.
    const SSH: &str = r##"#!/bin/sh
# The host's `ssh` in this test: each call goes to the fake machine whose
# host name it names. A link command (`serve --stdio`) runs in a process
# group of its own and leaves the group's id in the machine's control
# folder, so the test can end that one link and nothing else; a gated
# machine's link runs through the gate.
name=
for arg in "$@"; do
  case "$arg" in
__MACHINES__  esac
done
if [ -z "$name" ]; then
  echo "ssh: none of this test's machines is named in: $*" >&2
  exit 255
fi
control='__CONTROL__'/$name
for last in "$@"; do :; done
case "$last" in
  *'serve --stdio'*)
    echo $$ > "$control/link.new" && mv "$control/link.new" "$control/link"
    if [ -e "$control/gated" ]; then
      exec '__SETSID__' /usr/bin/perl '__GATE__' "$control" "$ssh" "$@"
    fi
    exec '__SETSID__' "$ssh" "$@" ;;
esac
exec "$ssh" "$@"
"##;

    /// The gate a gated machine's link runs through.
    const GATE: &str = r##"# The transport of a gated machine's link. It runs the real link command
# and carries its bytes, one process for each way so neither waits on the
# other. While `limit.in` (host to machine) or `limit.out` (machine to
# host) is in the control folder, that way lets through as many more bytes
# as the file says, counted from when the file appeared, then holds the
# rest until the file is removed. The count is exact to one read (64 KiB):
# a read already waiting when the file appears isn't counted.
use strict;
use warnings;
use IPC::Open2;

my ($ctl, @command) = @ARGV;
$SIG{PIPE} = 'IGNORE';

sub limit_of {
    open(my $file, '<', $_[0]) or return undef;
    my $line = <$file>;
    close($file);
    return defined $line && $line =~ /^(\d+)/ ? $1 : undef;
}

# Copies `$src` to `$dst` until `$src` ends, `$dst` is gone, or, while
# held, `$alive` says to stop.
sub pump {
    my ($src, $dst, $limit_file, $alive) = @_;
    binmode($src);
    binmode($dst);
    my $passed = 0;
    while (1) {
        my $limit = limit_of($limit_file);
        $passed = 0 unless defined $limit;
        my $room = defined $limit ? $limit - $passed : 65536;
        if ($room <= 0) {
            # Held. Looked at again shortly: removing the limit releases it.
            return unless $alive->();
            select(undef, undef, undef, 0.02);
            next;
        }
        $room = 65536 if $room > 65536;
        my $got = sysread($src, my $bytes, $room);
        last unless $got;
        my $sent = 0;
        while ($sent < $got) {
            my $wrote = syswrite($dst, $bytes, $got - $sent, $sent);
            return unless defined $wrote;
            $sent += $wrote;
        }
        $passed += $got if defined $limit;
    }
    close($dst);
}

my $link = open2(my $from, my $to, @command);
my $parent = $$;
my $inward = fork();
die "the link gate couldn't fork: $!" unless defined $inward;
if ($inward == 0) {
    close($from);
    # Not closed: a closed descriptor would be handed to the next file opened.
    open(STDOUT, '>', '/dev/null');
    pump(\*STDIN, $to, "$ctl/limit.in", sub { getppid() == $parent });
    exit(0);
}
close($to);
open(STDIN, '<', '/dev/null');
pump($from, \*STDOUT, "$ctl/limit.out", sub { 1 });
# The link command's output ended: the link is over.
kill('KILL', $inward);
waitpid($link, 0);
exit($? >> 8);
"##;

    fn executable(path: &Path, text: &str) {
        std::fs::write(path, text).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A tools bundle for linux-x86_64, the fake machine's platform, whose
    /// humatad2 is [`WRAPPER`] around the real binary.
    fn tools(dir: &Path) -> PathBuf {
        let root = dir.join("tools");
        std::fs::create_dir_all(root.join("linux-x86_64")).unwrap();
        let mut entries = serde_json::Map::new();
        for file in FILES {
            let body = if file == "humatad2" {
                let real = binary("HUMATAD2_RELEASE").display().to_string();
                WRAPPER.replace("__REAL__", &real)
            } else {
                format!("#!/bin/sh\n# {file}\nexit 0\n")
            };
            std::fs::write(root.join("linux-x86_64").join(file), &body).unwrap();
            let sha256: String = Sha256::digest(body.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            entries.insert(
                file.to_owned(),
                serde_json::json!({ "path": format!("linux-x86_64/{file}"), "sha256": sha256 }),
            );
        }
        let manifest = serde_json::json!({ "platforms": { "linux-x86_64": entries } });
        std::fs::write(root.join("manifest.json"), manifest.to_string()).unwrap();
        root
    }

    /// Writes the channel and the session into the store under `state`,
    /// before the daemon that owns the store starts. The host judges a
    /// location by its own record of the channel and the machine resolves
    /// it by its own, so both get the same one. The session is on
    /// `session_machine` as that store knows it: `box-1` on the host, the
    /// machine itself (empty) on a machine.
    fn seed_channel(state: &Path, session_machine: &str) {
        let store = Store::open(state.join("store")).unwrap();
        store
            .put_channel(&ChannelRecord {
                id: CHANNEL.to_owned(),
                name: "remote files".to_owned(),
                folder: CHANNEL_FOLDER.to_owned(),
                branch_prefix: None,
                default_agent: "zsh".to_owned(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 0,
            })
            .unwrap();
        store
            .put_session(&SessionRecord {
                id: SESSION.to_owned(),
                channel: CHANNEL.to_owned(),
                machine: session_machine.to_owned(),
                title: "remote pastes".to_owned(),
                agent: "zsh".to_owned(),
                folder: state.to_owned(),
                created_unix_ms: 0,
            })
            .unwrap();
    }

    /// `path` in the session's pastes on `machine` (none: the host).
    pub fn pastes(machine: Option<&str>, path: &str) -> pb::FileLocation {
        pb::FileLocation {
            machine: machine.map(|name| pb::MachineId {
                value: name.to_owned(),
            }),
            root: Some(pb::file_location::Root::SessionPastes(pb::SessionPastes {
                session: Some(pb::SessionId {
                    value: SESSION.to_owned(),
                }),
            })),
            path: path.to_owned(),
        }
    }

    /// `path` in the channel's shared folder on `machine` (none: the host).
    pub fn shared(machine: Option<&str>, path: &str) -> pb::FileLocation {
        pb::FileLocation {
            machine: machine.map(|name| pb::MachineId {
                value: name.to_owned(),
            }),
            root: Some(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                channel: Some(pb::ChannelId {
                    value: CHANNEL.to_owned(),
                }),
            })),
            path: path.to_owned(),
        }
    }

    /// A direction of a link's bytes.
    #[derive(Clone, Copy, Debug)]
    pub enum Way {
        /// From the host to the machine: what a copy to the machine sends.
        ToMachine,
        /// From the machine to the host: what a copy from the machine sends.
        ToHost,
    }

    impl Way {
        /// The gate's file for this way, in the machine's control folder.
        fn limit(self) -> &'static str {
            match self {
                Way::ToMachine => "limit.in",
                Way::ToHost => "limit.out",
            }
        }
    }

    /// One machine. Fields drop in order: its daemon ends before its
    /// folders go.
    pub struct Remote {
        pub name: &'static str,
        /// Where this rig's `ssh` and gate look for what to do with this
        /// machine's link, and where they leave the link's process group.
        control: PathBuf,
        /// The machine's own humatad2: what its link relays to.
        daemon: Daemon,
        machine: FakeMachine,
    }

    impl Remote {
        async fn start(name: &'static str, links: &Path, gated: bool) -> Self {
            let machine = FakeMachine::new();
            // The machine's `~/.humata2` is its daemon's folder: the add
            // writes the enrollment key into the state folder the daemon
            // uses, and the link finds the daemon's socket there.
            let dir = folder();
            seed_channel(&state_folder(dir.path()), "");
            std::os::unix::fs::symlink(dir.path(), machine.home().join(".humata2")).unwrap();
            let channels = machine.home().join("Humata");
            std::fs::create_dir_all(channels.join(CHANNEL_FOLDER).join("shared")).unwrap();
            let control = links.join(name);
            std::fs::create_dir_all(&control).unwrap();
            if gated {
                std::fs::write(control.join("gated"), "").unwrap();
            }
            let channels = channels.display().to_string();
            let home = machine.home().display().to_string();
            let daemon = Daemon::start_with_env(
                Build::Release,
                dir,
                &["--role", "machine", "--channels", channels.as_str()],
                &[("HOME", home.as_str())],
            )
            .await;
            Self {
                name,
                control,
                daemon,
                machine,
            }
        }

        /// The channel's shared folder on this machine's disk.
        pub fn shared(&self) -> PathBuf {
            self.machine.home().join("Humata").join(CHANNEL_FOLDER).join("shared")
        }

        /// This machine's Humata trash, on its disk.
        pub fn trash(&self) -> PathBuf {
            self.machine.home().join("Humata").join(".trash")
        }

        /// `path` in the channel's shared folder on this machine.
        pub fn at(&self, path: &str) -> pb::FileLocation {
            shared(Some(self.name), path)
        }

        /// From now on this machine's link carries `bytes` more that `way`
        /// (to within one 64 KiB read), then holds the rest until
        /// [`Self::release`]. Only a gated machine's link can be held.
        pub fn hold_after(&self, way: Way, bytes: u64) {
            assert!(
                self.control.join("gated").exists(),
                "{}'s link has no gate; start the rig with it among the gated machines",
                self.name
            );
            // Renamed into place, so the gate reads all of it or none.
            let new = self.control.join(format!("{}.new", way.limit()));
            std::fs::write(&new, bytes.to_string()).unwrap();
            std::fs::rename(&new, self.control.join(way.limit())).unwrap();
        }

        /// Lets the bytes held by [`Self::hold_after`] go on.
        pub fn release(&self, way: Way) {
            let limit = self.control.join(way.limit());
            std::fs::remove_file(&limit).unwrap_or_else(|error| {
                panic!("{}'s link wasn't held {way:?} ({}): {error}", self.name, limit.display())
            });
        }

        /// Ends this machine's link at once, as a dropped connection does:
        /// every process of it, and nothing else. The link command runs in
        /// a process group of its own, whose id this rig's `ssh` left in
        /// the control folder when the host dialled.
        fn cut(&self) {
            let file = self.control.join("link");
            let group = std::fs::read_to_string(&file)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
                .and_then(Pid::from_raw)
                .unwrap_or_else(|| {
                    panic!(
                        "{}'s link left no process group in {}; is its link up?",
                        self.name,
                        file.display()
                    )
                });
            kill_process_group(group, Signal::KILL).unwrap_or_else(|error| {
                panic!("couldn't end {}'s link (process group {group:?}): {error}", self.name)
            });
            // The group is gone: the next cut must find the next link's.
            let _ = std::fs::remove_file(&file);
            // A hold was on the link that just ended; the next one runs free.
            for way in [Way::ToMachine, Way::ToHost] {
                let _ = std::fs::remove_file(self.control.join(way.limit()));
            }
        }
    }

    /// The host and its machines, each added and linked. Fields drop in
    /// order: the host first, so its links end before the machines do.
    pub struct Rig {
        /// The admin host daemon.
        pub host: Daemon,
        /// The host's machine events from before any machine was added:
        /// what a wait on a machine's reachability wakes on.
        feed: Mutex<Streaming<pb::Event>>,
        boxes: Vec<Remote>,
    }

    impl Rig {
        /// A host with one machine per name, added and linked. The links of
        /// the `gated` ones run through the gate, so a test can hold them
        /// at a byte count ([`Remote::hold_after`]).
        pub async fn start(plain: &[&'static str], gated: &[&'static str]) -> Self {
            assert!(
                gated.is_empty() || Path::new(PERL).exists(),
                "a gated link needs {PERL}, which isn't on this machine (harness-needs.md has \
                 the helper that replaces it)"
            );
            let dir = folder();
            let links = dir.path().join("links");
            let mut boxes = Vec::new();
            for (names, with_gate) in [(plain, false), (gated, true)] {
                for &name in names {
                    boxes.push(Remote::start(name, &links, with_gate).await);
                }
            }
            let gate = dir.path().join("gate.pl");
            std::fs::write(&gate, GATE).unwrap();
            let machines: String = boxes
                .iter()
                .map(|remote| {
                    format!(
                        "    '{name}.internal') name='{name}'; ssh='{}/ssh' ;;\n",
                        remote.machine.ssh_dir().display(),
                        name = remote.name
                    )
                })
                .collect();
            let bin = dir.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            executable(
                &bin.join("ssh"),
                &SSH.replace("__MACHINES__", &machines)
                    .replace("__CONTROL__", &links.display().to_string())
                    .replace("__SETSID__", &binary("HUMATA2_SETSID").display().to_string())
                    .replace("__GATE__", &gate.display().to_string()),
            );
            seed_channel(&state_folder(dir.path()), "box-1");
            // The host's channels are in `Humata` in its root, the default.
            let shared = dir.path().join("Humata").join(CHANNEL_FOLDER).join("shared");
            std::fs::create_dir_all(shared).unwrap();
            // Nothing of the real home folder: no skills of the person
            // running the tests are read, or sent to a machine.
            let skills = dir.path().join("skills-home");
            std::fs::create_dir_all(&skills).unwrap();
            let extra = [
                "--tools".to_owned(),
                tools(dir.path()).display().to_string(),
                "--ssh-config".to_owned(),
                dir.path().join("ssh_config").display().to_string(),
                "--skills-home".to_owned(),
                skills.display().to_string(),
            ];
            let extra: Vec<&str> = extra.iter().map(String::as_str).collect();
            // The daemon's own PATH, given to it alone: this rig's `ssh`
            // first, and nothing of the test process's environment changes.
            let path = format!("{}:/usr/bin:/bin", bin.display());
            let host =
                Daemon::start_with_env(Build::Admin, dir, &extra, &[("PATH", path.as_str())]).await;
            let feed = pb::event_service_client::EventServiceClient::new(host.channel().await)
                .subscribe(pb::SubscribeRequest {
                    topics: vec![pb::Topic::Machines as i32],
                    ..Default::default()
                })
                .await
                .expect("the host's event feed opens")
                .into_inner();
            let rig = Self {
                host,
                feed: Mutex::new(feed),
                boxes,
            };
            for remote in &rig.boxes {
                rig.add(remote.name).await;
            }
            rig
        }

        pub fn the(&self, name: &str) -> &Remote {
            self.boxes
                .iter()
                .find(|remote| remote.name == name)
                .unwrap_or_else(|| panic!("this rig has no machine {name}"))
        }

        /// The channel's shared folder on the host's disk.
        pub fn host_shared(&self) -> PathBuf {
            self.host.dir.path().join("Humata").join(CHANNEL_FOLDER).join("shared")
        }

        /// The host's own Humata trash, on its disk.
        pub fn host_trash(&self) -> PathBuf {
            self.host.dir.path().join("Humata").join(".trash")
        }

        /// A connection to the host's socket.
        pub async fn channel(&self) -> Channel {
            self.host.channel().await
        }

        async fn machines(&self) -> pb::machine_service_client::MachineServiceClient<Channel> {
            pb::machine_service_client::MachineServiceClient::new(self.channel().await)
        }

        /// Adds `name`, confirming the host key it presents, and waits for
        /// its link.
        async fn add(&self, name: &str) {
            let mut machines = self.machines().await;
            let request = |step: &str, fingerprint: &str| pb::AddSshMachineRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: format!("add-{name}-{step}"),
                }),
                ssh_host: format!("me@{name}.internal"),
                name: name.to_owned(),
                root: String::new(),
                trust_host_key_fingerprint: fingerprint.to_owned(),
            };
            let first = machines
                .add_ssh_machine(request("ask", ""))
                .await
                .expect("the add starts")
                .into_inner();
            let asked = self.ended(&first).await;
            let Some(pb::error::Kind::HostKeyUnknown(key)) =
                asked.error.clone().and_then(|error| error.kind)
            else {
                panic!("the first add of {name} should stop at its host key; it ended {asked:?}");
            };
            let added = machines
                .add_ssh_machine(request("trust", &key.presented_fingerprint))
                .await
                .expect("the confirmed add starts")
                .into_inner();
            let ended = self.ended(&added).await;
            assert_eq!(
                ended.state,
                pb::OperationState::Succeeded as i32,
                "adding {name} failed: {:?}",
                ended.error
            );
            self.linked(name).await;
        }

        /// The operation as its watch last showed it, once the watch ends.
        async fn ended(&self, operation: &pb::Operation) -> pb::Operation {
            let mut watch =
                pb::operation_service_client::OperationServiceClient::new(self.channel().await)
                    .watch_operation(pb::WatchOperationRequest {
                        id: operation.id.clone(),
                        from_version: 0,
                    })
                    .await
                    .expect("the operation can be watched")
                    .into_inner();
            let deadline = tokio::time::Instant::now() + LONG;
            let mut last = operation.clone();
            loop {
                match tokio::time::timeout_at(deadline, watch.message()).await {
                    Ok(Ok(Some(next))) => last = next,
                    Ok(Ok(None)) => return last,
                    Ok(Err(status)) => {
                        panic!("the operation's watch failed ({status:?}); it last was {last:?}")
                    }
                    Err(_) => {
                        panic!("the operation never ended within {LONG:?}; it last was {last:?}")
                    }
                }
            }
        }

        /// The machine `name` once `reached` holds for it. Nothing sleeps:
        /// the machine is read, and read again each time the feed tells of a
        /// machine. The feed was opened before the first machine was added,
        /// so no change falls between a read and the next event.
        async fn machine_until(
            &self,
            name: &str,
            what: &str,
            reached: impl Fn(&pb::Machine) -> bool,
        ) -> pb::Machine {
            let mut feed = self.feed.lock().await;
            let deadline = tokio::time::Instant::now() + LONG;
            loop {
                let now = self
                    .machines()
                    .await
                    .list_machines(pb::ListMachinesRequest::default())
                    .await
                    .expect("the host lists its machines")
                    .into_inner()
                    .machines
                    .into_iter()
                    .find(|machine| machine.name == name);
                if let Some(machine) = now.as_ref().filter(|machine| reached(machine)) {
                    return machine.clone();
                }
                match tokio::time::timeout_at(deadline, feed.message()).await {
                    Ok(Ok(Some(_))) => {}
                    Ok(ended) => panic!(
                        "the host's event feed ended ({ended:?}) while {name} wasn't {what} yet; \
                         the host last showed {now:?}"
                    ),
                    Err(_) => panic!(
                        "{name} was never {what} within {LONG:?}; the host last showed {now:?}"
                    ),
                }
            }
        }

        /// Waits until the host shows `name` online with its link up. ONLINE
        /// alone isn't the link: an add shows it once ssh worked, before the
        /// link is dialled. The daemon version is what the link's own
        /// handshake said, and the host takes the link into use before it
        /// shows either.
        pub async fn linked(&self, name: &str) -> pb::Machine {
            self.machine_until(name, "online with its link up", |machine| {
                machine.reachability == pb::Reachability::Online as i32
                    && !machine.daemon_version.is_empty()
            })
            .await
        }

        /// Ends `name`'s link as a dropped connection does and keeps it
        /// away; returns once the host shows the machine offline. The host
        /// dials again by itself after its backoff. Each dial then waits in
        /// the fake `ssh` (`hang`) until [`Self::link_back`], so the machine
        /// stays away for as long as the test needs, however slow the test
        /// is.
        pub async fn link_lost(&self, name: &str) -> pb::Machine {
            let remote = self.the(name);
            remote.machine.set("hang");
            remote.cut();
            self.machine_until(name, "offline", |machine| {
                machine.reachability == pb::Reachability::Offline as i32
            })
            .await
        }

        /// Lets the host's dials reach `name` again, and returns once the
        /// host shows its link up. No reconnect is asked for: a reconnect
        /// shows ONLINE before its link is up, and the dial the host makes
        /// by itself is what the product promises.
        pub async fn link_back(&self, name: &str) -> pb::Machine {
            self.the(name).machine.clear("hang");
            self.linked(name).await
        }
    }
}
