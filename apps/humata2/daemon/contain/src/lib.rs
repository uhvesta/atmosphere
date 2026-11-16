//! SEC-6 contain agents: runs an agent's launch command inside an operating
//! system sandbox, so the agent can change only what its session may change.
//!
//! A contained command may write only in its session folder, the channel's
//! `shared/` folder, the folders it is also given (a fork parent's clone,
//! for example), its temp folder and its own agent state. It may read those
//! same folders, and no others it is told to hide: other sessions, Humata's
//! own state, SSH keys and the gh config. The agent's settings and hooks stay
//! readable but can't be changed. Its only network connections are HTTPS (port
//! 443), DNS and its daemon's socket; nothing on this machine but the socket.
//!
//! [`Containment::wrap`] turns a command into the argv that runs it contained.
//! On macOS that wraps the command in `sandbox-exec` with a profile generated
//! from the paths. Other systems get [`ContainError::Unsupported`] until their
//! containment ships. [`home_rules`] says which home-folder paths an agent
//! keeps, and which must be hidden or kept read-only (D217).
//! [`seed_claude_copy`] gives a contained Claude Code its own copy of the
//! user's config, so it can't change the real one.

#[cfg(target_os = "macos")]
mod macos;

mod claude;
mod git_exclude;
mod home;

pub use claude::{ClaudeCopy, seed_claude_copy};
pub use git_exclude::hide_from_git;
pub use home::{Agent, HomeRules, home_rules};

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

/// The folders and socket one contained session may use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Containment {
    /// The session's own folder. Readable and writable.
    pub session_folder: PathBuf,
    /// The channel's `shared/` folder. Readable and writable.
    pub shared: PathBuf,
    /// Folders the session also works in. Readable and writable, including
    /// where they sit inside a hidden folder.
    pub also_writable: Vec<PathBuf>,
    /// Paths the session may read but not write, even inside a hidden
    /// folder: the hook files its agent runs to tell Humata its turns. A path
    /// that doesn't exist yet is fine.
    pub also_readable: Vec<PathBuf>,
    /// Folders the session may not read, except the paths above that lie
    /// inside them. A hidden folder that doesn't exist yet stays hidden if it
    /// is created later.
    pub hidden: Vec<PathBuf>,
    /// The launched agent's own config and state, as folders or files.
    /// Readable and writable, even inside a hidden folder. A path that doesn't
    /// exist yet is fine. Each one also covers what lies under it and the
    /// siblings that add a dot-suffix to its name, such as the temp files an
    /// agent writes before it renames them over its config; a longer name such
    /// as `.claude-backup` is not covered.
    pub agent_state: Vec<PathBuf>,
    /// Paths, usually inside `agent_state`, that stay readable but never
    /// writable: the agent's settings and hooks.
    pub read_only: Vec<PathBuf>,
    /// The agent's temp folder. `wrap` creates it, and the contained command's
    /// `TMPDIR` points here. Readable and writable.
    pub temp: PathBuf,
    /// The daemon's unix socket: the only local connection the session may reach.
    pub socket: PathBuf,
    /// Variables the contained command runs with, besides `TMPDIR`; an empty
    /// value sets the variable to the empty string.
    pub env: Vec<(OsString, OsString)>,
}

impl Containment {
    /// The argv that runs `argv` contained. On macOS this is
    /// `/usr/bin/sandbox-exec -p <profile> /usr/bin/env TMPDIR=<temp>/`,
    /// then each of `env` as `NAME=value`, followed by `argv`.
    ///
    /// Every path is resolved before the profile is written, so a path that
    /// can't be resolved is an error, never a looser profile. The temp folder
    /// is created here, after the session folder has been checked.
    #[cfg(target_os = "macos")]
    pub fn wrap(&self, argv: &[OsString]) -> Result<Vec<OsString>, ContainError> {
        macos::wrap(self, argv)
    }

    /// The argv that runs `argv` contained. Contained agents only exist on
    /// macOS for now, so this always fails here.
    #[cfg(not(target_os = "macos"))]
    pub fn wrap(&self, _argv: &[OsString]) -> Result<Vec<OsString>, ContainError> {
        Err(ContainError::Unsupported {
            os: std::env::consts::OS,
        })
    }
}

