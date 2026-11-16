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

/// Why starting or stopping a recording was refused.
#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error(
        "a recording is already running, and only one runs at a time; stop it with Stop \
         Recording in the menu or `humata report stop`, which saves it, then start a new one. \
         To capture this moment without stopping it, take a snapshot instead"
    )]
    AlreadyRecording {
        /// When the running recording started, in milliseconds since the
        /// Unix epoch.
        started_unix_ms: u64,
    },

    #[error(
        "no recording is running, so there is nothing to stop; start one with Start Recording \
         in the menu or `humata report record`, or take a snapshot of this moment with \
         `humata report snapshot`"
    )]
    NotRecording,
}

/// Why the form couldn't be saved into a report that was already written.
/// The report's earlier `report.md` is kept when this happens.
#[derive(Debug, thiserror::Error)]
pub enum FormError {
    #[error(
        "couldn't read {}, which the form's summary is made from; check that the report's \
         folder still exists and belongs to you. The form wasn't saved",
        path.display()
    )]
    ReadEvents {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{} can't be read back, so the form's summary can't be made from it; the form wasn't \
         saved. Take a new report",
        path.display()
    )]
    Events {
        path: PathBuf,
        #[source]
        source: ReadError,
    },

    #[error(
        "couldn't write the form into {}; check that the disk has free space and the folder \
         is writable. The report's earlier report.md is unchanged",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Why a call made during a replay couldn't be answered from the
/// recording.
#[derive(Debug, thiserror::Error)]
pub enum FakeDaemonError {
    #[error(
        "the replay called {method} on {machine} more often than the recording did (it called \
         it {recorded} times). The app now makes a call it didn't make when the report was \
         taken, or the recording was full and dropped the call; if the app's calls changed on \
         purpose, take a new report"
    )]
    Unrecorded {
        machine: String,
        method: String,
        recorded: usize,
    },
}

/// Why a recording didn't replay to where it ended.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError<E: std::error::Error + 'static> {
    #[error(
        "the replay's app doesn't start where the recording started: its {tree} differs \
         (- recording, + replay):\n{diff}Set up the test's fixture (its state and fake \
         services) from the report's start, as state.txt shows it. A recording that was full \
         starts mid-way, after its oldest events were folded in"
    )]
    StartDiffers { tree: &'static str, diff: String },

    #[error("couldn't read the replay's state, view or layout {when}")]
    Trees {
        when: &'static str,
        #[source]
        source: E,
    },

    #[error("step {step} of the replay ({what}) couldn't be played")]
    Play {
        step: usize,
        what: String,
        #[source]
        source: E,
    },

    #[error(
        "the replay ended with a different {tree} from the recording (- recording, + replay):\n\
         {diff}If the report showed a bug and it is fixed, update the test to expect the fixed \
         end; otherwise the app or the test's fixture changed since the report was taken"
    )]
    EndDiffers { tree: &'static str, diff: String },
}

/// Why `humata report` couldn't reach the running app, or the app couldn't
/// listen for it.
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error(
        "Humata 2 isn't running (no app at {}). Open the app, then run the command again",
        path.display()
    )]
    NotRunning { path: PathBuf },

    #[error(
        "another Humata 2 is already answering at {}; quit it, or set HUMATA_APP_SOCKET to \
         another path for this one",
        path.display()
    )]
    InUse { path: PathBuf },

    #[error(
        "couldn't find the app's socket: HUMATA_APP_SOCKET is unset and HOME isn't set; set \
         HUMATA_APP_SOCKET to the socket's path"
    )]
    NoHome,

    #[error(
        "couldn't listen at {}; check that its folder exists and belongs to you, or set \
         HUMATA_APP_SOCKET to another path. Reports from `humata report` won't reach this app \
         until then",
        path.display()
    )]
    Bind {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't talk to the app at {}; if Humata 2 is stuck or quitting, restart it and run \
         the command again",
        path.display()
    )]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't write the request or answer as JSON; this is a bug in Humata")]
    Encode {
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "the other side of the socket sent something that isn't a Humata answer; make sure \
         `humata` and the app are the same version, then run the command again"
    )]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

/// Why the window's picture couldn't be saved into a report.
#[derive(Debug, thiserror::Error)]
pub enum PictureError {
    #[error(
        "couldn't encode the window's picture as a PNG; the report's text files are fine, so \
         copy the report without the picture, or take a new snapshot"
    )]
    Encode {
        #[source]
        source: image::ImageError,
    },

    #[error(
        "couldn't write window.png into {}; check that the disk has free space and the report's \
         folder is writable. The report's text files are unchanged",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
