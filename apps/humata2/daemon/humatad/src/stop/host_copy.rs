//! The host's copy of each box's stop (D261, D262): the stop it asked for
//! and whether it reached the box. It lives on the host's disk, so the
//! machine page shows a box stopped while its link is down, and a stop
//! that reached the box neither way is sent the next time the box links,
//! across host restarts too.
//!
//! One file per machine in `<state>/stop/machines/<machine>.json`, written
//! durably as the box's own record is. A folder of their own keeps a box
//! named `this` off the host's own `this.json`.

use std::io;
use std::path::{Path, PathBuf};

use humata2_store::StoreError;

use super::{Record, RecordError};

/// A box's stop as the host keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HostCopy {
    pub(crate) record: Record,
    /// The box has taken the stop (its reply, or its Hello naming it).
    pub(crate) delivered: bool,
}

/// The host's copies, one per machine.
#[derive(Clone, Debug)]
pub(crate) struct HostCopies {
    folder: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum HostCopyError {
    #[error(
        "{machine:?} can't name a stop file: a machine id is a plain name, with no `/` and not `.` or `..`. Nothing was saved; this is a bug in Humata, so report it"
    )]
    BadMachine { machine: String },

    #[error(
        "couldn't create the folder {} for the host's copies of stops; check that its parent belongs to you and that the disk has room, then stop again",
        path.display()
    )]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "couldn't save the host's copy of a stop at {}; check that the disk has free space and that the folder belongs to you, then stop again",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },

    #[error(
        "couldn't read the host's copy of a stop at {}; check that it belongs to you and that the disk is mounted",
        path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "the host's copy of a stop at {} is unreadable. It is kept, and the machine shows as stopped; move the file aside only once you have checked on the machine itself whether its sessions are stopped",
        path.display()
    )]
    Unreadable {
        path: PathBuf,
        #[source]
        source: RecordError,
    },

    #[error(
        "couldn't remove the host's copy of a stop at {}; check that the folder belongs to you, then try again",
        path.display()
    )]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl HostCopies {
    /// The copies for the state folder `state`.
    pub(crate) fn new(state: &Path) -> Self {
        Self {
            folder: state.join("stop").join("machines"),
        }
    }

    /// `machine`'s copy, if the host keeps one.
    pub(crate) fn read(&self, machine: &str) -> Result<Option<HostCopy>, HostCopyError> {
        read_file(&self.path_of(machine)?)
    }

    /// Saves `copy` as `machine`'s, in place of any before it.
    pub(crate) fn write(&self, machine: &str, copy: &HostCopy) -> Result<(), HostCopyError> {
        let path = self.path_of(machine)?;
        if !self.folder.is_dir() {
            humata2_durable::create_dir_all(&self.folder, 0o700).map_err(|source| HostCopyError::CreateDir {
                path: self.folder.clone(),
                source,
            })?;
            // The folders made on the way are named in their parents.
            for made in self.folder.ancestors().take(2) {
                if let Some(parent) = made.parent() {
                    humata2_durable::sync_dir(parent).map_err(|source| HostCopyError::CreateDir {
                        path: parent.to_owned(),
                        source,
                    })?;
                }
            }
        }
        humata2_store::write_atomic(&path, &copy.to_json())
            .map_err(|source| HostCopyError::Write { path, source })?;
        Ok(())
    }

    /// Marks `machine`'s copy delivered when it is stop `stop_id`. True when
    /// that changed it; a copy of another stop, or none, is left as it is.
    pub(crate) fn mark_delivered(&self, machine: &str, stop_id: &str) -> Result<bool, HostCopyError> {
        let Some(mut copy) = self.read(machine)? else {
            return Ok(false);
        };
        if copy.delivered || copy.record.stop_id != stop_id {
            return Ok(false);
        }
        copy.delivered = true;
        self.write(machine, &copy)?;
        Ok(true)
    }

    /// Removes `machine`'s copy (Start, Cancel stop, remove or forget the
    /// machine). Absent is fine.
    pub(crate) fn remove(&self, machine: &str) -> Result<(), HostCopyError> {
        let path = self.path_of(machine)?;
        match humata2_durable::remove_file(&path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(HostCopyError::Remove { path, source }),
        }
        humata2_durable::sync_dir(&self.folder).map_err(|source| HostCopyError::Remove { path, source })
    }

    /// Every machine the host keeps a copy for, with it, by machine.
    pub(crate) fn all(&self) -> Result<Vec<(String, HostCopy)>, HostCopyError> {
        let entries = match std::fs::read_dir(&self.folder) {
            Ok(entries) => entries,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(HostCopyError::Read {
                    path: self.folder.clone(),
                    source,
                });
            }
        };
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| HostCopyError::Read {
                path: self.folder.clone(),
                source,
            })?;
            let path = entry.path();
            // A half-written file (`.<name>.json.<pid>-<n>.partial`) is no copy.
            let Some(machine) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
                .filter(|machine| plain(machine))
            else {
                continue;
            };
            if let Some(copy) = read_file(&path)? {
                found.push((machine.to_owned(), copy));
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(found)
    }

    /// The file of `machine`'s copy.
    fn path_of(&self, machine: &str) -> Result<PathBuf, HostCopyError> {
        if !plain(machine) {
            return Err(HostCopyError::BadMachine {
                machine: machine.to_owned(),
            });
        }
        Ok(self.folder.join(format!("{machine}.json")))
    }
}

