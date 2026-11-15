use std::path::PathBuf;

use humata2_holder::HolderError;
use humata2_store::StoreError;

/// What launching or reviving a session can fail at. A resume the agent
/// didn't take up is not an error: it's [`crate::Revived::ResumeFailed`].
#[derive(Debug, thiserror::Error)]
pub enum ReviveError {
    #[error("couldn't {doing} for {what}")]
    Store {
        what: String,
        doing: &'static str,
        #[source]
        source: StoreError,
    },

    #[error("couldn't start the terminal for {what}")]
    Holder {
        what: String,
        #[source]
        source: HolderError,
    },

    #[error(
        "session {session}'s agent was saved with no command to run; start a new session for it from Humata"
    )]
    NoCommand { session: String },

    #[error("a background task stopped before it finished; restart humatad2 to try again")]
    Background {
        #[source]
        source: tokio::task::JoinError,
    },

    #[error(
        "couldn't mask the restart report's secrets, so it wasn't written; the sessions were still brought back as reconcile returned. This is a bug in Humata: report it"
    )]
    ScrubReport {
        #[source]
        source: humata2_report::ScrubError,
    },

    #[error(
        "couldn't write the restart report in {}; the sessions were still brought back as reconcile returned. Check the folder is writable and the disk has space; the next restart writes a new report",
        folder.display()
    )]
    WriteReport {
        folder: PathBuf,
        #[source]
        source: humata2_report::WriteError,
    },

    #[error(
        "couldn't write {}, the restart report's summary for programs; report.md beside it has the same facts. Check the folder is writable and the disk has space",
        path.display()
    )]
    RestartJson {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
