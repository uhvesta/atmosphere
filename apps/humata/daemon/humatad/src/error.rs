use std::path::PathBuf;

use humata_agent::{AgentError, WorkspaceError};
use humata_wire::{ClientError, WireError, pb};

/// What the host role can fail at, answered to the daemon as an error.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(transparent)]
    Agent(#[from] AgentError),

    #[error(
        "this host doesn't run the {backend} backend; pick one it has, or update humatad on this host"
    )]
    UnknownBackend { backend: &'static str },

    #[error(
        "couldn't create {path}, the folder for a chat that works in no folder of yours; check that humatad's state folder is writable and the disk isn't full, or give the chat a folder"
    )]
    CreateChatFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't list the files in {path}; check that the folder is still there and readable")]
    ReadFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't read {path}; check that the file is still there and readable")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{path:?} isn't a file in the session's folder; give a path from its list of files")]
    NotInFolder { path: String },

    #[error(
        "{path} is {size} bytes, too large to show here; open it in an editor on the host instead"
    )]
    FileTooLarge { path: PathBuf, size: u64 },

    #[error(
        "the watcher fell {missed} events behind and stopped; watch the session again to catch up from its history"
    )]
    FellBehind { missed: u64 },

    #[error(
        "only the daemon keeps projects and the board; send this to humatad's daemon on your Mac, not a host"
    )]
    NotForHosts,

    #[error("say which file: give its repository's name in the session and its path there")]
    NoFile,

    #[error("say what to do with the changes: stage, unstage or revert them")]
    NoAction,

    #[error("say which branch to compare the session's copy with")]
    NoBranch,

    #[error("say which turn's changes to show; the session's first turn is 1")]
    NoTurn,

    #[error(
        "couldn't record the setup script's run in {path}; check that the folder is writable and the disk isn't full. The script will run again next time"
    )]
    SaveSetup {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't create the folder {path} for repositories on this host; check that it is writable"
    )]
    CreateFolder {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't run git on this host; install git there and check that it is on the PATH")]
    RunGit {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "git couldn't clone or update {url} in {path} on this host (it said: {output}). Check that the host can reach the remote and has credentials for it; if the folder is a broken clone, delete it so it is cloned again, after saving any work in it"
    )]
    Clone {
        url: String,
        path: PathBuf,
        output: String,
    },

    #[error("{name:?} isn't a plain folder name; give a name without slashes or ..")]
    NotAName { name: String },
}

impl HostError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::Agent(AgentError::UnknownSession { .. }) => pb::ErrorCode::UnknownSession,
            Self::Agent(AgentError::UnknownEvent { .. }) => pb::ErrorCode::UnknownEvent,
            Self::Agent(AgentError::UnknownPermissionRequest { .. }) => {
                pb::ErrorCode::UnknownPermissionRequest
            }
            Self::Agent(AgentError::Archived { .. }) => pb::ErrorCode::Archived,
            Self::Agent(AgentError::ArchiveWhileWorking { .. }) => pb::ErrorCode::Busy,
            Self::Agent(AgentError::NoWorkspaces { .. }) => pb::ErrorCode::InvalidRequest,
            Self::Agent(AgentError::Workspace {
                source: WorkspaceError::ChangeGone { .. },
                ..
            }) => pb::ErrorCode::Changed,
            Self::Agent(AgentError::Workspace {
                source:
                    WorkspaceError::UnknownRepository { .. }
                    | WorkspaceError::NoCopy { .. }
                    | WorkspaceError::UnknownTurn { .. }
                    | WorkspaceError::UnknownBranch { .. }
                    | WorkspaceError::NoSuchFile { .. },
                ..
            }) => pb::ErrorCode::InvalidRequest,
            Self::Agent(AgentError::Workspace { .. }) => pb::ErrorCode::BackendFailed,
            Self::Agent(_) => pb::ErrorCode::StorageFailed,
            Self::UnknownBackend { .. } => pb::ErrorCode::UnknownBackend,
            Self::NotForHosts
            | Self::NotInFolder { .. }
            | Self::FileTooLarge { .. }
            | Self::NotAName { .. }
            | Self::NoFile
            | Self::NoAction
            | Self::NoBranch
            | Self::NoTurn => pb::ErrorCode::InvalidRequest,
            Self::FellBehind { .. } | Self::RunGit { .. } | Self::Clone { .. } => {
                pb::ErrorCode::BackendFailed
            }
            Self::SaveSetup { .. }
            | Self::CreateFolder { .. }
            | Self::CreateChatFolder { .. }
            | Self::ReadFolder { .. }
            | Self::ReadFile { .. } => pb::ErrorCode::StorageFailed,
        }
    }
}

