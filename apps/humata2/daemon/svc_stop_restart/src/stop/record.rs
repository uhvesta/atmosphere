//! The stop record on disk (D261, D265).
//!
//! Two files live in `<state>/stop`: `this.json` while the machine is
//! stopped, and `this.starting.json` while a Start brings it back; a
//! finished Start leaves it as `last.json`, so a stop that comes late (an
//! SSH that lost the race to the link) is known for the one already
//! applied (D262). Each
//! change is durable: a file is written with `write_atomic`, and a rename or
//! removal is followed by a sync of the folder, so a power cut leaves one of
//! the states whole. Where both files exist, the machine is stopped.
//!
//! `ended_early.json` names the holders `humatad2 stop` ended itself, for
//! a daemon that didn't answer, so the next start's sweep counts them too.

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};

use humata2_store::StoreError;
use serde_json::{Value, json};

use super::sweep::{HolderEnd, How};

/// The only record format this Humata writes and reads.
const FORMAT: u64 = 1;
const STOPPED_FILE: &str = "this.json";
const STARTING_FILE: &str = "this.starting.json";
const LAST_FILE: &str = "last.json";
const ENDED_EARLY_FILE: &str = "ended_early.json";

/// The `<state>/stop` folder and the record files in it.
#[derive(Clone, Debug)]
pub struct StopFolder {
    folder: PathBuf,
}

impl StopFolder {
    /// The folder for the state folder `state`: `<state>/stop`.
    pub fn new(state: &Path) -> Self {
        Self {
            folder: state.join("stop"),
        }
    }

    /// What the record says the machine is. A missing record reads as
    /// running. A record that can't be read is an error, and the caller
    /// keeps the machine stopped until it is fixed.
    pub fn read(&self) -> Result<StopState, RecordError> {
        if let Some(record) = self.read_at(&self.stopped_path())? {
            return Ok(StopState::Stopped(record));
        }
        match self.read_at(&self.starting_path())? {
            Some(record) => Ok(StopState::Starting(record)),
            None => Ok(StopState::Running),
        }
    }

    /// Records a stop, creating the folder if it is missing.
    pub fn write_stopped(&self, record: &Record) -> Result<(), RecordError> {
        self.ensure_folder()?;
        write_file(&self.stopped_path(), &record.to_json())?;
        humata2_daemon_runtime::fault::point("stop.record.written");
        Ok(())
    }

    /// Fills in what the stop ended, once the sessions are down. Fails with
    /// `NotStopped` when there is no stop record to fill in.
    pub fn settle(&self, ended: &Ended) -> Result<Record, RecordError> {
        let path = self.stopped_path();
        let mut record = self
            .read_at(&path)?
            .ok_or_else(|| RecordError::NotStopped { path: path.clone() })?;
        record.ended = Some(ended.clone());
        write_file(&path, &record.to_json())?;
        humata2_daemon_runtime::fault::point("stop.record.settled");
        Ok(record)
    }

    /// Moves the stop record aside to start the machine, and returns it.
    /// Fails with `NotStopped` when there is no stop record.
    pub fn begin_start(&self) -> Result<Record, RecordError> {
        let stopped = self.stopped_path();
        let record = self
            .read_at(&stopped)?
            .ok_or_else(|| RecordError::NotStopped {
                path: stopped.clone(),
            })?;
        let starting = self.starting_path();
        humata2_durable::rename(&stopped, &starting).map_err(|source| RecordError::Rename {
            from: stopped,
            to: starting,
            source,
        })?;
        self.sync_folder()?;
        humata2_daemon_runtime::fault::point("start.record.renamed");
        Ok(record)
    }

    /// Ends the start once the machine is up: the start record becomes
    /// `last.json`, the last stop applied. Absent is fine.
    pub fn finish_start(&self) -> Result<(), RecordError> {
        let starting = self.starting_path();
        let last = self.last_path();
        match humata2_durable::rename(&starting, &last) {
            Ok(()) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => {
                return Err(RecordError::Rename {
                    from: starting,
                    to: last,
                    source,
                });
            }
        }
        if self.folder.is_dir() {
            self.sync_folder()?;
        }
        humata2_daemon_runtime::fault::point("start.record.removed");
        Ok(())
    }

