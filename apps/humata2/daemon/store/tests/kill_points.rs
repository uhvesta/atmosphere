//! Kills a writer with SIGKILL at each named point around a commit and
//! checks the state it leaves: exactly as before the write, or exactly as
//! after it, and never a blob file without its row or a row without its
//! file. Needs the store built with `faults`.
//!
//! The writer is this test binary run again with `kill_child` selected;
//! that test does nothing otherwise.

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use humata2_store::{
    AgentLaunch, AgentRecord, AgentState, SessionRecord, Store, TabKind, TabRecord,
};

const CHILD: &str = "HUMATA2_STORE_KILL_CHILD";
const DIR: &str = "HUMATA2_STORE_DIR";
const BIG: usize = 300 << 10;

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

fn tab(id: &str) -> TabRecord {
    TabRecord {
        id: id.to_owned(),
        session: Some("se_a".to_owned()),
        kind: TabKind::Agent,
        title: "claude".to_owned(),
        holder_socket: PathBuf::from("/tmp/tb.sock"),
        holder_pid: 1,
        created_unix_ms: 1,
    }
}

fn agent() -> AgentRecord {
    AgentRecord {
        session: "se_a".to_owned(),
        tab: "tb_agent".to_owned(),
        launch: AgentLaunch {
            agent: "claude".to_owned(),
            folder: PathBuf::from("/work"),
            argv: vec!["claude".into()],
            conversation_id: "c1".to_owned(),
            ..AgentLaunch::default()
        },
        state: AgentState::Running { holder_pid: 1 },
        generation: 1,
        interruptions: Vec::new(),
        updated_unix_ms: 1,
    }
}

/// `len` bytes zstd can't shrink, so they go in a file of their own.
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
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

/// The writer. Runs only when a test below starts it.
#[test]
fn kill_child() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let store = Store::open(std::env::var(DIR).unwrap()).unwrap();
    match mode.as_str() {
        "rename" => store.put_session(&session("se_a", "new")).unwrap(),
        "append_big" => {
            store.journal().append("write", &noise(BIG, 9)).unwrap();
        }
        "append_many" => {
            for n in 0..10u8 {
                store.journal().append("write", &[n]).unwrap();
            }
        }
        "ack" => store.journal().ack_through(2).unwrap(),
        "launch" => store.put_agent_and_tab(&agent(), &tab("tb_agent")).unwrap(),
        "remove" => store.remove_session("se_a").unwrap(),
        other => panic!("no kill mode {other}"),
    }
}

/// Runs the writer in `mode` until `stop_at`, and checks it was killed
/// there rather than finishing.
fn kill_at(dir: &Path, mode: &str, stop_at: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["kill_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, mode)
        .env(DIR, dir)
        .env("HUMATA2_STOP_AT", stop_at)
        .env_remove("TESTBRIDGE_TEST_ONLY")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9), "{mode} never reached {stop_at}");
}

fn blob_files(dir: &Path) -> usize {
    fs::read_dir(dir.join("blobs")).unwrap().count()
}

/// A store holding session `se_a` titled "old", one agent tab, and three
/// journal entries.
fn base() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", "old")).unwrap();
    store.put_tab(&tab("tb_shell")).unwrap();
    for n in 0..3u8 {
        store.journal().append("write", &[n]).unwrap();
    }
    dir
}

#[test]
fn a_write_killed_before_its_commit_is_absent_and_after_it_is_whole() {
    for (stop_at, title) in [
        ("store.session.put.committing", "old"),
        ("store.session.put.committed", "new"),
    ] {
        let dir = base();
        kill_at(dir.path(), "rename", stop_at);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.session("se_a").unwrap(), Some(session("se_a", title)), "{stop_at}");
        assert_eq!(store.tabs().unwrap().len(), 1, "{stop_at} lost the session's tab");
    }
}

#[test]
fn a_big_append_killed_between_its_file_and_its_row_leaves_neither() {
    for stop_at in ["store.blob.written", "store.journal.append.committing"] {
        let dir = base();
        kill_at(dir.path(), "append_big", stop_at);
        let store = Store::open(dir.path()).unwrap();
        let seqs: Vec<u64> = store.journal().entries().unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3], "{stop_at}");
        assert_eq!(blob_files(dir.path()), 0, "{stop_at} left its file behind");
        // The sequence it would have taken is still free.
        assert_eq!(store.journal().append("write", b"next").unwrap(), 4);
    }

    let dir = base();
    kill_at(dir.path(), "append_big", "store.journal.append.committed");
    let store = Store::open(dir.path()).unwrap();
    let entries = store.journal().entries().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[3].payload, noise(BIG, 9));
    assert_eq!(blob_files(dir.path()), 1);
}

#[test]
fn appends_killed_part_way_keep_every_committed_entry_in_order() {
    for nth in [1, 4, 10] {
        let dir = base();
        kill_at(dir.path(), "append_many", &format!("store.journal.append.committed@{nth}"));
        let store = Store::open(dir.path()).unwrap();
        let entries = store.journal().entries().unwrap();
        let seqs: Vec<u64> = entries.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (1..=3 + nth).collect::<Vec<u64>>());
        for (n, entry) in entries[3..].iter().enumerate() {
            assert_eq!(entry.payload, vec![n as u8]);
        }
    }
}

#[test]
fn an_ack_killed_before_its_commit_keeps_the_entries_and_after_it_drops_them() {
    for (stop_at, acked, left) in [
        ("store.journal.ack.committing", 0, vec![1, 2, 3]),
        ("store.journal.ack.committed", 2, vec![3]),
    ] {
        let dir = base();
        kill_at(dir.path(), "ack", stop_at);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.journal().acked().unwrap(), acked, "{stop_at}");
        let seqs: Vec<u64> = store.journal().entries().unwrap().iter().map(|e| e.seq).collect();
        assert_eq!(seqs, left, "{stop_at}");
    }
}

#[test]
fn an_agent_and_its_tab_are_saved_together_or_not_at_all() {
    for (stop_at, saved) in [
        ("store.agent_and_tab.put.committing", false),
        ("store.agent_and_tab.put.committed", true),
    ] {
        let dir = base();
        kill_at(dir.path(), "launch", stop_at);
        let store = Store::open(dir.path()).unwrap();
        assert_eq!(store.agent("se_a").unwrap().is_some(), saved, "{stop_at}");
        assert_eq!(store.tab("tb_agent").unwrap().is_some(), saved, "{stop_at}");
    }
}

#[test]
fn a_removal_killed_before_its_commit_leaves_the_session_to_remove_again() {
    let dir = base();
    kill_at(dir.path(), "remove", "store.session.remove.committing");
    let store = Store::open(dir.path()).unwrap();
    assert!(store.session("se_a").unwrap().is_some());
    assert_eq!(store.tabs().unwrap().len(), 1);

    store.remove_session("se_a").unwrap();
    assert!(store.session("se_a").unwrap().is_none());
    assert!(store.tabs().unwrap().is_empty());
}
