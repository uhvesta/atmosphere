//! What the boards hide (D216): items on an archived session's board. The
//! session's items stay where they are; a fork's family walks through
//! archived forks, so the forks under an archived one still show.

use std::collections::HashSet;

use humata2_store::{BoardItemRecord, BoardKey, ForkOrigin, Store, StoreError};

/// The ids of the archived sessions, read once for a whole call.
pub(super) fn archived(store: &Store) -> Result<HashSet<String>, StoreError> {
    Ok(store.archives()?.into_iter().map(|row| row.session_id).collect())
}

/// Whether `item` is on an archived session's board, given the ids from
/// [`archived`].
pub(super) fn is_hidden(item: &BoardItemRecord, archived: &HashSet<String>) -> bool {
    matches!(&item.board, BoardKey::Session(session) if archived.contains(session))
}

/// Whether `item` is hidden, read from the store for this one item.
pub(super) fn is_hidden_now(store: &Store, item: &BoardItemRecord) -> Result<bool, StoreError> {
    match &item.board {
        BoardKey::Session(session) => match store.archive(session) {
            Ok(row) => Ok(row.is_some()),
            // An id the store can't hold has no archive row.
            Err(StoreError::InvalidId { .. }) => Ok(false),
            Err(error) => Err(error),
        },
        _ => Ok(false),
    }
}

/// `root` and every session forked from it at any depth, by true parent.
/// `seen` keeps a cycle in the origins from looping.
pub(super) fn family(origins: &[ForkOrigin], root: &str) -> Vec<String> {
    let mut seen = HashSet::from([root.to_owned()]);
    let mut found = vec![root.to_owned()];
    let mut next = 0;
    while let Some(parent) = found.get(next).cloned() {
        next += 1;
        for origin in origins.iter().filter(|origin| origin.parent == parent) {
            if seen.insert(origin.session_id.clone()) {
                found.push(origin.session_id.clone());
            }
        }
    }
    found
}