/// What the daemon's own store can fail at.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("couldn't create the folder {path} for humatad's store; check that it is writable")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't open humatad's store {path}; check that it is readable and writable and the disk isn't full"
    )]
    Open {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "couldn't update humatad's store {path} to this version; it is unchanged, so an older humatad still reads it. Free some disk space and start humatad again"
    )]
    Migrate {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error(
        "humatad's store {path} was written by a newer humatad (schema {found}, this one knows up to {known}); update Humata. Deleting the store loses your projects"
    )]
    NewerSchema {
        path: PathBuf,
        found: i64,
        known: i64,
    },

    #[error("humatad's store {path} failed; if the disk is full, free some space and try again")]
    Query {
        path: PathBuf,
        #[source]
        source: rusqlite::Error,
    },

    #[error("there is no project {id:?}; list projects to find its id")]
    UnknownProject { id: String },

    #[error("there is no task or subtask {id:?}; list the project's items to find its id")]
    UnknownItem { id: String },

    #[error(
        "humatad has never seen session {id:?}; list sessions to find its id, or check that its host is reachable"
    )]
    UnknownSession { id: String },

    #[error("there is no board change {id}; read the project's board log to find its number")]
    UnknownChange { id: u64 },

    #[error("a task or subtask needs a title; give it one")]
    NoTitle,

    #[error("there is no review thread {id:?}; list the session's threads to find its id")]
    UnknownThread { id: String },

    #[error(
        "there is no comment of yours {id:?} to delete; list the session's threads to find its id. The agent's replies can't be deleted"
    )]
    UnknownComment { id: String },

    #[error(
        "comment {id:?} was sent to the agent with a review, so it can't be taken back; reply to the thread instead"
    )]
    CommentSent { id: String },

    #[error(
        "{title:?} is a subtask, and a subtask can't have its own; add this under its task instead"
    )]
    NestedSubtask { title: String },

    #[error("the task {title:?} is in another project; add the subtask in that project")]
    ParentInAnotherProject { title: String },

    #[error(
        "{title:?} is in another project than the session; pick an item of the session's own project"
    )]
    ItemInAnotherProject { title: String },

    #[error(
        "can't undo adding {title:?}: {subtasks} subtasks and {sessions} sessions are on it now. Move it to Won't do instead, or remove its subtasks and move its sessions first"
    )]
    ItemInUse {
        title: String,
        subtasks: i64,
        sessions: i64,
    },

    #[error(
        "can't bring back {title:?}: its task was removed since. Revert the change that removed the task first"
    )]
    ParentRemoved { title: String },
}

/// What the daemon can fail at while reaching a host.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error(transparent)]
    Store(#[from] StoreError),

    /// The session's host said no; its message says why.
    #[error("{message}")]
    Refused {
        code: pb::ErrorCode,
        message: String,
    },

    #[error("say which file: give its repository's name in the session and its path there")]
    NoFile,

    #[error("a comment needs some text; write what you want the agent to know or change")]
    NoComment,

    #[error(
        "lines {start} to {end} aren't in {path}, which has {lines} lines now; read the file again and pick lines in it"
    )]
    LinesOutside {
        path: String,
        start: u32,
        end: u32,
        lines: usize,
    },

    #[error(
        "there is nothing to send: every comment was sent already or is on a resolved thread. Comment on a line, or reply to a thread, first"
    )]
    NothingToReview,

    #[error(
        "a project needs at least one repository; give the folder of each its sessions work in"
    )]
    NoRepositories,

    #[error(
        "the repository {repository} must be an absolute path to a folder on this Mac; check that it exists"
    )]
    NotAFolder {
        repository: PathBuf,
        #[source]
        source: Option<std::io::Error>,
    },

    #[error("only a host answers this; the daemon sends it to its hosts itself")]
    ForHostsOnly,

    #[error(
        "there is no host named {host:?}; add it with `humata host add` (it is kept in {hosts}), or leave the host empty for this Mac"
    )]
    UnknownHost { host: String, hosts: PathBuf },

    #[error("host {host:?} has no command to reach it; give it one in {hosts}")]
    NoCommand { host: String, hosts: PathBuf },

    #[error(
        "couldn't run {program:?} to reach host {host:?}; check that it is installed and on the PATH"
    )]
    Spawn {
        host: String,
        program: String,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't reach host {host:?}{}{}", said(.stderr), if *.managed {
        "; check that `ssh` to it works without a password prompt, and install humatad there with `humata host install` if it isn't yet"
    } else {
        "; check its command in the hosts file and that humatad runs on it"
    })]
    Connect {
        host: String,
        managed: bool,
        stderr: String,
        #[source]
        source: WireError,
    },

    #[error("there is already a host named {host:?}; pick another name, or remove that one first")]
    HostExists { host: String },

    #[error("{name:?} can't name a host; use letters, digits, '-', '_' or '.'")]
    BadHostName { name: String },

    #[error("a host needs what `ssh` connects to, such as a Host in ~/.ssh/config or user@machine")]
    NoDestination,

    #[error(
        "host {host:?} is reached by a command of its own, so Humata can't install on it or check it over SSH; add it again by its SSH destination to have Humata manage it"
    )]
    NotManaged { host: String },

    #[error(
        "host {host:?} is {theirs:?} and this Mac is {ours:?}, so this humatad won't run there; build humatad for {theirs:?} and give its path to install"
    )]
    OtherPlatform {
        host: String,
        theirs: String,
        ours: String,
    },

    #[error("couldn't read the humatad to install, {path}; check that it exists and is readable")]
    ReadBinary {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("`{script}` over SSH on host {host:?} failed{}; check that `ssh` to it works without a password prompt and its disk isn't full", said(.output))]
    Ssh {
        host: String,
        script: String,
        output: String,
    },

    #[error(
        "couldn't save the hosts file {path}; check that its folder is writable. The change isn't saved"
    )]
    SaveHosts {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the repository {path} has no `origin` remote, so host {host:?} can't clone it; add one with `git remote add origin <url>` that the host can reach, or give the session a folder on the host to work in"
    )]
    NoOrigin {
        host: String,
        path: PathBuf,
        #[source]
        source: Option<std::io::Error>,
    },

    #[error("host {host:?} didn't answer")]
    Host {
        host: String,
        #[source]
        source: ClientError,
    },
}

