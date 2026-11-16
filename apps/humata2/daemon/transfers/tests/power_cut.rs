//! PER-27 and PER-28 under a power cut: copies, moves, cancels, uploads and
//! trash changes are cut at each fault point, and once right after the call
//! returned a final state, with every write that wasn't fsynced lost (the
//! simulation in `humata2_durable`, folders named by
//! `HUMATA2_POWER_CUT_ROOTS`). Then the trash and the transfers are opened
//! again and the outcome must be finished (the destination identical to
//! the source, a moved source gone and in the trash) or undone (the
//! destination as it was, the source intact). Never a half file, never a
//! staging folder once a transfer is final.
//!
//! `kill_matrix.rs` kills at the same points but keeps unsynced writes;
//! this is the harder case. The process cut part way is this test binary
//! run again with `power_child` selected; that test does nothing otherwise.
//! Needs the crates built with `faults`.
//!
//! Not seen by the simulation, since `humata2_durable` has no wrapper for
//! them: symbolic links (`make_link`) and removing folders (`remove_tree`,
//! always treated as already on disk). A link lost to a power cut is
//! therefore not caught here.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, symlink};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use sha2::{Digest, Sha256};

use humata2_transfers::{
    Config, Disk, End, EndError, Entry, Item, Mode, NewItem, OnExisting, Request, Sink, State,
    Transfers, Trash, UploadRequest,
};

const CHILD: &str = "HUMATA2_TRANSFERS_POWER_CHILD";
const ROOT: &str = "HUMATA2_TRANSFERS_POWER_ROOT";
const AFTER: &str = "HUMATA2_TRANSFERS_POWER_AFTER";
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

