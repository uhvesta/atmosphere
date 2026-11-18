//! What a transfer's journal holds.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::end::Entry;
use crate::engine::{Landing, Mode, OnExisting, Problem, Snapshot, State};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Phase {
    Running,
    Verifying,
    Paused,
    Committing,
    Cancelling,
    Done,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Want {
    Run,
    Pause,
    Cancel,
}

/// One name that ends up in the destination folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Top {
    pub name: String,
    /// Where it is read from; none for an upload.
    pub source: Option<PathBuf>,
    pub shown: String,
    /// Where it landed; set while committing.
    #[serde(default)]
    pub landed: Landing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PlanFile {
    pub top: usize,
    pub entry: Entry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct UploadInfo {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    pub id: String,
    pub tops: Vec<Top>,
    pub destination: PathBuf,
    pub destination_shown: String,
    pub mode: Mode,
    pub on_existing: OnExisting,
    pub caller_data: String,
    pub now_unix_ms: i64,
    pub upload: Option<UploadInfo>,
    pub phase: Phase,
    pub problem: Option<Problem>,
    pub planned: bool,
    /// A move on one disk, done as renames.
    pub direct: bool,
    pub files: Vec<PlanFile>,
    pub file_index: usize,
    pub offset: u64,
    /// The part of the current file the destination has confirmed on disk
    /// (after a sync); a resume falls back to it when the destination holds
    /// less than `offset`.
    #[serde(default)]
    pub confirmed: u64,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub done_files: u32,
    pub total_files: u32,
    pub version: u64,
}

impl Record {
    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            id: self.id.clone(),
            sources: self.tops.iter().map(|t| t.shown.clone()).collect(),
            destination: self.destination_shown.clone(),
            mode: self.mode,
            state: match self.phase {
                Phase::Running | Phase::Committing | Phase::Cancelling => State::Running,
                Phase::Verifying => State::Verifying,
                Phase::Paused => State::Paused,
                Phase::Done => State::Done,
                Phase::Failed => State::Failed,
                Phase::Cancelled => State::Cancelled,
            },
            done_bytes: self.done_bytes,
            total_bytes: self.total_bytes,
            done_files: self.done_files,
            total_files: self.total_files,
            problem: self.problem.clone(),
            started_unix_ms: self.now_unix_ms,
            version: self.version,
            caller_data: self.caller_data.clone(),
            upload: self.upload.is_some(),
            landings: self.tops.iter().map(|t| t.landed.clone()).collect(),
        }
    }
}

pub(crate) struct Slot {
    pub rec: Record,
    pub want: Want,
    pub running: bool,
}
