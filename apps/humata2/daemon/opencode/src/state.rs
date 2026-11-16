//! The state file the `humata opencode` wrapper leaves for the daemon (D201).

use std::fmt;
use std::fs::{self, OpenOptions, Permissions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The format version this build writes and reads.
const VERSION: u64 = 1;
/// Only the file's owner can read or write it, since it holds the password.
const OWNER_ONLY: u32 = 0o600;

/// What the wrapper knows about its OpenCode server, for the daemon to read.
#[derive(Clone, PartialEq, Eq)]
pub struct StateFile {
    /// The server's loopback URL.
    pub url: String,
    /// The server's password, sent as HTTP basic auth.
    pub password: String,
    /// The session the wrapper runs.
    pub session: String,
    /// The process id of `opencode serve`.
    pub serve_pid: u32,
}

impl fmt::Debug for StateFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateFile")
            .field("url", &self.url)
            .field("password", &"<hidden>")
            .field("session", &self.session)
            .field("serve_pid", &self.serve_pid)
            .finish()
    }
}

/// The file's JSON: the fields and the format version.
#[derive(Serialize, Deserialize)]
struct Disk {
    version: u64,
    url: String,
    password: String,
    session: String,
    serve_pid: u32,
}

/// Why the state file could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// There is no file at the path.
    #[error(
        "no OpenCode state file at {}. Restart the OpenCode session so its `humata opencode` \
         wrapper writes the file again.",
        .path.display()
    )]
    Missing { path: PathBuf },
    /// The file is there but is not one this build can read.
    #[error(
        "the OpenCode state file at {} can't be read: {reason}. If a newer Humata wrote it, \
         update Humata. Otherwise restart the OpenCode session so its wrapper writes the file \
         again; don't edit the file by hand.",
        .path.display()
    )]
    Broken { path: PathBuf, reason: String },
    /// The file could not be read or written.
    #[error(
        "could not read or write the OpenCode state file at {}. Check that its folder exists \
         and that you can write to it, then try again.",
        .path.display()
    )]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The state could not be turned into JSON.
    #[error(
        "could not encode the OpenCode state file for {}. This is a bug in Humata: report it \
         with the Humata log.",
        .path.display()
    )]
    Encode {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

impl StateFile {
    /// Writes the file to `path` with owner-only permissions.
    ///
    /// The file is written beside `path` as `<path>.tmp` and renamed over it,
    /// so a reader sees the old file or the new one, never half of one.
    pub fn write(&self, path: &Path) -> Result<(), StateError> {
        let disk = Disk {
            version: VERSION,
            url: self.url.clone(),
            password: self.password.clone(),
            session: self.session.clone(),
            serve_pid: self.serve_pid,
        };
        let bytes = serde_json::to_vec_pretty(&disk).map_err(|source| StateError::Encode {
            path: path.to_path_buf(),
            source,
        })?;
        let tmp = tmp_path(path);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(OWNER_ONLY)
            .open(&tmp)
            .map_err(|source| io_error(path, source))?;
        // A leftover `.tmp` keeps its old mode when truncated, so set it again.
        file.set_permissions(Permissions::from_mode(OWNER_ONLY))
            .map_err(|source| io_error(path, source))?;
        file.write_all(&bytes)
            .map_err(|source| io_error(path, source))?;
        file.sync_all().map_err(|source| io_error(path, source))?;
        fs::rename(&tmp, path).map_err(|source| io_error(path, source))
    }

    /// Reads the file at `path`.
    pub fn read(path: &Path) -> Result<Self, StateError> {
        let bytes = fs::read(path).map_err(|source| match source.kind() {
            ErrorKind::NotFound => StateError::Missing {
                path: path.to_path_buf(),
            },
            _ => io_error(path, source),
        })?;
        let broken = |reason: String| StateError::Broken {
            path: path.to_path_buf(),
            reason,
        };
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|source| broken(format!("it is not JSON ({source})")))?;
        if value.get("version").and_then(Value::as_u64) != Some(VERSION) {
            let found = value
                .get("version")
                .map_or_else(|| "no version".to_owned(), Value::to_string);
            return Err(broken(format!(
                "it has {found}, and this Humata reads only version {VERSION}"
            )));
        }
        let disk: Disk = serde_json::from_value(value)
            .map_err(|source| broken(format!("its fields don't match ({source})")))?;
        Ok(Self {
            url: disk.url,
            password: disk.password,
            session: disk.session,
            serve_pid: disk.serve_pid,
        })
    }
}

/// The file a write goes through before it is renamed over `path`.
fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}

/// An IO error on the state file at `path`.
fn io_error(path: &Path, source: std::io::Error) -> StateError {
    StateError::Io {
        path: path.to_path_buf(),
        source,
    }
}