fn sha_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Every name under `root`, with its kind and, for a file, its mode,
/// length and SHA-256.
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
            out.insert(
                rel,
                format!("file {:o} {} {}", meta.mode() & 0o7777, content.len(), sha_hex(&content)),
            );
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
    /// `<dir>/w` is the simulated folder; the power cut's spare folder,
    /// `<dir>/spare`, is beside it on the same file system.
    fn new() -> Self {
        let dir = tempfile::Builder::new().prefix("h2p").tempdir_in("/tmp").unwrap();
        let root = dir.path().join("w");
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

    fn spare(&self) -> PathBuf {
        self.root.with_file_name("spare")
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

/// The source tree as `World::new` makes it.
fn expected_tree() -> &'static BTreeMap<String, String> {
    static EXPECTED: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    EXPECTED.get_or_init(|| tree(&World::new().root.join("src/tree")))
}

fn item(path: &Path) -> Item {
    Item { path: path.to_owned(), shown: path.display().to_string() }
}

fn copy_request(world: &World, mode: Mode, on_existing: OnExisting) -> Request {
    Request {
        id: "t".to_owned(),
        sources: vec![item(&world.root.join("src/tree"))],
        destination: item(&world.root.join("dst")),
        mode,
        on_existing,
        caller_data: String::new(),
        now_unix_ms: 1000,
    }
}

fn upload_data() -> Vec<u8> {
    bytes(UPLOAD, 7)
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

fn new_item() -> NewItem {
    NewItem { description: "t".to_owned(), caller_data: String::new(), now_unix_ms: 1 }
}

/// The process that gets cut. Runs only when a test below starts it.
#[test]
fn power_child() {
    let Ok(scenario) = std::env::var(CHILD) else { return };
    let world = World::at(PathBuf::from(std::env::var_os(ROOT).unwrap()));
    let trash = world.trash();
    match scenario.as_str() {
        "copy" | "move" | "move_same_disk" | "replace" => {
            let cross = scenario != "move_same_disk";
            let mode = if scenario == "copy" || scenario == "replace" { Mode::Copy } else { Mode::Move };
            let on_existing = if scenario == "replace" { OnExisting::Replace } else { OnExisting::Fail };
            let t = world.transfers(&trash, cross, false);
            t.start(copy_request(&world, mode, on_existing)).unwrap();
            t.wait("t").unwrap();
        }
        "cancel" => {
            let t = world.transfers(&trash, true, true);
            t.start(copy_request(&world, Mode::Copy, OnExisting::Fail)).unwrap();
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
        "trash_restore" | "trash_restore_deep" => {
            let id = trash.list().unwrap().remove(0).id;
            trash.restore(&id).unwrap();
        }
        "trash_empty" => {
            let plan = trash.plan_empty().unwrap();
            trash.empty(&plan).unwrap();
        }
        other => panic!("unknown scenario {other}"),
    }
    if std::env::var_os(AFTER).is_some() {
        // The call returned: what it reported finished must be on disk.
        humata2_durable::cut("after the scenario returned");
    }
}

/// Runs `scenario` in a child under the power-cut simulation and cuts it:
/// at the fault point `stop_at`, or, with none, right after the scenario
/// returns.
fn cut_at(world: &World, scenario: &str, stop_at: Option<&str>) {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["power_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, scenario)
        .env(ROOT, &world.root)
        .env("HUMATA2_POWER_CUT_ROOTS", &world.root)
        .env("HUMATA2_POWER_CUT_SPARE", world.spare())
        .env_remove("TESTBRIDGE_TEST_ONLY");
    match stop_at {
        Some(point) => command.env("HUMATA2_STOP_AT", point).env_remove(AFTER),
        None => command.env(AFTER, "1").env_remove("HUMATA2_STOP_AT"),
    };
    let status = command.status().unwrap();
    assert_eq!(
        status.signal(),
        Some(9),
        "{scenario} at {stop_at:?}: the child didn't stop at the cut (never reached the point, or the simulation couldn't undo a write): {status}"
    );
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

/// Whether any trash item folder holds `content`.
fn trash_has_content(world: &World) -> bool {
    names(&world.root.join("trash"))
        .iter()
        .any(|name| world.root.join("trash").join(name).join("content").exists())
}

fn deep_victim(world: &World) -> PathBuf {
    world.root.join("gone/inner/f")
}

/// What the disk holds right after the cut, before anything recovers: a
/// rename is whole or not there, so these hold at every cut.
fn check_raw(world: &World, scenario: &str, point: &str) {
    let src = world.root.join("src/tree");
    let landed = world.root.join("dst/tree");
    let expected = expected_tree();
    match scenario {
        "copy" | "move" => {
            if landed.exists() {
                assert_eq!(&tree(&landed), expected, "{point}: a half tree is visible");
            }
            if src.exists() {
                assert_eq!(&tree(&src), expected, "{point}: the source is half there");
            } else {
                assert_eq!(scenario, "move", "{point}: the source of a copy is gone");
            }
        }
        "move_same_disk" => {
            assert!(src.exists() != landed.exists(), "{point}: the name is in both places or neither");
            let there = if src.exists() { &src } else { &landed };
            assert_eq!(&tree(there), expected, "{point}: not whole");
        }
        "upload" => {
            let target = world.root.join("dst/up.bin");
            if target.exists() {
                assert_eq!(fs::read(&target).unwrap(), upload_data(), "{point}: a half file is visible");
            }
        }
        "trash_put" | "trash_restore" => {
            let victim = world.root.join("victim");
            assert!(
                !(victim.exists() && trash_has_content(world)),
                "{point}: both in the trash and in place"
            );
        }
        "trash_restore_deep" => {
            assert!(
                !(deep_victim(world).exists() && trash_has_content(world)),
                "{point}: both in the trash and in place"
            );
        }
        _ => {}
    }
}

/// Copies, moves and replacing copies: finished or undone.
fn check_transfer(world: &World, scenario: &str, point: &str, after: bool) {
    let trash = world.trash();
    let cross = scenario != "move_same_disk";
    let t = world.transfers(&trash, cross, false);
    let moving = scenario == "move" || scenario == "move_same_disk";
    let replacing = scenario == "replace";
    let src = world.root.join("src/tree");
    let dst = world.root.join("dst");
    let expected = expected_tree();
    match t.wait("t") {
        None => {
            assert!(!after, "{point}: the finished transfer's journal is gone");
            if replacing {
                assert_eq!(names(&dst), ["keep.txt", "tree"], "{point}: the destination changed");
                assert_eq!(fs::read(dst.join("tree/old.txt")).unwrap(), b"old", "{point}");
            } else {
                assert_eq!(names(&dst), ["keep.txt"], "{point}: the destination changed");
            }
            assert_eq!(&tree(&src), expected, "{point}: the source changed");
        }
        Some(snap) if snap.state == State::Done => {
            assert_eq!(&tree(&dst.join("tree")), expected, "{point}: not identical");
            assert_eq!(fs::read(dst.join("keep.txt")).unwrap(), b"keep", "{point}");
            let items = trash.list().unwrap();
            if moving {
                assert!(!src.exists(), "{point}: the source is still there");
                if cross {
                    assert_eq!(items.len(), 1, "{point}: the source isn't in trash");
                    assert_eq!(items[0].original, src, "{point}");
                    let kept = world.root.join("trash").join(&items[0].id).join("content");
                    assert_eq!(&tree(&kept), expected, "{point}: the trashed source is half there");
                } else {
                    assert!(items.is_empty(), "{point}: a same-disk move trashed something");
                }
            } else {
                assert_eq!(&tree(&src), expected, "{point}: the source changed");
            }
            if replacing {
                assert_eq!(items.len(), 1, "{point}: the replaced folder isn't in trash");
                assert_eq!(items[0].original, dst.join("tree"), "{point}");
                let kept = world.root.join("trash").join(&items[0].id).join("content");
                assert_eq!(fs::read(kept.join("old.txt")).unwrap(), b"old", "{point}");
                assert_eq!(names(&kept), ["old.txt"], "{point}");
            }
        }
        Some(snap) => panic!("{point}: ended {:?} {:?}", snap.state, snap.problem),
    }
    check_clean(world, point);
}

fn check_cancel(world: &World, point: &str, after: bool) {
    let trash = world.trash();
    let t = world.transfers(&trash, true, false);
    match t.wait("t") {
        Some(snap) => assert_eq!(snap.state, State::Cancelled, "{point}: {:?}", snap.problem),
        None => assert!(!after, "{point}: the cancelled transfer's journal is gone"),
    }
    assert_eq!(names(&world.root.join("dst")), ["keep.txt"], "{point}");
    assert_eq!(&tree(&world.root.join("src/tree")), expected_tree(), "{point}: the source changed");
    check_clean(world, point);
}

fn check_upload(world: &World, point: &str, after: bool) {
    let trash = world.trash();
    let t = world.transfers(&trash, true, false);
    let data = upload_data();
    let dst = world.root.join("dst");
    let target = dst.join("up.bin");
    let staged = dst.join(".humata-transfer-t/up.bin");
    match t.wait("t") {
        None => {
            assert!(!after, "{point}: the finished upload's journal is gone");
            assert_eq!(names(&dst), ["keep.txt"], "{point}");
        }
        Some(snap) if snap.state == State::Done => {
            assert_eq!(fs::read(&target).unwrap(), data, "{point}");
        }
        Some(snap) => {
            assert!(!after, "{point}: reported finished, but ended {:?}", snap.state);
            assert_eq!(snap.state, State::Paused, "{point}: {:?}", snap.problem);
            assert!(!target.exists(), "{point}: a half file is visible");
            // The bytes the journal counts as received must be on disk.
            let confirmed = usize::try_from(snap.done_bytes).unwrap();
            let have = fs::read(&staged).unwrap_or_default();
            assert!(have.len() >= confirmed, "{point}: {confirmed} bytes confirmed, {} on disk", have.len());
            assert_eq!(&have[..confirmed], &data[..confirmed], "{point}: confirmed bytes differ");
            let mut up = t.begin_upload(upload_request(world, Some("t"))).unwrap();
            let at = up.offset();
            assert_eq!(at, snap.done_bytes, "{point}: the resume offset isn't the confirmed one");
            up.write(at, &data[confirmed..]).unwrap();
            let done = up.finish().unwrap();
            assert_eq!(done.state, State::Done, "{point}");
            assert_eq!(fs::read(&target).unwrap(), data, "{point}");
        }
    }
    check_clean(world, point);
}

fn check_trash(world: &World, scenario: &str, point: &str, after: bool) {
    let trash = world.trash();
    let items = trash.list().unwrap();
    assert_eq!(names(&world.root.join("trash")).len(), items.len(), "{point}: stray entries in the trash");
    let (victim, content): (PathBuf, &[u8]) = match scenario {
        "trash_restore_deep" => (deep_victim(world), &b"deep bytes"[..]),
        _ => (world.root.join("victim"), &b"victim bytes"[..]),
    };
    match scenario {
        "trash_put" | "trash_restore" | "trash_restore_deep" => match items.as_slice() {
            [] => {
                assert!(
                    scenario != "trash_put" || !after,
                    "{point}: the trashed item is gone from the trash"
                );
                assert_eq!(fs::read(&victim).ok().as_deref(), Some(content), "{point}: neither in place nor in trash");
            }
            [one] => {
                assert!(
                    !after || scenario == "trash_put",
                    "{point}: the restored item is back in the trash"
                );
                assert!(!victim.exists(), "{point}: in trash and in place");
                let kept = world.root.join("trash").join(&one.id).join("content");
                if scenario == "trash_put" {
                    assert_eq!(fs::read(&kept).unwrap(), content, "{point}: the trashed bytes differ");
                }
                trash.restore(&one.id).unwrap();
                assert_eq!(fs::read(&victim).unwrap(), content, "{point}");
            }
            more => panic!("{point}: {} items", more.len()),
        },
        _ => {
            for (name, bytes) in [("a", b"aaa"), ("b", b"bbb")] {
                assert!(!world.root.join(name).exists(), "{point}: {name} is back in place");
                let original = world.root.join(name);
                for found in items.iter().filter(|i| i.original == original) {
                    let kept = world.root.join("trash").join(&found.id).join("content");
                    assert_eq!(fs::read(kept).unwrap(), bytes, "{point}: half an item");
                }
            }
            assert!(items.len() <= 2, "{point}");
            assert!(!after || items.is_empty(), "{point}: emptied items are back");
        }
    }
}

fn check(world: &World, scenario: &str, point: &str, after: bool) {
    match scenario {
        "copy" | "move" | "move_same_disk" | "replace" => check_transfer(world, scenario, point, after),
        "cancel" => check_cancel(world, point, after),
        "upload" => check_upload(world, point, after),
        _ => check_trash(world, scenario, point, after),
    }
}

/// Cuts `scenario` at each of `points`, and once after it returns, and
/// checks what recovery leaves.
fn matrix(scenario: &str, points: &[&str], prepare: impl Fn(&World)) {
    let stops = points.iter().map(|point| Some(*point)).chain([None]);
    for stop in stops {
        let world = World::new();
        prepare(&world);
        cut_at(&world, scenario, stop);
        let label = stop.unwrap_or("after the call returned");
        check_raw(&world, scenario, label);
        check(&world, scenario, label, stop.is_none());
    }
}

fn nothing(_: &World) {}

#[test]
fn a_copy_cut_at_any_point_finishes_or_is_undone() {
    matrix(
        "copy",
        &[
            "transfer.journaled",
            "transfer.chunk.written",
            "transfer.chunk.written@2",
            "transfer.chunk.written@8",
            "transfer.chunk.synced",
            "transfer.chunk.synced@3",
            "transfer.chunk.synced@5",
            "transfer.progress.journaled",
            "transfer.progress.journaled@4",
            "transfer.progress.journaled@5",
            "transfer.file.verified",
            "transfer.file.verified@3",
            "transfer.file.verified@5",
            "transfer.committing",
            "transfer.commit.renamed",
        ],
        nothing,
    );
}

#[test]
fn a_copy_that_replaces_cut_at_any_point_finishes_or_is_undone() {
    matrix(
        "replace",
        &[
            "transfer.chunk.synced@2",
            "transfer.committing",
            "trash.put.recorded",
            "trash.put.moved",
            "transfer.commit.renamed",
        ],
        |w| {
            fs::create_dir_all(w.root.join("dst/tree")).unwrap();
            fs::write(w.root.join("dst/tree/old.txt"), b"old").unwrap();
        },
    );
}

#[test]
fn a_move_across_disks_cut_at_any_point_finishes_or_is_undone() {
    matrix(
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
    );
}

#[test]
fn a_move_on_one_disk_cut_at_any_point_finishes_or_is_undone() {
    matrix(
        "move_same_disk",
        &["transfer.journaled", "transfer.committing", "transfer.commit.renamed"],
        nothing,
    );
}

#[test]
fn a_cancel_cut_at_any_point_ends_cancelled() {
    matrix("cancel", &["transfer.cancelling"], nothing);
}

#[test]
fn an_upload_cut_at_any_point_can_be_resumed_or_is_done() {
    matrix(
        "upload",
        &[
            "transfer.journaled",
            "transfer.upload.written",
            "transfer.upload.written@2",
            "transfer.upload.written@3",
            "transfer.upload.synced",
            "transfer.upload.synced@2",
            "transfer.upload.synced@3",
            "transfer.file.verified",
            "transfer.committing",
            "transfer.commit.renamed",
        ],
        nothing,
    );
}

#[test]
fn trashing_cut_at_any_point_keeps_or_loses_whole_items() {
    matrix("trash_put", &["trash.put.recorded", "trash.put.moved"], nothing);
}

#[test]
fn restoring_cut_at_any_point_keeps_or_loses_whole_items() {
    let one = |w: &World| {
        w.trash().put(&w.root.join("victim"), new_item()).unwrap();
    };
    matrix("trash_restore", &["trash.restore.moved", "trash.remove.marked"], one);
}

#[test]
fn restoring_into_folders_that_are_gone_cut_at_any_point_keeps_the_file() {
    let deep = |w: &World| {
        fs::create_dir_all(w.root.join("gone/inner")).unwrap();
        fs::write(deep_victim(w), b"deep bytes").unwrap();
        w.trash().put(&deep_victim(w), new_item()).unwrap();
        fs::remove_dir_all(w.root.join("gone")).unwrap();
    };
    matrix("trash_restore_deep", &["trash.restore.moved", "trash.remove.marked"], deep);
}

#[test]
fn emptying_cut_at_any_point_keeps_or_loses_whole_items() {
    let two = |w: &World| {
        let trash = w.trash();
        fs::write(w.root.join("a"), b"aaa").unwrap();
        fs::write(w.root.join("b"), b"bbb").unwrap();
        trash.put(&w.root.join("a"), new_item()).unwrap();
        trash.put(&w.root.join("b"), new_item()).unwrap();
    };
    matrix("trash_empty", &["trash.remove.marked", "trash.remove.marked@2"], two);
}
