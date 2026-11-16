//! The follow-ups of a remote archive or restore that wait for a machine's
//! link, kept on disk so a restart of the host goes on with them (D218,
//! D222). Each is one JSON file in the carry's `remote_archive` folder, written
//! before the wait starts and removed once the follow-up is done or found
//! moot. The start reads them all once, before anything is served.

use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use humata2_store::StoreError;
use prost::Message;
use serde_json::{Value, json};

use super::compare::{Restored, RestoredRepo, Turn};

/// A follow-up that waits for `machine`'s link.
#[derive(Clone, Debug)]
pub(super) enum Pending {
    /// The link was lost while `machine` archived `session`: learn whether
    /// it did, and write the host's archive row when it did.
    Reconcile {
        session: String,
        machine: String,
        /// What the machine last reported before its archive.
        reported: Vec<pb::Pointer>,
        by: pb::Actor,
        /// The machine's own archive operation, once it answered.
        asked: Option<String>,
    },
    /// `session` was restored on this Mac while `machine` was away: its
    /// copy there goes once the machine is back.
    Release {
        session: String,
        machine: String,
        channel: String,
        /// The restore's operation, which named what the machine staged.
        operation: String,
        /// What the restore brought here, which the machine's copy is
        /// compared with before it goes (D224). A journal from before has
        /// none, and its copy is kept.
        restored: Option<Restored>,
        /// What the machine's copy had more of at the last comparison, so
        /// it was kept.
        kept: Option<String>,
        /// The copy was compared and what it had more of carried here, so
        /// a retry only asks the machine to let go.
        cleared: bool,
    },
}

/// Why a follow-up couldn't be kept on disk.
#[derive(Debug, thiserror::Error)]
pub(super) enum PendingError {
    #[error("couldn't make the folder {path} for what waits on another machine's link")]
    Folder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't write {path}, what waits on another machine's link")]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
}

impl Pending {
    fn kind(&self) -> &'static str {
        match self {
            Self::Reconcile { .. } => "reconcile",
            Self::Release { .. } => "release",
        }
    }

    pub(super) fn session(&self) -> &str {
        match self {
            Self::Reconcile { session, .. } | Self::Release { session, .. } => session,
        }
    }

    fn value(&self) -> Value {
        match self {
            Self::Reconcile {
                session,
                machine,
                reported,
                by,
                asked,
            } => json!({
                "kind": self.kind(),
                "session": session,
                "machine": machine,
                "reported": reported.iter().map(|pointer| pointer.encode_to_vec()).collect::<Vec<_>>(),
                "by": by.encode_to_vec(),
                "asked": asked,
            }),
            Self::Release {
                session,
                machine,
                channel,
                operation,
                restored,
                kept,
                cleared,
            } => json!({
                "kind": self.kind(),
                "session": session,
                "machine": machine,
                "channel": channel,
                "operation": operation,
                "restored": restored.as_ref().map(restored_value),
                "kept": kept,
                "cleared": cleared,
            }),
        }
    }

    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        match value.get("kind").and_then(Value::as_str)? {
            "reconcile" => {
                let reported = value
                    .get("reported")?
                    .as_array()?
                    .iter()
                    .map(|one| bytes(one).and_then(|bytes| pb::Pointer::decode(bytes.as_slice()).ok()))
                    .collect::<Option<Vec<_>>>()?;
                let by = pb::Actor::decode(bytes(value.get("by")?)?.as_slice()).ok()?;
                Some(Self::Reconcile {
                    session: text("session")?,
                    machine: text("machine")?,
                    reported,
                    by,
                    asked: text("asked"),
                })
            }
            "release" => Some(Self::Release {
                session: text("session")?,
                machine: text("machine")?,
                channel: text("channel")?,
                operation: text("operation")?,
                // Missing or unreadable: unknown, so the copy is kept.
                restored: value.get("restored").and_then(restored_of),
                kept: text("kept"),
                cleared: value.get("cleared").and_then(Value::as_bool).unwrap_or(false),
            }),
            _ => None,
        }
    }
}

fn restored_value(restored: &Restored) -> Value {
    json!({
        "turns": restored.turns,
        "last_turn": restored.last_turn.as_ref().map(|turn| json!({
            "index": turn.index,
            "first_line": turn.first_line,
            "at": turn.at,
        })),
        "repos": restored.repos.iter().map(|repo| json!({
            "owner": repo.owner,
            "name": repo.name,
            "path": repo.path.display().to_string(),
            "sha": repo.sha,
        })).collect::<Vec<_>>(),
    })
}

