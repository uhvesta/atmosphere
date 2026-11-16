//! The machine's Humata trash (LD-6, LD-7, FM-3): one folder per item,
//! `<id>/item.json` and `<id>/content`.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use humata2_durable::{create_dir_all, rename, sync_dir};
use humata2_store::fault::point;
use serde::{Deserialize, Serialize};

use crate::error::TrashError;
use crate::fsutil::{make_dirs, remove_all, tree_size};

/// Items stay this long (LD-7).
pub const KEEP_MS: i64 = 30 * 24 * 60 * 60 * 1000;

const ITEM: &str = "item.json";
const CONTENT: &str = "content";
const DELETING: &str = ".deleting";

/// What the caller says about an item being deleted.
#[derive(Debug, Clone)]
pub struct NewItem {
    pub description: String,
    pub caller_data: String,
    pub now_unix_ms: i64,
}

/// One thing in the trash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashItem {
    pub id: String,
    pub original: PathBuf,
    pub description: String,
    pub bytes: u64,
    pub deleted_unix_ms: i64,
    pub caller_data: String,
}

/// What emptying now would remove and free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmptyPlan {
    pub ids: Vec<String>,
    pub bytes: u64,
}

pub struct Trash {
    folder: PathBuf,
    lock: Mutex<()>,
}

