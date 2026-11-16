//! Cuts the power while a daemon writes to a host's store and a machine's
//! (PER-25): after each cut, both must be exactly as a model says after
//! the writes that returned, or after one more, never torn, and with
//! every blob file a row names on disk and none that no row names.
//!
//! The same holds for the last good copy (PER-31): a power cut while it's
//! saved or restored leaves a store that opens whole, from the copy or
//! from what it replaced.
//!
//! The writer is this test binary run again with `power_cut_child`
//! selected, in the mode its env var names; that test does nothing
//! otherwise. The `writes` mode runs one fixed list of writes over both
//! stores and prints `done <n>` once write `n` returns. `HUMATA2_STOP_AT`
//! names where it's cut and `HUMATA2_POWER_CUT_ROOTS` which stores lose
//! what they hadn't synced; a store not named is left as a `kill -9` leaves
//! it. Needs the store built with `faults`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use humata2_store::{SessionRecord, Store};

const CHILD: &str = "HUMATA2_STORE_POWER_CUT_CHILD";
const DIR: &str = "HUMATA2_STORE_DIR";
/// Bigger than a row holds, so it goes in a file of its own.
const BIG: usize = 100 << 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Host,
    Machine,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Put(&'static str, &'static str),
    Remove(&'static str),
    Append(Payload),
    Ack(u64),
}

#[derive(Clone, Copy, Debug)]
enum Payload {
    Small(u8),
    Big(u64),
}

impl Payload {
    fn bytes(self) -> Vec<u8> {
        match self {
            Self::Small(byte) => vec![byte],
            Self::Big(seed) => noise(BIG, seed),
        }
    }
}

/// The writes, in order. Both stores are written in turn, as a daemon
/// writing the host's copy and a machine's would.
const WORK: &[(Side, Op)] = &[
    (Side::Host, Op::Put("se_h", "h0")),
    (Side::Machine, Op::Put("se_m", "m1")),
    (Side::Host, Op::Append(Payload::Big(2))),
    (Side::Machine, Op::Append(Payload::Big(3))),
    (Side::Host, Op::Put("se_h", "h4")),
    (Side::Machine, Op::Append(Payload::Small(5))),
    (Side::Host, Op::Append(Payload::Big(6))),
    (Side::Machine, Op::Ack(1)),
    (Side::Host, Op::Ack(1)),
    (Side::Machine, Op::Remove("se_m")),
    (Side::Host, Op::Append(Payload::Big(10))),
    (Side::Machine, Op::Put("se_m", "m11")),
];

/// `len` bytes zstd can't shrink.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut bytes = Vec::with_capacity(len + 8);
    while bytes.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        bytes.extend_from_slice(&state.to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

fn session(id: &str, title: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: title.to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work"),
        created_unix_ms: 1,
    }
}

fn root(dir: &Path, side: Side) -> PathBuf {
    dir.join(match side {
        Side::Host => "host",
        Side::Machine => "machine",
    })
}

/// The one store the last good copy tests use. Not `dir` itself, so the
/// `spare` folder the cut keeps its hard links in sits beside it.
fn store_root(dir: &Path) -> PathBuf {
    dir.join("store")
}

/// The writer. Runs only when a test below starts it, in the mode its env
/// var names.
#[test]
fn power_cut_child() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let dir = PathBuf::from(std::env::var(DIR).unwrap());
    match mode.as_str() {
        "writes" => write_work(&dir),
        "last_good" => {
            Store::open(store_root(&dir)).unwrap().save_last_good().unwrap();
        }
        "restore" => {
            Store::open_or_restore(store_root(&dir)).unwrap();
        }
        other => panic!("no power cut mode {other}"),
    }
}

/// Runs [`WORK`] over both stores, printing `done <n>` after each write.
fn write_work(dir: &Path) {
    let host = Store::open(root(dir, Side::Host)).unwrap();
    let machine = Store::open(root(dir, Side::Machine)).unwrap();
    let mut out = std::io::stdout().lock();
    for (n, (side, op)) in WORK.iter().enumerate() {
        let store = match side {
            Side::Host => &host,
            Side::Machine => &machine,
        };
        match *op {
            Op::Put(id, title) => store.put_session(&session(id, title)).unwrap(),
            Op::Remove(id) => store.remove_session(id).unwrap(),
            Op::Append(payload) => {
                store.journal().append("write", &payload.bytes()).unwrap();
            }
            Op::Ack(seq) => store.journal().ack_through(seq).unwrap(),
        }
        writeln!(out, "done {n}").unwrap();
        out.flush().unwrap();
    }
}

