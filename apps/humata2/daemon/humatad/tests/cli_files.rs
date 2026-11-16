//! The `humata` command line against the real daemon's file services:
//! `files ls/cat/put/mkdir/mv/rm`, `cp` and `mv` as transfers, `transfers`,
//! `transfer cancel` and the trash (CLI.md §5.5, FM-2, FM-3, FM-14). Each
//! test runs the real `humata` binary against a daemon that knows one
//! channel and one session, and checks the exit code, the JSON it printed
//! and the disk.

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{Daemon, folder, humata, machine_id, state_folder};
use humata2_store::{ChannelRecord, ChannelRepo, MainClone as MainState, SessionRecord, Store};
use serde_json::Value;
use sha2::{Digest, Sha256};

const SESSION: &str = "se_cf";
const CHANNEL: &str = "ch_cf";
const CHANNEL_FOLDER: &str = "cf-channel";
const REPO: &str = "api";

/// The most a test waits for a transfer to end.
const LONG: Duration = Duration::from_secs(90);

// The CLI's exit codes (cli/src/exit.rs), by meaning.
const SUCCESS: i32 = 0;
const USAGE: i32 = 2;
const CONFLICT: i32 = 6;
const PRECONDITION: i32 = 7;

struct World {
    daemon: Daemon,
    /// This daemon's own machine, as a location names it.
    machine: String,
    /// The session's clone of `REPO`.
    clone: PathBuf,
    shared: PathBuf,
    channel: PathBuf,
    /// A folder for local files `put` reads.
    local: PathBuf,
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
            title: "cli files".to_owned(),
            agent: "zsh".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    store
        .put_channel(&ChannelRecord {
            id: CHANNEL.to_owned(),
            name: "cli files".to_owned(),
            folder: CHANNEL_FOLDER.to_owned(),
            branch_prefix: String::new(),
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
    let local = dir.path().join("local");
    for path in [
        &clone,
        &shared,
        &local,
        &channel.join("main").join(REPO),
        &channel.join("sessions"),
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    let daemon = Daemon::start_in(dir).await;
    let machine = machine_id(&daemon).await.value;
    World {
        daemon,
        machine,
        clone,
        shared,
        channel,
        local,
    }
}

impl World {
    /// `humata --channel <channel> <args>`.
    fn run(&self, args: &[&str]) -> (i32, Value) {
        let mut all = vec!["--channel", CHANNEL];
        all.extend_from_slice(args);
        humata(&self.daemon, &all)
    }

    /// As `run`, which must have succeeded; the `result` it printed.
    fn ok(&self, args: &[&str]) -> Value {
        let (code, json) = self.run(args);
        assert_eq!(code, SUCCESS, "humata {args:?} failed: {json}");
        assert_eq!(json["ok"], true, "{json}");
        json["result"].clone()
    }

    /// A path in the channel's shared folder.
    fn shared_at(&self, path: &str) -> String {
        format!("{}:shared/{path}", self.machine)
    }

    /// A path in the session's clone of `REPO`.
    fn clone_at(&self, path: &str) -> String {
        format!("{}:{SESSION}/acme/{REPO}/{path}", self.machine)
    }

    /// Runs `transfers` until `id` is final, and returns it. A transfer
    /// that failed or was cancelled stops the test.
    async fn settled(&self, id: &str) -> Value {
        let deadline = Instant::now() + LONG;
        loop {
            let listed = self.ok(&["transfers"]);
            let found = listed["transfers"]
                .as_array()
                .and_then(|all| all.iter().find(|transfer| transfer["id"] == id))
                .cloned();
            if let Some(transfer) = found {
                match transfer["state"].as_str() {
                    Some("done") => return transfer,
                    Some("failed" | "cancelled") => panic!("the transfer ended badly: {transfer}"),
                    _ => {}
                }
            }
            assert!(Instant::now() < deadline, "transfer {id} never finished: {listed}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// The names `files ls` printed, by name, with each entry's kind and size.
fn entries(result: &Value) -> Vec<(String, String, u64)> {
    result["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("no entries in {result}"))
        .iter()
        .map(|entry| {
            let path = entry["location"]["path"].as_str().unwrap();
            (
                path.rsplit('/').next().unwrap().to_owned(),
                entry["kind"].as_str().unwrap().to_owned(),
                entry["size"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn names(folder: &Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

fn sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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

#[tokio::test(flavor = "multi_thread")]
async fn ls_lists_the_shared_folder_with_kinds_and_sizes() {
    let world = world().await;
    std::fs::create_dir(world.shared.join("docs")).unwrap();
    std::fs::write(world.shared.join("a.txt"), b"hello").unwrap();
    std::fs::write(world.shared.join("b.bin"), noise(1, 1000)).unwrap();

    let result = world.ok(&["files", "ls", &world.shared_at("")]);

    let mut listed = entries(&result);
    listed.sort();
    let file = |name: &str, size: u64| (name.to_owned(), "file".to_owned(), size);
    assert_eq!(listed.len(), 3, "{result}");
    assert!(listed.contains(&file("a.txt", 5)), "{result}");
    assert!(listed.contains(&file("b.bin", 1000)), "{result}");
    assert!(listed.iter().any(|(name, kind, _)| name == "docs" && kind == "folder"), "{result}");
}

#[tokio::test(flavor = "multi_thread")]
async fn mkdir_makes_a_folder_on_disk() {
    let world = world().await;

    world.ok(&["files", "mkdir", &world.shared_at("reports")]);

    assert!(world.shared.join("reports").is_dir());
}

#[tokio::test(flavor = "multi_thread")]
async fn mkdir_of_a_taken_name_fails_as_name_taken_and_keeps_what_is_there() {
    let world = world().await;
    std::fs::create_dir(world.shared.join("reports")).unwrap();
    std::fs::write(world.shared.join("reports").join("q1.txt"), b"keep").unwrap();
    std::fs::write(world.shared.join("notes.txt"), b"keep").unwrap();

    for taken in ["reports", "notes.txt"] {
        let (code, json) = world.run(&["files", "mkdir", &world.shared_at(taken)]);

        assert_eq!(code, CONFLICT, "{taken}: {json}");
        assert_eq!(json["ok"], false, "{json}");
        assert_eq!(json["error"]["kind"], "name_taken", "{taken}: {json}");
    }
    assert_eq!(std::fs::read(world.shared.join("reports/q1.txt")).unwrap(), b"keep");
    assert!(world.shared.join("notes.txt").is_file());
}

#[tokio::test(flavor = "multi_thread")]
async fn files_mv_renames_in_place() {
    let world = world().await;
    std::fs::write(world.shared.join("old.txt"), b"same bytes").unwrap();

    world.ok(&["files", "mv", &world.shared_at("old.txt"), "new.txt"]);

    assert_eq!(names(&world.shared), ["new.txt"]);
    assert_eq!(std::fs::read(world.shared.join("new.txt")).unwrap(), b"same bytes");
}

#[tokio::test(flavor = "multi_thread")]
async fn files_mv_refuses_a_name_with_a_slash_and_changes_nothing() {
    let world = world().await;
    std::fs::write(world.shared.join("old.txt"), b"x").unwrap();
    std::fs::create_dir(world.shared.join("docs")).unwrap();

    let (code, json) = world.run(&["files", "mv", &world.shared_at("old.txt"), "docs/new.txt"]);

    assert_eq!(code, USAGE, "{json}");
    assert_eq!(json["ok"], false, "{json}");
    assert_eq!(json["error"]["kind"], "invalid_field", "{json}");
    assert_eq!(names(&world.shared), ["docs", "old.txt"]);
    assert!(names(&world.shared.join("docs")).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn rm_moves_a_file_to_the_trash_and_restore_brings_it_back() {
    let world = world().await;
    let bytes = noise(2, 4096);
    std::fs::write(world.shared.join("gone.bin"), &bytes).unwrap();
    std::fs::write(world.shared.join("stays.txt"), b"stays").unwrap();

    let removed = world.ok(&["files", "rm", &world.shared_at("gone.bin")]);

    assert_eq!(names(&world.shared), ["stays.txt"]);
    let id = removed["items"][0].as_str().unwrap_or_else(|| panic!("no trash id in {removed}"));
    let listed = world.ok(&["--machine", &world.machine, "trash"]);
    let items = listed["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{listed}");
    assert_eq!(items[0]["id"], id, "{listed}");

    world.ok(&["trash", "restore", id]);

    assert_eq!(std::fs::read(world.shared.join("gone.bin")).unwrap(), bytes);
    let listed = world.ok(&["--machine", &world.machine, "trash"]);
    assert!(listed["items"].as_array().unwrap().is_empty(), "{listed}");
}

#[tokio::test(flavor = "multi_thread")]
async fn trash_empty_frees_what_it_listed() {
    let world = world().await;
    std::fs::write(world.shared.join("big.bin"), noise(3, 100_000)).unwrap();
    world.ok(&["files", "rm", &world.shared_at("big.bin")]);
    let before = world.ok(&["--machine", &world.machine, "trash"]);
    assert_eq!(before["items"].as_array().unwrap().len(), 1, "{before}");

    let emptied = world.ok(&["--machine", &world.machine, "--yes", "trash", "empty"]);

    assert!(emptied["freed_bytes"].as_u64().unwrap() >= 100_000, "{emptied}");
    let after = world.ok(&["--machine", &world.machine, "trash"]);
    assert!(after["items"].as_array().unwrap().is_empty(), "{after}");
    assert!(names(&world.shared).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn put_then_cat_returns_the_same_three_mebibytes() {
    let world = world().await;
    let bytes = noise(4, 3 << 20);
    let local = world.local.join("three.bin");
    std::fs::write(&local, &bytes).unwrap();

    let put = world.ok(&["files", "put", local.to_str().unwrap(), &world.shared_at("three.bin")]);

    assert_eq!(std::fs::read(world.shared.join("three.bin")).unwrap(), bytes);
    assert_eq!(put["size"], bytes.len(), "{put}");
    // `--json` prints cat as base64 plus a size and a digest.
    let cat = world.ok(&["files", "cat", &world.shared_at("three.bin")]);
    assert_eq!(cat["bytes"], bytes.len(), "{}", cat["bytes"]);
    assert_eq!(cat["sha256"], sha256(&bytes));
}

#[tokio::test(flavor = "multi_thread")]
async fn cp_from_the_session_clone_to_shared_arrives_with_identical_bytes() {
    let world = world().await;
    let bytes = noise(5, 3 << 20);
    std::fs::write(world.clone.join("data.bin"), &bytes).unwrap();

    let started = world.ok(&["cp", &world.clone_at("data.bin"), &world.shared_at("")]);

    let id = started["id"].as_str().unwrap_or_else(|| panic!("no transfer id in {started}")).to_owned();
    let end = world.settled(&id).await;
    assert_eq!(end["mode"], "copy", "{end}");
    assert_eq!(std::fs::read(world.shared.join("data.bin")).unwrap(), bytes);
    // A copy leaves the source where it was.
    assert_eq!(std::fs::read(world.clone.join("data.bin")).unwrap(), bytes);
    assert!(names(&world.shared).iter().all(|name| !name.starts_with(".humata-transfer-")));
}

#[tokio::test(flavor = "multi_thread")]
async fn mv_between_folders_moves_the_file() {
    let world = world().await;
    let bytes = noise(6, 200_000);
    std::fs::write(world.clone.join("move.bin"), &bytes).unwrap();

    let started = world.ok(&["mv", &world.clone_at("move.bin"), &world.shared_at("")]);

    let id = started["id"].as_str().unwrap_or_else(|| panic!("no transfer id in {started}")).to_owned();
    let end = world.settled(&id).await;
    assert_eq!(end["mode"], "move", "{end}");
    assert_eq!(std::fs::read(world.shared.join("move.bin")).unwrap(), bytes);
    assert!(!world.clone.join("move.bin").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn transfers_lists_a_finished_copy_and_the_active_list_leaves_it_out() {
    let world = world().await;
    std::fs::write(world.clone.join("small.txt"), b"hello").unwrap();
    let started = world.ok(&["cp", &world.clone_at("small.txt"), &world.shared_at("")]);
    let id = started["id"].as_str().unwrap().to_owned();
    world.settled(&id).await;

    let all = world.ok(&["transfers"]);
    let active = world.ok(&["transfers", "--active"]);

    let listed: Vec<&Value> = all["transfers"].as_array().unwrap().iter().collect();
    assert_eq!(listed.len(), 1, "{all}");
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(listed[0]["state"], "done");
    assert_eq!(listed[0]["total_bytes"], 5);
    assert!(active["transfers"].as_array().unwrap().is_empty(), "{active}");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_finished_transfer_fails_and_keeps_the_copy() {
    let world = world().await;
    std::fs::write(world.clone.join("kept.txt"), b"kept").unwrap();
    let started = world.ok(&["cp", &world.clone_at("kept.txt"), &world.shared_at("")]);
    let id = started["id"].as_str().unwrap().to_owned();
    world.settled(&id).await;

    let (code, json) = world.run(&["transfer", "cancel", &id]);

    assert_eq!(code, PRECONDITION, "{json}");
    assert_eq!(json["ok"], false, "{json}");
    assert_eq!(std::fs::read(world.shared.join("kept.txt")).unwrap(), b"kept");
    let after = world.settled(&id).await;
    assert_eq!(after["state"], "done");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_location_with_dot_dot_fails_as_outside_the_folders_and_changes_nothing() {
    let world = world().await;
    std::fs::write(world.shared.join("a.txt"), b"a").unwrap();
    let before_channel = names(&world.channel);
    let before_shared = names(&world.shared);

    for args in [
        vec!["files", "mkdir", "SHARED/../escaped"],
        vec!["files", "ls", "SHARED/.."],
        vec!["files", "rm", "SHARED/../main"],
        vec!["files", "mv", "SHARED/../main", "renamed"],
    ] {
        let args: Vec<String> = args
            .iter()
            .map(|arg| arg.replace("SHARED", &format!("{}:shared", world.machine)))
            .collect();
        let args: Vec<&str> = args.iter().map(String::as_str).collect();

        let (code, json) = world.run(&args);

        assert_eq!(code, USAGE, "{args:?}: {json}");
        assert_eq!(json["ok"], false, "{json}");
        assert_eq!(json["error"]["kind"], "outside_humata_folders", "{args:?}: {json}");
    }
    assert_eq!(names(&world.channel), before_channel);
    assert_eq!(names(&world.shared), before_shared);
}
