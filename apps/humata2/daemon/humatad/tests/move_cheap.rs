//! MV-17: a move is cheap. A clone whose history its remote has is fetched
//! on the target, not copied, so what crosses between the machines is the
//! work the remote lacks (the dirty files, the unpushed commits, the
//! folder's own files): far less than the clone's `.git`, and all of it
//! arrives whole. A file whose chunks the target already holds isn't sent
//! again. The copy's progress only rises, and ends full.
//!
//! What a move carried is read from its `copy` step (GRPC.md section 7):
//! `done_units` of `total_units` count its bytes, and its `sent_bytes` fact
//! is what crossed between the machines. The files are random bytes, so no
//! compression makes them small.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use common::git;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

const MIB: u64 = 1024 * 1024;

fn text(path: &Path) -> String {
    path.display().to_string()
}

/// `len` bytes no compressor can shrink, the same for the same `seed`.
fn noise(seed: u64, len: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let len = usize::try_from(len).unwrap();
    let mut bytes = Vec::with_capacity(len);
    while bytes.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

/// Every byte of the files under `path`.
fn bytes_under(path: &Path) -> u64 {
    let meta = std::fs::symlink_metadata(path).unwrap();
    if meta.is_dir() {
        std::fs::read_dir(path).unwrap().flatten().map(|entry| bytes_under(&entry.path())).sum()
    } else {
        meta.len()
    }
}

/// Pushes a commit of `history` to the remote every machine clones from,
/// so each clone made after holds it in its `.git`.
fn grow_remote(rig: &Rig, history: &[u8]) {
    let bare = rig.world.remotes.join(REPO.0).join(format!("{}.git", REPO.1));
    let work = rig.world.root.join("grow-remote");
    git(&["clone", &text(&bare), &text(&work)]);
    std::fs::write(work.join("history.bin"), history).unwrap();
    git(&["-C", &text(&work), "add", "history.bin"]);
    git(&["-C", &text(&work), "commit", "-m", "history"]);
    git(&["-C", &text(&work), "push", "origin", "main"]);
}

/// A dirty file, an untracked note in the session folder outside the clone,
/// and one commit the remote doesn't have; returns that commit.
fn make_work(clone: &Path) -> String {
    std::fs::write(clone.join("README.md"), "changed before the move\n").unwrap();
    std::fs::write(clone.join("committed.txt"), "kept\n").unwrap();
    git(&["-C", &text(clone), "add", "committed.txt"]);
    git(&["-C", &text(clone), "commit", "-m", "work before the move"]);
    std::fs::write(clone.parent().unwrap().join("PLAN.md"), "# the plan\n").unwrap();
    git(&["-C", &text(clone), "rev-parse", "HEAD"])
}

/// The channel Payments on this Mac, box-1 and box-2.
async fn payments(rig: &Rig) -> pb::Channel {
    let on = [rig.host_id().await, Rig::box_id("box-1"), Rig::box_id("box-2")];
    rig.create_channel("Payments", &[REPO], &on).await
}

/// Starts `title` on box-1 and waits for its agent.
async fn on_box_1(rig: &Rig, channel: &pb::Channel, title: &str) -> pb::SessionId {
    let id = rig.start_on(channel, &Rig::box_id("box-1"), title).await;
    rig.wait_session(&id, "live", is_live).await;
    id
}

/// `relative` in Payments' folder on `machine`.
fn on(rig: &Rig, machine: &str, channel: &pb::Channel, relative: &str) -> PathBuf {
    rig.the(machine).path(channel, relative)
}

/// Moves `id` to `to`; returns every version of the move's operation its
/// watch showed, the last one ended, once the session runs there.
async fn watched_move(rig: &Rig, id: &pb::SessionId, to: &pb::MachineId) -> Vec<pb::Operation> {
    let (started, mut watch) = begin_move(rig, id, to).await;
    let mut seen = vec![started];
    loop {
        match tokio::time::timeout(remote_rig::LONG, watch.message()).await {
            Ok(Ok(Some(next))) => seen.push(next),
            Ok(Ok(None)) => break,
            other => panic!("the move's watch failed or stalled ({other:?}); it last was {:?}", seen.last()),
        }
    }
    let ended = seen.last().expect("the watch showed the move");
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);
    rig.wait_session(id, "live where it moved", |s| s.machine.as_ref() == Some(to) && is_live(s)).await;
    seen
}

