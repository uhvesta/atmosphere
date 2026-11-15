use std::fs;
use std::path::{Path, PathBuf};

use humata2_store::{Store, StoreError};

#[test]
fn entries_replay_in_the_order_they_were_written() {
    let dir = tempfile::tempdir().unwrap();
    let written: Vec<(String, Vec<u8>)> = (0..30)
        .map(|n| (format!("kind-{}", n % 3), format!("payload {n}\nline two").into_bytes()))
        .collect();
    {
        let store = Store::open(dir.path()).unwrap();
        for (kind, payload) in &written {
            store.journal().append(kind, payload).unwrap();
        }
    }

    let store = Store::open(dir.path()).unwrap();
    let replayed = store.journal().entries().unwrap();
    let seqs: Vec<u64> = replayed.iter().map(|entry| entry.seq).collect();
    assert_eq!(seqs, (1..=30).collect::<Vec<u64>>());
    let contents: Vec<(String, Vec<u8>)> = replayed
        .into_iter()
        .map(|entry| (entry.kind, entry.payload))
        .collect();
    assert_eq!(contents, written);
}

#[test]
fn acked_entries_are_gone_and_their_sequences_never_return() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        for n in 0..5u8 {
            store.journal().append("write", &[n]).unwrap();
        }
        store.journal().ack_through(3).unwrap();
        let left: Vec<u64> = store
            .journal()
            .entries()
            .unwrap()
            .iter()
            .map(|entry| entry.seq)
            .collect();
        assert_eq!(left, vec![4, 5]);
    }
    {
        let store = Store::open(dir.path()).unwrap();
        store.journal().ack_through(5).unwrap();
        assert!(store.journal().entries().unwrap().is_empty());
    }

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.journal().acked().unwrap(), 5);
    assert_eq!(store.journal().append("write", b"later").unwrap(), 6);
}

#[test]
fn an_older_ack_does_not_undo_a_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for _ in 0..4 {
        store.journal().append("write", b"x").unwrap();
    }
    store.journal().ack_through(3).unwrap();
    store.journal().ack_through(1).unwrap();
    assert_eq!(store.journal().acked().unwrap(), 3);
    assert_eq!(store.journal().entries().unwrap().len(), 1);
}

#[test]
fn an_empty_payload_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.journal().append("ping", b"").unwrap();
    let entries = store.journal().entries().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].payload.is_empty());
}

/// `len` bytes zstd can't shrink.
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

fn blob_files(root: &Path) -> Vec<PathBuf> {
    fs::read_dir(root.join("blobs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

#[test]
fn small_and_big_payloads_round_trip_and_big_ones_live_in_files() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let payloads = [
        b"short".to_vec(),
        vec![b'a'; 300 << 10],
        noise(1 << 10, 1),
        noise(300 << 10, 2),
    ];
    for payload in &payloads {
        store.journal().append("write", payload).unwrap();
    }

    let read: Vec<Vec<u8>> = store
        .journal()
        .entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.payload)
        .collect();
    assert_eq!(read, payloads);
    // Only the big payload zstd couldn't shrink outgrew its row.
    assert_eq!(blob_files(dir.path()).len(), 1);
}

#[test]
fn acking_an_entry_removes_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.journal().append("write", &noise(300 << 10, 3)).unwrap();
    store.journal().append("write", &noise(300 << 10, 4)).unwrap();

    store.journal().ack_through(1).unwrap();

    assert_eq!(blob_files(dir.path()).len(), 1);
    assert_eq!(store.journal().entries().unwrap()[0].payload, noise(300 << 10, 4));
}

#[test]
fn a_damaged_or_missing_payload_file_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.journal().append("write", &noise(300 << 10, 5)).unwrap();
    let [file] = blob_files(dir.path()).try_into().unwrap();

    let mut bytes = fs::read(&file).unwrap();
    bytes[100] ^= 1;
    fs::write(&file, bytes).unwrap();
    assert!(matches!(store.journal().entries(), Err(StoreError::Damaged { .. })));

    fs::remove_file(&file).unwrap();
    assert!(matches!(store.journal().entries(), Err(StoreError::Damaged { .. })));
}

#[test]
fn files_no_entry_names_are_removed_on_open() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.journal().append("write", &noise(300 << 10, 6)).unwrap();
    let [kept] = blob_files(dir.path()).try_into().unwrap();
    drop(store);
    let orphan = dir.path().join("blobs").join("ab".repeat(32));
    let partial = dir.path().join("blobs").join(".x.1-0.partial");
    let foreign = dir.path().join("blobs").join("notes.txt");
    for path in [&orphan, &partial, &foreign] {
        fs::write(path, b"left").unwrap();
    }

    let store = Store::open(dir.path()).unwrap();

    assert!(kept.exists());
    assert!(!orphan.exists());
    assert!(!partial.exists());
    assert!(foreign.exists(), "a file the store didn't write was removed");
    assert_eq!(store.journal().entries().unwrap().len(), 1);
}
