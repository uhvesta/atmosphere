//! What a store keeps of keyed Type calls: a key is pending from before its
//! text goes to the shell until it is noted typed, across reopens, and the
//! keys older than the idempotency window can be dropped.

use humata2_store::{Store, TypedKey};

#[test]
fn a_key_is_pending_until_it_is_noted_typed_across_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let claimed = store.claim_typed_key("Type/a", 10).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    let pending = store.claim_typed_key("Type/a", 20).unwrap();
    store.finish_typed_key("Type/a").unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    let typed = store.claim_typed_key("Type/a", 30).unwrap();
    let other = store.claim_typed_key("Type/b", 30).unwrap();

    assert_eq!(claimed, TypedKey::Claimed);
    assert_eq!(pending, TypedKey::Pending { holder: 0 });
    assert_eq!(typed, TypedKey::Typed);
    assert_eq!(other, TypedKey::Claimed);
}

/// The holder a pending key's text went to is kept with it, across
/// reopens, so a retry knows whom to ask (D426).
#[test]
fn a_pending_key_keeps_the_holder_its_text_went_to() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.claim_typed_key("Type/a", 10).unwrap();
    store.note_typed_key_holder("Type/a", u64::MAX - 7).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.claim_typed_key("Type/a", 20).unwrap(),
        TypedKey::Pending {
            holder: u64::MAX - 7
        }
    );
}

#[test]
fn a_released_key_can_be_claimed_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.claim_typed_key("Type/a", 10).unwrap();

    store.release_typed_key("Type/a").unwrap();

    assert_eq!(store.claim_typed_key("Type/a", 20).unwrap(), TypedKey::Claimed);
}

#[test]
fn keys_claimed_before_the_cutoff_are_dropped_and_newer_ones_kept() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.claim_typed_key("Type/old", 10).unwrap();
    store.finish_typed_key("Type/old").unwrap();
    store.claim_typed_key("Type/new", 1_000).unwrap();

    let dropped = store.prune_typed_keys(500).unwrap();

    assert_eq!(dropped, 1);
    assert_eq!(store.claim_typed_key("Type/old", 2_000).unwrap(), TypedKey::Claimed);
    assert_eq!(
        store.claim_typed_key("Type/new", 2_000).unwrap(),
        TypedKey::Pending { holder: 0 }
    );
}
