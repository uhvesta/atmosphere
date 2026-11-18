//! A machine's sessions can be stopped (D261) and started again (D265).
//!
//! The stop record says a stop happened and what it ended; it lives on disk
//! so a reboot or a power cut keeps the machine stopped until a Start.

pub(crate) mod boot;
pub(crate) mod cli;
pub(crate) mod exec;
pub(crate) mod gate;
pub(crate) mod host_copy;
pub(crate) mod record;
pub(crate) mod remote;
pub(crate) mod service;
pub(crate) mod streams;
pub(crate) mod sweep;

pub(crate) use record::{
    ByKind, Ended, Record, RecordError, StopFolder, StopState, StoppedBy, Via,
};