/// Starts moving `id` to `to`; returns the move's operation and its watch,
/// which pushes each change.
async fn begin_move(
    rig: &Rig,
    id: &pb::SessionId,
    to: &pb::MachineId,
) -> (pb::Operation, tonic::Streaming<pb::Operation>) {
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .move_session(pb::MoveSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: format!("move-{}-{}", id.value, to.value),
            }),
            id: Some(id.clone()),
            to: Some(to.clone()),
            ..Default::default()
        })
        .await
        .expect("the move starts")
        .into_inner();
    let watch = pb::operation_service_client::OperationServiceClient::new(rig.channel().await)
        .watch_operation(pb::WatchOperationRequest {
            id: started.id.clone(),
            from_version: 0,
        })
        .await
        .expect("the move can be watched")
        .into_inner();
    (started, watch)
}

fn copy_step(operation: &pb::Operation) -> &pb::OperationStep {
    operation
        .steps
        .iter()
        .find(|step| step.key == "copy")
        .unwrap_or_else(|| panic!("a move has a copy step: {:?}", operation.steps))
}

/// What crossed between the machines for the move, as its copy step says.
fn sent_bytes(operation: &pb::Operation) -> u64 {
    let copy = copy_step(operation);
    copy.facts
        .get("sent_bytes")
        .unwrap_or_else(|| panic!("the copy step says what it sent: {copy:?}"))
        .parse()
        .unwrap_or_else(|_| panic!("sent_bytes is a decimal count: {copy:?}"))
}

/// MV-17: box-1's clone holds a large history its remote has. Moved to
/// box-2, the clone is fetched there: the move sends a small part of the
/// `.git`'s size, and the unpushed commit, the dirty file, the folder's
/// note and the history all arrive whole.
#[tokio::test(flavor = "multi_thread")]
async fn a_clone_its_remote_has_is_fetched_on_the_target_not_copied() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let history = noise(7, 8 * MIB);
    grow_remote(&rig, &history);
    let channel = payments(&rig).await;
    let id = on_box_1(&rig, &channel, "Cache warmup").await;
    let clone = on(&rig, "box-1", &channel, "sessions/cache-warmup/api");
    let common_git = {
        let raw = git(&["-C", &text(&clone), "rev-parse", "--git-common-dir"]);
        let p = PathBuf::from(raw.trim());
        if p.is_absolute() { p } else { clone.join(p) }
    };
    let git_size = bytes_under(&common_git);
    assert!(git_size >= 8 * MIB, "the clone holds the remote's history: {git_size} bytes");
    let head = make_work(&clone);

    let seen = watched_move(&rig, &id, &Rig::box_id("box-2")).await;

    let sent = sent_bytes(seen.last().unwrap());
    assert!(sent > 0, "the work the remote lacks has to be sent");
    assert!(
        sent < git_size / 8,
        "the move sent {sent} bytes for a clone whose .git is {git_size}: its history was copied, not fetched"
    );
    let moved = on(&rig, "box-2", &channel, "sessions/cache-warmup/api");
    assert!(moved.join(".git").is_file(), "box-2 rebuilt the moved clone as a linked worktree from its own .repo-cache");
    assert_eq!(git(&["-C", &text(&moved), "rev-parse", "HEAD"]), head, "the unpushed commit came along");
    assert_eq!(std::fs::read_to_string(moved.join("README.md")).unwrap(), "changed before the move\n");
    assert_eq!(std::fs::read_to_string(moved.join("committed.txt")).unwrap(), "kept\n");
    assert_eq!(std::fs::read_to_string(moved.parent().unwrap().join("PLAN.md")).unwrap(), "# the plan\n");
    assert_eq!(std::fs::read(moved.join("history.bin")).unwrap(), history, "the history checked out whole");
    git(&["-C", &text(&moved), "fsck", "--no-progress"]);
}

