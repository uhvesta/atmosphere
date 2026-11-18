//! Copies, moves, pauses, cancels and uploads on this machine's disk.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use humata2_transfers::{
    Disk, End, EndError, Entry, Item, Landing, Mode, OnExisting, Problem, Request, Sink, State, Transfers,
    Trash, TransferError, UploadRequest, Config,
};
use sha2::{Digest, Sha256};

struct World {
    dir: tempfile::TempDir,
    trash: Arc<Trash>,
    transfers: Arc<Transfers>,
}

/// A disk that says it is another one, reads slowly, and can't rename,
/// when asked.
struct Wrapped {
    inner: Disk,
    other_disk: bool,
    slow: bool,
    no_rename: bool,
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
        if self.no_rename {
            return Err(EndError::Io {
                doing: "move",
                path: from.to_owned(),
                source: std::io::Error::other("renames are refused here"),
            });
        }
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

struct Options {
    other_disk: bool,
    slow: bool,
    floor: u64,
    no_rename: bool,
}

fn world(options: Options) -> World {
    let dir = tempfile::tempdir().unwrap();
    let trash = Arc::new(Trash::open(dir.path().join("trash")).unwrap());
    let end = |slow| -> Arc<dyn End> {
        Arc::new(Wrapped {
            inner: Disk::new("local", trash.clone()),
            other_disk: options.other_disk,
            slow,
            no_rename: options.no_rename,
        })
    };
    let transfers = Transfers::open(Config {
        journal: dir.path().join("journal"),
        source: end(options.slow),
        destination: end(false),
        free_floor_bytes: options.floor,
        sync_every_bytes: 1 << 20,
        listener: Arc::new(|_| {}),
    })
    .unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::create_dir_all(dir.path().join("dst")).unwrap();
    World { dir, trash, transfers }
}

fn plain() -> Options {
    Options { other_disk: true, slow: false, floor: 0, no_rename: false }
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

fn item(path: &Path) -> Item {
    Item { path: path.to_owned(), shown: path.display().to_string() }
}

fn request(id: &str, sources: &[&Path], to: &Path, mode: Mode, on_existing: OnExisting) -> Request {
    Request {
        id: id.to_owned(),
        sources: sources.iter().map(|p| item(p)).collect(),
        destination: item(to),
        mode,
        on_existing,
        caller_data: String::new(),
        now_unix_ms: 1000,
    }
}

/// Everything under `root`: kind, mode, and content or link target.
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
                format!("file {:o} {} {:?}", meta.mode() & 0o7777, content.len(), Sha256::digest(&content)),
            );
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn make_tree(root: &Path) {
    fs::create_dir_all(root.join("nested/deep")).unwrap();
    fs::create_dir_all(root.join("empty")).unwrap();
    fs::write(root.join("nested/a.txt"), b"alpha").unwrap();
    fs::write(root.join("nested/deep/b.bin"), bytes(1000, 1)).unwrap();
    fs::write(root.join("empty.txt"), b"").unwrap();
    fs::write(root.join("big.bin"), bytes(3 << 20, 2)).unwrap();
    fs::write(root.join("run.sh"), b"#!/bin/sh\n").unwrap();
    fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
    symlink("nested/a.txt", root.join("link")).unwrap();
}

fn staging_left(folder: &Path) -> bool {
    fs::read_dir(folder)
        .unwrap()
        .any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".humata-transfer-"))
}

#[test]
fn a_copied_tree_is_identical_with_modes_and_links_kept() {
    let w = world(plain());
    let src = w.dir.path().join("src/tree");
    make_tree(&src);
    let dst = w.dir.path().join("dst");
    w.transfers
        .start(request("t1", &[&src], &dst, Mode::Copy, OnExisting::Fail))
        .unwrap();
    let done = w.transfers.wait("t1").unwrap();
    assert_eq!(done.state, State::Done, "{:?}", done.problem);
    assert_eq!(tree(&src), tree(&dst.join("tree")));
    assert!(fs::symlink_metadata(dst.join("tree/link")).unwrap().is_symlink());
    assert!(!staging_left(&dst));
    assert_eq!(done.done_bytes, done.total_bytes);
}

