//! Humata 2's state on one machine: session records, terminal tabs, where
//! each session's clones stand, and a journal of writes the host hasn't
//! taken yet.
//!
//! Every file is replaced atomically, so a crash or power cut at any
//! moment leaves the copy from before or after a write, never a torn one.
//! The folder carries a schema version, and a newer one is refused rather
//! than misread.

mod atomic;
mod error;
mod journal;
mod records;
mod store;

pub use atomic::write_atomic;
pub use error::StoreError;
pub use journal::{Journal, JournalEntry};
pub use records::{Pointers, RepoPointer, SessionRecord, TabKind, TabRecord};
pub use store::{SCHEMA_VERSION, Store};
