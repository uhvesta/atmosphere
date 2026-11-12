use std::path::PathBuf;

use humata_agent::{AgentError, WorkspaceError};
use humata_wire::{ClientError, WireError, pb};

/// What the host role can fail at, answered to the daemon as an error.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error(transparent)]
    Agent(#[from] AgentError),

    #[error(transparent)]
    Terminal(#[from] crate::terminals::TerminalError),

    #[error(transparent)]
    Port(#[from] crate::ports::PortError),

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

    #[error("{path:?} isn't in the folder being browsed; give a path from its list of files")]
    NotInFolder { path: String },

    #[error(
        "the watcher fell {missed} events behind and stopped; watch the session again to catch up from its history"
    )]
    FellBehind { missed: u64 },

    #[error(
        "only the daemon keeps projects and the board; send this to humatad's daemon on your Mac, not a host"
    )]
    NotForHosts,

    #[error(
        "no agent's tool call {id} is waiting for an answer; it may have timed out, or the daemon reconnected since"
    )]
    UnknownToolCall { id: u64 },

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

    #[error(
        "couldn't push the branch {branch} of {repository} to its origin (git said: {output}). Check that the host's GitHub token can write to the repository. If the branch is on the remote already with other commits, don't force-push over them: land on a new branch name instead"
    )]
    Push {
        repository: String,
        branch: String,
        output: String,
    },

    #[error(
        "couldn't fetch {repository}'s origin on this host (git said: {output}), so nothing was merged. Check that the host can reach the remote and its token can read it, then update again"
    )]
    Fetch { repository: String, output: String },

    #[error("{name:?} isn't a plain folder name; give a name without slashes or ..")]
    NotAName { name: String },

    #[error(
        "couldn't read the {part} in a session's recovery copy; it is damaged, or was made by a newer humatad. Update humatad on this host and restore again"
    )]
    BadBackup {
        part: &'static str,
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "couldn't encode a session's recovery copy as JSON, which is a bug in humatad; report it with the host's log"
    )]
    EncodeBackup {
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "the recovery copy has snapshots of the repository {name}, but the restore wasn't given a clone of it on this host. Clone the project's repositories here and restore again"
    )]
    NoClone { name: String },

    #[error(
        "couldn't move the snapshots of the repository {name} on this host. Check that git works there and the disk has room, then try again"
    )]
    Snapshots {
        name: String,
        #[source]
        source: WorkspaceError,
    },
}

impl HostError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::Agent(AgentError::UnknownSession { .. }) => pb::ErrorCode::UnknownSession,
            Self::Agent(AgentError::UnknownEvent { .. }) => pb::ErrorCode::UnknownEvent,
            Self::Agent(AgentError::UnknownPermissionRequest { .. }) => {
                pb::ErrorCode::UnknownPermissionRequest
            }
            Self::Agent(AgentError::UnknownQueuedMessage { .. }) => {
                pb::ErrorCode::UnknownQueuedMessage
            }
            Self::Agent(AgentError::Archived { .. }) => pb::ErrorCode::Archived,
            Self::Agent(AgentError::UnknownSkipped { .. }) => pb::ErrorCode::UnknownSession,
            Self::Agent(AgentError::UnknownImport { .. } | AgentError::NothingToImport { .. }) => {
                pb::ErrorCode::UnknownImport
            }
            Self::Agent(AgentError::AlreadyImported { .. }) => pb::ErrorCode::AlreadyImported,
            Self::Agent(AgentError::ImportedFolderMissing { .. } | AgentError::NotImportable) => {
                pb::ErrorCode::InvalidRequest
            }
            Self::Agent(
                AgentError::ArchiveWhileWorking { .. }
                | AgentError::LandWhileWorking { .. }
                | AgentError::UpdateWhileWorking { .. },
            ) => pb::ErrorCode::Busy,
            Self::Agent(
                AgentError::NoWorkspaces { .. }
                | AgentError::SessionExists { .. }
                | AgentError::MissingRepository { .. }
                | AgentError::BadBackendFile { .. }
                | AgentError::NoCodeToLand { .. }
                | AgentError::DeleteUnarchived { .. }
                | AgentError::DeleteForked { .. },
            ) => pb::ErrorCode::InvalidRequest,
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
                    | WorkspaceError::NoSuchFile { .. }
                    | WorkspaceError::BadBranch { .. }
                    | WorkspaceError::BranchTaken { .. }
                    | WorkspaceError::NoBase { .. }
                    | WorkspaceError::NoIdentity { .. },
                ..
            }) => pb::ErrorCode::InvalidRequest,
            Self::Agent(AgentError::Workspace { .. }) => pb::ErrorCode::BackendFailed,
            Self::Agent(AgentError::Copilot(_) | AgentError::ModelServer(_)) => {
                pb::ErrorCode::BackendFailed
            }
            Self::Agent(_) => pb::ErrorCode::StorageFailed,
            Self::UnknownBackend { .. } => pb::ErrorCode::UnknownBackend,
            Self::Terminal(error) => error.code(),
            Self::Port(error) => error.code(),
            Self::NotForHosts
            | Self::UnknownToolCall { .. }
            | Self::NotInFolder { .. }
            | Self::NotAName { .. }
            | Self::BadBackup { .. }
            | Self::NoClone { .. }
            | Self::NoFile
            | Self::NoAction
            | Self::NoBranch
            | Self::NoTurn => pb::ErrorCode::InvalidRequest,
            Self::FellBehind { .. }
            | Self::RunGit { .. }
            | Self::Clone { .. }
            | Self::Push { .. }
            | Self::Fetch { .. }
            | Self::Snapshots { .. } => pb::ErrorCode::BackendFailed,
            Self::EncodeBackup { .. } => pb::ErrorCode::StorageFailed,
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
        "no request {id:?} waits in the inbox; it may have been answered already, so list the inbox again"
    )]
    UnknownRequest { id: String },

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

    #[error(
        "proposal {proposal} was applied or declined already; ask the agent for a new one to change it again"
    )]
    AlreadyDecided { proposal: String },
}