#[test]
fn an_existing_name_fails_is_replaced_or_kept_beside() {
    let w = world(plain());
    let src = w.dir.path().join("src/a.txt");
    fs::write(&src, b"new").unwrap();
    let dst = w.dir.path().join("dst");
    fs::write(dst.join("a.txt"), b"old").unwrap();

    w.transfers.start(request("f", &[&src], &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    let failed = w.transfers.wait("f").unwrap();
    assert_eq!(failed.state, State::Failed);
    assert!(matches!(failed.problem, Some(Problem::Exists { .. })));
    assert_eq!(fs::read(dst.join("a.txt")).unwrap(), b"old");

    w.transfers.start(request("k", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    assert_eq!(w.transfers.wait("k").unwrap().state, State::Done);
    assert_eq!(fs::read(dst.join("a-2.txt")).unwrap(), b"new");
    assert_eq!(fs::read(dst.join("a.txt")).unwrap(), b"old");

    w.transfers.start(request("r", &[&src], &dst, Mode::Copy, OnExisting::Replace)).unwrap();
    assert_eq!(w.transfers.wait("r").unwrap().state, State::Done);
    assert_eq!(fs::read(dst.join("a.txt")).unwrap(), b"new");
    let trashed = w.trash.list().unwrap();
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].bytes, 3);
}

#[test]
fn keeping_both_twice_numbers_the_second_copy_two_then_three() {
    let w = world(plain());
    let src = w.dir.path().join("src/a.txt");
    fs::write(&src, b"new").unwrap();
    let dst = w.dir.path().join("dst");
    fs::write(dst.join("a.txt"), b"old").unwrap();

    w.transfers.start(request("k1", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    assert_eq!(w.transfers.wait("k1").unwrap().state, State::Done);
    w.transfers.start(request("k2", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    assert_eq!(w.transfers.wait("k2").unwrap().state, State::Done);

    assert_eq!(fs::read(dst.join("a.txt")).unwrap(), b"old");
    assert_eq!(fs::read(dst.join("a-2.txt")).unwrap(), b"new");
    assert_eq!(fs::read(dst.join("a-3.txt")).unwrap(), b"new");
}

#[test]
fn keeping_both_a_file_with_the_same_bytes_writes_nothing_and_says_it_was_there() {
    let w = world(plain());
    let src = w.dir.path().join("src/notes.md");
    fs::write(&src, b"same words").unwrap();
    let dst = w.dir.path().join("dst");
    fs::write(dst.join("notes.md"), b"same words").unwrap();

    w.transfers.start(request("k", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    let done = w.transfers.wait("k").unwrap();

    assert_eq!(done.state, State::Done);
    assert_eq!(done.landings, vec![Landing::AlreadyThere]);
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 1);
    assert_eq!(fs::read(dst.join("notes.md")).unwrap(), b"same words");
}

#[test]
fn keeping_both_a_file_with_other_bytes_of_the_same_size_is_saved_under_a_new_name() {
    let w = world(plain());
    let src = w.dir.path().join("src/notes.md");
    fs::write(&src, b"same words").unwrap();
    let dst = w.dir.path().join("dst");
    fs::write(dst.join("notes.md"), b"othr words").unwrap();

    w.transfers.start(request("k", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    let done = w.transfers.wait("k").unwrap();

    assert_eq!(done.state, State::Done);
    assert_eq!(done.landings, vec![Landing::SavedAs("notes-2.md".to_owned())]);
    assert_eq!(fs::read(dst.join("notes-2.md")).unwrap(), b"same words");
    assert_eq!(fs::read(dst.join("notes.md")).unwrap(), b"othr words");
}

#[test]
fn keeping_both_a_name_nobody_holds_lands_as_asked() {
    let w = world(plain());
    let src = w.dir.path().join("src/notes.md");
    fs::write(&src, b"words").unwrap();
    let dst = w.dir.path().join("dst");

    w.transfers.start(request("k", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    assert_eq!(w.transfers.wait("k").unwrap().landings, vec![Landing::AsAsked]);
}

#[test]
fn keeping_both_a_folder_is_already_there_only_when_every_file_matches() {
    let w = world(plain());
    let src = w.dir.path().join("src/tree");
    make_tree(&src);
    let dst = w.dir.path().join("dst");

    w.transfers.start(request("a", &[&src], &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    assert_eq!(w.transfers.wait("a").unwrap().state, State::Done);

    w.transfers.start(request("b", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    let same = w.transfers.wait("b").unwrap();
    assert_eq!(same.landings, vec![Landing::AlreadyThere]);
    assert!(!dst.join("tree-2").exists());

    let extra = dst.join("tree/extra.txt");
    fs::write(&extra, b"only here").unwrap();
    w.transfers.start(request("c", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    let differs = w.transfers.wait("c").unwrap();
    assert_eq!(differs.landings, vec![Landing::SavedAs("tree-2".to_owned())]);
    assert_eq!(tree(&dst.join("tree-2")), tree(&src));
}

#[test]
fn a_move_of_a_file_already_there_takes_it_from_the_source_and_writes_nothing() {
    let w = world(plain());
    let src = w.dir.path().join("src/notes.md");
    fs::write(&src, b"same words").unwrap();
    let dst = w.dir.path().join("dst");
    fs::write(dst.join("notes.md"), b"same words").unwrap();

    w.transfers.start(request("m", &[&src], &dst, Mode::Move, OnExisting::KeepBoth)).unwrap();
    let done = w.transfers.wait("m").unwrap();

    assert_eq!(done.landings, vec![Landing::AlreadyThere]);
    assert!(!src.exists());
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 1);
}

#[test]
fn a_move_across_disks_trashes_the_source_after_done() {
    let w = world(plain());
    let src = w.dir.path().join("src/tree");
    make_tree(&src);
    let before = tree(&src);
    let dst = w.dir.path().join("dst");
    w.transfers.start(request("m", &[&src], &dst, Mode::Move, OnExisting::Fail)).unwrap();
    assert_eq!(w.transfers.wait("m").unwrap().state, State::Done);
    assert!(!src.exists());
    assert_eq!(tree(&dst.join("tree")), before);
    let trashed = w.trash.list().unwrap();
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].original, src);
}

#[test]
fn a_move_on_one_disk_is_a_rename_with_nothing_in_trash() {
    let w = world(Options { other_disk: false, ..plain() });
    let src = w.dir.path().join("src/tree");
    make_tree(&src);
    let before = tree(&src);
    let dst = w.dir.path().join("dst");
    w.transfers.start(request("m", &[&src], &dst, Mode::Move, OnExisting::Fail)).unwrap();
    assert_eq!(w.transfers.wait("m").unwrap().state, State::Done);
    assert!(!src.exists());
    assert_eq!(tree(&dst.join("tree")), before);
    assert!(w.trash.list().unwrap().is_empty());
}

#[test]
fn pause_then_resume_completes() {
    let w = world(Options { slow: true, ..plain() });
    let src = w.dir.path().join("src/big.bin");
    fs::write(&src, bytes(4 << 20, 3)).unwrap();
    let dst = w.dir.path().join("dst");
    w.transfers.start(request("p", &[&src], &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    let paused = w.transfers.pause("p").unwrap();
    assert!(matches!(paused.state, State::Paused | State::Done), "{:?}", paused.state);
    if paused.state == State::Paused {
        assert!(!dst.join("big.bin").exists());
        w.transfers.resume("p").unwrap();
    }
    assert_eq!(w.transfers.wait("p").unwrap().state, State::Done);
    assert_eq!(fs::read(dst.join("big.bin")).unwrap(), bytes(4 << 20, 3));
    assert!(!staging_left(&dst));
}

#[test]
fn cancel_leaves_the_destination_as_before_and_the_sources_intact() {
    let w = world(Options { slow: true, ..plain() });
    let src = w.dir.path().join("src/big.bin");
    fs::write(&src, bytes(4 << 20, 4)).unwrap();
    let dst = w.dir.path().join("dst");
    w.transfers.start(request("c", &[&src], &dst, Mode::Move, OnExisting::Fail)).unwrap();
    let snap = w.transfers.cancel("c").unwrap();
    assert_eq!(snap.state, State::Cancelled);
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 0);
    assert_eq!(fs::read(&src).unwrap(), bytes(4 << 20, 4));
    assert!(w.trash.list().unwrap().is_empty());
}

#[test]
fn a_copy_that_would_leave_too_little_free_writes_nothing() {
    let w = world(Options { floor: u64::MAX / 2, ..plain() });
    let src = w.dir.path().join("src/a.txt");
    fs::write(&src, b"hello").unwrap();
    let dst = w.dir.path().join("dst");
    w.transfers.start(request("d", &[&src], &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    let snap = w.transfers.wait("d").unwrap();
    assert_eq!(snap.state, State::Failed);
    assert!(matches!(snap.problem, Some(Problem::DiskFull { .. })));
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 0);
}

#[test]
fn starting_the_same_id_twice_copies_once() {
    let w = world(plain());
    let src = w.dir.path().join("src/a.txt");
    fs::write(&src, b"hello").unwrap();
    let dst = w.dir.path().join("dst");
    let first = w.transfers.start(request("s", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    w.transfers.wait("s").unwrap();
    let again = w.transfers.start(request("s", &[&src], &dst, Mode::Copy, OnExisting::KeepBoth)).unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(w.transfers.wait("s").unwrap().state, State::Done);
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 1);
}

#[test]
fn a_source_given_a_new_name_lands_under_it_as_cp_a_b_does() {
    let w = world(plain());
    let src = w.dir.path().join("src/note.txt");
    fs::write(&src, b"note").unwrap();
    let dst = w.dir.path().join("dst");
    w.transfers
        .start_as(request("n", &[&src], &dst, Mode::Copy, OnExisting::Fail), Some("vm-b.txt".into()))
        .unwrap();
    let done = w.transfers.wait("n").unwrap();
    assert_eq!(done.state, State::Done, "{:?}", done.problem);
    assert_eq!(fs::read(dst.join("vm-b.txt")).unwrap(), b"note");
    assert!(!dst.join("note.txt").exists());
}

#[test]
fn a_new_name_for_two_sources_starts_nothing() {
    let w = world(plain());
    let (a, b) = (w.dir.path().join("src/a.txt"), w.dir.path().join("src/b.txt"));
    fs::write(&a, b"a").unwrap();
    fs::write(&b, b"b").unwrap();
    let dst = w.dir.path().join("dst");
    let refused = w
        .transfers
        .start_as(request("two", &[&a, &b], &dst, Mode::Copy, OnExisting::Fail), Some("one.txt".into()));
    assert!(matches!(refused, Err(TransferError::NameNeedsOneSource { .. })), "{refused:?}");
    assert!(fs::read_dir(&dst).unwrap().next().is_none());
}

#[test]
fn a_new_name_that_is_a_path_starts_nothing() {
    let w = world(plain());
    let src = w.dir.path().join("src/a.txt");
    fs::write(&src, b"a").unwrap();
    let dst = w.dir.path().join("dst");
    for name in ["../a.txt", "x/y", "..", "."] {
        let refused = w
            .transfers
            .start_as(request(&format!("p{}", name.len()), &[&src], &dst, Mode::Copy, OnExisting::Fail), Some(name.into()));
        assert!(matches!(refused, Err(TransferError::BadName { .. })), "{name}: {refused:?}");
    }
    assert!(fs::read_dir(&dst).unwrap().next().is_none());
}

fn upload_request(w: &World, id: &str, data: &[u8], sha: String) -> UploadRequest {
    UploadRequest {
        id: id.to_owned(),
        resume: None,
        destination: item(&w.dir.path().join("dst")),
        name: "up.bin".to_owned(),
        size: data.len() as u64,
        sha256: sha,
        on_existing: OnExisting::Fail,
        caller_data: String::new(),
        now_unix_ms: 5,
    }
}

fn sha_hex(data: &[u8]) -> String {
    Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn an_upload_resumes_from_its_confirmed_offset() {
    let w = world(plain());
    let data = bytes(3 << 20, 9);
    let request = upload_request(&w, "u", &data, sha_hex(&data));
    let mut up = w.transfers.begin_upload(request.clone()).unwrap();
    up.write(0, &data[..1 << 20]).unwrap();
    assert!(matches!(
        up.write(5, b"x"),
        Err(TransferError::WrongOffset { expected, .. }) if expected == 1 << 20
    ));
    drop(up);
    let paused = w.transfers.get("u").unwrap();
    assert_eq!(paused.state, State::Paused);
    assert_eq!(paused.problem, Some(Problem::Interrupted));
    assert!(!w.dir.path().join("dst/up.bin").exists());

    let mut up = w
        .transfers
        .begin_upload(UploadRequest { resume: Some("u".to_owned()), ..request })
        .unwrap();
    let at = up.offset();
    assert_eq!(at, 1 << 20);
    up.write(at, &data[at as usize..]).unwrap();
    let done = up.finish().unwrap();
    assert_eq!(done.state, State::Done);
    assert_eq!(fs::read(w.dir.path().join("dst/up.bin")).unwrap(), data);
    assert!(!staging_left(&w.dir.path().join("dst")));
}

#[test]
fn an_upload_with_the_wrong_checksum_is_refused_and_nothing_appears() {
    let w = world(plain());
    let data = bytes(1000, 1);
    let mut up = w.transfers.begin_upload(upload_request(&w, "bad", &data, sha_hex(b"other"))).unwrap();
    up.write(0, &data).unwrap();
    let error = up.finish().unwrap_err();
    assert!(matches!(
        error,
        TransferError::Refused { problem: Problem::ChecksumMismatch { .. }, .. }
    ));
    let dst = w.dir.path().join("dst");
    assert_eq!(fs::read_dir(&dst).unwrap().count(), 0);
    assert_eq!(w.transfers.get("bad").unwrap().state, State::Failed);
}

#[test]
fn a_finished_transfer_is_still_listed_after_reopening() {
    let w = world(plain());
    let src = w.dir.path().join("src/a.txt");
    fs::write(&src, b"hello").unwrap();
    let dst = w.dir.path().join("dst");
    w.transfers.start(request("l", &[&src], &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    w.transfers.wait("l").unwrap();
    let again = Transfers::open(Config {
        journal: w.dir.path().join("journal"),
        source: Arc::new(Disk::new("local", w.trash.clone())),
        destination: Arc::new(Disk::new("local", w.trash.clone())),
        free_floor_bytes: 0,
        sync_every_bytes: 1 << 20,
        listener: Arc::new(|_| {}),
    })
    .unwrap();
    assert_eq!(again.list().len(), 1);
    assert_eq!(again.get("l").unwrap().state, State::Done);
}

/// A umask other than the usual 022: anything made without its own mode
/// shows it (a 0666 default comes out 0664 here).
fn umask_002() {
    static SET: std::sync::Once = std::sync::Once::new();
    SET.call_once(|| {
        rustix::process::umask(rustix::fs::Mode::from_raw_mode(0o002));
    });
}

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn a_copy_keeps_each_files_exact_mode_under_another_umask() {
    umask_002();
    let w = world(plain());
    let src = w.dir.path().join("src/modes");
    fs::create_dir_all(&src).unwrap();
    let modes = [("run.sh", 0o755), ("private.key", 0o600), ("note.txt", 0o644), ("group.txt", 0o640)];
    for (name, mode) in modes {
        fs::write(src.join(name), bytes(100, 3)).unwrap();
        fs::set_permissions(src.join(name), fs::Permissions::from_mode(mode)).unwrap();
    }
    let dst = w.dir.path().join("dst");
    w.transfers.start(request("c", &[&src], &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    assert_eq!(w.transfers.wait("c").unwrap().state, State::Done);
    for (name, mode) in modes {
        assert_eq!(mode_of(&dst.join("modes").join(name)), mode, "{name} after copy");
    }
}

#[test]
fn a_moved_file_keeps_its_mode_at_the_destination_and_after_restore_from_trash() {
    umask_002();
    for mode in [0o644, 0o600, 0o755] {
        let w = world(plain());
        let src = w.dir.path().join(format!("src/file-{mode:o}"));
        fs::create_dir_all(src.parent().unwrap()).unwrap();
        fs::write(&src, bytes(500, 4)).unwrap();
        fs::set_permissions(&src, fs::Permissions::from_mode(mode)).unwrap();
        let dst = w.dir.path().join("dst");
        w.transfers.start(request("m", &[&src], &dst, Mode::Move, OnExisting::Fail)).unwrap();
        assert_eq!(w.transfers.wait("m").unwrap().state, State::Done);
        assert_eq!(mode_of(&dst.join(format!("file-{mode:o}"))), mode, "destination");
        let trashed = w.trash.list().unwrap();
        assert_eq!(trashed.len(), 1);
        let back = w.trash.restore(&trashed[0].id).unwrap();
        assert_eq!(back, src);
        assert_eq!(mode_of(&src), mode, "after restore from the trash");
    }
}

/// Folders `tree` (0750) holding `ro` (0555, with a file and a 0555
/// folder that holds one) and `private` (0700), and `locked` (0555, with a
/// file): read-only folders at every depth, the tops included.
fn make_moded_tree(src: &Path) -> Vec<(&'static str, u32)> {
    let modes = vec![
        ("tree", 0o750),
        ("tree/ro", 0o555),
        ("tree/ro/sub", 0o555),
        ("tree/ro/inside.txt", 0o644),
        ("tree/ro/sub/deep.txt", 0o600),
        ("tree/private", 0o700),
        ("locked", 0o555),
        ("locked/note.txt", 0o640),
    ];
    for (rel, _) in &modes {
        if rel.ends_with(".txt") {
            fs::write(src.join(rel), bytes(64, 5)).unwrap();
        } else {
            fs::create_dir_all(src.join(rel)).unwrap();
        }
    }
    // Deepest first, so each read-only folder is set once it is filled.
    for (rel, mode) in modes.iter().rev() {
        fs::set_permissions(src.join(rel), fs::Permissions::from_mode(*mode)).unwrap();
    }
    modes
}

/// Lets the temporary folder go: read-only folders under `root` are made
/// writable again.
fn unlock(root: &Path) {
    let Ok(meta) = fs::symlink_metadata(root) else { return };
    if meta.is_dir() {
        let _ = fs::set_permissions(root, fs::Permissions::from_mode(0o700));
        for entry in fs::read_dir(root).unwrap().flatten() {
            unlock(&entry.path());
        }
    }
}

#[test]
fn a_copy_keeps_each_folders_exact_mode_under_another_umask() {
    umask_002();
    let w = world(plain());
    let src = w.dir.path().join("src");
    let modes = make_moded_tree(&src);
    let dst = w.dir.path().join("dst");
    let sources = [src.join("tree"), src.join("locked")];
    let sources: Vec<&Path> = sources.iter().map(|p| p.as_path()).collect();
    w.transfers.start(request("c", &sources, &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    let done = w.transfers.wait("c").unwrap();
    let arrived: Vec<(&str, u32)> = modes.iter().map(|(rel, _)| (*rel, mode_of(&dst.join(rel)))).collect();
    let found_staging = staging_left(&dst);
    unlock(w.dir.path());
    assert_eq!(done.state, State::Done, "{:?}", done.problem);
    assert_eq!(arrived, modes);
    assert!(!found_staging);
}

#[test]
fn a_failed_copy_with_read_only_folders_staged_is_cancelled_without_a_trace() {
    umask_002();
    let w = world(Options { no_rename: true, ..plain() });
    let src = w.dir.path().join("src");
    make_moded_tree(&src);
    let dst = w.dir.path().join("dst");
    let sources = [src.join("tree"), src.join("locked")];
    let sources: Vec<&Path> = sources.iter().map(|p| p.as_path()).collect();
    w.transfers.start(request("f", &sources, &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    let failed = w.transfers.wait("f").unwrap();
    let cancelled = w.transfers.cancel("f").unwrap();
    let left: Vec<String> = fs::read_dir(&dst)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    unlock(w.dir.path());
    assert_eq!(failed.state, State::Failed);
    assert_eq!(cancelled.state, State::Cancelled, "{:?}", cancelled.problem);
    assert!(left.is_empty(), "left in the destination: {left:?}");
}

#[test]
fn a_cancelled_copy_of_a_read_only_folder_leaves_no_staging() {
    umask_002();
    let w = world(Options { slow: true, ..plain() });
    let src = w.dir.path().join("src");
    fs::create_dir_all(src.join("ro")).unwrap();
    fs::write(src.join("ro/big.bin"), bytes(4 << 20, 6)).unwrap();
    fs::set_permissions(src.join("ro"), fs::Permissions::from_mode(0o555)).unwrap();
    let dst = w.dir.path().join("dst");
    w.transfers.start(request("x", &[&src.join("ro")], &dst, Mode::Copy, OnExisting::Fail)).unwrap();
    let cancelled = w.transfers.cancel("x").unwrap();
    let left = fs::read_dir(&dst).unwrap().count();
    unlock(w.dir.path());
    assert_eq!(cancelled.state, State::Cancelled, "{:?}", cancelled.problem);
    assert_eq!(left, 0);
}

#[test]
fn read_only_folders_moved_on_one_disk_or_across_keep_their_modes() {
    umask_002();
    for other_disk in [false, true] {
        let w = world(Options { other_disk, ..plain() });
        let src = w.dir.path().join("src");
        let modes = make_moded_tree(&src);
        let dst = w.dir.path().join("dst");
        let sources = [src.join("tree"), src.join("locked")];
        let sources: Vec<&Path> = sources.iter().map(|p| p.as_path()).collect();
        w.transfers.start(request("m", &sources, &dst, Mode::Move, OnExisting::Fail)).unwrap();
        let done = w.transfers.wait("m").unwrap();
        let arrived: Vec<(&str, u32)> = modes.iter().map(|(rel, _)| (*rel, mode_of(&dst.join(rel)))).collect();
        let gone = !src.join("tree").exists() && !src.join("locked").exists();
        let trashed = w.trash.list().unwrap().len();
        unlock(w.dir.path());
        assert_eq!(done.state, State::Done, "{:?} (other disk: {other_disk})", done.problem);
        assert_eq!(arrived, modes, "other disk: {other_disk}");
        assert!(gone, "the sources are still there (other disk: {other_disk})");
        assert_eq!(trashed, if other_disk { 2 } else { 0 });
    }
}
