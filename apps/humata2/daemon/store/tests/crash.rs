//! Kills a writer part way through and checks what's left on disk.
//!
//! The writer is this test binary run again with `crash_child` selected and
//! the environment naming what to write; that test does nothing otherwise.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use humata2_store::{SessionRecord, Store};

const CHILD: &str = "HUMATA2_STORE_CRASH_CHILD";
const DIR: &str = "HUMATA2_STORE_DIR";
const TITLE_BYTES: usize = 2 << 20;
const PAYLOAD_BYTES: usize = 256 << 10;
const KILLS: u64 = 24;
/// What the writer prints once it is writing. libtest may print its own
/// text on the same line first.
const READY: &str = "humata2-store-writer-ready";

fn record(fill: char) -> SessionRecord {
    SessionRecord {
        id: "se_a".to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: fill.to_string().repeat(TITLE_BYTES),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work"),
        created_unix_ms: 1,
    }
}

fn payload(seq: u64) -> Vec<u8> {
    let mut bytes = seq.to_be_bytes().to_vec();
    bytes.resize(PAYLOAD_BYTES, (seq % 251) as u8);
    bytes
}

/// Tells the parent the writer has begun. Flushed, since a piped stdout
/// may hold the line back.
fn ready() {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    writeln!(out, "{READY}").unwrap();
    out.flush().unwrap();
}

/// The writer. Runs only when the parent test starts it.
#[test]
fn crash_child() {
    let Ok(mode) = std::env::var(CHILD) else {
        return;
    };
    let store = Store::open(std::env::var(DIR).unwrap()).unwrap();
    match mode.as_str() {
        "session" => {
            ready();
            for fill in ['b', 'a'].into_iter().cycle() {
                store.put_session(&record(fill)).unwrap();
            }
        }
        "journal" => {
            let mut seq = store.journal().entries().unwrap().last().map_or(1, |e| e.seq + 1);
            ready();
            loop {
                assert_eq!(store.journal().append("write", &payload(seq)).unwrap(), seq);
                seq += 1;
            }
        }
        other => panic!("no crash mode {other}"),
    }
}

struct Writer(Child);

impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Starts the writer, waits until it is writing, then kills it with
/// SIGKILL after `delay`.
fn write_then_kill(mode: &str, dir: &Path, delay: Duration) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["crash_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, mode)
        .env(DIR, dir)
        .env_remove("TESTBRIDGE_TEST_ONLY")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let writer = Writer(child);
    let ready = BufReader::new(stdout)
        .lines()
        .map_while(Result::ok)
        .any(|line| line.contains(READY));
    assert!(ready, "the writer stopped before it began writing");
    std::thread::sleep(delay);
    drop(writer);
}

fn delay(kill: u64) -> Duration {
    Duration::from_micros(500 + (kill * 7_919) % 60_000)
}

fn leftovers(folder: &Path) -> Vec<String> {
    fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".partial"))
        .collect()
}

#[test]
fn a_record_killed_mid_write_is_the_old_copy_or_the_new_one() {
    let dir = tempfile::tempdir().unwrap();
    Store::open(dir.path()).unwrap().put_session(&record('a')).unwrap();

    for kill in 0..KILLS {
        write_then_kill("session", dir.path(), delay(kill));

        let store = Store::open(dir.path()).unwrap();
        let found = store.session("se_a").unwrap().unwrap();
        let first = found.title.chars().next().unwrap();
        assert!(first == 'a' || first == 'b');
        assert_eq!(found, record(first), "kill {kill} left a torn record");
        assert!(leftovers(&dir.path().join("sessions")).is_empty());
    }
}

#[test]
fn a_journal_killed_mid_append_keeps_every_finished_entry_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut kept = 0;

    for kill in 0..KILLS {
        write_then_kill("journal", dir.path(), delay(kill));

        let store = Store::open(dir.path()).unwrap();
        let entries = store.journal().entries().unwrap();
        let seqs: Vec<u64> = entries.iter().map(|entry| entry.seq).collect();
        let expected: Vec<u64> = (1..=entries.len() as u64).collect();
        assert_eq!(seqs, expected, "kill {kill} left a gap");
        for entry in &entries {
            assert_eq!(entry.payload, payload(entry.seq), "entry {} is torn", entry.seq);
        }
        assert!(entries.len() >= kept, "kill {kill} lost finished entries");
        kept = entries.len();
        assert!(leftovers(&dir.path().join("journal")).is_empty());
    }
    assert!(kept > 0, "no entry was ever written");
}
