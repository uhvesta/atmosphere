//! A machine's host variables (HO-7, HO-13, SEC-1, LD-21): named secrets the
//! user sets on a machine for the agents it runs. The values live in a
//! [`SecretStore`] and nowhere else: no client call returns one, no log or
//! journal holds one, and a machine's value is never read by another machine.
//! The daemon reads them when it launches an agent (so agents started after a
//! change get the new values and running ones keep theirs), and when it
//! scrubs a conversation for the host's mirror (LD-21).
//!
//! Which store keeps them is chosen once, in `serve`, by [`install`]; every
//! caller asks [`HostVars::installed`] and so never knows. [`FileStore`] is
//! the one built so far (one 0600 file in a 0700 `secrets` folder of the
//! machine's state folder, never in the synced config folder). A
//! store that can't be read is an error for every reader: nothing is read as
//! "no variables".

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use humata2_grpc::humata::v1 as pb;
use tonic::{Code, Status};

use crate::status;

/// The 0700 folder in the state folder that holds the values.
const FOLDER: &str = "secrets";
/// The file in it.
const FILE: &str = "host-variables.json";
/// What a writer locks while it reads the file and replaces it, so writers
/// in other processes take turns too.
const LOCK: &str = "host-variables.lock";
/// The shortest value kept: shorter ones can't be told from other text
/// when a conversation or a restart report is scrubbed of them.
const VALUE_MIN: usize = 4;
/// The longest a variable's name may be, in bytes.
const NAME_MAX: usize = 128;
/// The longest a variable's value may be, in bytes.
const VALUE_MAX: usize = 64 * 1024;
/// The most variables a machine keeps.
const COUNT_MAX: usize = 256;
/// Names an agent's own environment depends on: a host variable can't
/// replace the session token or the daemon's socket.
const RESERVED_PREFIX: &str = "HUMATA";

#[derive(Debug, thiserror::Error)]
pub(crate) enum HostVarsError {
    #[error(
        "couldn't read this machine's host variables from {}. Check that the file is readable by this user, then try again; no variable was changed",
        path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "this machine's host variables in {} can't be understood, so none were used and none were changed. Restore the file from a backup, or remove it and set the variables again",
        path.display()
    )]
    Damaged {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "couldn't save this machine's host variables in {}. Check that its disk has room and its folder is writable, then try again; nothing was changed",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("this machine's host variable store isn't set up, so no variable was read or changed. Restart humatad2; if this repeats, report it")]
    NotInstalled,
    #[error("`{name}` can't be a host variable's name: {reason}")]
    BadName { name: String, reason: &'static str },
    #[error("the value of {name} is empty. Type the value, or remove the variable to delete it")]
    EmptyValue { name: String },
    #[error("the value of {name} is shorter than {min} bytes, which is too short to be kept out of conversations and reports. Use a longer value")]
    ValueTooShort { name: String, min: usize },
    #[error(
        "couldn't lock this machine's host variables in {} while changing them, so nothing was changed. Check that the folder is writable by this user, then try again",
        path.display()
    )]
    Lock {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the value of {name} is longer than {max} bytes. Keep a longer secret in a file on the machine and set the path as the value")]
    ValueTooLong { name: String, max: usize },
    #[error("this machine already has {max} host variables. Remove one before setting another")]
    TooMany { max: usize },
}

impl HostVarsError {
    /// The error as the API says it.
    pub(crate) fn status(&self) -> Status {
        let message = status::chain(self);
        let field = |field: &str, problem: pb::invalid_field::Problem, fix: &str| {
            status::invalid_field(field, problem, message.clone(), fix)
        };
        match self {
            Self::BadName { .. } => field(
                "name",
                pb::invalid_field::Problem::BadFormat,
                "use letters, digits and underscores, starting with a letter or underscore, and not beginning with HUMATA",
            ),
            Self::ValueTooShort { .. } => field(
                "value",
                pb::invalid_field::Problem::BadFormat,
                "use a value of at least four characters",
            ),
            Self::EmptyValue { .. } => field(
                "value",
                pb::invalid_field::Problem::Missing,
                "type the value, or remove the variable to delete it",
            ),
            Self::ValueTooLong { .. } => field(
                "value",
                pb::invalid_field::Problem::TooLong,
                "keep a longer secret in a file on the machine and set the path as the value",
            ),
            Self::TooMany { .. } => status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: message.clone(),
                    fix: "remove a host variable you no longer use, then set this one".to_owned(),
                    retry: None,
                    kind: None,
                },
            ),
            Self::NotInstalled | Self::Read { .. } | Self::Damaged { .. } | Self::Write { .. } | Self::Lock { .. } => status::internal(message.clone(), ""),
        }
    }
}

