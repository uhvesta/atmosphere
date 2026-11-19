//! The upgrade record on disk (UP-1, D281 g).
//!
//! `<state>/upgrade/this.json` says an upgrade of this daemon was started
//! and has not reached ready: it is written before anything is swapped, its
//! boot count goes up each time a daemon starts with it there, and the
//! daemon that runs the release it names settles it at ready by renaming it
//! to `last.json`, which the machine page reads. Each change is durable (a
//! file is written whole, and a rename is followed by a sync of the
//! folder), so a power cut leaves the record as it was or as it is.
//!
//! A build that never reaches ready, or an upgrade cut before its release
//! was whole, is undone by `rollback.rs`: it reads the boot count this file
//! keeps and ends the record with [`UpgradeFolder::end`], which leaves
//! `last.json` saying the upgrade did not take, until the Migrate item has
//! been told.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_daemon_runtime::fault;
use humata2_store::StoreError;
use serde::{Deserialize, Serialize};

use super::build::Release;

/// The only record format this Humata writes and reads.
const FORMAT: u64 = 1;
const THIS_FILE: &str = "this.json";
const LAST_FILE: &str = "last.json";

/// One upgrade: what it moved from and to, and how many starts it has had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub upgrade_id: String,
    pub from_version: String,
    pub from_bundle: String,
    pub to_bundle: String,
    pub started_unix_ms: i64,
    /// How many times a daemon has started with this record in place,
    /// counted before it does anything else.
    pub boots: u32,
}

/// What `this.json` and `last.json` hold, as JSON.
#[derive(Serialize, Deserialize)]
struct Stored {
    format: u64,
    upgrade_id: String,
    from_version: String,
    from_bundle: String,
    to_bundle: String,
    started_unix_ms: i64,
    boots: u32,
    /// Set once the new daemon is ready: when, and which version it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settled: Option<Settled>,
    /// Set when the upgrade was undone instead of settled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ended: Option<Ended>,
}

#[derive(Serialize, Deserialize)]
struct Settled {
    unix_ms: i64,
    to_version: String,
}

#[derive(Serialize, Deserialize)]
struct Ended {
    unix_ms: i64,
    result: EndKind,
    /// Whether the Migrate item has been told that this upgrade didn't take.
    #[serde(default)]
    reported: bool,
}

/// How an upgrade that did not settle ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndKind {
    /// The new build started but never reached ready: the old release and
    /// the old copy of the store were put back.
    RolledBack,
    /// The swap was cut before the new release was whole: the old release
    /// was put back whole.
    Abandoned,
}

impl Stored {
    fn of(record: &Record, settled: Option<Settled>) -> Self {
        Self {
            format: FORMAT,
            upgrade_id: record.upgrade_id.clone(),
            from_version: record.from_version.clone(),
            from_bundle: record.from_bundle.clone(),
            to_bundle: record.to_bundle.clone(),
            started_unix_ms: record.started_unix_ms,
            boots: record.boots,
            settled,
            ended: None,
        }
    }

    fn record(self) -> Record {
        Record {
            upgrade_id: self.upgrade_id,
            from_version: self.from_version,
            from_bundle: self.from_bundle,
            to_bundle: self.to_bundle,
            started_unix_ms: self.started_unix_ms,
            boots: self.boots,
        }
    }
}

