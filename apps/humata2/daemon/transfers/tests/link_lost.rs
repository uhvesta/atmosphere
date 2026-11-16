//! A destination whose link is lost part way (FM-8): what it had confirmed
//! is kept, what was only sent is gone, and the copy goes on from the
//! confirmed offset once the link is back.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use humata2_transfers::{
    Config, Disk, End, EndError, Entry, Item, Mode, OnExisting, Problem, Request, Sink, State,
    Transfers, Trash,
};

const MIB: u64 = 1 << 20;

/// What the far end has seen, shared by the end and its sinks.
#[derive(Default)]
struct Seen {
    /// Each `open_write`'s offset, in order.
    opened_at: Mutex<Vec<u64>>,
    /// While set, a write that would take the file past `cut_at` loses the
    /// link: the bytes after the last sync are gone, as on a machine whose
    /// link dropped with them still on the way.
    armed: AtomicBool,
}

/// A disk on a machine behind a link that can be lost.
struct Behind {
    inner: Disk,
    seen: Arc<Seen>,
    cut_at: u64,
}

struct BehindSink {
    inner: Box<dyn Sink>,
    path: PathBuf,
    seen: Arc<Seen>,
    cut_at: u64,
    written: u64,
    synced: u64,
}

impl Sink for BehindSink {
    fn write(&mut self, bytes: &[u8]) -> Result<(), EndError> {
        if self.seen.armed.load(Ordering::SeqCst) && self.written + bytes.len() as u64 > self.cut_at {
            self.seen.armed.store(false, Ordering::SeqCst);
            let file = fs::OpenOptions::new()
                .write(true)
                .open(&self.path)
                .expect("the staged file is there");
            file.set_len(self.synced).expect("the unconfirmed bytes go");
            return Err(EndError::Offline { machine: "box-1".to_owned() });
        }
        self.inner.write(bytes)?;
        self.written += bytes.len() as u64;
        Ok(())
    }

    fn sync(&mut self) -> Result<(), EndError> {
        self.inner.sync()?;
        self.synced = self.written;
        Ok(())
    }
}

impl End for Behind {
    fn label(&self) -> &str {
        "box-1"
    }
    fn same_disk(&self, _other: &dyn End, _from: &Path, _to: &Path) -> bool {
        false
    }
    fn plan(&self, source: &Path) -> Result<Vec<Entry>, EndError> {
        self.inner.plan(source)
    }
    fn open_read(&self, path: &Path, offset: u64) -> Result<Box<dyn Read + Send>, EndError> {
        self.inner.open_read(path, offset)
    }
    fn free_bytes(&self, folder: &Path) -> Result<u64, EndError> {
        self.inner.free_bytes(folder)
    }
    fn exists(&self, path: &Path) -> Result<bool, EndError> {
        self.inner.exists(path)
    }
    fn make_folder(&self, path: &Path, mode: u32) -> Result<(), EndError> {
        self.inner.make_folder(path, mode)
    }
    fn make_link(&self, path: &Path, target: &Path) -> Result<(), EndError> {
        self.inner.make_link(path, target)
    }
    fn open_write(&self, path: &Path, offset: u64) -> Result<Box<dyn Sink>, EndError> {
        self.seen.opened_at.lock().expect("the list").push(offset);
        let inner = self.inner.open_write(path, offset)?;
        Ok(Box::new(BehindSink {
            inner,
            path: path.to_owned(),
            seen: self.seen.clone(),
            cut_at: self.cut_at,
            written: offset,
            synced: offset,
        }))
    }
    fn hash(&self, path: &Path, len: u64) -> Result<[u8; 32], EndError> {
        self.inner.hash(path, len)
    }
    fn finish_file(&self, path: &Path, mode: u32, ms: i64) -> Result<(), EndError> {
        self.inner.finish_file(path, mode, ms)
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<(), EndError> {
        self.inner.rename(from, to)
    }
    fn sync_folder(&self, folder: &Path) -> Result<(), EndError> {
        self.inner.sync_folder(folder)
    }
    fn remove_tree(&self, path: &Path) -> Result<(), EndError> {
        self.inner.remove_tree(path)
    }
    fn trash(&self, path: &Path, description: &str, now: i64) -> Result<(), EndError> {
        self.inner.trash(path, description, now)
    }
}

struct World {
    dir: tempfile::TempDir,
    seen: Arc<Seen>,
    transfers: Arc<Transfers>,
}

/// A copy from this disk to `box-1`, which syncs every 4 MiB (the engine
/// writes 1 MiB at a time) and loses its link at `cut_at` bytes into the
/// file.
fn world(cut_at: u64) -> World {
    let dir = tempfile::tempdir().expect("a folder");
    let trash = Arc::new(Trash::open(dir.path().join("trash")).expect("a trash"));
    let seen = Arc::new(Seen::default());
    seen.armed.store(true, Ordering::SeqCst);
    let transfers = Transfers::open(Config {
        journal: dir.path().join("journal"),
        source: Arc::new(Disk::new("local", trash.clone())),
        destination: Arc::new(Behind {
            inner: Disk::new("box-1", trash),
            seen: seen.clone(),
            cut_at,
        }),
        free_floor_bytes: 0,
        sync_every_bytes: 4 * MIB,
        listener: Arc::new(|_| {}),
    })
    .expect("the engine opens");
    fs::create_dir_all(dir.path().join("src")).expect("src");
    fs::create_dir_all(dir.path().join("dst")).expect("dst");
    World { dir, seen, transfers }
}

fn bytes(len: u64, seed: u8) -> Vec<u8> {
    let mut state = u32::from(seed) + 1;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 24) as u8
        })
        .collect()
}