/// Where a machine's host variable values are kept. An implementation holds
/// the values only; naming rules and limits are [`HostVars`]'s. Every call
/// either does what it says or fails, and a failed read is never an empty one.
pub(crate) trait SecretStore: Send + Sync + std::fmt::Debug {
    /// The names, sorted. Never a value.
    fn names(&self) -> Result<Vec<String>, HostVarsError>;
    /// Every (name, value).
    fn get_all(&self) -> Result<BTreeMap<String, String>, HostVarsError>;
    /// Keeps `value` as `name`'s value, on disk (or wherever it lives) before
    /// it returns.
    fn set(&self, name: &str, value: &str) -> Result<(), HostVarsError>;
    /// Forgets `name`; whether it was there.
    fn remove(&self, name: &str) -> Result<bool, HostVarsError>;
}

/// The store this daemon uses, chosen once.
static INSTALLED: OnceLock<Arc<dyn SecretStore>> = OnceLock::new();

/// Makes `store` the machine's host variable store. Called once by `serve`
/// before anything can ask; a second call changes nothing.
pub(crate) fn install(store: Arc<dyn SecretStore>) {
    let _ = INSTALLED.set(store);
}

/// One machine's host variables: the rules over a store.
#[derive(Clone, Debug)]
pub(crate) struct HostVars {
    store: Arc<dyn SecretStore>,
}

impl HostVars {
    pub(crate) fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// The variables of the installed store.
    pub(crate) fn installed() -> Result<Self, HostVarsError> {
        INSTALLED
            .get()
            .map(|store| Self::new(store.clone()))
            .ok_or(HostVarsError::NotInstalled)
    }

    /// Whether `name` and `value` can be kept, before anything is sent to a
    /// machine or written.
    pub(crate) fn check(name: &str, value: &str) -> Result<(), HostVarsError> {
        check_name(name)?;
        if value.is_empty() {
            return Err(HostVarsError::EmptyValue { name: name.to_owned() });
        }
        if value.len() < VALUE_MIN {
            return Err(HostVarsError::ValueTooShort {
                name: name.to_owned(),
                min: VALUE_MIN,
            });
        }
        if value.len() > VALUE_MAX {
            return Err(HostVarsError::ValueTooLong {
                name: name.to_owned(),
                max: VALUE_MAX,
            });
        }
        Ok(())
    }

    /// The names, sorted. Never a value.
    pub(crate) fn names(&self) -> Result<Vec<String>, HostVarsError> {
        self.store.names()
    }

    /// Every (name, value), for scrubbing a conversation. Fails when the
    /// store can't be read: the caller must not go on as if there were none.
    pub(crate) fn values(&self) -> Result<Vec<(String, String)>, HostVarsError> {
        Ok(self.store.get_all()?.into_iter().collect())
    }

    /// The variables as an agent's launch environment.
    pub(crate) fn env(&self) -> Result<Vec<(OsString, OsString)>, HostVarsError> {
        Ok(self
            .store
            .get_all()?
            .into_iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect())
    }

    /// Keeps `value` as `name`'s value, replacing the one it had.
    pub(crate) fn set(&self, name: &str, value: &str) -> Result<(), HostVarsError> {
        Self::check(name, value)?;
        let kept = self.store.names()?;
        if kept.len() >= COUNT_MAX && !kept.iter().any(|kept| kept == name) {
            return Err(HostVarsError::TooMany { max: COUNT_MAX });
        }
        self.store.set(name, value)
    }

