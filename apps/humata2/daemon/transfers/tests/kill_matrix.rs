//! PER-27 and PER-28 for file transfers and the trash: a copy, a move, a
//! cancel, an upload and each trash change are killed at every fault
//! point; then the trash and the transfers are opened again and the
//! outcome is either finished (the destination identical to the source,
//! a moved source gone and in trash) or undone (the destination as it
//! was, the source intact). Never a half file, never a staging folder.
//!
//! The process killed part way is this test binary run again with
//! `kill_child` selected and `HUMATA2_STOP_AT` set; that test does nothing
//! otherwise. Needs the crates built with `faults`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, symlink};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};

use humata2_transfers::{
    Config, Disk, End, EndError, Entry, Item, Mode, NewItem, OnExisting, Request, Sink, State,
    Transfers, Trash, UploadRequest,
};

const CHILD: &str = "HUMATA2_TRANSFERS_KILL_CHILD";
const ROOT: &str = "HUMATA2_TRANSFERS_ROOT";
const SYNC_EVERY: u64 = 1 << 20;
const BIG: usize = 5 << 20;
const UPLOAD: usize = 3 << 20;

/// A disk that may say it is another one, and may read slowly.
struct Wrapped {
    inner: Disk,
    other_disk: bool,
    slow: bool,
}

struct SlowRead(Box<dyn Read + Send>);

impl Read for SlowRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::thread::sleep(Duration::from_millis(40));
        self.0.read(buf)
    }
}

impl End for Wrapped {
    fn label(&self) -> &str {
        self.inner.label()
    }
    fn same_disk(&self, other: &dyn End, from: &Path, to: &Path) -> bool {
        !self.other_disk && self.inner.same_disk(other, from, to)
    }
    fn plan(&self, source: &Path) -> Result<Vec<Entry>, EndError> {
        self.inner.plan(source)
    }
    fn open_read(&self, path: &Path, offset: u64) -> Result<Box<dyn Read + Send>, EndError> {
        let reader = self.inner.open_read(path, offset)?;
        Ok(if self.slow { Box::new(SlowRead(reader)) } else { reader })
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
        self.inner.open_write(path, offset)
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

fn bytes(len: usize, seed: u8) -> Vec<u8> {
    let mut state = u32::from(seed) + 1;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            (state >> 24) as u8
        })
        .collect()
}

fn tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(base: &Path, path: &Path, out: &mut BTreeMap<String, String>) {
        let rel = path.strip_prefix(base).unwrap().display().to_string();
        let meta = fs::symlink_metadata(path).unwrap();
        if meta.is_symlink() {
            out.insert(rel, format!("link {:?}", fs::read_link(path).unwrap()));
        } else if meta.is_dir() {
            out.insert(rel, "dir".to_owned());
            let mut names: Vec<_> = fs::read_dir(path).unwrap().map(|e| e.unwrap().path()).collect();
            names.sort();
            for child in names {
                walk(base, &child, out);
            }
        } else {
            let content = fs::read(path).unwrap();
            // A cheap digest of the bytes: length and a rolling sum.
            let sum = content
                .iter()
                .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(u64::from(*b)));
            out.insert(rel, format!("file {:o} {} {sum}", meta.mode() & 0o7777, content.len()));
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

struct World {
    root: PathBuf,
    _owned: Option<tempfile::TempDir>,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("h2t").tempdir_in("/tmp").unwrap();
        let root = dir.path().to_owned();
        for folder in ["src/tree/nested/deep", "src/tree/empty", "dst", "journal", "other"] {
            fs::create_dir_all(root.join(folder)).unwrap();
        }
        let src = root.join("src/tree");
        fs::write(src.join("big.bin"), bytes(BIG, 1)).unwrap();
        fs::write(src.join("empty.txt"), b"").unwrap();
        fs::write(src.join("nested/a.txt"), b"alpha").unwrap();
        fs::write(src.join("nested/deep/b.bin"), bytes(1000, 2)).unwrap();
        fs::write(src.join("run.sh"), b"#!/bin/sh\n").unwrap();
        symlink("nested/a.txt", src.join("link")).unwrap();
        fs::write(root.join("dst/keep.txt"), b"keep").unwrap();
        fs::write(root.join("victim"), b"victim bytes").unwrap();
        Self { root, _owned: Some(dir) }
    }

    fn at(root: PathBuf) -> Self {
        Self { root, _owned: None }
    }

    fn trash(&self) -> Arc<Trash> {
        Arc::new(Trash::open(self.root.join("trash")).unwrap())
    }

    fn transfers(&self, trash: &Arc<Trash>, cross: bool, slow: bool) -> Arc<Transfers> {
        let end = |slow| -> Arc<dyn End> {
            Arc::new(Wrapped { inner: Disk::new("local", trash.clone()), other_disk: cross, slow })
        };
        Transfers::open(Config {
            journal: self.root.join("journal"),
            source: end(slow),
            destination: end(false),
            free_floor_bytes: 0,
            sync_every_bytes: SYNC_EVERY,
            listener: Arc::new(|_| {}),
        })
        .unwrap()
    }
}

