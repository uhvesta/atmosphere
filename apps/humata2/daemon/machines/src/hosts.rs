//! `hosts.json` (MP-19): the machines the host knows, mode 0600, holding no
//! secrets. Each entry records how far adding or removing it got, so a
//! restart at any step finishes or undoes it (PER-27). The file is replaced
//! whole (temporary file, fsync, rename, folder fsync), so a power cut
//! leaves the old list or the new one.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::install;

/// The file's format; a newer one is refused, never rewritten.
const VERSION: u32 = 1;

/// How far a machine got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    /// Add machine runs, or was cut short and is undone at the next start.
    Adding,
    /// Set up and registered.
    Ready,
    /// Remove machine runs, or was cut short and finishes at the next start.
    Removing,
}

/// One machine, as `hosts.json` keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEntry {
    /// The machine's name ([`crate::MachineName`] rules).
    pub name: String,
    /// What the user typed: an alias from `~/.ssh/config` or
    /// `user@host[:port]` (MP-10).
    pub ssh_host: String,
    /// The folder channels live under on the machine (FC-10).
    pub root: String,
    pub state: EntryState,
    /// `linux`, `macos`; empty before the first connect.
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    /// Files whose new copy waits beside the old one (TS-1).
    #[serde(default)]
    pub staged: Vec<String>,
    /// An undo that couldn't reach the machine; it runs again at the next
    /// start or reconnect.
    #[serde(default)]
    pub undo_pending: bool,
    /// Set for a machine a provider made or reaches (MP-1); an SSH machine
    /// has none. Older files have none, which reads as a plain SSH machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderMachine>,
}

/// What `hosts.json` keeps of a provider machine (MP-1): which provider,
/// the params it was made with (never secrets: host variables are named,
/// not kept), and how to reach it, written as soon as that is known so a
/// restart can undo a cut-short create.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderMachine {
    /// The provider's id in `providers.toml`.
    pub id: String,
    #[serde(default)]
    pub params: BTreeMap<String, String>,
    /// None until the provider's `create` and `identity` gave one.
    #[serde(default)]
    pub link: Option<StoredLink>,
}

/// A link as `hosts.json` keeps it. The host key of an identity provider's
/// machine lives in Humata's own `known_hosts`, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "how", rename_all = "snake_case")]
pub enum StoredLink {
    /// The provider's `connect` template as it was when the machine was
    /// made, and the variables it gets (`None`: the host variable of that
    /// name, whose value is never kept).
    Connect {
        template: String,
        env: Vec<(String, Option<String>)>,
    },
    /// SSH, from the provider's identity.
    Ssh {
        host: String,
        #[serde(default)]
        ssh_config: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        identity_file: Option<String>,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HostsJson {
    version: u32,
    #[serde(default)]
    machines: Vec<HostEntry>,
}

#[derive(Debug, Clone, Error)]
pub enum HostsError {
    #[error(
        "couldn't read the machine list {path}: {source}. Check that the file is yours and readable (chmod 600), then try again"
    )]
    Read {
        path: PathBuf,
        #[source]
        source: Arc<io::Error>,
    },
    #[error(
        "the machine list {path} isn't valid ({source}). Restore it from a backup, or move it aside to start with no machines (each machine then has to be added again)"
    )]
    Invalid {
        path: PathBuf,
        #[source]
        source: Arc<serde_json::Error>,
    },
    #[error(
        "the machine list {path} was written by a newer Humata (format {found}, this one reads {VERSION}). Update Humata; the file was left as it is"
    )]
    Newer { path: PathBuf, found: u32 },
    #[error(
        "couldn't save the machine list {path}: {source}. Free some disk space or fix the folder's permissions, then try again; the old list is still in place"
    )]
    Write {
        path: PathBuf,
        #[source]
        source: Arc<io::Error>,
    },
}

