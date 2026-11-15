use std::fs;

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

#[test]
fn a_damaged_entry_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.journal().append("write", b"twelve bytes").unwrap();
    let entry = fs::read_dir(dir.path().join("journal"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "entry"))
        .unwrap();
    let mut bytes = fs::read(&entry).unwrap();
    bytes.truncate(bytes.len() - 4);
    fs::write(&entry, bytes).unwrap();

    assert!(matches!(
        store.journal().entries(),
        Err(StoreError::JournalEntryDamaged { .. })
    ));
}