fn item(path: &Path) -> Item {
    Item { path: path.to_owned(), shown: path.display().to_string() }
}

fn copy_request(world: &World, mode: Mode) -> Request {
    Request {
        id: "t".to_owned(),
        sources: vec![item(&world.root.join("src/tree"))],
        destination: item(&world.root.join("dst")),
        mode,
        on_existing: OnExisting::Fail,
        caller_data: String::new(),
        now_unix_ms: 1000,
    }
}

fn upload_data() -> Vec<u8> {
    bytes(UPLOAD, 7)
}

fn sha_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn upload_request(world: &World, resume: Option<&str>) -> UploadRequest {
    let data = upload_data();
    UploadRequest {
        id: "t".to_owned(),
        resume: resume.map(str::to_owned),
        destination: item(&world.root.join("dst")),
        name: "up.bin".to_owned(),
        size: data.len() as u64,
        sha256: sha_hex(&data),
        on_existing: OnExisting::Fail,
        caller_data: String::new(),
        now_unix_ms: 5,
    }
}

/// The process that gets killed. Runs only when a test below starts it.
#[test]
fn kill_child() {
    let Ok(scenario) = std::env::var(CHILD) else { return };
    let world = World::at(PathBuf::from(std::env::var_os(ROOT).unwrap()));
    let trash = world.trash();
    match scenario.as_str() {
        "copy" | "move" | "move_same_disk" => {
            let cross = scenario != "move_same_disk";
            let mode = if scenario == "copy" { Mode::Copy } else { Mode::Move };
            let t = world.transfers(&trash, cross, false);
            t.start(copy_request(&world, mode)).unwrap();
            t.wait("t").unwrap();
        }
        "cancel" => {
            let t = world.transfers(&trash, true, true);
            t.start(copy_request(&world, Mode::Copy)).unwrap();
            t.pause("t").unwrap();
            t.cancel("t").unwrap();
        }
        "upload" => {
            let t = world.transfers(&trash, true, false);
            let data = upload_data();
            let mut up = t.begin_upload(upload_request(&world, None)).unwrap();
            for (index, part) in data.chunks(SYNC_EVERY as usize).enumerate() {
                up.write((index as u64) * SYNC_EVERY, part).unwrap();
            }
            up.finish().unwrap();
        }
        "trash_put" => {
            trash.put(&world.root.join("victim"), new_item()).unwrap();
        }
        "trash_restore" => {
            let id = trash.list().unwrap().remove(0).id;
            trash.restore(&id).unwrap();
        }
        "trash_empty" => {
            let plan = trash.plan_empty().unwrap();
            trash.empty(&plan).unwrap();
        }
        other => panic!("unknown scenario {other}"),
    }
}

fn new_item() -> NewItem {
    NewItem { description: "t".to_owned(), caller_data: String::new(), now_unix_ms: 1 }
}