/// MV-17: two sessions on box-1 hold the same large file. Once the first
/// has moved to box-2, box-2 holds every chunk of it, so moving the second
/// sends next to nothing for it, and both arrive whole.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_move_of_an_unchanged_file_sends_none_of_the_chunks_the_target_has() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let two = Rig::box_id("box-2");
    let channel = payments(&rig).await;
    let first = on_box_1(&rig, &channel, "Cache warmup").await;
    let second = on_box_1(&rig, &channel, "Index rebuild").await;
    let data = noise(11, 4 * MIB);
    for folder in ["sessions/cache-warmup", "sessions/index-rebuild"] {
        std::fs::write(on(&rig, "box-1", &channel, folder).join("data.bin"), &data).unwrap();
    }

    let sent_first = sent_bytes(watched_move(&rig, &first, &two).await.last().unwrap());
    assert!(sent_first >= 4 * MIB, "box-2 held none of data.bin, yet the first move sent only {sent_first} bytes");

    let sent_second = sent_bytes(watched_move(&rig, &second, &two).await.last().unwrap());
    assert!(
        sent_second < MIB / 2,
        "box-2 already held every chunk of data.bin, yet the second move sent {sent_second} bytes"
    );
    for folder in ["sessions/cache-warmup", "sessions/index-rebuild"] {
        assert_eq!(std::fs::read(on(&rig, "box-2", &channel, folder).join("data.bin")).unwrap(), data, "{folder}");
    }
}

/// MV-17: as a move runs, no step's done count ever goes back or passes its
/// total, and the copy ends with every byte it counted done.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_s_progress_only_rises_and_its_copy_ends_full() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let channel = payments(&rig).await;
    let id = on_box_1(&rig, &channel, "Cache warmup").await;
    std::fs::write(on(&rig, "box-1", &channel, "sessions/cache-warmup").join("data.bin"), noise(13, 4 * MIB)).unwrap();

    let seen = watched_move(&rig, &id, &Rig::box_id("box-2")).await;

    let mut done: HashMap<&str, u64> = HashMap::new();
    for operation in &seen {
        for step in &operation.steps {
            let before = done.entry(step.key.as_str()).or_default();
            assert!(
                step.done_units >= *before,
                "step {} went back from {before} to {} done (version {})",
                step.key,
                step.done_units,
                operation.version
            );
            assert!(
                step.total_units == 0 || step.done_units <= step.total_units,
                "step {} shows {} done of {}",
                step.key,
                step.done_units,
                step.total_units
            );
            *before = step.done_units;
        }
    }
    let copy = copy_step(seen.last().unwrap());
    assert!(copy.total_units >= 4 * MIB, "the copy counts the bytes it carries: {copy:?}");
    assert_eq!(copy.done_units, copy.total_units, "the copy ends full: {copy:?}");
}

/// What the target took from chunks it already held, as the copy step says.
fn reused_bytes(operation: &pb::Operation) -> u64 {
    let copy = copy_step(operation);
    copy.facts
        .get("reused_bytes")
        .unwrap_or_else(|| panic!("the copy step says what it reused: {copy:?}"))
        .parse()
        .unwrap_or_else(|_| panic!("reused_bytes is a decimal count: {copy:?}"))
}

/// MV-17, FM-6: a file that grew at its end is sent as its new tail. Box-2
/// holds the 4 MiB of a file a first session moved there, so a second
/// session whose copy of it has a tail more sends the tail, not the file,
/// and the file arrives byte for byte.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_grew_is_sent_as_its_new_tail_only() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let two = Rig::box_id("box-2");
    let channel = payments(&rig).await;
    let first = on_box_1(&rig, &channel, "Cache warmup").await;
    let second = on_box_1(&rig, &channel, "Index rebuild").await;
    let data = noise(17, 4 * MIB);
    let tail = noise(19, MIB / 10);
    let grown = [data.clone(), tail.clone()].concat();
    std::fs::write(on(&rig, "box-1", &channel, "sessions/cache-warmup").join("data.bin"), &data).unwrap();
    std::fs::write(on(&rig, "box-1", &channel, "sessions/index-rebuild").join("data.bin"), &grown).unwrap();

    let sent_first = sent_bytes(watched_move(&rig, &first, &two).await.last().unwrap());
    assert!(sent_first >= 4 * MIB, "box-2 held none of the file, yet the first move sent only {sent_first} bytes");

    let seen = watched_move(&rig, &second, &two).await;
    let ended = seen.last().unwrap();
    let (sent, reused) = (sent_bytes(ended), reused_bytes(ended));
    assert!(sent >= MIB / 10, "the new tail has to be sent: {sent} bytes");
    assert!(sent < 2 * MIB, "only the tail was new, yet the move sent {sent} bytes of a {} byte file", grown.len());
    assert!(reused >= 3 * MIB, "box-2 held 4 MiB of the file (less a re-cut last chunk), yet the move reused {reused} bytes");
    let moved = on(&rig, "box-2", &channel, "sessions/index-rebuild").join("data.bin");
    assert_eq!(std::fs::read(moved).unwrap(), grown, "the grown file arrived byte for byte");
}