    /// Fails with `AlreadyApplied` when `stop_id` is the stop this machine
    /// was last started from: it was applied and undone, and a copy of it
    /// arriving late must not stop the machine again. A `last.json` that
    /// can't be read says nothing, so the stop goes ahead.
    pub fn refuse_applied(&self, stop_id: &str) -> Result<(), RecordError> {
        match self.read_at(&self.last_path()) {
            Ok(Some(last)) if last.stop_id == stop_id => Err(RecordError::AlreadyApplied {
                stop_id: stop_id.to_owned(),
            }),
            Ok(_) | Err(_) => Ok(()),
        }
    }

    /// Notes `holders`, which `humatad2 stop` ended before the daemon's
    /// next start, on top of any noted already, for that start's sweep to
    /// count.
    pub fn note_ended_early(&self, holders: &[HolderEnd]) -> Result<(), RecordError> {
        if holders.is_empty() {
            return Ok(());
        }
        self.ensure_folder()?;
        let mut all = self.ended_early();
        all.extend_from_slice(holders);
        let listed: Vec<Value> = all
            .iter()
            .map(|holder| {
                json!({
                    "socket": holder.socket.to_string_lossy(),
                    "pid": holder.pid,
                    "killed": holder.how == How::Killed,
                })
            })
            .collect();
        let value = json!({ "format": FORMAT, "holders": listed });
        write_file(&self.ended_early_path(), format!("{value:#}").as_bytes())
    }