fn kill_at(world: &World, scenario: &str, stop_at: &str) {
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["kill_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, scenario)
        .env(ROOT, &world.root)
        .env("HUMATA2_STOP_AT", stop_at)
        .env_remove("TESTBRIDGE_TEST_ONLY")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9), "{scenario}: never reached {stop_at}");
}

fn names(folder: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

fn check_clean(world: &World, point: &str) {
    for name in names(&world.root.join("dst")) {
        assert!(!name.starts_with(".humata-transfer-"), "{point}: staging left: {name}");
    }
    for name in names(&world.root.join("trash")) {
        assert!(!name.ends_with(".deleting"), "{point}: {name} left in trash");
    }
}

/// Copies and moves: finished or undone.
fn check_transfer(world: &World, scenario: &str, point: &str) {
    let trash = world.trash();
    let cross = scenario != "move_same_disk";
    let t = world.transfers(&trash, cross, false);
    let moving = scenario != "copy";
    let src = world.root.join("src/tree");
    let dst = world.root.join("dst");
    let expected = {
        // The source as it was made.
        let fresh = World::new();
        tree(&fresh.root.join("src/tree"))
    };
    match t.wait("t") {
        None => {
            assert_eq!(names(&dst), ["keep.txt"], "{point}: the destination changed");
            assert_eq!(tree(&src), expected, "{point}: the source changed");
        }
        Some(snap) if snap.state == State::Done => {
            assert_eq!(tree(&dst.join("tree")), expected, "{point}: not identical");
            if moving {
                assert!(!src.exists(), "{point}: the source is still there");
                if cross {
                    let items = trash.list().unwrap();
                    assert_eq!(items.len(), 1, "{point}: the source isn't in trash");
                    assert_eq!(items[0].original, src);
                } else {
                    assert!(trash.list().unwrap().is_empty(), "{point}");
                }
            } else {
                assert_eq!(tree(&src), expected, "{point}: the source changed");
            }
        }
        Some(snap) => panic!("{point}: ended {:?} {:?}", snap.state, snap.problem),
    }
    check_clean(world, point);
}

fn check_cancel(world: &World, point: &str) {
    let trash = world.trash();
    let t = world.transfers(&trash, true, false);
    let expected = tree(&World::new().root.join("src/tree"));
    if let Some(snap) = t.wait("t") {
        assert_eq!(snap.state, State::Cancelled, "{point}: {:?}", snap.problem);
    }
    assert_eq!(names(&world.root.join("dst")), ["keep.txt"], "{point}");
    assert_eq!(tree(&world.root.join("src/tree")), expected, "{point}: the source changed");
    check_clean(world, point);
}

fn check_upload(world: &World, point: &str) {
    let trash = world.trash();
    let t = world.transfers(&trash, true, false);
    let data = upload_data();
    let target = world.root.join("dst/up.bin");
    match t.wait("t") {
        None => assert_eq!(names(&world.root.join("dst")), ["keep.txt"], "{point}"),
        Some(snap) if snap.state == State::Done => {
            assert_eq!(fs::read(&target).unwrap(), data, "{point}");
        }
        Some(snap) => {
            assert_eq!(snap.state, State::Paused, "{point}: {:?}", snap.problem);
            assert!(!target.exists(), "{point}: a half file is visible");
            let mut up = t.begin_upload(upload_request(world, Some("t"))).unwrap();
            let at = up.offset();
            up.write(at, &data[at as usize..]).unwrap();
            let done = up.finish().unwrap();
            assert_eq!(done.state, State::Done, "{point}");
            assert_eq!(fs::read(&target).unwrap(), data, "{point}");
        }
    }
    check_clean(world, point);
}

fn trash_entries(world: &World) -> usize {
    names(&world.root.join("trash")).len()
}

fn check_trash(world: &World, scenario: &str, point: &str) {
    let trash = world.trash();
    let items = trash.list().unwrap();
    assert_eq!(trash_entries(world), items.len(), "{point}: stray entries in the trash");
    let victim = world.root.join("victim");
    match scenario {
        "trash_put" | "trash_restore" => match items.as_slice() {
            [] => assert_eq!(fs::read(&victim).unwrap(), b"victim bytes", "{point}"),
            [one] => {
                assert!(!victim.exists(), "{point}: in trash and in place");
                trash.restore(&one.id).unwrap();
                assert_eq!(fs::read(&victim).unwrap(), b"victim bytes", "{point}");
            }
            more => panic!("{point}: {} items", more.len()),
        },
        _ => {
            for name in ["a", "b"] {
                let path = world.root.join(name);
                assert!(!path.exists(), "{point}: {name} is back in place");
            }
            assert!(items.len() <= 2, "{point}");
        }
    }
}

fn run(scenario: &str, points: &[&str], prepare: impl Fn(&World), check: impl Fn(&World, &str)) {
    for &point in points {
        let world = World::new();
        prepare(&world);
        kill_at(&world, scenario, point);
        check(&world, point);
    }
}

fn nothing(_: &World) {}

#[test]
fn a_copy_killed_at_any_point_finishes_or_is_undone() {
    run(
        "copy",
        &[
            "transfer.journaled",
            "transfer.chunk.written",
            "transfer.chunk.written@2",
            "transfer.chunk.written@7",
            "transfer.chunk.synced",
            "transfer.chunk.synced@3",
            "transfer.chunk.synced@5",
            "transfer.progress.journaled",
            "transfer.progress.journaled@4",
            "transfer.file.verified",
            "transfer.file.verified@5",
            "transfer.committing",
            "transfer.commit.renamed",
        ],
        nothing,
        |w, p| check_transfer(w, "copy", p),
    );
}

#[test]
fn a_move_across_disks_killed_at_any_point_finishes_or_is_undone() {
    run(
        "move",
        &[
            "transfer.journaled",
            "transfer.chunk.synced@2",
            "transfer.file.verified@3",
            "transfer.committing",
            "transfer.commit.renamed",
            "transfer.move.trashed",
            "trash.put.recorded",
            "trash.put.moved",
        ],
        nothing,
        |w, p| check_transfer(w, "move", p),
    );
}

#[test]
fn a_move_on_one_disk_killed_at_any_point_finishes_or_is_undone() {
    run(
        "move_same_disk",
        &["transfer.journaled", "transfer.committing", "transfer.commit.renamed"],
        nothing,
        |w, p| check_transfer(w, "move_same_disk", p),
    );
}

#[test]
fn a_cancel_killed_at_any_point_ends_cancelled() {
    run("cancel", &["transfer.cancelling"], nothing, check_cancel);
}


#[test]
fn an_upload_killed_at_any_point_can_be_resumed_or_is_done() {
    run(
        "upload",
        &[
            "transfer.journaled",
            "transfer.upload.written",
            "transfer.upload.written@3",
            "transfer.upload.synced",
            "transfer.upload.synced@2",
            "transfer.file.verified",
            "transfer.committing",
            "transfer.commit.renamed",
        ],
        nothing,
        check_upload,
    );
}

#[test]
fn the_trash_killed_at_any_point_keeps_or_loses_whole_items() {
    run(
        "trash_put",
        &["trash.put.recorded", "trash.put.moved"],
        nothing,
        |w, p| check_trash(w, "trash_put", p),
    );
    let two = |w: &World| {
        let trash = w.trash();
        fs::write(w.root.join("a"), b"aaa").unwrap();
        fs::write(w.root.join("b"), b"bbb").unwrap();
        trash.put(&w.root.join("a"), new_item()).unwrap();
        trash.put(&w.root.join("b"), new_item()).unwrap();
    };
    run(
        "trash_empty",
        &["trash.remove.marked", "trash.remove.marked@2"],
        two,
        |w, p| check_trash(w, "trash_empty", p),
    );
    let one = |w: &World| {
        let trash = w.trash();
        trash.put(&w.root.join("victim"), new_item()).unwrap();
    };
    run(
        "trash_restore",
        &["trash.restore.moved", "trash.remove.marked"],
        one,
        |w, p| check_trash(w, "trash_restore", p),
    );
}