/// Whether `machine` is a plain name: one path component that isn't `.`,
/// `..` or hidden, with no `/` or NUL.
fn plain(machine: &str) -> bool {
    !machine.is_empty() && !machine.starts_with('.') && !machine.contains(['/', '\0'])
}

fn read_file(path: &Path) -> Result<Option<HostCopy>, HostCopyError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(HostCopyError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let unreadable = |source| HostCopyError::Unreadable {
        path: path.to_owned(),
        source,
    };
    let record = Record::from_json(path, &bytes).map_err(unreadable)?;
    let delivered = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|value| value.get("delivered").and_then(serde_json::Value::as_bool))
        .ok_or_else(|| {
            unreadable(RecordError::Unreadable {
                path: path.to_owned(),
                reason: "the field `delivered` is missing or not true or false".to_owned(),
                source: None,
            })
        })?;
    Ok(Some(HostCopy { record, delivered }))
}

impl HostCopy {
    /// The file's bytes: the record's JSON with `delivered` beside it.
    fn to_json(&self) -> Vec<u8> {
        let mut value = serde_json::from_slice::<serde_json::Value>(&self.record.to_json())
            .unwrap_or(serde_json::Value::Null);
        if let Some(object) = value.as_object_mut() {
            object.insert("delivered".to_owned(), serde_json::Value::Bool(self.delivered));
        }
        format!("{value:#}").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::stop::{ByKind, Ended, StopFolder, StopState, StoppedBy, Via};

    fn record(stop_id: &str) -> Record {
        Record {
            stop_id: stop_id.to_owned(),
            stopped_at_unix_ms: 1_763_300_000_000,
            by: StoppedBy {
                kind: ByKind::User,
                via: Via::App,
                machine: "this-mac".to_owned(),
                session: None,
            },
            reason: Some("agents running wild".to_owned()),
            ended: None,
        }
    }

    fn copy(stop_id: &str, delivered: bool) -> HostCopy {
        HostCopy {
            record: record(stop_id),
            delivered,
        }
    }

    #[test]
    fn a_copy_reads_back_as_written_with_its_delivered_flag() {
        let dir = TempDir::new().expect("a temp dir");
        let copies = HostCopies::new(dir.path());
        for delivered in [false, true] {
            let mut written = copy("s1", delivered);
            written.record.ended = Some(Ended {
                agents: 2,
                shells: 1,
                killed: 1,
                left: vec![],
            });
            copies.write("devbox-1", &written).expect("saved");
            assert_eq!(copies.read("devbox-1").expect("readable"), Some(written));
        }
    }

    #[test]
    fn a_machine_with_no_copy_reads_as_none() {
        let dir = TempDir::new().expect("a temp dir");
        assert_eq!(HostCopies::new(dir.path()).read("devbox-1").expect("readable"), None);
    }

    #[test]
    fn a_copy_survives_being_opened_again() {
        let dir = TempDir::new().expect("a temp dir");
        HostCopies::new(dir.path()).write("devbox-1", &copy("s1", false)).expect("saved");
        assert_eq!(
            HostCopies::new(dir.path()).read("devbox-1").expect("readable"),
            Some(copy("s1", false))
        );
    }

    #[test]
    fn marking_the_stop_delivered_sets_the_flag_once() {
        let dir = TempDir::new().expect("a temp dir");
        let copies = HostCopies::new(dir.path());
        copies.write("devbox-1", &copy("s1", false)).expect("saved");
        assert!(copies.mark_delivered("devbox-1", "s1").expect("marked"));
        assert_eq!(copies.read("devbox-1").expect("readable"), Some(copy("s1", true)));
        assert!(!copies.mark_delivered("devbox-1", "s1").expect("marked again"));
    }

    #[test]
    fn marking_another_stop_delivered_changes_nothing() {
        let dir = TempDir::new().expect("a temp dir");
        let copies = HostCopies::new(dir.path());
        copies.write("devbox-1", &copy("s1", false)).expect("saved");
        assert!(!copies.mark_delivered("devbox-1", "s2").expect("asked"));
        assert!(!copies.mark_delivered("devbox-2", "s1").expect("asked"));
        assert_eq!(copies.read("devbox-1").expect("readable"), Some(copy("s1", false)));
        assert_eq!(copies.read("devbox-2").expect("readable"), None);
    }

    #[test]
    fn a_removed_copy_is_gone_and_removing_none_is_fine() {
        let dir = TempDir::new().expect("a temp dir");
        let copies = HostCopies::new(dir.path());
        copies.remove("devbox-1").expect("nothing to remove");
        copies.write("devbox-1", &copy("s1", true)).expect("saved");
        copies.remove("devbox-1").expect("removed");
        assert_eq!(copies.read("devbox-1").expect("readable"), None);
    }

    #[test]
    fn every_machine_with_a_copy_is_listed_with_it() {
        let dir = TempDir::new().expect("a temp dir");
        let copies = HostCopies::new(dir.path());
        assert!(copies.all().expect("listed").is_empty());
        copies.write("devbox-2", &copy("s2", true)).expect("saved");
        copies.write("devbox-1", &copy("s1", false)).expect("saved");
        let mut all = copies.all().expect("listed");
        all.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            all,
            vec![
                ("devbox-1".to_owned(), copy("s1", false)),
                ("devbox-2".to_owned(), copy("s2", true)),
            ]
        );
    }

