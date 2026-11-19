//! Since when each session has had the status it shows (PROGRESS T5).
//!
//! A status is worked out when a session is read, so nothing writes it.
//! Each read tells this book the status it found: a different one starts
//! a new "since", the same one keeps the old. The book is saved to
//! `status-since.json` in the state folder when a status changes, so a
//! restart doesn't reset the times. Entries of sessions that are gone stay
//! (there is no cheap list of live ids here); they are small.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};

use humata2_grpc::humata::v1 as pb;

use crate::status;

const FILE: &str = "status-since.json";

#[derive(Default)]
struct Book {
    /// Session id to (status, since in unix ms).
    seen: HashMap<String, (i32, i64)>,
    path: Option<PathBuf>,
    warned: bool,
}

static BOOK: LazyLock<Mutex<Book>> = LazyLock::new(Mutex::default);

impl Book {
    /// Loads the saved book from `<state>/status-since.json`; a missing or
    /// unreadable file gives an empty book.
    fn open(state: &Path) -> Self {
        let path = state.join(FILE);
        let seen = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            seen,
            path: Some(path),
            warned: false,
        }
    }

    /// Records `status` for `session` and returns its since. The second
    /// value is the text to save when something changed.
    fn observe(&mut self, session: &str, status: i32, now_ms: i64) -> (i64, Option<(PathBuf, Vec<u8>)>) {
        match self.seen.get_mut(session) {
            Some((had, since)) if *had == status => return (*since, None),
            Some(entry) => *entry = (status, now_ms),
            None => {
                self.seen.insert(session.to_owned(), (status, now_ms));
            }
        }
        let save = self.path.as_ref().and_then(|path| {
            serde_json::to_vec(&self.seen)
                .ok()
                .map(|bytes| (path.clone(), bytes))
        });
        (now_ms, save)
    }
}

/// Gives the book its state folder and loads what was saved. At the
/// daemon's start.
pub(crate) fn open(state: &Path) {
    *BOOK.lock().unwrap_or_else(PoisonError::into_inner) = Book::open(state);
}

/// Notes that `session` shows `status` now and returns since when it has.
/// A change is saved while the book stays locked, so two changes at once
/// can't land on disk in the wrong order; changes are rare and the file is
/// small.
pub(super) fn observe(session: &str, status: pb::SessionStatus, now_ms: i64) -> i64 {
    let mut book = BOOK.lock().unwrap_or_else(PoisonError::into_inner);
    let (since, save) = book.observe(session, status as i32, now_ms);
    if let Some((path, bytes)) = save
        && let Err(error) = humata2_store::write_atomic(&path, &bytes)
        && !std::mem::replace(&mut book.warned, true)
    {
        crate::log::say!(
            "humatad2: couldn't save since when sessions have had their status, so those times start over when the daemon restarts: {}",
            status::chain(&error)
        );
    }
    since
}

#[cfg(test)]
mod tests {
    use super::*;

    const WORKING: i32 = pb::SessionStatus::Working as i32;
    const IDLE: i32 = pb::SessionStatus::Idle as i32;

    #[test]
    fn a_change_moves_since_and_a_repeat_keeps_it() {
        let mut book = Book::default();
        assert_eq!(book.observe("s", WORKING, 10).0, 10);
        assert_eq!(book.observe("s", WORKING, 20).0, 10);
        assert_eq!(book.observe("s", IDLE, 30).0, 30);
        assert_eq!(book.observe("s", IDLE, 40).0, 30);
        assert_eq!(book.observe("s", WORKING, 50).0, 50);
        assert_eq!(book.observe("other", WORKING, 60).0, 60);
    }

    #[test]
    fn a_saved_book_keeps_since_after_a_restart() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut book = Book::open(dir.path());
        let (_, save) = book.observe("s", WORKING, 10);
        let (path, bytes) = save.expect("a change is saved");
        humata2_store::write_atomic(&path, &bytes).expect("write");
        let mut fresh = Book::open(dir.path());
        assert_eq!(fresh.observe("s", WORKING, 99).0, 10);
        assert_eq!(fresh.observe("s", IDLE, 120).0, 120);
    }

    #[test]
    fn a_repeat_is_not_saved() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut book = Book::open(dir.path());
        assert!(book.observe("s", WORKING, 10).1.is_some());
        assert!(book.observe("s", WORKING, 20).1.is_none());
    }

    #[test]
    fn a_missing_or_corrupt_file_loads_empty() {
        let dir = tempfile::tempdir().expect("temp dir");
        assert!(Book::open(dir.path()).seen.is_empty());
        std::fs::write(dir.path().join(FILE), b"{not json").expect("write");
        assert!(Book::open(dir.path()).seen.is_empty());
    }
}