/// What one store should hold.
#[derive(Debug, Default, PartialEq, Eq)]
struct State {
    titles: BTreeMap<String, String>,
    /// Sequence and payload of each entry not acked, the payload as its
    /// length and a hash so a failure prints readably.
    journal: Vec<(u64, usize, u64)>,
    acked: u64,
    /// Blob files the rows name.
    files: usize,
}

fn entry(seq: u64, payload: &[u8]) -> (u64, usize, u64) {
    let hash = payload.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    });
    (seq, payload.len(), hash)
}

/// What `side`'s store holds after the first `done` writes.
fn model(side: Side, done: usize) -> State {
    let mut state = State::default();
    let mut payloads: Vec<(u64, Payload)> = Vec::new();
    let mut next = 1;
    for (_, op) in WORK.iter().take(done).filter(|(on, _)| *on == side) {
        match *op {
            Op::Put(id, title) => {
                state.titles.insert(id.to_owned(), title.to_owned());
            }
            Op::Remove(id) => {
                state.titles.remove(id);
            }
            Op::Append(payload) => {
                payloads.push((next, payload));
                next += 1;
            }
            Op::Ack(seq) => {
                state.acked = state.acked.max(seq);
                payloads.retain(|(entry, _)| *entry > seq);
            }
        }
    }
    state.files = payloads.iter().filter(|(_, payload)| matches!(payload, Payload::Big(_))).count();
    state.journal = payloads.into_iter().map(|(seq, payload)| entry(seq, &payload.bytes())).collect();
    state
}

/// What `side`'s store holds on disk, checking SQLite finds it whole.
fn found(dir: &Path, side: Side) -> State {
    let root = root(dir, side);
    let database = root.join("state.sqlite");
    if database.exists() {
        let db = rusqlite::Connection::open(&database).unwrap();
        let check: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0)).unwrap();
        assert_eq!(check, "ok", "{side:?}'s database is damaged");
    }
    // Opening sweeps files no row names, as the daemon's start does.
    let store = Store::open(&root).unwrap();
    State {
        titles: store
            .sessions()
            .unwrap()
            .into_iter()
            .map(|session| (session.id, session.title))
            .collect(),
        journal: store
            .journal()
            .entries()
            .unwrap()
            .into_iter()
            .map(|found| entry(found.seq, &found.payload))
            .collect(),
        acked: store.journal().acked().unwrap(),
        files: fs::read_dir(root.join("blobs")).unwrap().count(),
    }
}