fn io(doing: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> TrashError {
    let path = path.to_owned();
    move |source| TrashError::Io { doing, path, source }
}

impl Trash {
    /// Opens (and makes, 0700) the trash folder, and removes what a crash
    /// left: items never moved in, and items being removed.
    pub fn open(folder: PathBuf) -> Result<Self, TrashError> {
        make_dirs(&folder, 0o700).map_err(io("create the trash folder", &folder))?;
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))
            .map_err(io("restrict the trash folder", &folder))?;
        if let Some(parent) = folder.parent() {
            let _ = sync_dir(parent);
        }
        let trash = Self {
            folder,
            lock: Mutex::new(()),
        };
        trash.sweep()?;
        Ok(trash)
    }

    fn guard(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn sweep(&self) -> Result<(), TrashError> {
        let entries = fs::read_dir(&self.folder).map_err(io("read the trash folder", &self.folder))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let leftover = name.ends_with(DELETING)
                || (path.is_dir() && fs::symlink_metadata(path.join(CONTENT)).is_err());
            if leftover {
                remove_all(&path).map_err(io("remove a half-made trash item", &path))?;
            }
        }
        Ok(())
    }

    fn item_folder(&self, id: &str) -> Result<PathBuf, TrashError> {
        if id.is_empty() || id.starts_with('.') || id.contains('/') {
            return Err(TrashError::NoSuchItem { id: id.to_owned() });
        }
        Ok(self.folder.join(id))
    }

    fn read_item(&self, id: &str) -> Result<TrashItem, TrashError> {
        let path = self.item_folder(id)?.join(ITEM);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(TrashError::NoSuchItem { id: id.to_owned() });
            }
            Err(error) => return Err(io("read the trash record", &path)(error)),
        };
        serde_json::from_slice(&bytes).map_err(|source| TrashError::Format { path, source })
    }

    /// Moves `path` into the trash.
    pub fn put(&self, path: &Path, item: NewItem) -> Result<TrashItem, TrashError> {
        let _guard = self.guard();
        let original = std::path::absolute(path).map_err(io("resolve", path))?;
        match fs::symlink_metadata(&original) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(TrashError::Missing { path: original });
            }
            Err(error) => return Err(io("look at", &original)(error)),
        }
        let bytes = tree_size(&original).map_err(io("measure", &original))?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let dir = self.folder.join(&id);
        let record = TrashItem {
            id: id.clone(),
            original: original.clone(),
            description: item.description,
            bytes,
            deleted_unix_ms: item.now_unix_ms,
            caller_data: item.caller_data,
        };
        let recorded = (|| {
            create_dir_all(&dir, 0o700).map_err(io("create a trash item", &dir))?;
            let file = dir.join(ITEM);
            let json = serde_json::to_vec(&record).map_err(|source| TrashError::Format {
                path: file.clone(),
                source,
            })?;
            humata2_store::write_atomic(&file, &json)
                .map_err(|source| TrashError::Record { path: file, source })?;
            sync_dir(&dir).map_err(io("save", &dir))?;
            sync_dir(&self.folder).map_err(io("save", &self.folder))
        })();
        if let Err(error) = recorded {
            let _ = remove_all(&dir);
            return Err(error);
        }
        point("trash.put.recorded");
        if let Err(source) = rename(&original, &dir.join(CONTENT)) {
            let _ = remove_all(&dir);
            return Err(if source.kind() == ErrorKind::CrossesDevices {
                TrashError::OtherDisk {
                    path: original,
                    source,
                }
            } else {
                io("move to the trash", &original)(source)
            });
        }
        if let Some(parent) = original.parent() {
            sync_dir(parent).map_err(io("save", parent))?;
        }
        sync_dir(&dir).map_err(io("save", &dir))?;
        point("trash.put.moved");
        Ok(record)
    }

    /// Everything in the trash, oldest first.
    pub fn list(&self) -> Result<Vec<TrashItem>, TrashError> {
        let _guard = self.guard();
        self.list_locked()
    }

    fn list_locked(&self) -> Result<Vec<TrashItem>, TrashError> {
        let entries = fs::read_dir(&self.folder).map_err(io("read the trash folder", &self.folder))?;
        let mut items = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with(DELETING) {
                continue;
            }
            let dir = entry.path();
            if fs::symlink_metadata(dir.join(CONTENT)).is_err() {
                continue;
            }
            if let Ok(item) = self.read_item(&name) {
                items.push(item);
            }
        }
        items.sort_by(|a, b| (a.deleted_unix_ms, &a.id).cmp(&(b.deleted_unix_ms, &b.id)));
        Ok(items)
    }

    /// Puts an item back where it was deleted from.
    pub fn restore(&self, id: &str) -> Result<PathBuf, TrashError> {
        let _guard = self.guard();
        let item = self.read_item(id)?;
        let dir = self.item_folder(id)?;
        if fs::symlink_metadata(&item.original).is_ok() {
            return Err(TrashError::Occupied {
                path: item.original,
            });
        }
        if let Some(parent) = item.original.parent() {
            make_dirs(parent, 0o755).map_err(io("create the folder", parent))?;
        }
        if let Err(source) = rename(&dir.join(CONTENT), &item.original) {
            return Err(if source.kind() == ErrorKind::CrossesDevices {
                TrashError::OtherDisk {
                    path: item.original,
                    source,
                }
            } else {
                io("restore", &item.original)(source)
            });
        }
        if let Some(parent) = item.original.parent() {
            sync_dir(parent).map_err(io("save", parent))?;
        }
        sync_dir(&dir).map_err(io("save", &dir))?;
        point("trash.restore.moved");
        self.remove_item(id)?;
        Ok(item.original)
    }

    /// Removes an item for good: renamed first (the commit point), then
    /// removed.
    fn remove_item(&self, id: &str) -> Result<(), TrashError> {
        let dir = self.item_folder(id)?;
        let marked = self.folder.join(format!("{id}{DELETING}"));
        rename(&dir, &marked).map_err(io("remove from the trash", &dir))?;
        sync_dir(&self.folder).map_err(io("save", &self.folder))?;
        point("trash.remove.marked");
        remove_all(&marked).map_err(io("remove from the trash", &marked))?;
        let _ = sync_dir(&self.folder);
        Ok(())
    }

    /// What emptying now would remove.
    pub fn plan_empty(&self) -> Result<EmptyPlan, TrashError> {
        let items = self.list()?;
        Ok(EmptyPlan {
            bytes: items.iter().map(|item| item.bytes).sum(),
            ids: items.into_iter().map(|item| item.id).collect(),
        })
    }

    /// Removes the plan's items that are still there; returns bytes freed.
    pub fn empty(&self, plan: &EmptyPlan) -> Result<u64, TrashError> {
        let _guard = self.guard();
        let mut freed = 0;
        for id in &plan.ids {
            let Ok(item) = self.read_item(id) else { continue };
            self.remove_item(id)?;
            freed += item.bytes;
        }
        Ok(freed)
    }

    /// Removes items deleted [`KEEP_MS`] or more before `now_unix_ms`.
    pub fn expire(&self, now_unix_ms: i64) -> Result<u64, TrashError> {
        let _guard = self.guard();
        let mut freed = 0;
        for item in self.list_locked()? {
            if now_unix_ms.saturating_sub(item.deleted_unix_ms) >= KEEP_MS {
                self.remove_item(&item.id)?;
                freed += item.bytes;
            }
        }
        Ok(freed)
    }
}