    /// Forgets `name`; whether it was there. A name that isn't is not an
    /// error, so a retried removal succeeds.
    pub(crate) fn remove(&self, name: &str) -> Result<bool, HostVarsError> {
        self.store.remove(name)
    }
}

/// The values in one file in the machine's `secrets` folder: the folder is
/// 0700 and the file 0600, written whole and put on disk before it replaces
/// the old one. Writers hold a lock on a file beside it from reading to
/// replacing, so two processes never lose each other's change.
#[derive(Debug)]
pub(crate) struct FileStore {
    state: PathBuf,
    folder: PathBuf,
    file: PathBuf,
    lock: PathBuf,
    /// Writers in this process take turns (the file lock is per open file).
    writing: Mutex<()>,
}

impl FileStore {
    /// The store kept in the state folder `state`.
    pub(crate) fn in_folder(state: &Path) -> Self {
        let folder = state.join(FOLDER);
        Self {
            state: state.to_owned(),
            file: folder.join(FILE),
            lock: folder.join(LOCK),
            folder,
            writing: Mutex::new(()),
        }
    }

    /// Makes the folder (0700) if it isn't there, and puts it on disk.
    fn ensure_folder(&self) -> Result<(), HostVarsError> {
        let failed = |source| HostVarsError::Write {
            path: self.folder.clone(),
            source,
        };
        humata2_durable::create_dir_all(&self.folder, 0o700).map_err(failed)?;
        std::fs::set_permissions(&self.folder, std::fs::Permissions::from_mode(0o700)).map_err(failed)?;
        humata2_durable::sync_dir(&self.state).map_err(failed)
    }

    /// Holds the writers' lock until the returned file is dropped.
    fn flock(&self) -> Result<std::fs::File, HostVarsError> {
        let failed = |source| HostVarsError::Lock {
            path: self.lock.clone(),
            source,
        };
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&self.lock)
            .map_err(failed)?;
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
            .map_err(|errno| failed(io::Error::from(errno)))?;
        Ok(file)
    }

    fn load(&self) -> Result<BTreeMap<String, String>, HostVarsError> {
        match std::fs::read(&self.file) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| HostVarsError::Damaged {
                path: self.file.clone(),
                source,
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(source) => Err(HostVarsError::Read {
                path: self.file.clone(),
                source,
            }),
        }
    }

    /// Writes the whole file beside the old one and puts it on disk before it
    /// replaces it, so a power cut leaves the old file or the new one.
    fn save(&self, kept: &BTreeMap<String, String>) -> Result<(), HostVarsError> {
        let failed = |source| HostVarsError::Write {
            path: self.file.clone(),
            source,
        };
        let bytes = serde_json::to_vec(kept).map_err(|error| failed(io::Error::other(error)))?;
        let temporary = self.file.with_file_name(format!("{FILE}.new"));
        let write = || -> io::Result<()> {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true).mode(0o600);
            let file = humata2_durable::File::open(&options, &temporary)?;
            file.write_all_at(&bytes, 0)?;
            file.sync_all()?;
            // Power cut here: the temporary file is whole, the old file is
            // still the file.
            crate::fault::point("host_variables.temporary_written");
            humata2_durable::rename(&temporary, &self.file)?;
            // Power cut here: the rename isn't on disk yet, so the old file
            // or the new one is the file, whole either way.
            crate::fault::point("host_variables.renamed");
            humata2_durable::sync_dir(&self.folder)
        };
        write().map_err(|source| {
            let _ = humata2_durable::remove_file(&temporary);
            failed(source)
        })
    }
}

impl SecretStore for FileStore {
    fn names(&self) -> Result<Vec<String>, HostVarsError> {
        Ok(self.load()?.into_keys().collect())
    }

    fn get_all(&self) -> Result<BTreeMap<String, String>, HostVarsError> {
        self.load()
    }

    fn set(&self, name: &str, value: &str) -> Result<(), HostVarsError> {
        let _turn = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        self.ensure_folder()?;
        let _locked = self.flock()?;
        let mut kept = self.load()?;
        kept.insert(name.to_owned(), value.to_owned());
        self.save(&kept)
    }

