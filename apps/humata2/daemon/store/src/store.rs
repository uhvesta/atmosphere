use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::atomic::{create_dir, is_partial, remove_partials, write_atomic};
use crate::journal::Journal;
use crate::{Pointers, SessionRecord, StoreError, TabRecord};

/// The layout this build writes and reads.
pub const SCHEMA_VERSION: u32 = 1;

const SCHEMA: &str = "schema";
const SESSIONS: &str = "sessions";
const TABS: &str = "tabs";
const POINTERS: &str = "pointers";
const FILES: &str = "files";
const JOURNAL: &str = "journal";
const PASTE: &str = "paste";

static PASTES: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
struct Schema {
    version: u32,
}

/// One machine's state folder.
///
/// ```text
/// schema                    {"version":1}
/// sessions/<id>.json        SessionRecord
/// tabs/<id>.json            TabRecord
/// pointers/<session>.json   Pointers
/// files/<session>/paste/    images pasted into the session's terminals
/// journal/                  see Journal
/// ```
///
/// Folders are 0700 and files 0600.
pub struct Store {
    root: PathBuf,
    journal: Journal,
}

impl Store {
    /// Opens the state in `root`, making it if it's missing, and deletes
    /// what writes cut short by a crash left behind.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        create_dir(&root)?;
        remove_partials(&root)?;
        check_schema(&root)?;
        for folder in [SESSIONS, TABS, POINTERS, FILES] {
            let folder = root.join(folder);
            create_dir(&folder)?;
            remove_partials(&folder)?;
        }
        let journal = Journal::open(root.join(JOURNAL))?;
        Ok(Self { root, journal })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn journal(&self) -> &Journal {
        &self.journal
    }

    pub fn put_session(&self, session: &SessionRecord) -> Result<(), StoreError> {
        write_json(&self.record(SESSIONS, &session.id)?, session)
    }

    pub fn session(&self, id: &str) -> Result<Option<SessionRecord>, StoreError> {
        read_json(&self.record(SESSIONS, id)?)
    }

    /// Every session, oldest first.
    pub fn sessions(&self) -> Result<Vec<SessionRecord>, StoreError> {
        let mut sessions: Vec<SessionRecord> = list_json(&self.root.join(SESSIONS))?;
        sessions.sort_by(|a, b| (a.created_unix_ms, &a.id).cmp(&(b.created_unix_ms, &b.id)));
        Ok(sessions)
    }

    /// Removes a session with its tabs, pointers and pasted files. The
    /// record goes last, so a crash part way leaves it to remove again.
    pub fn remove_session(&self, id: &str) -> Result<(), StoreError> {
        let record = self.record(SESSIONS, id)?;
        for tab in self.tabs()? {
            if tab.session.as_deref() == Some(id) {
                self.remove_tab(&tab.id)?;
            }
        }
        remove_file(&self.record(POINTERS, id)?)?;
        let files = self.root.join(FILES).join(id);
        match fs::remove_dir_all(&files) {
            Err(source) if source.kind() != ErrorKind::NotFound => {
                return Err(StoreError::Remove {
                    path: files,
                    source,
                });
            }
            _ => {}
        }
        remove_file(&record)
    }

    pub fn put_tab(&self, tab: &TabRecord) -> Result<(), StoreError> {
        write_json(&self.record(TABS, &tab.id)?, tab)
    }

    pub fn tab(&self, id: &str) -> Result<Option<TabRecord>, StoreError> {
        read_json(&self.record(TABS, id)?)
    }

    /// Every tab, oldest first.
    pub fn tabs(&self) -> Result<Vec<TabRecord>, StoreError> {
        let mut tabs: Vec<TabRecord> = list_json(&self.root.join(TABS))?;
        tabs.sort_by(|a, b| (a.created_unix_ms, &a.id).cmp(&(b.created_unix_ms, &b.id)));
        Ok(tabs)
    }

    pub fn remove_tab(&self, id: &str) -> Result<(), StoreError> {
        remove_file(&self.record(TABS, id)?)
    }

    pub fn put_pointers(&self, session: &str, pointers: &Pointers) -> Result<(), StoreError> {
        write_json(&self.record(POINTERS, session)?, pointers)
    }

    pub fn pointers(&self, session: &str) -> Result<Option<Pointers>, StoreError> {
        read_json(&self.record(POINTERS, session)?)
    }

    /// Saves an image pasted into a session's terminal under the session's
    /// own folder, readable only by this user, and returns where. The file
    /// goes when the session does.
    pub fn save_paste(
        &self,
        session: &str,
        extension: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, StoreError> {
        check_id(extension)?;
        if self.session(session)?.is_none() {
            return Err(StoreError::NoSession {
                id: session.to_owned(),
            });
        }
        let folder = self.root.join(FILES).join(session).join(PASTE);
        create_dir(&folder)?;
        let path = folder.join(format!(
            "paste-{}-{}.{extension}",
            unix_ms(),
            PASTES.fetch_add(1, Ordering::Relaxed),
        ));
        write_atomic(&path, bytes)?;
        Ok(path)
    }

    fn record(&self, folder: &str, id: &str) -> Result<PathBuf, StoreError> {
        check_id(id)?;
        Ok(self.root.join(folder).join(format!("{id}.json")))
    }
}

fn check_schema(root: &Path) -> Result<(), StoreError> {
    let path = root.join(SCHEMA);
    match read_json::<Schema>(&path)? {
        None => write_json(
            &path,
            &Schema {
                version: SCHEMA_VERSION,
            },
        ),
        Some(schema) if schema.version > SCHEMA_VERSION => Err(StoreError::SchemaTooNew {
            path: root.to_owned(),
            found: schema.version,
            supported: SCHEMA_VERSION,
        }),
        Some(_) => Ok(()),
    }
}

/// Ids name files, so they keep to characters that can't climb out of
/// their folder or hide.
pub(crate) fn check_id(id: &str) -> Result<(), StoreError> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
    if id.is_empty() || id.len() > 128 || id.starts_with('.') || !id.chars().all(allowed) {
        return Err(StoreError::InvalidId { id: id.to_owned() });
    }
    Ok(())
}

pub(crate) fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

pub(crate) fn write_json(path: &Path, value: &impl Serialize) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|source| StoreError::Write {
        path: path.to_owned(),
        source: std::io::Error::other(source),
    })?;
    write_atomic(path, &bytes)
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StoreError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(StoreError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| StoreError::Corrupt {
            path: path.to_owned(),
            source,
        })
}

fn list_json<T: DeserializeOwned>(folder: &Path) -> Result<Vec<T>, StoreError> {
    let read_failed = |source| StoreError::Read {
        path: folder.to_owned(),
        source,
    };
    let mut found = Vec::new();
    for entry in fs::read_dir(folder).map_err(read_failed)? {
        let path = entry.map_err(read_failed)?.path();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        if is_partial(&name) || !name.ends_with(".json") {
            continue;
        }
        // Removed since the folder was listed: not an error.
        if let Some(record) = read_json(&path)? {
            found.push(record);
        }
    }
    Ok(found)
}

pub(crate) fn remove_file(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Err(source) if source.kind() != ErrorKind::NotFound => Err(StoreError::Remove {
            path: path.to_owned(),
            source,
        }),
        _ => Ok(()),
    }
}