    /// The holders `humatad2 stop` ended before this start. A note that is
    /// missing or can't be read names none: the stop holds either way, and
    /// only its count is short.
    pub fn ended_early(&self) -> Vec<HolderEnd> {
        let Ok(bytes) = fs::read(self.ended_early_path()) else {
            return Vec::new();
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            return Vec::new();
        };
        if value["format"].as_u64() != Some(FORMAT) {
            return Vec::new();
        }
        value["holders"]
            .as_array()
            .map(|holders| {
                holders
                    .iter()
                    .filter_map(|holder| {
                        Some(HolderEnd {
                            socket: PathBuf::from(holder["socket"].as_str()?),
                            pid: u32::try_from(holder["pid"].as_u64()?).ok()?,
                            how: if holder["killed"].as_bool()? {
                                How::Killed
                            } else {
                                How::Ended
                            },
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Removes the note of holders ended early, once a sweep has counted
    /// them into the record. Absent is fine.
    pub fn clear_ended_early(&self) -> Result<(), RecordError> {
        let path = self.ended_early_path();
        match fs::remove_file(&path) {
            Ok(()) => self.sync_folder(),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(()),
            Err(source) => Err(RecordError::Remove { path, source }),
        }
    }

    fn ended_early_path(&self) -> PathBuf {
        self.folder.join(ENDED_EARLY_FILE)
    }

    fn last_path(&self) -> PathBuf {
        self.folder.join(LAST_FILE)
    }

    fn stopped_path(&self) -> PathBuf {
        self.folder.join(STOPPED_FILE)
    }

    fn starting_path(&self) -> PathBuf {
        self.folder.join(STARTING_FILE)
    }

    fn read_at(&self, path: &Path) -> Result<Option<Record>, RecordError> {
        match fs::read(path) {
            Ok(bytes) => Record::from_json(path, &bytes).map(Some),
            Err(source) if source.kind() == ErrorKind::NotFound => Ok(None),
            Err(source) => Err(RecordError::Read {
                path: path.to_owned(),
                source,
            }),
        }
    }

    fn ensure_folder(&self) -> Result<(), RecordError> {
        if self.folder.is_dir() {
            return Ok(());
        }
        humata2_durable::create_dir_all(&self.folder, 0o700).map_err(|source| {
            RecordError::CreateDir {
                path: self.folder.clone(),
                source,
            }
        })?;
        if let Some(state) = self.folder.parent() {
            humata2_durable::sync_dir(state).map_err(|source| RecordError::SyncDir {
                path: state.to_owned(),
                source,
            })?;
        }
        Ok(())
    }

    fn sync_folder(&self) -> Result<(), RecordError> {
        humata2_durable::sync_dir(&self.folder).map_err(|source| RecordError::SyncDir {
            path: self.folder.clone(),
            source,
        })
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), RecordError> {
    humata2_store::write_atomic(path, bytes).map_err(|source| RecordError::Write {
        path: path.to_owned(),
        source,
    })
}

/// What the stop record says the machine is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopState {
    Running,
    Stopped(Record),
    Starting(Record),
}

/// One stop: who asked, when, why, and what it ended once settled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub stop_id: String,
    pub stopped_at_unix_ms: i64,
    pub by: StoppedBy,
    pub reason: Option<String>,
    pub ended: Option<Ended>,
}

/// Who asked for a stop, and how it reached the machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoppedBy {
    pub kind: ByKind,
    pub via: Via,
    pub machine: String,
    pub session: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ByKind {
    User,
    Agent,
    Terminal,
    /// A script whose token lists the capability.
    Script,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Via {
    App,
    Cli,
    Ssh,
    Direct,
}

/// What a settled stop ended: sessions and shells it ended, processes it
/// had to kill, and any process still alive after the grace period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    pub agents: u32,
    pub shells: u32,
    pub killed: u32,
    pub left: Vec<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error(
        "couldn't create the stop folder {}; check that its parent belongs to you and that the disk has room. Nothing was stopped",
        path.display()
    )]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "couldn't save the stop record {}; check that the disk has free space and that the folder belongs to you. Nothing else was changed, so run the command again",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },

    #[error(
        "couldn't read the stop record {}; check that it belongs to you and that the disk is mounted",
        path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "the stop record {} is unreadable: {reason}. The file is kept and this machine stays stopped. Update Humata if the file came from a newer version; if the file is damaged, move it aside, then run `humata start` to start the sessions again",
        path.display()
    )]
    Unreadable {
        path: PathBuf,
        reason: String,
        #[source]
        source: Option<serde_json::Error>,
    },

    #[error(
        "couldn't rename {} to {}; check that the folder belongs to you. The machine is still stopped, so run `humata start` again",
        from.display(),
        to.display()
    )]
    Rename {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "couldn't sync the stop folder {} to disk; check that the disk is working, then run the command again so the change is made durable",
        path.display()
    )]
    SyncDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "couldn't remove {}; check that the folder belongs to you. The machine is stopped either way; until the file is removed, each start of the daemon counts the holders it names again in what the stop ended",
        path.display()
    )]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "the stop {stop_id} was applied and then started again on this machine, so it isn't applied a second time. To stop its sessions now, run `humata stop` for a new stop"
    )]
    AlreadyApplied { stop_id: String },

    #[error(
        "there is no stop record at {}, so the machine isn't stopped and there is nothing to change. To stop its sessions, run `humata stop`",
        path.display()
    )]
    NotStopped { path: PathBuf },
}

impl Record {
    /// The record as its JSON file, pretty-printed for a person to read.
    pub fn to_json(&self) -> Vec<u8> {
        let mut by = json!({
            "kind": self.by.kind.name(),
            "via": self.by.via.name(),
            "machine": self.by.machine,
        });
        if let Some(session) = &self.by.session {
            by["session"] = json!(session);
        }
        let mut value = json!({
            "format": FORMAT,
            "stop_id": self.stop_id,
            "stopped_at_unix_ms": self.stopped_at_unix_ms,
            "by": by,
        });
        if let Some(reason) = &self.reason {
            value["reason"] = json!(reason);
        }
        if let Some(ended) = &self.ended {
            value["ended"] = json!({
                "agents": ended.agents,
                "shells": ended.shells,
                "killed": ended.killed,
                "left": ended.left,
            });
        }
        format!("{value:#}").into_bytes()
    }