/// MV-17, FM-6: a session moved away and back, its file grown meanwhile,
/// brings back only the tail: the machine it left still holds the chunks it
/// had.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_grew_while_away_comes_back_as_its_new_tail_only() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let channel = payments(&rig).await;
    let id = on_box_1(&rig, &channel, "Cache warmup").await;
    let data = noise(23, 4 * MIB);
    std::fs::write(on(&rig, "box-1", &channel, "sessions/cache-warmup").join("data.bin"), &data).unwrap();
    watched_move(&rig, &id, &Rig::box_id("box-2")).await;

    let tail = noise(29, MIB / 10);
    let there = on(&rig, "box-2", &channel, "sessions/cache-warmup").join("data.bin");
    let mut file = std::fs::OpenOptions::new().append(true).open(&there).unwrap();
    std::io::Write::write_all(&mut file, &tail).unwrap();
    drop(file);

    let seen = watched_move(&rig, &id, &Rig::box_id("box-1")).await;
    let ended = seen.last().unwrap();
    let (sent, reused) = (sent_bytes(ended), reused_bytes(ended));
    assert!(sent < 2 * MIB, "only the tail grew, yet the move back sent {sent} bytes");
    assert!(reused >= 3 * MIB, "box-1 still held the 4 MiB it sent away (less a re-cut last chunk), yet the move reused {reused} bytes");
    let back = on(&rig, "box-1", &channel, "sessions/cache-warmup").join("data.bin");
    assert_eq!(std::fs::read(back).unwrap(), [data, tail].concat(), "the grown file came back byte for byte");
}

/// Lowercase hex of `bytes`.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Record `n` of a Claude-style conversation, one after another in a chain:
/// written by a CLI running in `cwd`, holding `body`.
fn claude_line(n: u64, cwd: &str, conversation: &str, body: &str) -> String {
    let uuid = |n: u64| format!("7c0e5a10-{n:04x}-4a8e-9b1d-3f2a6c8d{n:04x}");
    let user = n % 2 == 1;
    let content = if user {
        serde_json::json!(body)
    } else {
        serde_json::json!([{ "type": "text", "text": body }])
    };
    let record = serde_json::json!({
        "parentUuid": if n == 1 { serde_json::Value::Null } else { serde_json::Value::String(uuid(n - 1)) },
        "isSidechain": false,
        "userType": "external",
        "cwd": cwd,
        "sessionId": conversation,
        "version": "2.1.325",
        "type": if user { "user" } else { "assistant" },
        "message": { "role": if user { "user" } else { "assistant" }, "content": content },
        "uuid": uuid(n),
        "timestamp": "2026-11-16T09:00:00.000Z",
    });
    format!("{record}\n")
}

/// The records from number `first` on until they hold at least `bytes`;
/// the text, and the number the next record would have. Random bodies, so
/// no compressor makes them small.
fn claude_lines(first: u64, bytes: u64, cwd: &str, conversation: &str) -> (String, u64) {
    let mut text = String::new();
    let mut n = first;
    while u64::try_from(text.len()).unwrap() < bytes {
        text.push_str(&claude_line(n, cwd, conversation, &hex(&noise(n, 512))));
        n += 1;
    }
    (text, n)
}