/// Why a command could not be run contained, or why the home folder could not
/// be read for its rules.
#[derive(Debug, thiserror::Error)]
pub enum ContainError {
    /// The session's folder is gone, so the session has nowhere to write.
    #[error(
        "the session folder {} is missing, so this session can't be started contained; restore the folder, or start a new session in the channel, then start the session again",
        path.display()
    )]
    SessionFolderMissing {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A folder or the socket's folder could not be looked up.
    #[error(
        "couldn't look up the {role} {}: {source}. Check that the folder exists and that Humata can read it, then start the session again",
        path.display()
    )]
    Unresolved {
        role: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The temp folder the contained command writes to could not be made.
    #[error(
        "couldn't make the temp folder {} for this session: {source}. Check the session folder's disk space and permissions, then start the session again",
        path.display()
    )]
    TempFolder {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The home folder could not be listed, so its visible folders can't be hidden.
    #[error(
        "couldn't list the home folder {} to hide it from a contained agent: {source}. Check that Humata can read it, then start the session again",
        path.display()
    )]
    HomeUnreadable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The sandbox profile is text, so every path it names must be UTF-8.
    #[error(
        "the {role} {} has a name that isn't valid UTF-8, which the sandbox can't name; rename the folder to use only UTF-8 characters, then start the session again",
        path.display()
    )]
    NotUtf8 { role: &'static str, path: PathBuf },

    /// The socket path names no file, so it can't be the one network a
    /// contained agent is allowed.
    #[error(
        "the daemon socket {} doesn't name a socket file; point the daemon's socket setting at a file such as humatad.sock inside the Humata folder, then start the session again",
        path.display()
    )]
    SocketWithoutFileName { path: PathBuf },

    /// The copy of Claude Code's config could not be made or refreshed.
    #[error(
        "couldn't refresh this session's copy of the Claude Code config at {}: {source}. Check the disk has space and that Humata can read your ~/.claude folder, then start the session again; it isn't started on a half-made copy",
        path.display()
    )]
    ClaudeCopy {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// A Claude Code config file isn't JSON, so it can't be copied safely.
    #[error(
        "the Claude Code config {} isn't valid JSON ({source}), so this session can't be given a copy of it. Fix the file, or move it aside to let Claude Code make a new one (you'll then sign in and accept its prompts again), then start the session again",
        path.display()
    )]
    ClaudeConfigUnreadable {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    /// A Claude Code config file is JSON but not an object.
    #[error(
        "the Claude Code config {} isn't a JSON object, so this session can't be given a copy of it. Fix the file, or move it aside to let Claude Code make a new one (you'll then sign in and accept its prompts again), then start the session again",
        path.display()
    )]
    ClaudeConfigNotAnObject { path: PathBuf },

    /// The copy's `projects` is a folder of its own, not the link to the
    /// real one, so conversations written there would be lost by relinking.
    #[error(
        "{} should link to {} but is a folder, so this session's conversations may be in it. Move anything you want to keep into {}, delete the folder, then start the session again",
        path.display(), real.display(), real.display()
    )]
    ClaudeProjectsNotALink { path: PathBuf, real: PathBuf },

    /// The repository's exclude file could not be read or written, so the
    /// session's `.humata/` folder would show up as changes.
    #[error(
        "couldn't add the session's .humata/ folder to {}: {source}. Check that Humata can write the repository's .git folder, then start the session again",
        path.display()
    )]
    GitExclude {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    /// The launch gave no program to run.
    #[error(
        "the launch has no program to run contained; give the session a command to run (its agent program), then start the session again"
    )]
    NoCommand,

    /// This system has no containment yet.
    #[error(
        "can't start this session contained: Contain agents isn't available on {os} machines yet. Start it on a Mac, or turn Contain agents off for this channel; its agents will then be able to read and write anything your user can on this machine"
    )]
    Unsupported { os: &'static str },
}
