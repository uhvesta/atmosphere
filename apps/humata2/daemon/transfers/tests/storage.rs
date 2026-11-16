//! Measuring what Humata keeps, finding build caches, clearing them and
//! trimming logs.

use std::fs;
use std::path::Path;

use humata2_transfers::{build_caches, clear, disk_space, measure, sweep_cleared, trim_logs};

fn write(path: &Path, bytes: usize) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![b'x'; bytes]).unwrap();
}

fn found(folder: &Path) -> Vec<String> {
    let mut paths: Vec<String> = build_caches(folder)
        .unwrap()
        .into_iter()
        .map(|cache| cache.path.to_string_lossy().into_owned())
        .collect();
    paths.sort();
    paths
}

#[test]
fn a_hard_link_counts_once_and_a_symlink_is_not_followed() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("outside/big");
    write(&outside, 4 << 20);
    let tree = dir.path().join("tree");
    write(&tree.join("a"), 1 << 20);
    fs::hard_link(tree.join("a"), tree.join("b")).unwrap();
    std::os::unix::fs::symlink(&outside, tree.join("link")).unwrap();
    let measured = measure(&tree).unwrap();
    assert!(measured.bytes >= 1 << 20, "{}", measured.bytes);
    assert!(measured.bytes < 2 << 20, "{}", measured.bytes);
}

#[test]
fn a_missing_path_measures_as_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let measured = measure(&dir.path().join("none")).unwrap();
    assert_eq!(measured.bytes, 0);
    assert_eq!(measured.last_used_unix_ms, None);
}

#[test]
fn last_used_follows_the_newest_file() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("old"), 10);
    write(&dir.path().join("sub/new"), 10);
    let old = fs::File::options().write(true).open(dir.path().join("old")).unwrap();
    old.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000))
        .unwrap();
    let new = fs::File::options().write(true).open(dir.path().join("sub/new")).unwrap();
    new.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(5_000))
        .unwrap();
    assert_eq!(measure(dir.path()).unwrap().last_used_unix_ms, Some(5_000_000));
}

#[test]
fn a_disk_has_free_and_total_space() {
    let dir = tempfile::tempdir().unwrap();
    let space = disk_space(dir.path()).unwrap();
    assert!(space.total_bytes > 0);
    assert!(space.free_bytes <= space.total_bytes);
}

#[test]
fn caches_are_found_by_the_marker_beside_them() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(&root.join("rust/Cargo.toml"), 1);
    write(&root.join("rust/target/x"), 10);
    write(&root.join("web/package.json"), 1);
    write(&root.join("web/node_modules/x"), 10);
    write(&root.join("web/.next/x"), 10);
    write(&root.join("swift/Package.swift"), 1);
    write(&root.join("swift/.build/x"), 10);
    write(&root.join("jvm/build.gradle.kts"), 1);
    write(&root.join("jvm/build/x"), 10);
    write(&root.join("jvm/.gradle/x"), 10);
    write(&root.join("py/pyproject.toml"), 1);
    write(&root.join("py/.venv/x"), 10);
    write(&root.join("py/pkg/__pycache__/x"), 10);
    write(&root.join("tagged/out/CACHEDIR.TAG"), 1);
    assert_eq!(
        found(root),
        [
            "jvm/.gradle",
            "jvm/build",
            "py/.venv",
            "py/pkg/__pycache__",
            "rust/target",
            "swift/.build",
            "tagged/out",
            "web/.next",
            "web/node_modules",
        ]
    );
}

#[test]
fn a_folder_without_its_marker_is_not_a_cache() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a/target/x"), 10);
    write(&dir.path().join("b/build/x"), 10);
    write(&dir.path().join("c/.venv/x"), 10);
    assert!(build_caches(dir.path()).unwrap().is_empty());
}

