//! What a caller wants to do, and to what.

use crate::capability::Capability;
use crate::ids::{AgentId, ChannelId, MachineId, RepoRef, SessionId, SessionRef};
use crate::path::RepoPath;

/// One call to check. File and board calls name only the place; the policy
/// works out which capability, if any, they need, since that depends on
/// whose place it is. Everything else names its capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// Read files at a place.
    ReadFiles(Place),
    /// Write files at a place.
    WriteFiles(Place),
    /// Copy files from one place into another (a clone or `shared/`).
    CopyFiles { from: Place, into: Place },
    /// Read a session's record: title, status, repos, branches.
    ReadSession(SessionRef),
    /// Read, add to or edit items on a board.
    Board { board: BoardRef, act: BoardAct },
    /// Anything else, against the target its capability takes.
    Use {
        capability: Capability,
        target: Target,
    },
}

/// Where files are: a machine, the Humata folder on it, and a path in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub machine: MachineId,
    pub root: Root,
    pub path: RepoPath,
    pub entry: Entry,
}

/// The Humata folder a place is in.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Root {
    /// A session's clone.
    Clone(SessionRef),
    /// A channel's `shared/` folder on the place's machine.
    Shared(ChannelId),
    /// A channel's folder on the place's machine.
    ChannelFolder(ChannelId),
    /// A main clone, the read-only reference new clones start from (FC-2).
    MainClone { channel: ChannelId, repo: RepoRef },
    /// The user's home folder on the place's machine (FM-10). The user
    /// only: no agent reaches it and no grant covers it.
    Home,
    /// What was pasted into a session's terminals (D335). The user only,
    /// like the home folder.
    Pastes(SessionRef),
    /// Where files sent from another machine land, `received/<machine>/`
    /// (PL-4). The user only, like the home folder.
    Received,
    /// The path left Humata's folders, by `..` or a symlink. Never allowed.
    Outside,
}

/// Whether a place's path names a file or a folder. A grant made from a
/// file read covers the folder holding the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Entry {
    File,
    Folder,
}

impl Place {
    /// The folder a request for this place asks about: the path itself for
    /// a folder, the folder holding it for a file.
    pub fn folder(&self) -> RepoPath {
        match self.entry {
            Entry::Folder => self.path.clone(),
            Entry::File => self.path.parent(),
        }
    }
}

/// A board (`BoardRef`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BoardRef {
    Global,
    Channel(ChannelId),
    Session(SessionRef),
}

/// What a board call does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardAct {
    Read,
    Add,
    /// Edit, move or delete an item. `created_by` is the session that made
    /// it, or `None` when the user or a script did.
    Edit { created_by: Option<SessionId> },
}

/// The target of a [`Call::Use`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Across channels: the global board, every channel, creating one.
    Global,
    /// One channel as a whole.
    Channel(ChannelId),
    /// One session.
    Session(SessionRef),
    /// Start or fork a session with this agent on this machine.
    Start {
        channel: ChannelId,
        machine: MachineId,
        agent: AgentId,
        /// The session being forked; `None` for a fresh start. A channel
        /// Desk session's "Start and fork sessions" switch covers a fork of
        /// its target only (CD-3).
        fork_of: Option<SessionId>,
    },
    /// A port forward from `machine` to `host:port` (HO-11).
    Network(NetworkTarget),
    /// A push or pull request for this repo and branch (IN-24).
    Push(RepoBranch),
    /// One action on one machine (MP-24).
    Machine(MachineAction),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NetworkTarget {
    /// The machine the forward opens from: the requester's own.
    pub machine: MachineId,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepoBranch {
    pub repo: RepoRef,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MachineAction {
    pub machine: MachineId,
    pub action: MachineActionKind,
}

/// `humata.v1.MachineActionKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MachineActionKind {
    Add,
    Create,
    Start,
    Stop,
    Reconnect,
    SyncSkills,
    Remove,
    Forget,
    Delete,
    /// Stop every session on the machine (STOP, D263).
    StopSessions,
    /// Start them again after a stop.
    StartSessions,
}

impl MachineActionKind {
    /// The capability the action needs (MP-24).
    pub fn capability(self) -> Capability {
        match self {
            Self::Add
            | Self::Create
            | Self::Start
            | Self::Stop
            | Self::Reconnect
            | Self::SyncSkills => Capability::MachineProvision,
            Self::Remove | Self::Forget | Self::Delete => Capability::MachineDestroy,
            Self::StopSessions | Self::StartSessions => Capability::MachineSessionsStop,
        }
    }
}

/// What a grant covers (IN-22, `GrantTarget`). The deprecated proto kinds
/// (`machine`, `board_item`) are not modelled.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GrantTarget {
    /// Note, question or typing into that session.
    Session(SessionId),
    /// Edit, move or delete others' items on that board.
    Board(BoardRef),
    /// Read or copy under a folder of a session or machine.
    Path(PathScope),
    /// A port forward from that machine to that host and port.
    Network(NetworkTarget),
    /// Start or fork a session on that machine with that agent.
    AgentOnMachine { machine: MachineId, agent: AgentId },
    /// Push or open a pull request for that repo and branch.
    RepoBranch(RepoBranch),
    /// One action on one machine.
    MachineAction(MachineAction),
}

/// A folder of a session's clone or of another Humata folder on a machine.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PathScope {
    pub machine: MachineId,
    pub root: Root,
    /// The folder; the root path means the whole clone or folder.
    pub prefix: RepoPath,
}

impl GrantTarget {
    /// True when a grant on `self` covers a call whose exact target is
    /// `asked`. Paths are covered by a folder above them; everything else
    /// must be equal.
    pub fn covers(&self, asked: &GrantTarget) -> bool {
        match (self, asked) {
            (GrantTarget::Path(granted), GrantTarget::Path(asked)) => {
                granted.machine == asked.machine
                    && granted.root == asked.root
                    && granted.prefix.contains(&asked.prefix)
            }
            _ => self == asked,
        }
    }

    /// True when this kind of target is the one IN-22 names for the
    /// capability.
    pub fn fits(&self, capability: Capability) -> bool {
        match self {
            GrantTarget::Path(scope) => {
                matches!(
                    capability,
                    Capability::FilesReadOtherSession
                        | Capability::FilesReadOtherMachine
                        | Capability::FilesCopyIn
                ) && !matches!(scope.root, Root::Outside | Root::Home | Root::Pastes(_) | Root::Received)
            }
            GrantTarget::Session(_) => {
                matches!(capability, Capability::SessionNote | Capability::SessionType)
            }
            GrantTarget::AgentOnMachine { .. } => matches!(
                capability,
                Capability::SessionStartSameMachine | Capability::SessionStartOtherMachine
            ),
            GrantTarget::Board(_) => matches!(
                capability,
                Capability::ChannelBoardEditOthers | Capability::GlobalBoardWrite
            ),
            GrantTarget::Network(_) => capability == Capability::NetworkReach,
            GrantTarget::RepoBranch(_) => capability == Capability::GitPush,
            GrantTarget::MachineAction(action) => action.action.capability() == capability,
        }
    }
}