fn restored_of(value: &Value) -> Option<Restored> {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let number = |value: &Value, key: &str| value.get(key).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok());
    let last_turn = match value.get("last_turn")? {
        Value::Null => None,
        turn => Some(Turn {
            index: number(turn, "index")?,
            first_line: text(turn, "first_line")?,
            at: text(turn, "at"),
        }),
    };
    let repos = value
        .get("repos")?
        .as_array()?
        .iter()
        .map(|repo| {
            Some(RestoredRepo {
                owner: text(repo, "owner")?,
                name: text(repo, "name")?,
                path: PathBuf::from(text(repo, "path")?),
                sha: text(repo, "sha")?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Restored {
        turns: number(value, "turns")?,
        last_turn,
        repos,
    })
}

/// Bytes kept as a JSON array of numbers.
fn bytes(value: &Value) -> Option<Vec<u8>> {
    value
        .as_array()?
        .iter()
        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
        .collect()
}

/// `pending`'s file in `folder`, the carry's `remote_archive` folder.
fn path_of(folder: &Path, pending: &Pending) -> PathBuf {
    folder.join(format!("{}.{}.json", pending.session(), pending.kind()))
}

/// Keeps `pending` on disk in `folder`. Blocking.
pub(super) fn write(folder: &Path, pending: &Pending) -> Result<(), PendingError> {
    std::fs::create_dir_all(folder).map_err(|source| PendingError::Folder {
        path: folder.to_owned(),
        source,
    })?;
    let path = path_of(folder, pending);
    let bytes = serde_json::to_vec_pretty(&pending.value()).unwrap_or_default();
    humata2_store::write_atomic(&path, &bytes).map_err(|source| PendingError::Write { path, source })
}

/// `pending` is done or moot; its file goes. Blocking.
pub(super) fn remove(folder: &Path, pending: &Pending) {
    let _ = std::fs::remove_file(path_of(folder, pending));
}

/// The conversation a restore here is about to resume, kept beside its
/// follow-ups: `<session>.resume` holds the folder it was written for, a
/// newline, then the lines.
fn resume_path(folder: &Path, session: &str) -> PathBuf {
    folder.join(format!("{session}.resume"))
}

/// Keeps the conversation `lines`, written for `old_folder`. Blocking.
pub(super) fn write_resume(folder: &Path, session: &str, old_folder: &str, lines: &[u8]) -> Result<(), PendingError> {
    std::fs::create_dir_all(folder).map_err(|source| PendingError::Folder {
        path: folder.to_owned(),
        source,
    })?;
    let path = resume_path(folder, session);
    let mut bytes = old_folder.as_bytes().to_vec();
    bytes.push(b'\n');
    bytes.extend_from_slice(lines);
    humata2_store::write_atomic(&path, &bytes).map_err(|source| PendingError::Write { path, source })
}

/// The kept conversation of `session` is done or moot. Blocking.
pub(super) fn remove_resume(folder: &Path, session: &str) {
    let _ = std::fs::remove_file(resume_path(folder, session));
}

/// Every kept conversation: the session, the folder it was written for and
/// the lines. Blocking.
pub(super) fn read_resumes(folder: &Path) -> Vec<(String, String, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("resume") || !path.is_file() {
            continue;
        }
        let (Some(session), Ok(bytes)) = (path.file_stem().and_then(|stem| stem.to_str()), std::fs::read(&path)) else {
            continue;
        };
        let Some(at) = bytes.iter().position(|byte| *byte == b'\n') else {
            continue;
        };
        let old_folder = String::from_utf8_lossy(&bytes[..at]).into_owned();
        found.push((session.to_owned(), old_folder, bytes[at + 1..].to_vec()));
    }
    found
}

/// Every follow-up the last run left. A file that can't be read is logged
/// and left in place. Blocking.
pub(super) fn read_all(folder: &Path) -> Vec<Pending> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        let read = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .and_then(|value| Pending::from_value(&value));
        match read {
            Some(pending) => found.push(pending),
            None => crate::log::say!(
                "humatad2: couldn't read {}, a remote archive's or restore's follow-up, so it isn't done. Nothing was deleted; look at the session on its machine, then delete the file",
                path.display()
            ),
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(pending: &Pending) -> Pending {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        write(&store, pending).unwrap();
        let mut read = read_all(&store);
        assert_eq!(read.len(), 1);
        remove(&store, pending);
        assert!(read_all(&store).is_empty());
        read.remove(0)
    }

    #[test]
    fn a_reconcile_reads_back_as_written() {
        let pointer = pb::Pointer {
            head_sha: "abc".to_owned(),
            pushed: true,
            unpushed_subjects: vec!["Warm the cache".to_owned()],
            ..Default::default()
        };
        let by = pb::Actor::default();
        let Pending::Reconcile {
            session,
            machine,
            reported,
            by: read_by,
            asked,
        } = kept(&Pending::Reconcile {
            session: "se_1".to_owned(),
            machine: "box-1".to_owned(),
            reported: vec![pointer.clone()],
            by: by.clone(),
            asked: Some("op_9".to_owned()),
        })
        else {
            panic!("a reconcile reads back as a reconcile");
        };
        assert_eq!((session.as_str(), machine.as_str()), ("se_1", "box-1"));
        assert_eq!(reported, vec![pointer]);
        assert_eq!(read_by, by);
        assert_eq!(asked.as_deref(), Some("op_9"));
    }

    fn restored() -> Restored {
        Restored {
            turns: 2,
            last_turn: Some(Turn {
                index: 2,
                first_line: "Now the tests".to_owned(),
                at: Some("2026-11-16T10:05:00Z".to_owned()),
            }),
            repos: vec![RestoredRepo {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                path: PathBuf::from("/work/se_1/api"),
                sha: "1111111111111111111111111111111111111111".to_owned(),
            }],
        }
    }

    fn release(restored: Option<Restored>, kept: Option<String>) -> Pending {
        Pending::Release {
            session: "se_1".to_owned(),
            machine: "box-1".to_owned(),
            channel: "ch_1".to_owned(),
            operation: "op_1".to_owned(),
            restored,
            kept,
            cleared: false,
        }
    }

    #[test]
    fn a_release_reads_back_as_written() {
        let Pending::Release {
            session,
            machine,
            channel,
            operation,
            restored: read,
            kept: was_kept,
            ..
        } = kept(&release(Some(restored()), Some("1 later turn(s)".to_owned())))
        else {
            panic!("a release reads back as a release");
        };
        assert_eq!(
            [session, machine, channel, operation],
            ["se_1", "box-1", "ch_1", "op_1"].map(str::to_owned)
        );
        assert_eq!(read, Some(restored()));
        assert_eq!(was_kept.as_deref(), Some("1 later turn(s)"));
    }

    #[test]
    fn a_release_with_no_turns_restored_reads_back_as_written() {
        let none = Restored {
            turns: 0,
            last_turn: None,
            repos: Vec::new(),
        };
        let Pending::Release { restored: read, .. } = kept(&release(Some(none.clone()), None)) else {
            panic!("a release reads back as a release");
        };
        assert_eq!(read, Some(none));
    }

    #[test]
    fn a_release_kept_before_the_comparison_reads_with_nothing_restored() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        std::fs::create_dir_all(&store).unwrap();
        let old = json!({
            "kind": "release",
            "session": "se_1",
            "machine": "box-1",
            "channel": "ch_1",
            "operation": "op_1",
        });
        std::fs::write(store.join("se_1.release.json"), old.to_string()).unwrap();
        let read = read_all(&store);
        assert!(
            matches!(read.as_slice(), [Pending::Release { restored: None, kept: None, .. }]),
            "{read:?}"
        );
    }

    #[test]
    fn a_cleared_release_reads_back_cleared_and_an_old_one_does_not() {
        let Pending::Release { cleared, .. } = kept(&Pending::Release {
            session: "se_1".to_owned(),
            machine: "box-1".to_owned(),
            channel: "ch_1".to_owned(),
            operation: "op_1".to_owned(),
            restored: None,
            kept: None,
            cleared: true,
        }) else {
            panic!("a release reads back as a release");
        };
        assert!(cleared);
        let Pending::Release { cleared, .. } = kept(&release(None, None)) else {
            panic!("a release reads back as a release");
        };
        assert!(!cleared);
    }

    #[test]
    fn a_reconcile_and_a_release_of_one_session_are_kept_apart() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let release = release(None, None);
        let reconcile = Pending::Reconcile {
            session: "se_1".to_owned(),
            machine: "box-1".to_owned(),
            reported: Vec::new(),
            by: pb::Actor::default(),
            asked: None,
        };
        write(&store, &release).unwrap();
        write(&store, &reconcile).unwrap();
        remove(&store, &release);
        let left = read_all(&store);
        assert!(matches!(left.as_slice(), [Pending::Reconcile { .. }]), "{left:?}");
    }
}

#[cfg(test)]
mod resume_tests {
    use super::*;

    #[test]
    fn a_kept_conversation_is_read_back_with_its_folder_and_lines() {
        let dir = tempfile::tempdir().unwrap();
        let lines = b"{\"a\":1}\n{\"b\":2}\n".to_vec();
        write_resume(dir.path(), "se_1", "/old/folder", &lines).unwrap();
        assert_eq!(read_resumes(dir.path()), vec![("se_1".to_owned(), "/old/folder".to_owned(), lines)]);
        remove_resume(dir.path(), "se_1");
        assert!(read_resumes(dir.path()).is_empty());
    }
}
