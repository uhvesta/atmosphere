//! The Desk (DK-1 to DK-5, DK-10 to DK-14, SE-26): sessions that belong to
//! no channel, each working in a scratch folder Humata made or in a folder
//! the person picked.
//!
//! A Desk session is an ordinary [`Session`] whose channel is [`channel`],
//! so its page, terminals and panels work like any session's. What only the
//! Desk knows about it (its folder, repos, the two access switches, when it
//! was last used, whether it is closed) is a [`DeskSession`] beside it in
//! [`DeskData`], which the workspace carries. [`DeskClient`] is what the app
//! asks a daemon to do with them; the fake daemon is the only
//! implementation until a real daemon serves Desk sessions.

use std::collections::BTreeMap;

use crate::daemon::Reply;
use crate::model::{Agent, ChannelId, MachineId, Session, SessionId};

/// The Humata root the fakes use on every machine (FC-1).
pub const ROOT: &str = "~/Humata";

/// The folder under a machine's root that holds scratch folders. No channel
/// may take this name (DK-2).
pub const FOLDER: &str = "desk";

/// The longest name a scratch folder gets from a title (FC-13).
const NAME_LIMIT: usize = 40;

/// The channel id every Desk session carries. No channel has it, because no
/// channel can be named after the Desk's folder (DK-2).
#[must_use]
pub fn channel() -> ChannelId {
    ChannelId::new(FOLDER)
}

/// Whether `session` is a Desk session.
#[must_use]
pub fn is_desk(session: &Session) -> bool {
    session.channel.as_str() == FOLDER
}

/// The words for where a session lives: "Desk" for the Desk's id, `# name`
/// for a channel, and the bare id for one the workspace doesn't know.
#[must_use]
pub fn place(workspace: &crate::workspace::Workspace, channel: &ChannelId) -> String {
    if channel.as_str() == FOLDER {
        "Desk".to_owned()
    } else {
        workspace.channel(channel).map_or_else(
            || channel.as_str().to_owned(),
            |channel| format!("# {}", channel.name),
        )
    }
}

/// The folder name a title gives (FC-13): lower case, letters and digits
/// kept, everything else turned into single hyphens, cut at a word boundary
/// to at most 40 characters. Empty when the title has no letter or digit.
#[must_use]
pub fn folder_name(title: &str) -> String {
    let mut name = String::new();
    for word in title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        let word = word.to_lowercase();
        let joined = if name.is_empty() {
            word.chars().count()
        } else {
            name.chars().count() + 1 + word.chars().count()
        };
        if joined > NAME_LIMIT {
            if name.is_empty() {
                name = word.chars().take(NAME_LIMIT).collect();
            }
            break;
        }
        if !name.is_empty() {
            name.push('-');
        }
        name.push_str(&word);
    }
    name
}

/// Where the scratch folder called `name` sits under the default root.
#[must_use]
pub fn scratch_path(name: &str) -> String {
    format!("{ROOT}/{FOLDER}/{name}")
}

/// Where a Desk session works (DK-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Folder {
    /// A folder Humata made under the machine's root. It is deleted 7 days
    /// after the session closes unless it was kept.
    Scratch {
        /// Its path, such as `~/Humata/desk/plan-q4`.
        path: String,
        /// Whether the person kept it, so it stays after the session closes.
        kept: bool,
    },
    /// A folder the person picked. Humata never deletes, trashes or moves it.
    Picked {
        /// Its path on the session's machine.
        path: String,
    },
}

impl Folder {
    /// Its path.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Scratch { path, .. } | Self::Picked { path } => path,
        }
    }

    /// Whether Humata made it.
    #[must_use]
    pub const fn is_scratch(&self) -> bool {
        matches!(self, Self::Scratch { .. })
    }

    /// Whether it goes 7 days after its session closes: a scratch folder
    /// nobody kept.
    #[must_use]
    pub const fn expires(&self) -> bool {
        matches!(self, Self::Scratch { kept: false, .. })
    }
}