/// Why the record couldn't be read, written or settled. Each message says
/// what to do; the cause is its source.
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    #[error(
        "couldn't make the folder {} for the upgrade record; nothing was changed. Make sure your user can write to it and the disk has room, then migrate again",
        .path.display()
    )]
    Folder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't save the upgrade record {}; check that your user can write there and the disk has room. If this happened before the daemon was replaced, nothing changed and you can migrate again",
        .path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },

    #[error(
        "couldn't encode the upgrade record {}; this is a bug in Humata, so report it. Nothing was changed",
        .path.display()
    )]
    Encode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "couldn't read the upgrade record {}; make the file readable by your user. Don't delete it unless you know no upgrade is under way: it is how the daemon finishes or undoes one",
        .path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the upgrade record {} isn't a record this Humata wrote; if no upgrade is under way you can remove it, otherwise install the Humata release that wrote it",
        .path.display()
    )]
    Invalid {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "the upgrade record {} is format {found}, and this Humata reads format {FORMAT}; install the Humata release that wrote it",
        .path.display()
    )]
    Format { path: PathBuf, found: u64 },

    #[error(
        "couldn't move the upgrade record {} to {}; the new daemon runs, but the machine page won't show that it was upgraded until this is fixed. Check that your user can write in that folder",
        .from.display(),
        .to.display()
    )]
    Settle {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't move the upgrade record {} to {}; the old release is back in place, but the daemon can't finish undoing the upgrade until this works. Check that your user can write in that folder, then start humatad2 again",
        .from.display(),
        .to.display()
    )]
    End {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// The `<state>/upgrade` folder and the record files in it.
#[derive(Clone, Debug)]
pub struct UpgradeFolder {
    folder: PathBuf,
}

impl UpgradeFolder {
    /// The folder for the state folder `state`: `<state>/upgrade`.
    pub fn new(state: &Path) -> Self {
        Self {
            folder: state.join("upgrade"),
        }
    }

    fn this_path(&self) -> PathBuf {
        self.folder.join(THIS_FILE)
    }

    /// The state folder this record folder is in.
    pub fn state(&self) -> &Path {
        self.folder.parent().unwrap_or(&self.folder)
    }

    fn last_path(&self) -> PathBuf {
        self.folder.join(LAST_FILE)
    }

    /// The upgrade under way, if there is one. A record that can't be read
    /// is an error: the caller must not go on as if there were none.
    pub fn read(&self) -> Result<Option<Record>, RecordError> {
        Ok(read_at(&self.this_path())?.map(Stored::record))
    }

    /// Records that `record`'s upgrade has started, creating the folder if
    /// it is missing. Written before anything is swapped.
    pub fn write_started(&self, record: &Record) -> Result<(), RecordError> {
        humata2_durable::create_dir_all(&self.folder, 0o700).map_err(|source| RecordError::Folder {
            path: self.folder.clone(),
            source,
        })?;
        write_at(&self.this_path(), &Stored::of(record, None))?;
        fault::point("upgrade.record.written");
        Ok(())
    }

    /// Counts this start against the upgrade under way and returns the
    /// count, fsynced before anything else runs. `None` when none is.
    pub fn count_boot(&self) -> Result<Option<u32>, RecordError> {
        let path = self.this_path();
        let Some(mut stored) = read_at(&path)? else {
            return Ok(None);
        };
        stored.boots = stored.boots.saturating_add(1);
        write_at(&path, &stored)?;
        fault::point("upgrade.boot.counted");
        Ok(Some(stored.boots))
    }

    /// This daemon is ready. When the upgrade under way is to the release
    /// it runs, the record is settled: it becomes `last.json`, with when
    /// and as which version. Returns that record. `None` when no upgrade is
    /// under way, or when it is to another release (the build that was
    /// asked for never started, which a rollback handles).
    pub fn settle(&self, release: &Release) -> Result<Option<Record>, RecordError> {
        let path = self.this_path();
        let Some(mut stored) = read_at(&path)? else {
            return Ok(None);
        };
        if stored.to_bundle != release.bundle() {
            return Ok(None);
        }
        fault::point("upgrade.ready");
        stored.settled = Some(Settled {
            unix_ms: unix_ms(SystemTime::now()),
            to_version: release.info().version,
        });
        write_at(&path, &stored)?;
        let last = self.last_path();
        let renamed = humata2_durable::rename(&path, &last)
            .and_then(|()| humata2_durable::sync_dir(&self.folder));
        renamed.map_err(|source| RecordError::Settle {
            from: path,
            to: last,
            source,
        })?;
        fault::point("upgrade.record.settled");
        Ok(Some(stored.record()))
    }

    /// The upgrade under way did not take, and what it changed has been put
    /// back: its record becomes `last.json`, marked as `kind` and not yet
    /// told to the Migrate item. Returns that record; `None` when no upgrade
    /// is under way, so running it again after it finished does nothing.
    pub fn end(&self, kind: EndKind) -> Result<Option<Record>, RecordError> {
        let path = self.this_path();
        let Some(mut stored) = read_at(&path)? else {
            return Ok(None);
        };
        stored.ended = Some(Ended {
            unix_ms: unix_ms(SystemTime::now()),
            result: kind,
            reported: false,
        });
        write_at(&path, &stored)?;
        let last = self.last_path();
        let renamed = humata2_durable::rename(&path, &last)
            .and_then(|()| humata2_durable::sync_dir(&self.folder));
        renamed.map_err(|source| RecordError::End {
            from: path,
            to: last,
            source,
        })?;
        Ok(Some(stored.record()))
    }

    /// The upgrade id of the last upgrade that finished, settled or not.
    pub fn last_id(&self) -> Result<Option<String>, RecordError> {
        Ok(read_at(&self.last_path())?.map(|stored| stored.upgrade_id))
    }

    /// The last upgrade, when it ended without settling and the Migrate
    /// item hasn't been told yet.
    pub fn unreported_end(&self) -> Result<Option<(Record, EndKind)>, RecordError> {
        let Some(stored) = read_at(&self.last_path())? else {
            return Ok(None);
        };
        let ended = match &stored.ended {
            Some(ended) if !ended.reported => ended.result,
            _ => return Ok(None),
        };
        Ok(Some((stored.record(), ended)))
    }

    /// Notes that the Migrate item has been told how the last upgrade ended.
    pub fn mark_reported(&self) -> Result<(), RecordError> {
        let path = self.last_path();
        let Some(mut stored) = read_at(&path)? else {
            return Ok(());
        };
        if let Some(ended) = stored.ended.as_mut() {
            ended.reported = true;
            write_at(&path, &stored)?;
        }
        Ok(())
    }
}

fn read_at(path: &Path) -> Result<Option<Stored>, RecordError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(RecordError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let stored: Stored = serde_json::from_slice(&bytes).map_err(|source| RecordError::Invalid {
        path: path.to_owned(),
        source,
    })?;
    if stored.format != FORMAT {
        return Err(RecordError::Format {
            path: path.to_owned(),
            found: stored.format,
        });
    }
    Ok(Some(stored))
}

fn write_at(path: &Path, stored: &Stored) -> Result<(), RecordError> {
    let bytes = serde_json::to_vec_pretty(stored).map_err(|source| RecordError::Encode {
        path: path.to_owned(),
        source,
    })?;
    humata2_store::write_atomic(path, &bytes).map_err(|source| RecordError::Write {
        path: path.to_owned(),
        source,
    })
}

fn unix_ms(at: SystemTime) -> i64 {
    at.duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

/// Now, as a record's start time.
pub fn now_unix_ms() -> i64 {
    unix_ms(SystemTime::now())
}

#[cfg(test)]
mod tests {
    use humata2_machines::Platform;
    use sha2::{Digest, Sha256};

    use super::*;

    const DAEMON: &[u8] = b"a daemon";

    /// A release whose bundle names `DAEMON` as this platform's humatad2:
    /// the daemon is the file `humatad2` in the returned folder.
    fn release() -> (tempfile::TempDir, Release) {
        let dir = tempfile::tempdir().expect("a temp folder");
        let exe = dir.path().join("humatad2");
        std::fs::write(&exe, DAEMON).expect("the daemon is written");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("bin is made");
        let sha = Sha256::digest(DAEMON).iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let entry = serde_json::json!({
            "humatad2": {"path": "humatad2", "sha256": sha},
            "humata2_holder": {"path": "holder", "sha256": sha},
            "humata": {"path": "humata", "sha256": sha},
        });
        let platform = Platform::host().expect("a platform Humata ships").bundle_name();
        let manifest = serde_json::json!({"platforms": {platform: entry}});
        std::fs::write(bin.join("bundle.json"), manifest.to_string()).expect("bundle.json is written");
        let release = Release::read(&bin, &exe, "2.0.5", 3);
        (dir, release)
    }

    fn record(to_bundle: &str) -> Record {
        Record {
            upgrade_id: "up_1".to_owned(),
            from_version: "2.0.4".to_owned(),
            from_bundle: "a".repeat(64),
            to_bundle: to_bundle.to_owned(),
            started_unix_ms: 1_000,
            boots: 0,
        }
    }

    #[test]
    fn a_started_upgrade_reads_back_as_written() {
        let state = tempfile::tempdir().expect("a state folder");
        let folder = UpgradeFolder::new(state.path());
        assert_eq!(folder.read().expect("no record reads"), None);

        folder.write_started(&record("b")).expect("the record is written");

        assert_eq!(folder.read().expect("the record reads"), Some(record("b")));
    }

    #[test]
    fn each_start_with_an_upgrade_under_way_is_counted_and_with_none_nothing_is() {
        let state = tempfile::tempdir().expect("a state folder");
        let folder = UpgradeFolder::new(state.path());
        assert_eq!(folder.count_boot().expect("counts"), None);
        folder.write_started(&record("b")).expect("the record is written");

        assert_eq!(folder.count_boot().expect("counts"), Some(1));
        assert_eq!(folder.count_boot().expect("counts"), Some(2));

        assert_eq!(folder.read().expect("reads").map(|record| record.boots), Some(2));
    }

    #[test]
    fn the_release_an_upgrade_names_settles_it_into_last_json() {
        let (_dir, release) = release();
        let state = tempfile::tempdir().expect("a state folder");
        let folder = UpgradeFolder::new(state.path());
        folder
            .write_started(&record(release.bundle()))
            .expect("the record is written");

        let settled = folder.settle(&release).expect("settles");

        assert_eq!(settled.map(|record| record.upgrade_id), Some("up_1".to_owned()));
        assert_eq!(folder.read().expect("reads"), None);
        let last = std::fs::read_to_string(state.path().join("upgrade/last.json")).expect("last.json is there");
        assert!(last.contains(release.bundle()), "{last}");
    }

    #[test]
    fn another_release_leaves_the_record_for_a_rollback() {
        let (_dir, release) = release();
        let state = tempfile::tempdir().expect("a state folder");
        let folder = UpgradeFolder::new(state.path());
        folder.write_started(&record("not this release")).expect("the record is written");

        assert_eq!(folder.settle(&release).expect("settles"), None);

        assert!(folder.read().expect("reads").is_some());
        assert!(!state.path().join("upgrade/last.json").exists());
    }

    #[test]
    fn a_record_this_humata_cannot_read_is_an_error_not_an_absent_record() {
        let state = tempfile::tempdir().expect("a state folder");
        let folder = UpgradeFolder::new(state.path());
        folder.write_started(&record("b")).expect("the record is written");
        std::fs::write(state.path().join("upgrade/this.json"), b"{ not json").expect("overwritten");

        assert!(matches!(folder.read(), Err(RecordError::Invalid { .. })));
        assert!(matches!(folder.count_boot(), Err(RecordError::Invalid { .. })));
    }

    #[test]
    fn a_record_of_another_format_is_refused() {
        let state = tempfile::tempdir().expect("a state folder");
        let folder = UpgradeFolder::new(state.path());
        folder.write_started(&record("b")).expect("the record is written");
        let path = state.path().join("upgrade/this.json");
        let text = std::fs::read_to_string(&path).expect("reads").replace("\"format\": 1", "\"format\": 9");
        std::fs::write(&path, text).expect("overwritten");

        assert!(matches!(folder.read(), Err(RecordError::Format { found: 9, .. })));
    }

    #[test]
    fn an_ended_upgrade_becomes_last_json_until_it_is_reported_and_ending_again_does_nothing() {
        let state = tempfile::tempdir().expect("a state folder");
        let folder = UpgradeFolder::new(state.path());
        folder.write_started(&record("b")).expect("the record is written");

        let ended = folder.end(EndKind::RolledBack).expect("ends");

        assert_eq!(ended.map(|record| record.upgrade_id), Some("up_1".to_owned()));
        assert_eq!(folder.read().expect("reads"), None, "nothing is under way any more");
        assert_eq!(folder.last_id().expect("reads"), Some("up_1".to_owned()));
        let told = folder.unreported_end().expect("reads");
        assert_eq!(told.map(|(record, kind)| (record.upgrade_id, kind)), Some(("up_1".to_owned(), EndKind::RolledBack)));
        assert_eq!(folder.end(EndKind::RolledBack).expect("ends again"), None);

        folder.mark_reported().expect("marks");

        assert_eq!(folder.unreported_end().expect("reads"), None);
        assert_eq!(folder.last_id().expect("reads"), Some("up_1".to_owned()), "last.json is kept");
    }

    #[test]
    fn a_settled_upgrade_has_nothing_to_report() {
        let (_dir, release) = release();
        let state = tempfile::tempdir().expect("a state folder");
        let folder = UpgradeFolder::new(state.path());
        folder.write_started(&record(release.bundle())).expect("the record is written");
        folder.settle(&release).expect("settles");

        assert_eq!(folder.unreported_end().expect("reads"), None);
    }
}