    fn remove(&self, name: &str) -> Result<bool, HostVarsError> {
        let _turn = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.file.exists() {
            return Ok(false);
        }
        let _locked = self.flock()?;
        let mut kept = self.load()?;
        if kept.remove(name).is_none() {
            return Ok(false);
        }
        self.save(&kept)?;
        Ok(true)
    }
}

/// This machine's host variables as the commands Humata runs for it see
/// them: a provider's commands, its login terminal and its link (MP-4). The
/// installed store is read each time a command needs a variable, so a
/// change applies to the next command; a store that can't be read stops
/// the command rather than reading as "not set".
pub(crate) struct Installed;

impl humata2_machines::HostVariables for Installed {
    fn value(&self, name: &str) -> Result<Option<String>, humata2_machines::HostVariablesError> {
        HostVars::installed()
            .and_then(|vars| vars.store.get_all())
            .map(|mut all| all.remove(name))
            .map_err(|error| humata2_machines::HostVariablesError(Box::new(error)))
    }
}

/// Values in memory only, for tests of what sits above a store.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MemoryStore {
    kept: Mutex<BTreeMap<String, String>>,
}

#[cfg(test)]
impl MemoryStore {
    pub(crate) fn vars() -> HostVars {
        HostVars::new(Arc::new(Self::default()))
    }
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn names(&self) -> Result<Vec<String>, HostVarsError> {
        Ok(self.kept.lock().unwrap_or_else(PoisonError::into_inner).keys().cloned().collect())
    }

    fn get_all(&self) -> Result<BTreeMap<String, String>, HostVarsError> {
        Ok(self.kept.lock().unwrap_or_else(PoisonError::into_inner).clone())
    }

