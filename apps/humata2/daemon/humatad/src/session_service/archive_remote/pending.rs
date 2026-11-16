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
            } => json!({
                "kind": self.kind(),
                "session": session,
                "machine": machine,
                "channel": channel,
                "operation": operation,
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
            }),
            _ => None,
        }
    }
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

    #[test]
    fn a_release_reads_back_as_written() {
        let Pending::Release {
            session,
            machine,
            channel,
            operation,
        } = kept(&Pending::Release {
            session: "se_1".to_owned(),
            machine: "box-1".to_owned(),
            channel: "ch_1".to_owned(),
            operation: "op_1".to_owned(),
        })
        else {
            panic!("a release reads back as a release");
        };
        assert_eq!(
            [session, machine, channel, operation],
            ["se_1", "box-1", "ch_1", "op_1"].map(str::to_owned)
        );
    }

    #[test]
    fn a_reconcile_and_a_release_of_one_session_are_kept_apart() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let release = Pending::Release {
            session: "se_1".to_owned(),
            machine: "box-1".to_owned(),
            channel: "ch_1".to_owned(),
            operation: "op_1".to_owned(),
        };
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