/// A Desk session's two Agent access switches (DK-4, DK-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    /// Read and change every channel's board without asking.
    pub boards: bool,
    /// Make channels and add machines without asking.
    pub channels_and_machines: bool,
}

impl Default for Access {
    /// Boards on, channels and machines off, as a new Desk session starts.
    fn default() -> Self {
        Self {
            boards: true,
            channels_and_machines: false,
        }
    }
}

/// When the person last typed into a session, opened it or answered a
/// request from it (SE-26). What the agent does on its own doesn't count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastUsed {
    /// Orders sessions by use: the larger, the more recent.
    pub order: u64,
    /// The time as people read it: the live daemon's client says how long
    /// ago, such as "now", "9m ago" or "2d ago"; the design fixture draws
    /// clock and day labels such as "Today 12:05" or "Nov 2".
    pub label: String,
}

/// A closed Desk session's state (DK-11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Closed {
    /// When it was closed, as people read it: "3h ago" from the live
    /// daemon's client, "Nov 14" in the design fixture.
    pub when: String,
    /// Days until its folder is deleted. `None` when the folder stays: it
    /// was kept, the person picked it, or unpushed work holds it.
    pub days_left: Option<u32>,
}

/// What the Desk knows about one of its sessions, beside its [`Session`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeskSession {
    /// The session.
    pub session: SessionId,
    /// Where it works.
    pub folder: Folder,
    /// The repositories cloned into its folder, such as `acme/ledger` (DK-3).
    pub repos: Vec<String>,
    /// Its Agent access switches.
    pub access: Access,
    /// When it was last used.
    pub last_used: LastUsed,
    /// Set once it is closed; `None` while it is open.
    pub closed: Option<Closed>,
    /// Uncommitted or unpushed work in its clones, in words, such as
    /// "acme/api: 2 commits on port-cron never pushed". Shown only where it
    /// blocks deleting the folder (FC-9, DK-11).
    pub unpushed: Option<String>,
}

/// A markdown file in a Desk session's folder (DK-5).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Document {
    /// The session whose folder holds it.
    pub session: SessionId,
    /// Its path inside that folder, such as `rfc-brin.md`.
    pub path: String,
}

/// Everything the Desk holds, as the workspace carries it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeskData {
    /// What the Desk knows about each of its sessions, open and closed.
    pub sessions: BTreeMap<SessionId, DeskSession>,
    /// The markdown files in Desk folders, in the order the Desk lists them.
    pub documents: Vec<Document>,
    /// How many bytes each scratch folder holds, as the fake host's disk
    /// reports it. The app reads sizes only through
    /// [`DeskClient::clean_up_list`].
    pub folder_bytes: BTreeMap<SessionId, u64>,
    /// The branch each clone in a Desk folder is on, by session and
    /// repository: the repository's default branch, since a Desk session has
    /// no branch of its own (DK-3).
    pub clone_branches: BTreeMap<(SessionId, String), String>,
}

/// Which folder a new Desk session gets (DK-14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderChoice {
    /// A new scratch folder with this name, as the dialog showed it. The
    /// daemon adds a suffix (`-2`, `-3`, …) only when the name is taken.
    Scratch {
        /// The folder's name, such as `plan-q4`.
        name: String,
    },
    /// A folder on the machine that the person picked.
    Picked {
        /// Its path there.
        path: String,
    },
}

/// What New desk session asks for (DK-14).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewDeskSession {
    /// Its title.
    pub title: String,
    /// What it runs: an agent, or [`Agent::Shell`] for a plain terminal.
    pub agent: Agent,
    /// The machine it runs on: any online one.
    pub machine: MachineId,
    /// Its folder.
    pub folder: FolderChoice,
    /// Repositories to clone into the folder.
    pub repos: Vec<String>,
    /// Its Agent access switches.
    pub access: Access,
}

