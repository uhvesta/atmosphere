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
mod away_left_out;
mod blob;
mod boards;
mod channel_snapshot;
mod channels;
mod comments;
mod desk;
mod error;
mod fresh_seeds;
pub mod fault;
mod inbox;
mod journal;
mod last_copy;
mod last_good;
mod lifecycle;
mod machine_journal;
mod previous;
mod records;
mod remote_reports;
mod reviews;
mod rules;
mod settings;
mod schema;
mod script_tokens;
mod session_snapshot;
mod store;
mod views;

use humata2_store_proto::humata::store::v1 as proto;

pub use atomic::write_atomic;
pub use channel_snapshot::{DeletedChannel, SIDE_FILE, Placement};
pub use boards::{
    BoardHistoryEntry, BoardHistoryKind, BoardItemChange, BoardItemRecord, BoardKey,
    BoardReplay, BoardReplayed, BoardWrite, NewBoardItem, PrefixClaim,
};
pub use comments::{
    CommentChange, CommentDeliveryRecord, CommentDeliveryState, CommentNumbers, CommentRecord, SessionComments,
};
pub use desk::{DeskFolderKind, DeskRow};
pub use error::StoreError;
pub use fresh_seeds::FreshSeed;
pub use journal::{Journal, JournalEntry};
pub use last_copy::LastCopy;
pub use last_good::{Change, LastGood, Restored, RowChange};
pub use lifecycle::{ArchiveRow, ArchiveState, ForkOrigin};
pub use previous::PreviousConversation;
pub use records::{
    AgentLaunch, AgentRecord, AgentState, ChannelRecord, ChannelRepo, InboxRecord, Interruption,
    MainClone, Pointers, RepoPointer, ResumeFailure, ReviewRecord, SessionDetail, SessionRecord,
    SessionRepo, StartStep, TabKind, TabRecord, Told, ViewRecord,
};
pub use remote_reports::RemoteReportRow;
pub use rules::SavedRules;
pub use settings::SavedSetting;
pub use away_left_out::{AwayLeftOut, LeftOut};
pub use schema::SCHEMA_VERSION;
pub use script_tokens::SavedScriptToken;
pub use session_snapshot::Take;
pub use store::Store;
