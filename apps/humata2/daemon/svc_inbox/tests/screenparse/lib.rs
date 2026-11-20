//! Records what agent CLIs put on a terminal screen, as labelled text
//! files, so tests can replay them against the prompt parser. The CLI runs
//! under the real holder and its output goes through the engine the daemon
//! reads screens with.

mod record;
mod script;
mod snapshot;

pub use record::{Options, RecordError, record};
pub use script::{ScriptError, parse_script};
pub use snapshot::{Expect, Snapshot, SnapshotError, read_snapshot};
