use std::path::PathBuf;

/// A name that cannot become a folder.
#[derive(Debug, thiserror::Error)]
pub enum NameError {
    #[error(
        "the name {name:?} has no letters or digits, so no folder name can be made from it; \
         give the channel a name with at least one letter or digit"
    )]
    NoLettersOrDigits { name: String },
    #[error(
        "the name {name:?} would make the folder {folder:?}, which Desk sessions use; \
         give the channel another name, such as {name:?} with a word added"
    )]
    Reserved { name: String, folder: String },
}

/// A git step that failed.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error(
        "could not run git ({args}): {source}; check that git is installed and on the \
         daemon's PATH"
    )]
    Spawn {
        args: String,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "git {args} failed ({status}): {stderr}; check the repository address, that this \
         machine can reach it, and that it has access (git never asks for a password here)"
    )]
    Failed {
        args: String,
        status: String,
        stderr: String,
    },
    #[error(
        "could not prepare {path}: {source}; check the folder is writable and that nothing \
         else is using it"
    )]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the repository at {path} has no branch to start from; make sure the remote has at \
         least one commit"
    )]
    NoDefaultBranch { path: PathBuf },
    #[error("the repository address template {template:?} must contain {{owner}} and {{name}}")]
    BadTemplate { template: String },
}

/// An agent definition problem.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("there is no agent named {name:?}; the agents are: {known}")]
    Unknown { name: String, known: String },
    #[error("the agent definitions are not valid; fix the JSON in Settings > Agents")]
    Parse(#[source] serde_json::Error),
    #[error("the agent {name:?} is defined twice; give each agent a different name")]
    Duplicate { name: String },
}

/// A folder step that failed.
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("could not create {path}: {source}; check the Humata folder is writable")]
    Create {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Why setting up a channel failed.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error(transparent)]
    Name(#[from] NameError),
    #[error(
        "the folder {folder:?} is already used by the channel {other:?}; pick another name for \
         this channel"
    )]
    FolderTaken { folder: String, other: String },
    #[error("the channel {id} is not saved; create it first")]
    NoChannel { id: String },
    #[error(
        "could not {doing}: {source}; check that the Humata state folder is writable and the \
         disk has space"
    )]
    Store {
        doing: &'static str,
        #[source]
        source: humata2_store::StoreError,
    },
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error("a background task stopped before it finished; restart humatad2 and try again")]
    Background {
        #[source]
        source: tokio::task::JoinError,
    },
}

/// Why starting a session failed.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("the channel {id} does not exist; pick a channel from the list")]
    NoChannel { id: String },
    #[error("the channel {channel} has no repository {owner}/{name}; pick one of its repositories")]
    UnknownRepo {
        channel: String,
        owner: String,
        name: String,
    },
    #[error("the channel {channel} has no repositories to start a session in; add one first")]
    NoRepos { channel: String },
    #[error(
        "{owner}/{name} is not cloned yet ({state}); wait for the channel's setup to finish, or \
         if it failed, fix the cause and set the channel up again"
    )]
    NotReady {
        owner: String,
        name: String,
        state: String,
    },
    #[error(transparent)]
    Agent(#[from] AgentError),
    #[error("{owner}/{name}: {source}")]
    Git {
        owner: String,
        name: String,
        #[source]
        source: GitError,
    },
    #[error("{branch:?} is not a name git accepts for a branch; use letters, digits, `-`, `_`, `.` and `/`, with no spaces, `..`, `~`, `^`, `:`, `?`, `*`, `[` or `\\`")]
    BadBranch { branch: String },
    #[error("the branch {branch:?} is already used; use another name, such as {suggestion:?}")]
    BranchTaken { branch: String, suggestion: String },
    #[error("the session folder {path} already exists; start the session with another title")]
    FolderExists { path: PathBuf },
    #[error(
        "could not {doing}: {source}; check that the Humata state folder is writable and the \
         disk has space"
    )]
    Store {
        doing: &'static str,
        #[source]
        source: humata2_store::StoreError,
    },
    #[error("could not start the agent: {source}")]
    Revive {
        #[source]
        source: humata2_revive::ReviveError,
    },
    #[error(
        "the agent started but could not be given its first message: {source}; type it into \
         the session yourself"
    )]
    FirstMessage {
        #[source]
        source: humata2_holder::HolderError,
    },
    #[error("could not remove {path}: {source}; delete the folder by hand")]
    Remove {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the session {id} does not exist; pick a session from the list")]
    NoSession { id: String },
    #[error(
        "the session {session} has no agent to copy the conversation from; fork it with the \
         code only, or start the session's agent first"
    )]
    NoAgent { session: String },
    #[error(
        "could not read the conversation at {path}: {source}; fork with the code only, or \
         check the file is still there"
    )]
    NoConversation {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "could not fork the conversation {id}: {source}. Nothing was kept; fork with the code only, \
         or fork from now once the agent has saved a turn"
    )]
    Conversation {
        id: String,
        #[source]
        source: humata2_conversation::ConversationError,
    },
    #[error(
        "this agent's conversation is in a format Humata can't fork; fork with the code only, \
         or fork it as a handover to another agent"
    )]
    NoConversationFormat,
    #[error("could not copy to {path}: {source}; check the disk has space and is writable")]
    Copy {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("a background task stopped before it finished; restart humatad2 and try again")]
    Background {
        #[source]
        source: tokio::task::JoinError,
    },
}