/// Makes the agent of each box stand in for Claude Code: its conversations
/// are Claude's JSONL under `<home>/.claude/projects/<dashed folder>/`, so a
/// move carries them through the conversation adapter (MV-16). The box's
/// daemon starts again to read it.
async fn claude_boxes(rig: &mut Rig, boxes: &[&'static str]) {
    for &name in boxes {
        let home = rig.the(name).home().to_path_buf();
        let script = home.join("agent.sh");
        std::fs::write(
            &script,
            "mode=$1; id=$2; dir=$3\nmkdir -p \"$dir\"\nf=\"$dir/$id.jsonl\"\n\
             case \"$mode\" in --session-id) : > \"$f\" ;; esac\n\
             stty -icanon 2>/dev/null\necho \"agent up $mode\"\nwhile IFS= read -r line; do echo \"got: $line\"; done\n",
        )
        .unwrap();
        let project = "{home}/.claude/projects/{folder_dashed}";
        let agents = serde_json::json!([{
            "name": common::AGENT_NAME,
            "program": "/bin/sh",
            "start_args": [script, "--session-id", "{conversation}", project],
            "resume_args": [script, "--resume", "{conversation}", project],
            "conversation_file": format!("{project}/{{conversation}}.jsonl"),
            "conversation_format": "claude-jsonl",
            "version_args": [],
        }]);
        std::fs::write(home.join("agents.json"), agents.to_string()).unwrap();
        rig.restart_box_env(name, &[]).await;
    }
}

/// Where `machine`'s agent keeps session `id`'s conversation, and the folder
/// the session is in there, as that machine's records say.
fn conversation_of(rig: &Rig, machine: &str, id: &pb::SessionId) -> (PathBuf, PathBuf) {
    let store = rig.the(machine).store();
    let agent = store.agent(&id.value).unwrap().expect("the machine has the session's agent");
    let session = store.session(&id.value).unwrap().expect("the machine has the session");
    (agent.launch.conversation_file, session.folder)
}

/// The folder as the CLI records it: its real path.
fn real(folder: &Path) -> String {
    text(&std::fs::canonicalize(folder).unwrap())
}

/// MV-16, MV-17: every move changes the session's folder, and the conversation
/// names it on every line. A conversation of several MiB moved to a box under
/// another root, grown there by a little, and moved back, is written for each
/// place it goes, and the box it left still holds the chunks it had: the move
/// back sends about the tail, not the file, and the file is what the CLI
/// would have written there.
#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_that_grew_while_away_comes_back_as_its_new_tail_only() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    claude_boxes(&mut rig, &["box-1", "box-2"]).await;
    let channel = payments(&rig).await;
    let id = on_box_1(&rig, &channel, "Cache warmup").await;
    let (file_1, folder_1) = conversation_of(&rig, "box-1", &id);
    let conversation = rig.the("box-1").conversation_id(&id);
    let (original, next) = claude_lines(1, 4 * MIB, &real(&folder_1), &conversation);
    std::fs::write(&file_1, &original).unwrap();

    watched_move(&rig, &id, &Rig::box_id("box-2")).await;

    let (file_2, folder_2) = conversation_of(&rig, "box-2", &id);
    assert_ne!(folder_1, folder_2, "the boxes keep their channels under different roots");
    let (tail, _) = claude_lines(next, MIB / 10, &real(&folder_2), &conversation);
    let mut file = std::fs::OpenOptions::new().append(true).open(&file_2).unwrap();
    std::io::Write::write_all(&mut file, tail.as_bytes()).unwrap();
    drop(file);

    let seen = watched_move(&rig, &id, &Rig::box_id("box-1")).await;

    let sent = sent_bytes(seen.last().unwrap());
    let tail_len = u64::try_from(tail.len()).unwrap();
    assert!(
        sent < 2 * MIB + tail_len,
        "only the tail grew, yet the move back sent {sent} bytes of a {} byte conversation",
        original.len()
    );
    let (file_back, folder_back) = conversation_of(&rig, "box-1", &id);
    let (written_back, _) = claude_lines(next, MIB / 10, &real(&folder_back), &conversation);
    assert_eq!(
        std::fs::read(file_back).unwrap(),
        format!("{original}{written_back}").into_bytes(),
        "the conversation came back as box-1's CLI would have written it"
    );
}

/// How far the copy step shows it is; 0 before the move has one.
fn copy_done(operation: &pb::Operation) -> u64 {
    operation.steps.iter().find(|step| step.key == "copy").map_or(0, |step| step.done_units)
}

