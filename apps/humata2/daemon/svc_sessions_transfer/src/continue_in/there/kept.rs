//! The host's journal of the continues it asks another machine for (D554):
//! one file per host operation in the carry's `remote_archive/continue`
//! folder, written before the machine is asked and removed once the host
//! has recorded the new session, or once the machine made nothing or was
//! forgotten. A restart finds each file and asks the machine again under the
//! same key once its link is up, so the host records the session the
//! machine made, once. The file also names that session from the machine's
//! answer on, so a replay of the caller's key names it while it runs.

use std::path::{Path, PathBuf};

use humata2_store::StoreError;
use humata2_svc_sessions_common::pb;
use prost::Message;
use serde_json::{Value, json};

/// Why a continue's journal couldn't be kept on disk or taken away.
#[derive(Debug, thiserror::Error)]
pub enum KeptError {
    #[error("couldn't make the folder {path} for the continues that wait on another machine")]
    Folder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't write {path}, a continue that waits on another machine")]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
    #[error("couldn't remove {path}, a continue the host has finished with")]
    Remove {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't read {path}, a continue the host asked another machine for")]
    Unreadable {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}, a continue the host asked another machine for, isn't whole JSON (it may be cut short or overwritten)")]
    NotJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{path} is JSON but doesn't hold a continue the host asked another machine for")]
    NotAContinue { path: PathBuf },
    #[error("couldn't move {path} aside")]
    SetAside {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// What a restart needs to ask a machine again for a continue.
#[derive(Clone, Debug, PartialEq)]
pub struct Kept {
    /// The host's operation, which names the file.
    pub operation: String,
    pub machine: String,
    /// The channel the new session is in.
    pub channel: String,
    /// The ask, under the host operation's key.
    pub request: pb::ContinueInChannelRequest,
    /// The session the machine named in its answer; empty before.
    pub session: String,
}

impl Kept {
    fn value(&self) -> Value {
        json!({
            "operation": self.operation,
            "machine": self.machine,
            "channel": self.channel,
            "request": self.request.encode_to_vec(),
            "session": self.session,
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let request = pb::ContinueInChannelRequest::decode(bytes(value.get("request")?)?.as_slice()).ok()?;
        Some(Self {
            operation: text("operation")?,
            machine: text("machine")?,
            channel: text("channel")?,
            request,
            session: text("session")?,
        })
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

fn sync_dir(dir: &Path) -> std::io::Result<()> {
    let file = std::fs::File::open(dir)?;
    file.sync_all()
}

/// Where the continues wait, in the carry's `remote_archive` folder `root`.
pub fn folder_of(root: &Path) -> PathBuf {
    root.join("continue")
}

fn path_of(folder: &Path, operation: &str) -> PathBuf {
    folder.join(format!("{operation}.json"))
}

/// Keeps `kept` on disk in `folder`, the entry of each folder made for it
/// included, so a power cut right after can't lose it. Blocking.
pub fn write(folder: &Path, kept: &Kept) -> Result<(), KeptError> {
    let missing: Vec<&Path> = folder
        .ancestors()
        .take_while(|made| !made.as_os_str().is_empty() && !made.exists())
        .collect();
    let made = (|| {
        std::fs::create_dir_all(folder)?;
        for made in missing.iter().rev() {
            if let Some(parent) = made.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                sync_dir(parent)?;
            }
        }
        Ok(())
    })();
    made.map_err(|source| KeptError::Folder {
        path: folder.to_owned(),
        source,
    })?;
    let path = path_of(folder, &kept.operation);
    let bytes = serde_json::to_vec_pretty(&kept.value()).unwrap_or_default();
    humata2_store::write_atomic(&path, &bytes).map_err(|source| KeptError::Write { path, source })
}

/// Takes away the journal of `operation` and puts the removal on disk, so a
/// finished continue can't come back after a power cut. One already gone is
/// fine. Blocking.
pub fn remove(folder: &Path, operation: &str) -> Result<(), KeptError> {
    let path = path_of(folder, operation);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(KeptError::Remove { path, source }),
    }
    sync_dir(folder).map_err(|source| KeptError::Remove { path, source })
}

/// The journal of `operation`, if there is one that reads. Blocking.
pub fn read(folder: &Path, operation: &str) -> Option<Kept> {
    let bytes = std::fs::read(path_of(folder, operation)).ok()?;
    let value = serde_json::from_slice::<Value>(&bytes).ok()?;
    Kept::from_value(&value)
}

/// A journal that couldn't be read, and where it went.
#[derive(Debug)]
pub struct Unreadable {
    /// The host operation the file's name says it stood for.
    pub operation: String,
    pub problem: KeptError,
    /// Where the file was moved, or why it couldn't be (it stays).
    pub set_aside: Result<PathBuf, KeptError>,
}

/// What a restart found in the journal folder.
#[derive(Debug, Default)]
pub struct Found {
    pub kept: Vec<Kept>,
    pub unreadable: Vec<Unreadable>,
}

fn read_file(path: &Path) -> Result<Kept, KeptError> {
    let bytes = std::fs::read(path).map_err(|source| KeptError::Unreadable {
        path: path.to_owned(),
        source,
    })?;
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|source| KeptError::NotJson {
        path: path.to_owned(),
        source,
    })?;
    Kept::from_value(&value).ok_or_else(|| KeptError::NotAContinue { path: path.to_owned() })
}

/// Moves the journal at `path` to a `.bad` sibling in `folder` that no
/// read looks at (a name already taken gets a number), and puts the move
/// on disk. The contents are kept for the owner to look at.
fn set_aside(folder: &Path, path: &Path) -> Result<PathBuf, KeptError> {
    let fail = |source: std::io::Error| KeptError::SetAside {
        path: path.to_owned(),
        source,
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut aside = folder.join(format!("{name}.bad"));
    let mut number = 1u32;
    while aside.exists() {
        number += 1;
        aside = folder.join(format!("{name}.bad{number}"));
    }
    std::fs::rename(path, &aside).map_err(fail)?;
    sync_dir(folder).map_err(fail)?;
    Ok(aside)
}

/// Every continue the last run left. A file that can't be read is moved
/// aside (`.bad`) and named in the result, so it is neither asked again
/// at each start nor lost. Blocking.
pub fn read_all(folder: &Path) -> Found {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Found::default();
    };
    let mut found = Found::default();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") || !path.is_file() {
            continue;
        }
        match read_file(&path) {
            Ok(kept) => found.kept.push(kept),
            Err(problem) => {
                let operation = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let set_aside = set_aside(folder, &path);
                found.unreadable.push(Unreadable {
                    operation,
                    problem,
                    set_aside,
                });
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(operation: &str, session: &str) -> Kept {
        Kept {
            operation: operation.to_owned(),
            machine: "box-2".to_owned(),
            channel: "ch_billing".to_owned(),
            request: pb::ContinueInChannelRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: format!("host-continue-{operation}"),
                }),
                session: Some(pb::SessionId {
                    value: "se_source".to_owned(),
                }),
                channel: Some(pb::ChannelId {
                    value: "ch_billing".to_owned(),
                }),
                before_turn: 3,
            },
            session: session.to_owned(),
        }
    }

    #[test]
    fn a_kept_continue_reads_back_as_written_and_goes_when_removed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder_of(dir.path());
        let one = kept("op_1", "");
        write(&folder, &one).unwrap();
        assert_eq!(read_all(&folder).kept, vec![one.clone()]);
        assert_eq!(read(&folder, "op_1"), Some(one));
        remove(&folder, "op_1").unwrap();
        assert!(read_all(&folder).kept.is_empty());
        assert_eq!(read(&folder, "op_1"), None);
    }

    #[test]
    fn writing_again_keeps_the_session_the_machine_named() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder_of(dir.path());
        write(&folder, &kept("op_1", "")).unwrap();
        write(&folder, &kept("op_1", "se_new")).unwrap();
        let read = read_all(&folder).kept;
        assert_eq!(read.len(), 1, "one journal per operation: {read:?}");
        assert_eq!(read[0].session, "se_new");
    }