/// Runs the child in `mode` until `stop_at`, with the folders in `roots`
/// losing what wasn't synced, and returns how it ended.
fn child(dir: &Path, mode: &str, stop_at: &str, roots: &[PathBuf]) -> Output {
    let roots = std::env::join_paths(roots).unwrap();
    Command::new(std::env::current_exe().unwrap())
        .args(["power_cut_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, mode)
        .env(DIR, dir)
        .env("HUMATA2_STOP_AT", stop_at)
        .env("HUMATA2_POWER_CUT_ROOTS", roots)
        .env("HUMATA2_POWER_CUT_SPARE", dir.join("spare"))
        .env_remove("TESTBRIDGE_TEST_ONLY")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .unwrap()
}

/// Runs the writer, cut at `stop_at` with `cut` losing what wasn't
/// synced, and returns how many writes returned before it.
fn run(dir: &Path, stop_at: &str, cut: &[Side]) -> usize {
    let roots: Vec<PathBuf> = cut.iter().map(|side| root(dir, *side)).collect();
    let output = child(dir, "writes", stop_at, &roots);
    let done = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.contains("done "))
        .count();
    if output.status.signal() == Some(9) {
        done
    } else {
        assert!(output.status.success(), "the writer failed: {}", output.status);
        assert_eq!(done, WORK.len(), "the writer ended without finishing");
        done
    }
}

/// Runs the writer in `mode` cut at `stop_at` with `roots` losing what
/// wasn't synced, and checks it was cut there rather than finishing.
fn cut_at(dir: &Path, mode: &str, stop_at: &str, roots: &[PathBuf]) {
    let output = child(dir, mode, stop_at, roots);
    assert_eq!(output.status.signal(), Some(9), "{mode} never reached {stop_at}");
}

/// Every point a write passes, each as often as the work passes it.
fn cut_points() -> Vec<String> {
    let mut points = Vec::new();
    for op in ["session.put", "session.remove", "journal.append", "journal.ack"] {
        for stage in ["committing", "committed"] {
            for nth in 1..=4 {
                points.push(format!("store.{op}.{stage}@{nth}"));
            }
        }
    }
    for nth in 1..=4 {
        points.push(format!("store.blob.written@{nth}"));
    }
    points
}

fn check(cut: &[Side]) {
    let mut cuts = 0;
    for stop_at in cut_points() {
        let dir = tempfile::tempdir().unwrap();
        let done = run(dir.path(), &stop_at, cut);
        if done < WORK.len() {
            cuts += 1;
        }
        let host = found(dir.path(), Side::Host);
        let machine = found(dir.path(), Side::Machine);
        let matches = [done, (done + 1).min(WORK.len())]
            .into_iter()
            .any(|after| host == model(Side::Host, after) && machine == model(Side::Machine, after));
        assert!(
            matches,
            "cut at {stop_at} after {done} writes left host {host:?}, machine {machine:?}; \
             expected {:?}, {:?}",
            model(Side::Host, done),
            model(Side::Machine, done),
        );
    }
    assert!(cuts > 0, "no cut point was ever reached");
}

#[test]
fn a_cut_losing_the_hosts_unsynced_writes_leaves_both_stores_as_the_model() {
    check(&[Side::Host]);
}

#[test]
fn a_cut_losing_the_machines_unsynced_writes_leaves_both_stores_as_the_model() {
    check(&[Side::Machine]);
}

#[test]
fn a_cut_losing_both_unsynced_writes_leaves_both_stores_as_the_model() {
    check(&[Side::Host, Side::Machine]);
}

/// Overwrites the database's header, as a disk fault might.
fn damage(root: &Path) {
    let path = root.join("state.sqlite");
    let mut bytes = fs::read(&path).unwrap();
    bytes[..16].copy_from_slice(b"not a database!!");
    fs::write(&path, bytes).unwrap();
}

/// Every file under `root` a write left part way.
fn partials(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if path.to_string_lossy().ends_with(".partial") {
                found.push(path);
            }
        }
    }
    found
}

/// A store at [`store_root`] holding session `se_a` titled "old" under a
/// last good copy of it, then session `se_a` retitled `later`, unsaved.
fn store_with_copy(dir: &Path, later: &str) {
    let store = Store::open(store_root(dir)).unwrap();
    store.put_session(&session("se_a", "old")).unwrap();
    store.save_last_good().unwrap();
    store.put_session(&session("se_a", later)).unwrap();
}

#[test]
fn a_power_cut_while_saving_the_last_good_copy_keeps_a_copy_that_restores() {
    let dir = tempfile::tempdir().unwrap();
    let root = store_root(dir.path());
    store_with_copy(dir.path(), "new");
    cut_at(dir.path(), "last_good", "store.last_good.copied", std::slice::from_ref(&root));
    damage(&root);

    let (store, restored) = Store::open_or_restore(&root).unwrap();

    assert!(restored.is_some(), "the damaged database wasn't restored");
    let restored_session = store.session("se_a").unwrap();
    assert!(
        restored_session == Some(session("se_a", "old")) || restored_session == Some(session("se_a", "new")),
        "the restored store has {restored_session:?}"
    );
    assert_eq!(partials(&root), Vec::<PathBuf>::new());
}

#[test]
fn a_power_cut_part_way_through_a_restore_is_finished_when_the_store_opens_again() {
    for stop_at in ["store.restore.copied", "store.restore.moved"] {
        let dir = tempfile::tempdir().unwrap();
        let root = store_root(dir.path());
        store_with_copy(dir.path(), "since the copy");
        damage(&root);
        cut_at(dir.path(), "restore", stop_at, std::slice::from_ref(&root));

        let (store, restored) = Store::open_or_restore(&root).unwrap();

        let restored = restored.unwrap_or_else(|| panic!("{stop_at}: not restored"));
        assert!(restored.damaged_kept_at.exists(), "{stop_at}: the damaged database wasn't kept");
        assert_eq!(store.session("se_a").unwrap(), Some(session("se_a", "old")), "{stop_at}");
        assert_eq!(partials(&root), Vec::<PathBuf>::new(), "{stop_at}");
        drop(store);
        // Restored once: opening again finds nothing to do.
        let (_, again) = Store::open_or_restore(&root).unwrap();
        assert!(again.is_none(), "{stop_at}");
    }
}