fn copy(w: &World, id: &str, source: &Path) {
    let item = |path: &Path| Item {
        path: path.to_owned(),
        shown: path.display().to_string(),
    };
    w.transfers
        .start(Request {
            id: id.to_owned(),
            sources: vec![item(source)],
            destination: item(&w.dir.path().join("dst")),
            mode: Mode::Copy,
            on_existing: OnExisting::Fail,
            caller_data: String::new(),
            now_unix_ms: 1000,
        })
        .expect("the copy starts");
}

/// Waits for the link loss, which must leave the transfer waiting for its
/// machine, not failed for good.
fn lost(w: &World, id: &str) {
    let ended = w.transfers.wait(id).expect("the transfer is known");
    assert_eq!(ended.state, State::Failed, "{ended:?}");
    assert!(
        matches!(&ended.problem, Some(Problem::Offline { machine }) if machine == "box-1"),
        "a lost link is the machine being offline: {:?}",
        ended.problem
    );
}

#[test]
fn a_copy_goes_on_from_what_the_machine_confirmed_when_its_link_is_back() {
    let w = world(6 * MIB + MIB / 2);
    let source = w.dir.path().join("src/big.bin");
    fs::write(&source, bytes(8 * MIB, 1)).expect("the source");
    copy(&w, "t", &source);
    lost(&w, "t");

    w.transfers.resume("t").expect("the copy resumes");
    let done = w.transfers.wait("t").expect("the transfer is known");
    assert_eq!(done.state, State::Done, "{done:?}");
    assert_eq!(fs::read(w.dir.path().join("dst/big.bin")).expect("copied"), bytes(8 * MIB, 1));
    let opened = w.seen.opened_at.lock().expect("the list").clone();
    assert_eq!(
        opened.last().copied(),
        Some(4 * MIB),
        "after the link is back the file goes on from the 4 MiB the machine confirmed, not from byte 0: opened at {opened:?}"
    );
}

#[test]
fn a_confirmed_prefix_that_no_longer_matches_is_copied_again_from_the_start() {
    let w = world(6 * MIB + MIB / 2);
    let source = w.dir.path().join("src/big.bin");
    fs::write(&source, bytes(8 * MIB, 2)).expect("the source");
    copy(&w, "t", &source);
    lost(&w, "t");

    // Something changed the first MiB on the machine while the link was down.
    let staged = fs::read_dir(w.dir.path().join("dst"))
        .expect("dst")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("big.bin"))
        .find(|path| path.exists())
        .expect("the staged file is there");
    let mut changed = fs::read(&staged).expect("staged");
    changed[..1024].fill(0);
    fs::write(&staged, &changed).expect("changed");

    w.transfers.resume("t").expect("the copy resumes");
    let done = w.transfers.wait("t").expect("the transfer is known");
    assert_eq!(done.state, State::Done, "{done:?}");
    assert_eq!(fs::read(w.dir.path().join("dst/big.bin")).expect("copied"), bytes(8 * MIB, 2));
    let opened = w.seen.opened_at.lock().expect("the list").clone();
    assert_eq!(opened.last().copied(), Some(0), "opened at {opened:?}");
}
