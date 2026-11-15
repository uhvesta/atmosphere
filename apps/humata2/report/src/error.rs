//! What taking, saving and reading a report can fail at.

use std::path::PathBuf;

/// Why a report couldn't be scrubbed. Nothing is written when this
/// happens.
#[derive(Debug, thiserror::Error)]
pub enum ScrubError {
    #[error(
        "the recording's {tree} changes at {at_ms} ms don't fit the {tree} recorded before them \
         (line {line}), so its trees can't be rebuilt to take secrets out, and nothing was saved. \
         This is a bug in Humata's recorder; take a snapshot instead, which needs no rebuilding"
    )]
    Mismatch {
        tree: &'static str,
        at_ms: u64,
        line: usize,
    },
}

/// Why a report couldn't be saved. Nothing is left half-written: the
/// folder appears whole or not at all.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error(
        "couldn't create the reports folder {}; check that its parent exists and belongs to you",
        path.display()
    )]
    CreateReports {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't write the report's {} in {}; check that the disk has free space and the \
         reports folder is writable. Nothing was saved",
        file,
        path.display()
    )]
    WriteFile {
        file: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't move the finished report into place at {}; check that the reports folder is \
         writable. Nothing was saved, and the partly written copy was removed",
        path.display()
    )]
    Publish {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "there are already {tries} reports named {name} in {}; rename or delete some of them, \
         or describe what happened differently so the report gets another name",
        reports.display()
    )]
    NoFreeName {
        reports: PathBuf,
        name: String,
        tries: usize,
    },
}

/// Why a saved `events.jsonl` couldn't be read back.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error(
        "line {line} of events.jsonl isn't a report event; the file was edited or cut short. \
         Restore it from the report's folder if you copied it elsewhere, or take a new report"
    )]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "events.jsonl was written by a newer Humata (schema {found}; this one reads up to \
         {supported}); update Humata to read it"
    )]
    Schema { found: u32, supported: u32 },

    #[error(
        "line {line} of events.jsonl is out of order: {what}. The file was edited or two \
         reports were joined; take a new report, or restore this one's events.jsonl"
    )]
    OutOfPlace { line: usize, what: &'static str },

    #[error(
        "events.jsonl has no {what} line, so it was cut short or isn't a Humata report; a \
         report missing its {what} can't be replayed. Take a new report"
    )]
    Missing { what: &'static str },
}

/// Why the Copy report prompt couldn't be made.
#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error(
        "couldn't read the report folder {}; check that it still exists and belongs to you",
        path.display()
    )]
    ReadFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{} has no report.md, so it isn't a Humata report or its summary was deleted; pick the \
         report's folder, or take a new report",
        path.display()
    )]
    NotAReport { path: PathBuf },

    #[error(
        "couldn't read {}; check that it belongs to you",
        path.display()
    )]
    ReadSummary {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