    /// Reads a record from the bytes of its file at `path`, which names the
    /// file in the error.
    pub fn from_json(path: &Path, bytes: &[u8]) -> Result<Self, RecordError> {
        let value: Value = serde_json::from_slice(bytes).map_err(|source| {
            unreadable(
                path,
                "it isn't JSON, so it is damaged or was not written by Humata",
                Some(source),
            )
        })?;
        match value.get("format").and_then(Value::as_u64) {
            Some(FORMAT) => {}
            Some(newer) if newer > FORMAT => {
                return Err(unreadable(
                    path,
                    format!(
                        "it was written by a newer Humata (format {newer}; this one reads format {FORMAT})"
                    ),
                    None,
                ));
            }
            _ => {
                return Err(unreadable(
                    path,
                    "it has no format number this Humata can read",
                    None,
                ));
            }
        }
        let stop_id = required_text(path, &value, "stop_id")?;
        let stopped_at_unix_ms = value
            .get("stopped_at_unix_ms")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                unreadable(
                    path,
                    "the field `stopped_at_unix_ms` is missing or not a whole number",
                    None,
                )
            })?;
        let by = parse_by(path, &value)?;
        let reason = optional_text(path, &value, "reason")?;
        let ended = match value.get("ended") {
            None | Some(Value::Null) => None,
            Some(ended) => Some(parse_ended(path, ended)?),
        };
        Ok(Self {
            stop_id,
            stopped_at_unix_ms,
            by,
            reason,
            ended,
        })
    }
}

impl ByKind {
    fn name(&self) -> &'static str {
        match self {
            ByKind::User => "user",
            ByKind::Agent => "agent",
            ByKind::Terminal => "terminal",
            ByKind::Script => "script",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "user" => Some(ByKind::User),
            "agent" => Some(ByKind::Agent),
            "terminal" => Some(ByKind::Terminal),
            "script" => Some(ByKind::Script),
            _ => None,
        }
    }
}

impl Via {
    fn name(&self) -> &'static str {
        match self {
            Via::App => "app",
            Via::Cli => "cli",
            Via::Ssh => "ssh",
            Via::Direct => "direct",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        match name {
            "app" => Some(Via::App),
            "cli" => Some(Via::Cli),
            "ssh" => Some(Via::Ssh),
            "direct" => Some(Via::Direct),
            _ => None,
        }
    }
}

fn unreadable(
    path: &Path,
    reason: impl Into<String>,
    source: Option<serde_json::Error>,
) -> RecordError {
    RecordError::Unreadable {
        path: path.to_owned(),
        reason: reason.into(),
        source,
    }
}

fn required_text(path: &Path, object: &Value, key: &str) -> Result<String, RecordError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            unreadable(
                path,
                format!("the field `{key}` is missing or not text"),
                None,
            )
        })
}

fn optional_text(path: &Path, object: &Value, key: &str) -> Result<Option<String>, RecordError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(text) => text
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| unreadable(path, format!("the field `{key}` is not text"), None)),
    }
}

fn parse_by(path: &Path, value: &Value) -> Result<StoppedBy, RecordError> {
    let by = value
        .get("by")
        .filter(|by| by.is_object())
        .ok_or_else(|| unreadable(path, "the object `by` is missing", None))?;
    let kind = by
        .get("kind")
        .and_then(Value::as_str)
        .and_then(ByKind::from_name)
        .ok_or_else(|| {
            unreadable(
                path,
                "the field `by.kind` is missing or not one of user, agent, terminal, script",
                None,
            )
        })?;
    let via = by
        .get("via")
        .and_then(Value::as_str)
        .and_then(Via::from_name)
        .ok_or_else(|| {
            unreadable(
                path,
                "the field `by.via` is missing or not one of app, cli, ssh, direct",
                None,
            )
        })?;
    Ok(StoppedBy {
        kind,
        via,
        machine: required_text(path, by, "machine")?,
        session: optional_text(path, by, "session")?,
    })
}