#[test]
fn nothing_is_found_in_git_inside_a_cache_or_through_a_link() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(&root.join(".git/Cargo.toml"), 1);
    write(&root.join(".git/target/x"), 10);
    write(&root.join("web/package.json"), 1);
    write(&root.join("web/node_modules/dep/package.json"), 1);
    write(&root.join("web/node_modules/dep/node_modules/x"), 10);
    write(&root.join("elsewhere/Cargo.toml"), 1);
    write(&root.join("elsewhere/target/x"), 10);
    fs::create_dir(root.join("ws")).unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere"), root.join("ws/bazel-out")).unwrap();
    std::os::unix::fs::symlink(root.join("elsewhere/target"), root.join("ws/target")).unwrap();
    write(&root.join("ws/Cargo.toml"), 1);
    assert_eq!(found(root), ["elsewhere/target", "web/node_modules"]);
}

#[test]
fn caches_are_looked_for_only_four_levels_down() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a/b/c/Cargo.toml"), 1);
    write(&dir.path().join("a/b/c/target/x"), 10);
    write(&dir.path().join("a/b/c/d/Cargo.toml"), 1);
    write(&dir.path().join("a/b/c/d/target/x"), 10);
    assert_eq!(found(dir.path()), ["a/b/c/target"]);
}

#[test]
fn caches_come_largest_first() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("a/Cargo.toml"), 1);
    write(&dir.path().join("a/target/x"), 1 << 10);
    write(&dir.path().join("b/Cargo.toml"), 1);
    write(&dir.path().join("b/target/x"), 1 << 20);
    let caches = build_caches(dir.path()).unwrap();
    assert_eq!(caches[0].path, Path::new("b/target"));
    assert!(caches[0].bytes > caches[1].bytes);
}

#[test]
fn clearing_frees_what_measure_said_and_leaves_siblings() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("target/deep/x"), 1 << 20);
    write(&dir.path().join("keep/y"), 10);
    let said = measure(&dir.path().join("target")).unwrap().bytes;
    let freed = clear(&dir.path().join("target")).unwrap();
    assert_eq!(freed, said);
    assert!(!dir.path().join("target").exists());
    assert!(dir.path().join("keep/y").exists());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1, "no leftover beside it");
}

#[test]
fn clearing_a_link_is_refused_and_removes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join("real/x"), 10);
    std::os::unix::fs::symlink(dir.path().join("real"), dir.path().join("link")).unwrap();
    assert!(clear(&dir.path().join("link")).is_err());
    assert!(dir.path().join("real/x").exists());
    assert!(dir.path().join("link").exists());
}

#[test]
fn sweeping_removes_what_an_interrupted_clear_left() {
    let dir = tempfile::tempdir().unwrap();
    write(&dir.path().join(".humata-clearing-abc/x"), 10);
    write(&dir.path().join("keep/y"), 10);
    sweep_cleared(dir.path()).unwrap();
    assert!(!dir.path().join(".humata-clearing-abc").exists());
    assert!(dir.path().join("keep/y").exists());
}

#[test]
fn trimming_keeps_the_tail_from_a_line_start() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("humatad.log");
    let text: String = (0..100).map(|n| format!("line {n:03}\n")).collect();
    fs::write(&log, &text).unwrap();
    let before = text.len() as u64;
    let freed = trim_logs(dir.path(), 55).unwrap();
    let kept = fs::read_to_string(&log).unwrap();
    assert!(kept.len() <= 55);
    assert!(kept.starts_with("line "), "{kept:?}");
    assert!(kept.ends_with("line 099\n"));
    assert!(text.ends_with(&kept));
    assert_eq!(freed, before - kept.len() as u64);
}

#[test]
fn trimming_leaves_small_files_alone_and_reaches_one_level_down() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("small.log"), "a\nb\n").unwrap();
    let big: String = (0..1000).map(|n| format!("line {n:04}\n")).collect();
    write(&dir.path().join("holders/h.log"), 0);
    fs::write(dir.path().join("holders/h.log"), &big).unwrap();
    let freed = trim_logs(dir.path(), 100).unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("small.log")).unwrap(), "a\nb\n");
    assert!(fs::metadata(dir.path().join("holders/h.log")).unwrap().len() <= 100);
    assert!(freed >= big.len() as u64 - 100);
}