    #[test]
    fn removing_a_journal_already_gone_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder_of(dir.path());
        std::fs::create_dir_all(&folder).unwrap();
        assert!(remove(&folder, "op_gone").is_ok());
    }

    #[test]
    fn a_file_that_is_not_a_kept_continue_is_named_and_set_aside_with_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder_of(dir.path());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("op_2.json"), "not json").unwrap();
        std::fs::write(folder.join("op_4.json"), "{}").unwrap();
        write(&folder, &kept("op_3", "se_3")).unwrap();
        let found = read_all(&folder);
        assert_eq!(
            found.kept.iter().map(|kept| kept.operation.as_str()).collect::<Vec<_>>(),
            vec!["op_3"]
        );
        let mut named: Vec<&str> = found.unreadable.iter().map(|bad| bad.operation.as_str()).collect();
        named.sort_unstable();
        assert_eq!(named, vec!["op_2", "op_4"]);
        for bad in &found.unreadable {
            let aside = bad.set_aside.as_ref().expect("the file is moved aside");
            assert!(aside.exists(), "{aside:?} holds the file");
            assert!(!folder.join(format!("{}.json", bad.operation)).exists());
        }
        assert_eq!(std::fs::read_to_string(folder.join("op_2.json.bad")).unwrap(), "not json");
        // Nothing is met again at the next read.
        let again = read_all(&folder);
        assert!(again.unreadable.is_empty(), "{again:?}");
        assert_eq!(again.kept.len(), 1);
    }

    #[test]
    fn a_second_unreadable_file_of_one_operation_does_not_replace_the_first_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let folder = folder_of(dir.path());
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("op_2.json"), "first").unwrap();
        read_all(&folder);
        std::fs::write(folder.join("op_2.json"), "second").unwrap();
        read_all(&folder);
        assert_eq!(std::fs::read_to_string(folder.join("op_2.json.bad")).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(folder.join("op_2.json.bad2")).unwrap(), "second");
    }
}