fn parse_ended(path: &Path, ended: &Value) -> Result<Ended, RecordError> {
    let count = |key: &str| -> Result<u32, RecordError> {
        ended
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| {
                unreadable(
                    path,
                    format!("the field `ended.{key}` is missing or not a count"),
                    None,
                )
            })
    };
    let left = ended
        .get("left")
        .and_then(Value::as_array)
        .and_then(|pids| {
            pids.iter()
                .map(|pid| pid.as_u64().and_then(|n| u32::try_from(n).ok()))
                .collect::<Option<Vec<u32>>>()
        })
        .ok_or_else(|| {
            unreadable(
                path,
                "the field `ended.left` is missing or not a list of process ids",
                None,
            )
        })?;
    Ok(Ended {
        agents: count("agents")?,
        shells: count("shells")?,
        killed: count("killed")?,
        left,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn full_record() -> Record {
        Record {
            stop_id: "stop-1".to_owned(),
            stopped_at_unix_ms: 1_700_000_000_000,
            by: StoppedBy {
                kind: ByKind::Agent,
                via: Via::Cli,
                machine: "box-a".to_owned(),
                session: Some("session-9".to_owned()),
            },
            reason: Some("runaway loop".to_owned()),
            ended: None,
        }
    }

    fn bare_record() -> Record {
        Record {
            stop_id: "stop-2".to_owned(),
            stopped_at_unix_ms: 0,
            by: StoppedBy {
                kind: ByKind::User,
                via: Via::App,
                machine: "this-mac".to_owned(),
                session: None,
            },
            reason: None,
            ended: None,
        }
    }

    fn ended() -> Ended {
        Ended {
            agents: 2,
            shells: 1,
            killed: 1,
            left: vec![4242],
        }
    }

    fn folder() -> (TempDir, StopFolder) {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        (dir, folder)
    }

    fn put(dir: &TempDir, name: &str, bytes: &[u8]) {
        let stop = dir.path().join("stop");
        fs::create_dir_all(&stop).expect("the stop folder");
        fs::write(stop.join(name), bytes).expect("the record file");
    }

    #[test]
    fn a_machine_with_no_stop_record_reads_as_running() {
        let (_dir, folder) = folder();
        assert_eq!(
            folder.read().expect("a readable folder"),
            StopState::Running
        );
    }

    #[test]
    fn a_written_stop_record_reads_back_equal() {
        let (_dir, folder) = folder();
        folder.write_stopped(&full_record()).expect("written");
        assert_eq!(
            folder.read().expect("readable"),
            StopState::Stopped(full_record())
        );
    }

    #[test]
    fn a_stop_record_without_optional_fields_reads_back_equal() {
        let (_dir, folder) = folder();
        folder.write_stopped(&bare_record()).expect("written");
        assert_eq!(
            folder.read().expect("readable"),
            StopState::Stopped(bare_record())
        );
    }

    #[test]
    fn settling_fills_in_what_the_stop_ended() {
        let (_dir, folder) = folder();
        folder.write_stopped(&bare_record()).expect("written");
        let settled = folder.settle(&ended()).expect("settled");
        assert_eq!(settled.ended, Some(ended()));
        assert_eq!(
            folder.read().expect("readable"),
            StopState::Stopped(settled)
        );
    }

    #[test]
    fn settling_without_a_stop_record_is_refused() {
        let (_dir, folder) = folder();
        assert!(matches!(
            folder.settle(&ended()),
            Err(RecordError::NotStopped { .. })
        ));
    }

    #[test]
    fn a_start_hands_back_the_stop_record_and_reads_as_starting() {
        let (_dir, folder) = folder();
        folder.write_stopped(&full_record()).expect("written");
        assert_eq!(folder.begin_start().expect("started"), full_record());
        assert_eq!(
            folder.read().expect("readable"),
            StopState::Starting(full_record())
        );
    }

    #[test]
    fn a_finished_start_reads_as_running_and_finishing_again_is_fine() {
        let (_dir, folder) = folder();
        folder.write_stopped(&full_record()).expect("written");
        folder.begin_start().expect("started");
        folder.finish_start().expect("finished");
        assert_eq!(folder.read().expect("readable"), StopState::Running);
        folder.finish_start().expect("finishing again is fine");
    }

    #[test]
    fn a_stop_the_machine_was_started_from_is_refused_when_it_comes_again() {
        let (_dir, folder) = folder();
        let record = full_record();
        folder
            .refuse_applied(&record.stop_id)
            .expect("none applied yet");
        folder.write_stopped(&record).expect("written");
        folder.begin_start().expect("started");
        folder
            .refuse_applied(&record.stop_id)
            .expect("a start still under way doesn't refuse it");
        folder.finish_start().expect("finished");
        assert!(matches!(
            folder.refuse_applied(&record.stop_id),
            Err(RecordError::AlreadyApplied { .. })
        ));
        folder
            .refuse_applied("another-stop")
            .expect("a different stop is taken");
    }

    #[test]
    fn starting_a_machine_that_is_not_stopped_is_refused() {
        let (_dir, folder) = folder();
        assert!(matches!(
            folder.begin_start(),
            Err(RecordError::NotStopped { .. })
        ));
    }

    #[test]
    fn a_stop_record_wins_over_a_start_record_left_beside_it() {
        let (_dir, folder) = folder();
        folder.write_stopped(&full_record()).expect("written");
        folder.begin_start().expect("started");
        folder.write_stopped(&bare_record()).expect("stopped again");
        assert_eq!(
            folder.read().expect("readable"),
            StopState::Stopped(bare_record())
        );
    }

    #[test]
    fn a_garbage_stop_record_is_unreadable() {
        let (dir, folder) = folder();
        put(&dir, STOPPED_FILE, b"\x00 not a stop record");
        assert!(matches!(folder.read(), Err(RecordError::Unreadable { .. })));
    }

    #[test]
    fn a_record_from_a_newer_format_is_unreadable() {
        let (dir, folder) = folder();
        put(
            &dir,
            STOPPED_FILE,
            br#"{"format":2,"stop_id":"x","stopped_at_unix_ms":0,"by":{"kind":"user","via":"app","machine":"m"}}"#,
        );
        assert!(matches!(folder.read(), Err(RecordError::Unreadable { .. })));
    }

    #[test]
    fn a_record_naming_an_unknown_stopper_is_unreadable() {
        let (dir, folder) = folder();
        put(
            &dir,
            STOPPED_FILE,
            br#"{"format":1,"stop_id":"x","stopped_at_unix_ms":0,"by":{"kind":"robot","via":"app","machine":"m"}}"#,
        );
        assert!(matches!(folder.read(), Err(RecordError::Unreadable { .. })));
    }

    fn holder(socket: &str, how: How) -> HolderEnd {
        HolderEnd {
            socket: PathBuf::from(socket),
            pid: 7,
            how,
        }
    }

    #[test]
    fn holders_ended_early_read_back_and_add_up_across_notes() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        assert!(folder.ended_early().is_empty());
        folder
            .note_ended_early(&[holder("/h/a.sock", How::Ended)])
            .expect("noted");
        folder
            .note_ended_early(&[holder("/h/b.sock", How::Killed)])
            .expect("noted");
        assert_eq!(
            folder.ended_early(),
            vec![
                holder("/h/a.sock", How::Ended),
                holder("/h/b.sock", How::Killed)
            ]
        );
        folder.clear_ended_early().expect("cleared");
        assert!(folder.ended_early().is_empty());
        folder
            .clear_ended_early()
            .expect("clearing nothing is fine");
    }

    #[test]
    fn a_damaged_note_of_holders_ended_early_names_none() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        folder
            .note_ended_early(&[holder("/h/a.sock", How::Ended)])
            .expect("noted");
        fs::write(dir.path().join("stop").join(ENDED_EARLY_FILE), b"{not json").expect("damaged");
        assert!(folder.ended_early().is_empty());
    }
}