    fn set(&self, name: &str, value: &str) -> Result<(), HostVarsError> {
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<bool, HostVarsError> {
        Ok(self.kept.lock().unwrap_or_else(PoisonError::into_inner).remove(name).is_some())
    }
}

fn check_name(name: &str) -> Result<(), HostVarsError> {
    let bad = |reason| {
        Err(HostVarsError::BadName {
            name: name.to_owned(),
            reason,
        })
    };
    if name.is_empty() {
        return bad("it is empty");
    }
    if name.len() > NAME_MAX {
        return bad("it is too long");
    }
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !first_ok || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return bad("it may hold only letters, digits and underscores, and must not start with a digit");
    }
    if name.len() >= RESERVED_PREFIX.len() && name[..RESERVED_PREFIX.len()].eq_ignore_ascii_case(RESERVED_PREFIX) {
        return bad("names starting with HUMATA belong to Humata's own session settings");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_kept_listed_by_name_replaced_and_removed() {
        let vars = MemoryStore::vars();
        assert!(vars.names().unwrap().is_empty());

        vars.set("API_TOKEN", "first").unwrap();
        vars.set("DB_URL", "postgres://x").unwrap();
        vars.set("API_TOKEN", "second").unwrap();

        assert_eq!(vars.names().unwrap(), ["API_TOKEN", "DB_URL"]);
        assert_eq!(
            vars.values().unwrap(),
            [
                ("API_TOKEN".to_owned(), "second".to_owned()),
                ("DB_URL".to_owned(), "postgres://x".to_owned())
            ]
        );
        assert!(vars.remove("DB_URL").unwrap());
        assert!(!vars.remove("DB_URL").unwrap(), "removing it again changes nothing");
        assert_eq!(vars.names().unwrap(), ["API_TOKEN"]);
    }

    #[test]
    fn a_name_or_value_that_can_not_be_kept_is_refused_before_anything_is_written() {
        let vars = MemoryStore::vars();
        let too_long = "A".repeat(NAME_MAX + 1);
        for name in ["", "1ABC", "A-B", "A B", "HUMATA_SESSION_TOKEN", "humata_socket", too_long.as_str()] {
            assert!(matches!(vars.set(name, "v"), Err(HostVarsError::BadName { .. })), "{name:?}");
        }
        assert!(matches!(vars.set("OK", ""), Err(HostVarsError::EmptyValue { .. })));
        assert!(matches!(vars.set("OK", "abc"), Err(HostVarsError::ValueTooShort { .. })));
        assert!(matches!(
            vars.set("OK", &"v".repeat(VALUE_MAX + 1)),
            Err(HostVarsError::ValueTooLong { .. })
        ));
        assert!(vars.names().unwrap().is_empty());
    }

    #[test]
    fn a_machine_keeps_no_more_than_its_limit() {
        let vars = MemoryStore::vars();
        for number in 0..COUNT_MAX {
            vars.set(&format!("V{number}"), "xxxx").unwrap();
        }
        assert!(matches!(vars.set("ONE_MORE", "xxxx"), Err(HostVarsError::TooMany { .. })));
        // Replacing one it has is still fine.
        vars.set("V0", "yyyy").unwrap();
    }

    #[test]
    fn the_variables_come_as_an_agents_launch_environment() {
        let vars = MemoryStore::vars();
        vars.set("API_TOKEN", "s3cret").unwrap();
        assert_eq!(
            vars.env().unwrap(),
            [(OsString::from("API_TOKEN"), OsString::from("s3cret"))]
        );
    }

    #[test]
    fn nothing_is_read_before_a_store_is_installed_or_through_one_that_fails() {
        // No test installs a store, so the process-wide choice is unset.
        assert!(matches!(HostVars::installed(), Err(HostVarsError::NotInstalled)));
    }

    fn file_vars() -> (tempfile::TempDir, HostVars) {
        let dir = tempfile::tempdir().unwrap();
        let vars = HostVars::new(Arc::new(FileStore::in_folder(dir.path())));
        (dir, vars)
    }

    #[test]
    fn the_file_store_keeps_values_in_a_file_only_this_user_can_read_that_survives_a_new_handle() {
        let (dir, vars) = file_vars();
        vars.set("API_TOKEN", "s3cret").unwrap();

        let folder = dir.path().join(FOLDER);
        let mode = std::fs::metadata(folder.join(FILE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(std::fs::metadata(&folder).unwrap().permissions().mode() & 0o777, 0o700);
        let again = HostVars::new(Arc::new(FileStore::in_folder(dir.path())));
        assert_eq!(again.values().unwrap(), [("API_TOKEN".to_owned(), "s3cret".to_owned())]);
        let temporary = std::fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".new"))
            .count();
        assert_eq!(temporary, 0, "no temporary file stays behind");
    }

    #[test]
    fn the_file_store_with_a_damaged_file_is_an_error_for_every_reader_and_leaves_it_as_it_was() {
        let (dir, vars) = file_vars();
        let file = dir.path().join(FOLDER).join(FILE);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"{not json").unwrap();

        assert!(matches!(vars.values(), Err(HostVarsError::Damaged { .. })));
        assert!(matches!(vars.names(), Err(HostVarsError::Damaged { .. })));
        assert!(matches!(vars.env(), Err(HostVarsError::Damaged { .. })));
        assert!(matches!(vars.set("A", "bbbb"), Err(HostVarsError::Damaged { .. })));
        assert!(matches!(vars.remove("A"), Err(HostVarsError::Damaged { .. })));
        assert_eq!(std::fs::read(&file).unwrap(), b"{not json");
    }

    #[test]
    fn writers_in_different_processes_never_lose_each_others_change() {
        let dir = tempfile::tempdir().unwrap();
        // One store each, as separate processes have: only the file lock keeps them in turn.
        let writers: Vec<_> = (0..8)
            .map(|number| {
                let vars = HostVars::new(Arc::new(FileStore::in_folder(dir.path())));
                std::thread::spawn(move || {
                    for round in 0..10 {
                        vars.set(&format!("W{number}_{round}"), "value").unwrap();
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let vars = HostVars::new(Arc::new(FileStore::in_folder(dir.path())));
        assert_eq!(vars.names().unwrap().len(), 80);
    }
}
