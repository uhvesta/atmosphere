//! Saving chosen parts of the state, such as the layout or which tabs are
//! open, and restoring them when the app starts again.

use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::clock::Clock;
use crate::effect::Effect;
use crate::reducer::Reducer;

/// Where saved state is kept: bytes under a key. Its calls block, so keep
/// what's saved small.
pub trait Storage: Send + Sync + 'static {
    /// What was saved under `key`, or None if nothing was.
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError>;

    /// Keeps `bytes` under `key`, in place of what was there.
    fn save(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError>;

    /// Forgets what's under `key`; nothing there is no error.
    fn remove(&self, key: &str) -> Result<(), StorageError>;
}

/// Why storage couldn't load, save or remove.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Keys name files, so they may hold only letters, digits, `-`, `_`
    /// and `.`, and mayn't start with `.`.
    #[error(
        "{key:?} can't be a storage key: use only letters, digits, '-', '_' and '.', not starting with '.'."
    )]
    InvalidKey { key: String },
    #[error("couldn't make the folder {} for saved state: {source}. Check it can be written to.", .path.display())]
    CreateFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't read saved state from {}: {source}. Check it can be read.", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The save stopped before replacing what was saved, which is kept.
    #[error(
        "couldn't write saved state to {}: {source}. The copy saved before is untouched; check the disk isn't full and the folder can be written to.",
        .path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The new copy was written but couldn't take the old one's place,
    /// which is kept.
    #[error(
        "couldn't move saved state from {} to {}: {source}. The copy saved before is untouched.",
        .from.display(),
        .to.display()
    )]
    Replace {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't remove saved state at {}: {source}. Check the folder can be written to.", .path.display())]
    Remove {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Storage in memory, for tests: clones share what's saved, so a new store
/// made from a clone restores what the last one saved, as after a restart.
#[derive(Clone, Default)]
pub struct MemoryStorage {
    saved: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    fn saved(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<u8>>> {
        self.saved.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for MemoryStorage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryStorage")
            .field("keys", &self.saved().keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Storage for MemoryStorage {
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(self.saved().get(key).cloned())
    }

    fn save(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError> {
        self.saved().insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), StorageError> {
        self.saved().remove(key);
        Ok(())
    }
}

/// Storage in a folder, a file per key. A save writes a temporary file
/// beside the old one and renames it into place, so what's saved is
/// either the old copy or the new one whole, never a mix, even if the app
/// stops partway.
#[derive(Clone, Debug)]
pub struct FileStorage {
    folder: PathBuf,
}

/// Keeps temporary files of saves at once apart.
static TEMPORARY: AtomicU64 = AtomicU64::new(0);

impl FileStorage {
    /// Storage in `folder`, made when first saved to.
    pub fn new(folder: impl Into<PathBuf>) -> Self {
        Self {
            folder: folder.into(),
        }
    }

    /// The folder it keeps files in.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn path(&self, key: &str) -> Result<PathBuf, StorageError> {
        let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.');
        if key.is_empty() || key.starts_with('.') || !key.chars().all(allowed) {
            return Err(StorageError::InvalidKey {
                key: key.to_owned(),
            });
        }
        Ok(self.folder.join(key))
    }
}

impl Storage for FileStorage {
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        let path = self.path(key)?;
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StorageError::Read { path, source }),
        }
    }

    fn save(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let path = self.path(key)?;
        std::fs::create_dir_all(&self.folder).map_err(|source| StorageError::CreateFolder {
            path: self.folder.clone(),
            source,
        })?;
        // Starts with '.', so it's never a key's file.
        let temporary = self.folder.join(format!(
            ".{key}.{}.{}.tmp",
            std::process::id(),
            TEMPORARY.fetch_add(1, Ordering::Relaxed)
        ));
        let written = std::fs::File::create(&temporary).and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        });
        if let Err(source) = written {
            let _ = std::fs::remove_file(&temporary);
            return Err(StorageError::Write {
                path: temporary,
                source,
            });
        }
        if let Err(source) = std::fs::rename(&temporary, &path) {
            let _ = std::fs::remove_file(&temporary);
            return Err(StorageError::Replace {
                from: temporary,
                to: path,
                source,
            });
        }
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), StorageError> {
        let path = self.path(key)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StorageError::Remove { path, source }),
        }
    }
}

/// Turns a value into bytes to save and back. The library has no format
/// of its own: wrap `serde_json`, say, in one.
pub trait Codec<T>: Send + Sync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    fn encode(&self, value: &T) -> Result<Vec<u8>, Self::Error>;

    fn decode(&self, bytes: &[u8]) -> Result<T, Self::Error>;
}

/// Why saved state couldn't be restored or saved.
#[derive(Debug, thiserror::Error)]
pub enum PersistError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    #[error("couldn't restore the saved {key}, so it starts as new: {source}")]
    Load {
        key: String,
        #[source]
        source: StorageError,
    },
    /// What was saved isn't what the codec reads, as when the file was
    /// cut short or written by another version. It starts as new, and the
    /// next save replaces what was there.
    #[error(
        "the saved {key} couldn't be read, so it starts as new and the next save replaces it: {source}"
    )]
    Decode {
        key: String,
        #[source]
        source: E,
    },
    #[error("couldn't save the {key}: it couldn't be encoded: {source}")]
    Encode {
        key: String,
        #[source]
        source: E,
    },
    #[error("couldn't save the {key}: {source}")]
    Save {
        key: String,
        #[source]
        source: StorageError,
    },
}

