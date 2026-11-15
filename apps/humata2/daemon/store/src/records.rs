//! The records the store keeps, as plain data.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A session as this machine knows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub channel: String,
    pub machine: String,
    pub title: String,
    /// The agent as named in Settings › Agents ("claude", "zsh", …).
    pub agent: String,
    /// The session's folder, where its shells start.
    pub folder: PathBuf,
    pub created_unix_ms: i64,
}

/// What runs in a terminal tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabKind {
    Agent,
    Shell,
}

/// A terminal tab and the holder process that keeps its PTY.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabRecord {
    pub id: String,
    /// Unset for a machine shell, which belongs to no session.
    pub session: Option<String>,
    pub kind: TabKind,
    pub title: String,
    /// The holder's own socket.
    pub holder_socket: PathBuf,
    pub holder_pid: u32,
    pub created_unix_ms: i64,
}

/// Where a session's clones stand (SL-3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointers {
    pub repos: Vec<RepoPointer>,
    pub reported_unix_ms: i64,
}

/// One clone's position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoPointer {
    pub owner: String,
    pub name: String,
    pub branch: String,
    pub sha: String,
    /// The commit `sha` is on the remote.
    pub pushed: bool,
    /// Repo-relative paths with uncommitted changes.
    pub dirty_files: Vec<String>,
}