/// The machine list. Changes are made one at a time.
#[derive(Debug)]
pub struct Hosts {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Hosts {
    /// The list at `path`; a missing file is an empty list.
    pub fn open(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> Result<Vec<HostEntry>, HostsError> {
        let _held = self.hold();
        Ok(self.read()?.machines)
    }

    pub fn get(&self, name: &str) -> Result<Option<HostEntry>, HostsError> {
        Ok(self.list()?.into_iter().find(|entry| entry.name == name))
    }

    /// Adds `entry`, or replaces the entry with its name.
    pub(crate) fn put(&self, entry: HostEntry) -> Result<(), HostsError> {
        self.change(|machines| {
            match machines.iter_mut().find(|old| old.name == entry.name) {
                Some(old) => *old = entry,
                None => machines.push(entry),
            }
            true
        })
    }

    /// Adds `entry` unless a machine has its name; false when one has.
    pub(crate) fn insert_new(&self, entry: HostEntry) -> Result<bool, HostsError> {
        let mut added = false;
        self.change(|machines| {
            if machines.iter().any(|old| old.name == entry.name) {
                return false;
            }
            machines.push(entry);
            added = true;
            true
        })?;
        Ok(added)
    }

    pub(crate) fn remove(&self, name: &str) -> Result<(), HostsError> {
        self.change(|machines| {
            let before = machines.len();
            machines.retain(|entry| entry.name != name);
            machines.len() != before
        })
    }

    fn hold(&self) -> std::sync::MutexGuard<'_, ()> {
        // A panic while holding it left the file whole (it is replaced
        // whole), so the lock is still good.
        self.lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Runs `edit` on the list and saves it when `edit` says it changed.
    fn change(&self, edit: impl FnOnce(&mut Vec<HostEntry>) -> bool) -> Result<(), HostsError> {
        let _held = self.hold();
        let mut file = self.read()?;
        if edit(&mut file.machines) {
            file.version = VERSION;
            self.write(&file)?;
        }
        Ok(())
    }

    fn read(&self) -> Result<HostsJson, HostsError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(HostsJson {
                    version: VERSION,
                    machines: Vec::new(),
                });
            }
            Err(source) => {
                return Err(HostsError::Read {
                    path: self.path.clone(),
                    source: Arc::new(source),
                });
            }
        };
        let file: HostsJson = serde_json::from_slice(&bytes).map_err(|source| HostsError::Invalid {
            path: self.path.clone(),
            source: Arc::new(source),
        })?;
        if file.version > VERSION {
            return Err(HostsError::Newer {
                path: self.path.clone(),
                found: file.version,
            });
        }
        Ok(file)
    }

    fn write(&self, file: &HostsJson) -> Result<(), HostsError> {
        let failed = |source: io::Error| HostsError::Write {
            path: self.path.clone(),
            source: Arc::new(source),
        };
        let bytes = serde_json::to_vec_pretty(file).map_err(|source| failed(io::Error::other(source)))?;
        let folder = self
            .path
            .parent()
            .filter(|folder| !folder.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)
            .map_err(failed)?;
        let mut temporary = self.path.as_os_str().to_owned();
        temporary.push(format!(".{}.new", install::nonce()));
        let temporary = PathBuf::from(temporary);
        let result = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .and_then(|mut out| {
                out.write_all(&bytes)?;
                out.sync_all()
            })
            .and_then(|()| fs::rename(&temporary, &self.path))
            .and_then(|()| File::open(folder)?.sync_all());
        if let Err(source) = result {
            let _ = fs::remove_file(&temporary);
            return Err(failed(source));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn entry(name: &str, state: EntryState) -> HostEntry {
        HostEntry {
            name: name.to_owned(),
            ssh_host: format!("me@{name}.internal"),
            root: "~/Humata".to_owned(),
            state,
            os: String::new(),
            arch: String::new(),
            staged: Vec::new(),
            undo_pending: false,
            provider: None,
        }
    }

    #[test]
    fn entries_are_kept_across_opens_in_a_private_file() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("state/hosts.json");
        let hosts = Hosts::open(&path);
        assert_eq!(hosts.list().unwrap(), []);

        assert!(hosts.insert_new(entry("box-1", EntryState::Adding)).unwrap());
        assert!(!hosts.insert_new(entry("box-1", EntryState::Ready)).unwrap());
        hosts.put(entry("box-1", EntryState::Ready)).unwrap();
        hosts.put(entry("box-2", EntryState::Adding)).unwrap();
        hosts.remove("box-2").unwrap();

        let again = Hosts::open(&path);
        assert_eq!(again.list().unwrap(), [entry("box-1", EntryState::Ready)]);
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // No temporary file is left beside it.
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn a_provider_machine_keeps_its_provider_and_an_older_file_reads_as_plain_ssh() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("hosts.json");
        let hosts = Hosts::open(&path);
        let mut made = entry("vm-1", EntryState::Ready);
        made.provider = Some(ProviderMachine {
            id: "lima".to_owned(),
            params: BTreeMap::from([("cpus".to_owned(), "2".to_owned())]),
            link: Some(StoredLink::Connect {
                template: "limactl shell {name} {cmd}".to_owned(),
                env: vec![("HUMATA_NAME".to_owned(), Some("vm-1".to_owned())), ("TOKEN".to_owned(), None)],
            }),
        });
        hosts.put(made.clone()).unwrap();
        assert_eq!(Hosts::open(&path).list().unwrap(), [made]);

        fs::write(
            &path,
            br#"{"version":1,"machines":[{"name":"old","ssh_host":"me@old","root":"~/Humata","state":"ready"}]}"#,
        )
        .unwrap();
        assert_eq!(hosts.list().unwrap()[0].provider, None);
    }

    #[test]
    fn a_damaged_or_newer_file_is_refused_and_left_alone() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("hosts.json");
        fs::write(&path, b"{\"version\": 1, \"machines\": [").unwrap();
        let hosts = Hosts::open(&path);
        assert!(matches!(hosts.list(), Err(HostsError::Invalid { .. })));
        assert!(matches!(
            hosts.put(entry("box-1", EntryState::Ready)),
            Err(HostsError::Invalid { .. })
        ));

        fs::write(&path, b"{\"version\": 9, \"machines\": []}").unwrap();
        assert!(matches!(hosts.list(), Err(HostsError::Newer { found: 9, .. })));
        assert_eq!(fs::read(&path).unwrap(), b"{\"version\": 9, \"machines\": []}");
    }
}
