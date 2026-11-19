//! A machine's sessions can be stopped (D261) and started again (D265).
//!
//! The stop record says a stop happened and what it ended; it lives on disk
//! so a reboot or a power cut keeps the machine stopped until a Start.

pub mod boot;
pub mod cli;
pub mod exec;
pub mod gate;
pub mod host_copy;
pub mod record;
pub mod remote;
pub mod service;
pub mod streams;
pub mod sweep;

pub use record::{ByKind, Ended, Record, RecordError, StopFolder, StopState, StoppedBy, Via};
