//! The Humata trash: put, list, restore, empty, expire.

use std::fs;

use humata2_transfers::{KEEP_MS, NewItem, Trash, TrashError};

fn new(now: i64) -> NewItem {
    NewItem {
        description: "test".to_owned(),
        caller_data: "data".to_owned(),
        now_unix_ms: now,
    }
}

fn setup() -> (tempfile::TempDir, Trash) {
    let dir = tempfile::tempdir().unwrap();
    let trash = Trash::open(dir.path().join(".trash")).unwrap();
    (dir, trash)
}

#[test]
fn put_list_restore_brings_the_same_bytes_back() {
    let (dir, trash) = setup();
    let file = dir.path().join("sub/a.txt");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, b"hello").unwrap();
    let item = trash.put(&file, new(10)).unwrap();
    assert!(!file.exists());
    assert_eq!(item.bytes, 5);
    let listed = trash.list().unwrap();
    assert_eq!(listed, vec![item.clone()]);
    fs::remove_dir(dir.path().join("sub")).unwrap();
    let back = trash.restore(&item.id).unwrap();
    assert_eq!(back, file);
    assert_eq!(fs::read(&file).unwrap(), b"hello");
    assert!(trash.list().unwrap().is_empty());
}

#[test]
fn restore_refuses_when_the_path_is_taken_and_leaves_both() {
    let (dir, trash) = setup();
    let file = dir.path().join("a.txt");
    fs::write(&file, b"old").unwrap();
    let item = trash.put(&file, new(1)).unwrap();
    fs::write(&file, b"new").unwrap();
    let error = trash.restore(&item.id).unwrap_err();
    assert!(matches!(error, TrashError::Occupied { .. }));
    assert_eq!(fs::read(&file).unwrap(), b"new");
    assert_eq!(trash.list().unwrap().len(), 1);
}

#[test]
fn a_folder_goes_in_whole_and_comes_back_whole() {
    let (dir, trash) = setup();
    let folder = dir.path().join("f");
    fs::create_dir_all(folder.join("in")).unwrap();
    fs::write(folder.join("in/x"), b"12345678").unwrap();
    let item = trash.put(&folder, new(1)).unwrap();
    assert_eq!(item.bytes, 8);
    trash.restore(&item.id).unwrap();
    assert_eq!(fs::read(folder.join("in/x")).unwrap(), b"12345678");
}

#[test]
fn empty_frees_only_the_plans_items() {
    let (dir, trash) = setup();
    for (name, now) in [("a", 1), ("b", 2)] {
        let path = dir.path().join(name);
        fs::write(&path, b"abc").unwrap();
        trash.put(&path, new(now)).unwrap();
    }
    let plan = trash.plan_empty().unwrap();
    assert_eq!(plan.ids.len(), 2);
    assert_eq!(plan.bytes, 6);
    let late = dir.path().join("c");
    fs::write(&late, b"zz").unwrap();
    trash.put(&late, new(3)).unwrap();
    assert_eq!(trash.empty(&plan).unwrap(), 6);
    let left = trash.list().unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].original, late);
}

#[test]
fn expire_removes_only_items_thirty_days_old() {
    let (dir, trash) = setup();
    for (name, at) in [("old", 0), ("fresh", 1000)] {
        let path = dir.path().join(name);
        fs::write(&path, b"1").unwrap();
        trash.put(&path, new(at)).unwrap();
    }
    assert_eq!(trash.expire(KEEP_MS - 1).unwrap(), 0);
    assert_eq!(trash.list().unwrap().len(), 2);
    assert_eq!(trash.expire(KEEP_MS).unwrap(), 1);
    let left = trash.list().unwrap();
    assert_eq!(left.len(), 1);
    assert!(left[0].original.ends_with("fresh"));
}

#[test]
fn a_file_put_twice_gives_two_items() {
    let (dir, trash) = setup();
    let path = dir.path().join("a");
    fs::write(&path, b"1").unwrap();
    let first = trash.put(&path, new(1)).unwrap();
    fs::write(&path, b"2").unwrap();
    let second = trash.put(&path, new(2)).unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(trash.list().unwrap().len(), 2);
}

#[test]
fn putting_something_missing_is_an_error() {
    let (dir, trash) = setup();
    let error = trash.put(&dir.path().join("nope"), new(1)).unwrap_err();
    assert!(matches!(error, TrashError::Missing { .. }));
}

#[test]
fn open_removes_an_item_that_never_got_its_content() {
    let (dir, trash) = setup();
    drop(trash);
    let half = dir.path().join(".trash/abc");
    fs::create_dir_all(&half).unwrap();
    fs::write(half.join("item.json"), b"{}").unwrap();
    fs::create_dir_all(dir.path().join(".trash/def.deleting/content")).unwrap();
    let trash = Trash::open(dir.path().join(".trash")).unwrap();
    assert!(trash.list().unwrap().is_empty());
    assert!(!half.exists());
    assert!(!dir.path().join(".trash/def.deleting").exists());
}