/// What the daemon can fail at while reaching a host.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error(transparent)]
    Store(#[from] StoreError),

    #[error(transparent)]
    Commands(#[from] crate::commands::CommandsError),

    #[error(transparent)]
    GitHub(#[from] crate::github::GitHubError),

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

    #[error("say what to ask the person, or for a move, what you did")]
    NoRequestText,

    #[error(
        "an agent asks to move its task only to in_review, done or wont_do, which the person approves; set other statuses with update_item"
    )]
    NotAMove,

    #[error("this session works on no task, so there's nothing to move; give item_id, from board")]
    NoTaskToMove,

    #[error("a question is answered with a reply, not approved; reply to it or dismiss it")]
    NothingToApprove,

    #[error(
        "a reply needs some text; write what the agent should know, or dismiss the request instead"
    )]
    NoReply,

    #[error(
        "{port} isn't a port the model server can listen on; give its port from 1 to 65535, as it printed when it started"
    )]
    ModelServerPort { port: u32 },

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
        "there are no words to search for; give at least one word, not only punctuation or a time such as \"last week\""
    )]
    NothingToSearch,

    #[error(
        "the repository {repository} must be an absolute path to a folder on this Mac; check that it exists"
    )]
    NotAFolder {
        repository: PathBuf,
        #[source]
        source: Option<std::io::Error>,
    },

    #[error(
        "couldn't clone {url}: {output}. Check the URL, and that git on this Mac can reach it: for a private repository, sign git in, as with `gh auth login`"
    )]
    CloneUrl { url: String, output: String },

    #[error(
        "couldn't keep the clones of repositories added by URL in {path}; check that the disk has room and the cache folder is writable"
    )]
    CreateClones {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't run git; install it, as with `xcode-select --install`, and try again")]
    RunGit {
        #[source]
        source: std::io::Error,
    },

    #[error("only a host answers this; the daemon sends it to its hosts itself")]
    ForHostsOnly,

    #[error(
        "this Mac has no setup script; Humata only runs one on remote hosts. Install what its agents need on this Mac yourself"
    )]
    NoSetupHere,

    #[error("port {port} isn't a TCP port; give one from 1 to 65535")]
    BadPort { port: u32 },

    #[error(
        "couldn't listen on 127.0.0.1 on this Mac to forward the port; another program may hold every port, so close some and try again"
    )]
    Listen {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "{host} answered a port connection with something else; update humatad on {host} to this Mac's build, then forward the port again"
    )]
    NotAPortStream { host: String },

    #[error(
        "no forwarded port {id}; it was stopped already, or the daemon restarted since. Forward the port again"
    )]
    UnknownForward { id: String },

    #[error(
        "an agent on {host} asked for {asked}, which is outside the projects of the sessions running there; its humata tools reach only those sessions' boards and review threads"
    )]
    OutOfScope { host: String, asked: &'static str },

    #[error(
        "{name} still has {working} sessions that aren't archived; archive them first. Its archived sessions are kept, under Archived"
    )]
    ProjectInUse { name: String, working: usize },

    #[error(
        "{title:?} has {live} sessions that aren't archived; archive them or move them to another task, then delete it"
    )]
    ItemHasSessions { title: String, live: usize },

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

    #[error(
        "host {host:?} took more than {seconds} seconds to answer; check that it is on and its network is up, then use Check in Hosts"
    )]
    ConnectTimedOut { host: String, seconds: u64 },

    #[error(
        "host {host:?} couldn't be reached a moment ago ({why}); humatad tries again in {seconds} seconds, or now when you use Check in Hosts"
    )]
    Resting {
        host: String,
        why: String,
        seconds: u64,
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
        "host {host:?} is {platform:?}, and this Humata has no humatad built for it beside {looked_in}; install a Humata that includes that platform, or build humatad for it and give its path to `humata host install --binary`"
    )]
    NoBuildFor {
        host: String,
        platform: String,
        looked_in: PathBuf,
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

    #[error(
        "{name:?} isn't archived, so it can't be deleted; archive it first. Deleting can't be undone"
    )]
    DeleteUnarchived { name: String },

    #[error(
        "{name:?} can't be deleted while {fork:?} was forked from it; delete that fork first, or keep both"
    )]
    DeleteForked { name: String, fork: String },

    #[error(
        "{name:?} can't be deleted while its host {host} can't be reached: its files there would be left behind. Try again once {host} is back"
    )]
    DeleteOffline { name: String, host: String },

    #[error("{name:?} is in a turn; stop it or wait for it to finish, then move it")]
    MoveWhileWorking { name: String },

    #[error("{name:?} is on {host} already")]
    AlreadyThere { name: String, host: String },

    #[error(
        "{name:?} can't move while {fork:?}, forked from it, is on the same machine: its copy there is deleted once it moves. Move or delete the fork first"
    )]
    MoveForked { name: String, fork: String },

    #[error(
        "couldn't delete session {session}'s recovery copy at {path}; check that the folder is writable and try again"
    )]
    DeleteRecovery {
        session: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't keep session {session}'s code in its recovery copy at {path}; check that the disk has room and the folder is writable. Deleting the folder loses the code a restore needs"
    )]
    Mirror {
        session: String,
        path: PathBuf,
        #[source]
        source: humata_agent::WorkspaceError,
    },

    #[error(
        "host {host:?} named a repository {name:?}, which can't name a folder; the host may run another build of humatad, so install this one on it"
    )]
    BadRepositoryName { host: String, name: String },

    #[error(
        "there is no recovery copy of session {session}: one is kept after each turn while its host is reachable, so a session that never finished a turn while it was has none"
    )]
    NoRecovery { session: String },

    #[error(
        "session {session} has no proposal {proposal}; list its proposals, since the agent may have proposed it in a session forked from this one"
    )]
    UnknownProposal { session: String, proposal: String },

    #[error(
        "session {session} isn't in a project, so there is no board for its proposals; start a planning session in a project"
    )]
    NoBoard { session: String },

    #[error(
        "the agent's proposal {proposal} names no item to change, or one no longer on the board; decline it and ask the agent to propose it again"
    )]
    NoProposedItem { proposal: String },

    #[error(
        "a proposed task needs a title, and a proposed change needs item_id and a new title or spec, or both; give them and propose again"
    )]
    IncompleteProposal,

    #[error(
        "only the Humata app can decide approvals and revoke grants; open Humata and decide it in the inbox, or revoke it in Settings > Approvals"
    )]
    AppOnly,

    #[error(
        "there is no approval {id}; decided ones are kept only for the last 200 actions, so it may be gone. Ask again if the action is still wanted"
    )]
    UnknownApproval { id: String },

    #[error("there is no grant {id}; list the grants in force with `humata approval list`")]
    UnknownGrant { id: String },

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
            Self::Store(StoreError::UnknownRequest { .. }) => pb::ErrorCode::UnknownRequest,
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
            Self::Store(StoreError::AlreadyDecided { .. }) | Self::UnknownProposal { .. } => {
                pb::ErrorCode::UnknownProposal
            }
            Self::Store(_)
            | Self::Commands(_)
            | Self::CreateClones { .. }
            | Self::RunGit { .. } => pb::ErrorCode::StorageFailed,
            Self::Refused { code, .. } => *code,
            Self::GitHub(_) => pb::ErrorCode::Github,
            Self::AppOnly => pb::ErrorCode::AppOnly,
            Self::UnknownApproval { .. } | Self::UnknownGrant { .. } => {
                pb::ErrorCode::UnknownApproval
            }
            Self::ProjectInUse { .. } | Self::ItemHasSessions { .. } => pb::ErrorCode::Busy,
            Self::DeleteUnarchived { .. } | Self::DeleteForked { .. } => {
                pb::ErrorCode::InvalidRequest
            }
            Self::DeleteOffline { .. } => pb::ErrorCode::HostUnavailable,
            Self::MoveWhileWorking { .. } => pb::ErrorCode::Busy,
            Self::AlreadyThere { .. } | Self::MoveForked { .. } => pb::ErrorCode::InvalidRequest,
            Self::DeleteRecovery { .. } => pb::ErrorCode::StorageFailed,
            Self::NoFile
            | Self::NoComment
            | Self::LinesOutside { .. }
            | Self::NothingToReview
            | Self::NoRequestText
            | Self::NotAMove
            | Self::NoTaskToMove
            | Self::NothingToApprove
            | Self::NoReply
            | Self::ModelServerPort { .. } => pb::ErrorCode::InvalidRequest,
            Self::UnknownHost { .. } => pb::ErrorCode::UnknownHost,
            Self::SaveHosts { .. } | Self::Mirror { .. } => pb::ErrorCode::StorageFailed,
            Self::BadRepositoryName { .. } | Self::NoRecovery { .. } => {
                pb::ErrorCode::InvalidRequest
            }
            Self::HostExists { .. }
            | Self::BadHostName { .. }
            | Self::NoDestination
            | Self::NotManaged { .. }
            | Self::NoBuildFor { .. }
            | Self::ReadBinary { .. }
            | Self::NoOrigin { .. }
            | Self::NoCommand { .. }
            | Self::NotAFolder { .. }
            | Self::NoRepositories
            | Self::NothingToSearch
            | Self::CloneUrl { .. }
            | Self::ForHostsOnly
            | Self::NoSetupHere
            | Self::BadPort { .. }
            | Self::OutOfScope { .. }
            | Self::NoBoard { .. }
            | Self::NoProposedItem { .. }
            | Self::IncompleteProposal => pb::ErrorCode::InvalidRequest,
            Self::Listen { .. } => pb::ErrorCode::Busy,
            Self::NotAPortStream { .. } => pb::ErrorCode::HostUnavailable,
            Self::UnknownForward { .. } => pb::ErrorCode::UnknownForward,
            Self::Spawn { .. }
            | Self::Connect { .. }
            | Self::ConnectTimedOut { .. }
            | Self::Resting { .. }
            | Self::Ssh { .. } => pb::ErrorCode::HostUnavailable,
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

    #[error("couldn't find this humatad's own program to start the host with")]
    OwnProgram {
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't open the host's log {path}; check that its folder is writable")]
    OpenHostLog {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("couldn't start the host program {program}; check that it is there and executable")]
    SpawnHost {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("the host stopped before it was listening ({status}); its log {log} says why:\n{said}")]
    HostExited {
        status: std::process::ExitStatus,
        log: PathBuf,
        said: String,
    },

    #[error(
        "the host started but isn't listening on {socket} after {seconds} seconds; see its log {log}, and stop it by hand if it is stuck"
    )]
    HostNotListening {
        socket: PathBuf,
        seconds: u64,
        log: PathBuf,
    },

    #[error("the relay between this connection and the host's socket {socket} failed")]
    Relay {
        socket: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Why an agent's call to the humata tools on a host wasn't answered.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error(
        "the humata tools answer the board and review threads only, and this isn't one of their requests"
    )]
    NotATool,

    #[error(
        "Humata on the Mac isn't connected to this host, so the board and review threads can't be reached; open Humata on the Mac and try again, the session's work is kept"
    )]
    NotWatched,

    #[error(
        "Humata on the Mac didn't answer within {seconds}s; it may have lost its connection to this host, so try again"
    )]
    TimedOut { seconds: u64 },

    #[error(
        "couldn't read the questions and move requests agents made while the Mac was away, from {path}"
    )]
    ReadQueue {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "the questions and move requests agents made while the Mac was away, in {path}, aren't in a form this build reads"
    )]
    BadQueue {
        path: PathBuf,
        #[source]
        source: prost::DecodeError,
    },

    #[error(
        "Humata on the Mac isn't connected, and the request couldn't be kept for when it is, in {path}; check the host's disk has room and its state folder is writable, then ask again"
    )]
    WriteQueue {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl RelayError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::NotATool => pb::ErrorCode::InvalidRequest,
            Self::NotWatched | Self::TimedOut { .. } => pb::ErrorCode::HostUnavailable,
            Self::ReadQueue { .. } | Self::BadQueue { .. } | Self::WriteQueue { .. } => {
                pb::ErrorCode::StorageFailed
            }
        }
    }
}
