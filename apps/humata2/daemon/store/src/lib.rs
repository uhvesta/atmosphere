//! Humata 2's state on one machine: session records, terminal tabs, each
//! session's agent and how to resume it, where each session's clones
//! stand, and a journal of writes the host hasn't taken yet.
//!
//! The state is one SQLite database in WAL mode, every write a
//! transaction that is on disk when it returns, so a crash or power cut at
//! any moment leaves it whole from before or after a write. Data kept
//! whole rather than in columns is a blob: a header numbering its codec,
//! type and format version, then its body, compressed when that pays; a big one lives in a file named by its hash, written before the
//! row that names it. The database carries a schema version, and a newer
//! one is refused rather than misread.

mod agents;
mod atomic;
mod blob;
mod error;
pub mod fault;
mod journal;
mod last_good;
mod records;
mod schema;
mod store;

use humata2_store_proto::humata::store::v1 as proto;

pub use atomic::write_atomic;
pub use error::StoreError;
pub use journal::{Journal, JournalEntry};
pub use last_good::{Change, LastGood, Restored, RowChange};
pub use records::{
    AgentLaunch, AgentRecord, AgentState, Interruption, Pointers, RepoPointer, ResumeFailure,
    SessionRecord, TabKind, TabRecord, Told,
};
pub use schema::SCHEMA_VERSION;
pub use store::Store;