fn said(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!(" (it said: {stderr})")
    }
}

impl DaemonError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::Store(StoreError::UnknownProject { .. }) => pb::ErrorCode::UnknownProject,
            Self::Store(StoreError::UnknownItem { .. }) => pb::ErrorCode::UnknownItem,
            Self::Store(StoreError::UnknownSession { .. }) => pb::ErrorCode::UnknownSession,
            Self::Store(StoreError::UnknownChange { .. }) => pb::ErrorCode::UnknownBoardChange,
            Self::Store(StoreError::UnknownThread { .. }) => pb::ErrorCode::UnknownThread,
            Self::Store(
                StoreError::NoTitle
                | StoreError::UnknownComment { .. }
                | StoreError::CommentSent { .. }
                | StoreError::NestedSubtask { .. }
                | StoreError::ParentInAnotherProject { .. }
                | StoreError::ItemInAnotherProject { .. }
                | StoreError::ItemInUse { .. }
                | StoreError::ParentRemoved { .. },
            ) => pb::ErrorCode::InvalidRequest,
            Self::Store(_) => pb::ErrorCode::StorageFailed,
            Self::Refused { code, .. } => *code,
            Self::NoFile | Self::NoComment | Self::LinesOutside { .. } | Self::NothingToReview => {
                pb::ErrorCode::InvalidRequest
            }
            Self::UnknownHost { .. } => pb::ErrorCode::UnknownHost,
            Self::SaveHosts { .. } => pb::ErrorCode::StorageFailed,
            Self::HostExists { .. }
            | Self::BadHostName { .. }
            | Self::NoDestination
            | Self::NotManaged { .. }
            | Self::OtherPlatform { .. }
            | Self::ReadBinary { .. }
            | Self::NoOrigin { .. }
            | Self::NoCommand { .. }
            | Self::NotAFolder { .. }
            | Self::NoRepositories
            | Self::ForHostsOnly => pb::ErrorCode::InvalidRequest,
            Self::Spawn { .. } | Self::Connect { .. } | Self::Ssh { .. } => {
                pb::ErrorCode::HostUnavailable
            }
            Self::Host {
                source: ClientError::Refused { code, .. },
                ..
            } => *code,
            Self::Host { .. } => pb::ErrorCode::HostUnavailable,
        }
    }
}

/// What stops humatad from starting.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Host(#[from] HostError),

    #[error(transparent)]
    Mcp(#[from] crate::mcp::McpError),

    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(
        "there is no home folder for humatad's files; set HOME, or pass --state, --cache and --socket"
    )]
    NoHome,

    #[error("couldn't read the hosts file {path}; check that it is readable")]
    ReadHosts {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("the hosts file {path} isn't valid; fix its JSON, or remove it to use only this Mac")]
    BadHosts {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("couldn't create the folder {path} for humatad's socket; check that it is writable")]
    CreateSocketDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "humatad is already running on {path}; use that one, or stop it first. If it is a different user's, set HUMATA_SOCKET to another path"
    )]
    AlreadyRunning { path: PathBuf },

    #[error(
        "couldn't remove the stale socket {path}; delete it by hand once no humatad is running"
    )]
    RemoveStaleSocket {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't listen on {path}; check that its folder is writable and the path is short enough for a socket"
    )]
    Listen {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't make the socket {path} private to you; check that you own it")]
    ProtectSocket {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't accept a connection on the socket")]
    Accept {
        #[source]
        source: std::io::Error,
    },

    #[error("the connection on stdin and stdout failed")]
    Stdio {
        #[source]
        source: WireError,
    },
}