/// MV-17: the copy's progress rises as chunks are sent and reused, never
/// passes its total, and ends equal to it. Box-2 holds 4 MiB of a file, so a
/// second session's copy of it with 8 MiB more goes as chunks: 4 MiB reused,
/// 8 MiB sent as content-defined chunks (64 KiB to 1 MiB each). The host holds
/// the copy after its third sent chunk until the test opens a FIFO, so the
/// progress is seen partway (above the reused bytes, below the total) with
/// nothing racing it: no further update can come until the release.
#[tokio::test(flavor = "multi_thread")]
async fn a_chunked_copy_shows_its_progress_rising_as_chunks_are_sent() {
    const HELD_AFTER: u64 = 3;
    tokio::time::timeout(remote_rig::LONG * 4, async {
        let mut rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
        let two = Rig::box_id("box-2");
        let channel = payments(&rig).await;
        let first = on_box_1(&rig, &channel, "Cache warmup").await;
        let second = on_box_1(&rig, &channel, "Index rebuild").await;
        let data = noise(31, 4 * MIB);
        let grown = [data.clone(), noise(37, 8 * MIB)].concat();
        std::fs::write(on(&rig, "box-1", &channel, "sessions/cache-warmup").join("data.bin"), &data).unwrap();
        std::fs::write(on(&rig, "box-1", &channel, "sessions/index-rebuild").join("data.bin"), &grown).unwrap();
        watched_move(&rig, &first, &two).await;

        // Restarted after the first move, so the count starts at the second
        // move's first chunk.
        let hold = rig.host.dir.path().join("hold-chunks.fifo");
        assert!(std::process::Command::new("mkfifo").arg(&hold).status().unwrap().success());
        let at = format!("move.copy.chunk_sent@{HELD_AFTER}");
        rig.restart_host(&[("HUMATA2_HOLD_AT", &at), ("HUMATA2_HOLD_FIFO", &text(&hold))]).await;
        rig.wait_session(&second, "live again", is_live).await;

        let (started, mut watch) = begin_move(&rig, &second, &two).await;
        let mut seen = vec![started];
        // Reused chunks cover at least 3 MiB of the 4 MiB (the last one may be
        // re-cut), and three sent chunks are at least 64 KiB each.
        let held_at = 3 * MIB + HELD_AFTER * 64 * 1024;
        while copy_done(seen.last().unwrap()) < held_at {
            match tokio::time::timeout(remote_rig::LONG, watch.message()).await {
                Ok(Ok(Some(next))) => seen.push(next),
                other => panic!("the copy never reached the hold ({other:?}); it last was {:?}", seen.last()),
            }
        }
        let held = copy_step(seen.last().unwrap()).clone();
        assert!(held.done_units < held.total_units, "the copy is not whole while held: {held:?}");
        assert!(held.total_units >= 12 * MIB, "the copy counts the chunks it sends and reuses: {held:?}");
        assert_ne!(
            seen.last().unwrap().state,
            pb::OperationState::Succeeded as i32,
            "the move ended while its copy was held"
        );

        let release = hold.clone();
        let opened =
            tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(release).map(drop));
        tokio::time::timeout(remote_rig::LONG, opened)
            .await
            .expect("the copy never got to where it is held")
            .unwrap()
            .expect("the held copy's FIFO opens");
        loop {
            match tokio::time::timeout(remote_rig::LONG, watch.message()).await {
                Ok(Ok(Some(next))) => seen.push(next),
                Ok(Ok(None)) => break,
                other => panic!("the move's watch failed or stalled ({other:?}); it last was {:?}", seen.last()),
            }
        }

        let done: Vec<u64> = seen.iter().map(copy_done).collect();
        assert!(done.windows(2).all(|pair| pair[1] >= pair[0]), "the copy's progress went back: {done:?}");
        for operation in &seen {
            if let Some(copy) = operation.steps.iter().find(|step| step.key == "copy") {
                assert!(
                    copy.done_units <= copy.total_units,
                    "the copy shows {} done of {} (version {})",
                    copy.done_units,
                    copy.total_units,
                    operation.version
                );
            }
        }
        let ended = seen.last().unwrap();
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);
        let copy = copy_step(ended);
        assert!(copy.done_units > held.done_units, "the copy went on after the release: {copy:?}");
        assert_eq!(copy.done_units, copy.total_units, "the copy ends full: {copy:?}");
    })
    .await
    .expect("the test finished in time");
}