/// What [`Persistence::restore`] found: the value saved, or the default
/// with why the saved one couldn't be used.
#[derive(Debug)]
pub struct Restored<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    pub value: T,
    /// Why the value is the default though something was saved, or
    /// storage couldn't be read. None when it was restored, or nothing was
    /// saved yet.
    pub error: Option<PersistError<E>>,
}

/// Saves a value under a key and restores it: one slice of a feature's
/// state, such as its layout. Saves wait for changes to pause, on its
/// clock, so a burst of changes, like dragging a divider, is written once.
pub struct Persistence<T, C> {
    key: Arc<str>,
    storage: Arc<dyn Storage>,
    codec: Arc<C>,
    clock: Arc<dyn Clock>,
    debounce: Duration,
    _value: PhantomData<fn(&T) -> T>,
}

impl<T, C> Clone for Persistence<T, C> {
    fn clone(&self) -> Self {
        Self {
            key: self.key.clone(),
            storage: self.storage.clone(),
            codec: self.codec.clone(),
            clock: self.clock.clone(),
            debounce: self.debounce,
            _value: PhantomData,
        }
    }
}

impl<T, C: Codec<T>> Persistence<T, C> {
    /// Saves under `key` in `storage`, `debounce` after the last change on
    /// `clock`.
    pub fn new(
        key: impl Into<String>,
        storage: Arc<dyn Storage>,
        codec: C,
        clock: Arc<dyn Clock>,
        debounce: Duration,
    ) -> Self {
        Self {
            key: key.into().into(),
            storage,
            codec: Arc::new(codec),
            clock,
            debounce,
            _value: PhantomData,
        }
    }

    /// The key it saves under.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// What was saved, or the default if nothing was or it can't be read,
    /// with why.
    pub fn restore(&self) -> Restored<T, C::Error>
    where
        T: Default,
    {
        let key = self.key.to_string();
        match self.storage.load(&key) {
            Ok(None) => Restored {
                value: T::default(),
                error: None,
            },
            Ok(Some(bytes)) => match self.codec.decode(&bytes) {
                Ok(value) => Restored { value, error: None },
                Err(source) => Restored {
                    value: T::default(),
                    error: Some(PersistError::Decode { key, source }),
                },
            },
            Err(source) => Restored {
                value: T::default(),
                error: Some(PersistError::Load { key, source }),
            },
        }
    }

    /// Saves `value` now, as when the app quits.
    ///
    /// # Errors
    ///
    /// [`PersistError::Encode`] or [`PersistError::Save`].
    pub fn save_now(&self, value: &T) -> Result<(), PersistError<C::Error>> {
        let bytes = self.encode(value)?;
        self.storage
            .save(&self.key, &bytes)
            .map_err(|source| PersistError::Save {
                key: self.key.to_string(),
                source,
            })
    }

    /// An effect that saves `value` once changes pause, in place of a save
    /// still waiting. If the save fails, it sends what `failed` makes of
    /// why.
    pub fn save<A: Send + 'static>(
        &self,
        value: &T,
        failed: impl FnOnce(PersistError<C::Error>) -> A + Send + 'static,
    ) -> Effect<A> {
        let bytes = match self.encode(value) {
            Ok(bytes) => bytes,
            Err(error) => return Effect::send(failed(error)),
        };
        let key = self.key.clone();
        let storage = self.storage.clone();
        Effect::run(move |send| async move {
            if let Err(source) = storage.save(&key, &bytes) {
                send.send(failed(PersistError::Save {
                    key: key.to_string(),
                    source,
                }));
            }
        })
        .debounce(
            format!("persist:{}", self.key),
            self.debounce,
            self.clock.clone(),
        )
    }

    fn encode(&self, value: &T) -> Result<Vec<u8>, PersistError<C::Error>> {
        self.codec
            .encode(value)
            .map_err(|source| PersistError::Encode {
                key: self.key.to_string(),
                source,
            })
    }
}

/// A reducer that saves part of its state whenever an action changes it,
/// once changes pause. Restore that part with
/// [`Persistence::restore`] when making the state.
///
/// It reads the part before and after each action, so pick a small one.
pub struct Persisting<R, T, C>
where
    R: Reducer,
    C: Codec<T>,
{
    reducer: R,
    persistence: Persistence<T, C>,
    select: Box<dyn Fn(&R::State) -> T + Send + Sync>,
    failed: Arc<dyn Fn(PersistError<C::Error>) -> R::Action + Send + Sync>,
}

impl<R, T, C> Persisting<R, T, C>
where
    R: Reducer,
    C: Codec<T>,
{
    /// `reducer`, saving with `persistence` the part `select` picks. A save
    /// that fails sends what `failed` makes of why.
    pub fn new(
        reducer: R,
        persistence: Persistence<T, C>,
        select: impl Fn(&R::State) -> T + Send + Sync + 'static,
        failed: impl Fn(PersistError<C::Error>) -> R::Action + Send + Sync + 'static,
    ) -> Self {
        Self {
            reducer,
            persistence,
            select: Box::new(select),
            failed: Arc::new(failed),
        }
    }

    /// The reducer it saves for.
    pub fn reducer(&self) -> &R {
        &self.reducer
    }
}

impl<R, T, C> Reducer for Persisting<R, T, C>
where
    R: Reducer,
    R::Action: Send + 'static,
    T: PartialEq,
    C: Codec<T>,
{
    type State = R::State;
    type Action = R::Action;

    fn reduce(&self, state: &mut R::State, action: R::Action) -> Effect<R::Action> {
        let before = (self.select)(state);
        let effect = self.reducer.reduce(state, action);
        let after = (self.select)(state);
        if after == before {
            return effect;
        }
        let failed = self.failed.clone();
        effect.and(self.persistence.save(&after, move |error| failed(error)))
    }
}