/// A closed session as Clean up… lists it (DK-11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanUpRow {
    /// The closed session.
    pub session: SessionId,
    /// How much deleting it frees: the size of its scratch folder. `None`
    /// for a picked folder, which is left in place.
    pub size_bytes: Option<u64>,
    /// Uncommitted or unpushed work in its clones, in words, such as
    /// "acme/api: 2 commits on port-cron never pushed". Deleting the session
    /// deletes that work (MV-9).
    pub unpushed: Option<String>,
}

/// What a clean-up did (NV-27).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanUp {
    /// The ticked sessions went to the trash.
    Deleted {
        /// How many sessions were deleted.
        sessions: usize,
        /// How many bytes their scratch folders held.
        freed_bytes: u64,
    },
    /// The closed sessions are no longer what the dialog showed, so nothing
    /// was deleted. This is the list now, to show and ask again.
    Changed(Vec<CleanUpRow>),
}

/// The Desk calls a fake can be told to fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    /// [`DeskClient::start`].
    Start,
    /// [`DeskClient::close`].
    Close,
    /// [`DeskClient::reopen`].
    Reopen,
    /// [`DeskClient::keep`].
    Keep,
    /// [`DeskClient::clean_up_list`].
    CleanUpList,
    /// [`DeskClient::clean_up`].
    CleanUp,
    /// [`DeskClient::copy_document`].
    CopyDocument,
    /// [`DeskClient::add_repo`].
    AddRepo,
    /// [`DeskClient::remove_repo`].
    RemoveRepo,
    /// [`DeskClient::move_to`].
    Move,
    /// [`DeskClient::set_access`].
    SetAccess,
}

/// What the app asks a daemon to do with Desk sessions. Each change shows
/// up in the next workspace the daemon hands out.
pub trait DeskClient: Send + Sync + 'static {
    /// Starts a Desk session in a new scratch folder or a picked one.
    fn start(&self, new: NewDeskSession) -> Reply<SessionId>;

    /// Closes a session (DK-11): stops its agent and terminals, keeps its
    /// record, conversation and folder, and starts the 7 days of a scratch
    /// folder that wasn't kept.
    fn close(&self, session: &SessionId) -> Reply<()>;

    /// Reopens a closed session: its agent resumes its conversation in the
    /// same folder.
    fn reopen(&self, session: &SessionId) -> Reply<()>;

    /// Keeps a scratch folder, so it stays after its session closes. Does
    /// nothing for a picked folder, which always stays.
    fn keep(&self, session: &SessionId) -> Reply<()>;

    /// The closed sessions, least recently used first, with what deleting
    /// each would free and the work it would lose.
    fn clean_up_list(&self) -> Reply<Vec<CleanUpRow>>;

    /// Deletes the closed sessions in `delete` to the trash, but only when
    /// `shown` is still what [`DeskClient::clean_up_list`] gives (NV-27).
    /// A picked folder is left where it is.
    fn clean_up(&self, shown: Vec<CleanUpRow>, delete: Vec<SessionId>) -> Reply<CleanUp>;

    /// Copies a document into a channel's `shared/` folder (DK-13). The
    /// machine must be named when the channel has several. The Desk's copy
    /// stays.
    fn copy_document(
        &self,
        document: &Document,
        channel: &ChannelId,
        machine: Option<&MachineId>,
    ) -> Reply<()>;

    /// Clones a repository's default branch into the session's folder (DK-3).
    fn add_repo(&self, session: &SessionId, repo: &str) -> Reply<()>;

    /// Deletes a repository's clone from the session's folder (DK-3).
    fn remove_repo(&self, session: &SessionId, repo: &str) -> Reply<()>;

    /// Moves the session to another online machine (DK-3, MV-1): the folder
    /// and conversation are carried and the agent restarts there. A picked
    /// folder is copied and the original stays.
    fn move_to(&self, session: &SessionId, machine: &MachineId) -> Reply<()>;

    /// Changes the session's Agent access switches (DK-4).
    fn set_access(&self, session: &SessionId, access: Access) -> Reply<()>;
}