    #[test]
    fn a_copy_for_a_machine_named_this_leaves_the_hosts_own_stop_alone() {
        let dir = TempDir::new().expect("a temp dir");
        let copies = HostCopies::new(dir.path());
        copies.write("this", &copy("s1", false)).expect("saved");
        copies.write("this.starting", &copy("s2", false)).expect("saved");
        assert_eq!(StopFolder::new(dir.path()).read().expect("readable"), StopState::Running);
        copies.remove("this").expect("removed");
        assert_eq!(StopFolder::new(dir.path()).read().expect("readable"), StopState::Running);
    }

    #[test]
    fn a_machine_id_that_is_not_a_plain_name_is_refused() {
        let dir = TempDir::new().expect("a temp dir");
        let copies = HostCopies::new(dir.path());
        for machine in ["", ".", "..", "../escape", "a/b"] {
            assert!(
                matches!(
                    copies.write(machine, &copy("s1", false)),
                    Err(HostCopyError::BadMachine { .. })
                ),
                "{machine:?} was taken"
            );
            assert!(
                matches!(copies.read(machine), Err(HostCopyError::BadMachine { .. })),
                "{machine:?} was read"
            );
        }
        assert!(!dir.path().join("escape.json").exists());
    }

    #[test]
    fn a_damaged_copy_is_an_error_not_a_missing_stop() {
        let dir = TempDir::new().expect("a temp dir");
        let copies = HostCopies::new(dir.path());
        copies.write("devbox-1", &copy("s1", false)).expect("saved");
        let path = copies.path_of("devbox-1").expect("a plain name");
        std::fs::write(&path, b"{ not json").expect("damaged");
        assert!(matches!(copies.read("devbox-1"), Err(HostCopyError::Unreadable { .. })));
        assert!(copies.all().is_err(), "a damaged copy isn't skipped as if no stop was kept");
    }
}
