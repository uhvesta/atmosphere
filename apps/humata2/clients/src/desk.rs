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

use crate::daemon::{DaemonError, Reply};
use crate::model::{
    AccessRule, Agent, ChannelId, Machine, MachineId, RuleSetting, Session, SessionId,
};

/// A machine's Humata folder until someone changes it (PL-1, FC-1).
pub const DEFAULT_ROOT: &str = "~/Humata";

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
    scratch_path_in(DEFAULT_ROOT, name)
}

/// Where the scratch folder called `name` sits under `root`.
#[must_use]
pub fn scratch_path_in(root: &str, name: &str) -> String {
    format!("{root}/{FOLDER}/{name}")
}

/// `machine`'s Humata folder among `machines`; the default root when the
/// machine is not listed.
#[must_use]
pub fn root_of<'a>(machines: &'a [Machine], machine: &MachineId) -> &'a str {
    machines
        .iter()
        .find(|found| &found.id == machine)
        .map_or(DEFAULT_ROOT, |found| found.root.as_str())
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
    /// The folder is a versions repository (DV-1). False for a picked folder
    /// and for one made before versions, until it is migrated.
    pub has_versions: bool,
    /// The folder's newest commit, empty when it has none. It changes when
    /// anyone commits, so a changed value means the versions are stale
    /// (DV-5).
    pub doc_head: String,
    /// How many versions the folder holds; 0 when it keeps none or none
    /// is saved yet (DV-5).
    pub version_count: u32,
    /// The folder holds a file outside its clones and `.git` (DV-4).
    pub holds_files: bool,
}

impl DeskSession {
    /// A session just started: open, nothing in its clones, the default
    /// access, and no versions yet.
    #[must_use]
    pub fn fresh(session: SessionId, folder: Folder, last_used: LastUsed) -> Self {
        Self {
            session,
            folder,
            repos: Vec::new(),
            access: Access::default(),
            last_used,
            closed: None,
            unpushed: None,
            has_versions: false,
            doc_head: String::new(),
            version_count: 0,
            holds_files: false,
        }
    }
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
    /// Every channel's own Desk session, open and closed, by session
    /// (CD-1). They are in no channel's Sessions tree and not in
    /// [`DeskData::sessions`], which is the plain Desk.
    pub channel_sessions: BTreeMap<SessionId, ChannelDeskSession>,
}

/// One change to the Desk, as a daemon tells it (DESKWATCH).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeskChange {
    /// A plain Desk session as it is now.
    Session {
        /// The session's Desk record.
        desk: DeskSession,
        /// The Markdown files in its folder, by name.
        documents: Vec<String>,
    },
    /// A channel's own Desk session as it is now, with its channel, target
    /// and files.
    ChannelSession(ChannelDeskSession),
    /// The session left the Desk: deleted, or moved into a channel.
    Gone(SessionId),
}

impl DeskData {
    /// Applies one change told by the daemon, in place. A session's files
    /// keep their place in the list; a session that is not there yet adds
    /// its files at the end.
    pub fn apply(&mut self, change: DeskChange) {
        match change {
            DeskChange::Session { desk, documents } => {
                let id = desk.session.clone();
                self.set_documents(&id, documents);
                self.sessions.insert(id, desk);
            }
            DeskChange::ChannelSession(found) => {
                self.channel_sessions
                    .insert(found.desk.session.clone(), found);
            }
            DeskChange::Gone(id) => {
                self.sessions.remove(&id);
                self.channel_sessions.remove(&id);
                self.documents.retain(|document| document.session != id);
                self.folder_bytes.remove(&id);
            }
        }
    }

    fn documents_of(&self, session: &SessionId) -> Vec<String> {
        self.documents
            .iter()
            .filter(|document| &document.session == session)
            .map(|document| document.path.clone())
            .collect()
    }

    fn set_documents(&mut self, session: &SessionId, paths: Vec<String>) {
        let at = self
            .documents
            .iter()
            .position(|document| &document.session == session)
            .unwrap_or(self.documents.len());
        self.documents.retain(|document| &document.session != session);
        let at = at.min(self.documents.len());
        for (offset, path) in paths.into_iter().enumerate() {
            self.documents.insert(
                at + offset,
                Document {
                    session: session.clone(),
                    path,
                },
            );
        }
    }

    /// What changed from `before` to this Desk, as the changes a daemon
    /// would have told: the sessions that left first, then those added or
    /// changed.
    #[doc(hidden)]
    pub fn changes_since(&self, before: &Self) -> Vec<DeskChange> {
        let mut changes: Vec<DeskChange> = before
            .sessions
            .keys()
            .filter(|id| !self.sessions.contains_key(*id))
            .chain(
                before
                    .channel_sessions
                    .keys()
                    .filter(|id| !self.channel_sessions.contains_key(*id)),
            )
            .map(|id| DeskChange::Gone(id.clone()))
            .collect();
        for (id, desk) in &self.sessions {
            let documents = self.documents_of(id);
            if before.sessions.get(id) != Some(desk) || before.documents_of(id) != documents {
                changes.push(DeskChange::Session {
                    desk: desk.clone(),
                    documents,
                });
            }
        }
        for (id, found) in &self.channel_sessions {
            if before.channel_sessions.get(id) != Some(found) {
                changes.push(DeskChange::ChannelSession(found.clone()));
            }
        }
        changes
    }
}

/// One of a channel Desk session's management switches (CD-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Switch {
    /// Read the target's files, or every session's in the channel.
    ReadFiles,
    /// Send notes and questions.
    SendNotes,
    /// Type into a terminal.
    TypeIntoTerminal,
    /// Start and fork sessions.
    StartAndFork,
    /// Stop and archive sessions.
    StopAndArchive,
    /// Change anyone's items on the channel board.
    ChangeBoardItems,
}

impl Switch {
    /// Every switch, in the order the dialog and the Info panel list them.
    pub const ALL: [Self; 6] = [
        Self::ReadFiles,
        Self::SendNotes,
        Self::TypeIntoTerminal,
        Self::StartAndFork,
        Self::StopAndArchive,
        Self::ChangeBoardItems,
    ];

    /// The switch in words.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ReadFiles => "Read its files",
            Self::SendNotes => "Send notes and questions",
            Self::TypeIntoTerminal => "Type into its terminal",
            Self::StartAndFork => "Start and fork sessions",
            Self::StopAndArchive => "Stop and archive sessions",
            Self::ChangeBoardItems => "Change anyone's items on the channel board",
        }
    }

    /// The channel's access rows (IN-17) a switch applies after. A row set
    /// to Never still refuses, so the switch shows as blocked (CD-3).
    #[must_use]
    pub const fn capabilities(self) -> &'static [&'static str] {
        match self {
            Self::ReadFiles => &["files_read_other_session", "files_read_other_machine"],
            Self::SendNotes => &["session_note"],
            Self::TypeIntoTerminal => &["session_type"],
            Self::StartAndFork => &["session_start_same_machine", "session_start_other_machine"],
            Self::StopAndArchive => &["session_stop_other"],
            Self::ChangeBoardItems => &["channel_board_edit_others"],
        }
    }
}

/// The switches a channel's rules block: those with a row set to Never,
/// whether the switch is on or off.
#[must_use]
pub fn blocked_by(rules: &[AccessRule]) -> Vec<Switch> {
    Switch::ALL
        .into_iter()
        .filter(|switch| {
            rules.iter().any(|rule| {
                rule.setting == Some(RuleSetting::Never)
                    && switch.capabilities().contains(&rule.capability.as_str())
            })
        })
        .collect()
}

/// A channel Desk session's six switches (CD-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switches {
    /// Read its files.
    pub read_files: bool,
    /// Send notes and questions.
    pub send_notes: bool,
    /// Type into its terminal.
    pub type_into_terminal: bool,
    /// Start and fork sessions.
    pub start_and_fork: bool,
    /// Stop and archive sessions.
    pub stop_and_archive: bool,
    /// Change anyone's items on the channel board.
    pub change_board_items: bool,
}

impl Default for Switches {
    /// Read files and start/fork sessions in the channel are on by default;
    /// notes and terminal typing require explicit consent (ATM-41, ATM-42).
    fn default() -> Self {
        Self {
            read_files: true,
            send_notes: false,
            type_into_terminal: false,
            start_and_fork: true,
            stop_and_archive: false,
            change_board_items: false,
        }
    }
}

impl Switches {
    /// Whether `switch` is on.
    #[must_use]
    pub const fn get(self, switch: Switch) -> bool {
        match switch {
            Switch::ReadFiles => self.read_files,
            Switch::SendNotes => self.send_notes,
            Switch::TypeIntoTerminal => self.type_into_terminal,
            Switch::StartAndFork => self.start_and_fork,
            Switch::StopAndArchive => self.stop_and_archive,
            Switch::ChangeBoardItems => self.change_board_items,
        }
    }

    /// These switches with `switch` set to `on`.
    #[must_use]
    pub const fn with(mut self, switch: Switch, on: bool) -> Self {
        match switch {
            Switch::ReadFiles => self.read_files = on,
            Switch::SendNotes => self.send_notes = on,
            Switch::TypeIntoTerminal => self.type_into_terminal = on,
            Switch::StartAndFork => self.start_and_fork = on,
            Switch::StopAndArchive => self.stop_and_archive = on,
            Switch::ChangeBoardItems => self.change_board_items = on,
        }
        self
    }
}

/// What a channel's own Desk session knows beyond a plain Desk session
/// (CD-1 to CD-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelDeskSession {
    /// Its folder, access, last use and whether it is closed.
    pub desk: DeskSession,
    /// The channel it looks after.
    pub channel: ChannelId,
    /// The session it looks after; `None` for the whole channel.
    pub target: Option<SessionId>,
    /// Its switches.
    pub switches: Switches,
    /// The switches the channel's rules set to Never.
    pub blocked: Vec<Switch>,
    /// The target went while the folder held Markdown files (CD-5).
    pub target_gone: bool,
    /// The Markdown files directly in its folder, by name.
    pub documents: Vec<String>,
    /// How many lines each of `documents` has, by name, where the daemon
    /// says (frame 94's "54 lines"). A file with no entry shows no count.
    pub document_lines: BTreeMap<String, u32>,
}

/// What New desk session on a channel's Desk asks for (CD-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewChannelDesk {
    /// The channel it looks after.
    pub channel: ChannelId,
    /// The session it looks after; `None` for the whole channel.
    pub target: Option<SessionId>,
    /// Its title.
    pub title: String,
    /// What it runs: an agent, or [`Agent::Shell`] for a plain terminal.
    pub agent: Agent,
    /// The machine it runs on.
    pub machine: MachineId,
    /// Its switches.
    pub switches: Switches,
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
    /// [`DeskClient::start_channel`].
    StartChannel,
    /// [`DeskClient::set_switches`].
    SetSwitches,
    /// [`DeskClient::set_target`].
    SetTarget,
    /// [`DeskClient::list_versions`].
    ListVersions,
    /// [`DeskClient::read_version`].
    ReadVersion,
    /// [`DeskClient::save_version`].
    SaveVersion,
    /// [`DeskClient::restore_version`].
    RestoreVersion,
    /// [`DeskClient::diff_version`].
    DiffVersion,
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

    /// A channel's open and closed Desk sessions (CD-2). A daemon that
    /// doesn't serve a channel's Desk answers [`DaemonError::NotServedYet`].
    fn channel_sessions(&self, channel: &ChannelId) -> Reply<Vec<ChannelDeskSession>> {
        let what = format!("The Desk of channel {channel}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }

    /// The open channel Desk sessions looking after `target` (CD-2), for its
    /// Info panel. A daemon that doesn't serve a channel's Desk answers
    /// [`DaemonError::NotServedYet`].
    fn target_sessions(&self, target: &SessionId) -> Reply<Vec<ChannelDeskSession>> {
        let what = format!("The Desk sessions looking after {target}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }

    /// Starts a channel Desk session on the channel's Desk tab (CD-6).
    /// Closing one is [`DeskClient::close`], which asks the daemon to send
    /// its folder to the trash at once (CD-4).
    fn start_channel(&self, new: NewChannelDesk) -> Reply<SessionId> {
        let what = format!("Starting the channel desk session {}", new.title);
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }

    /// Changes a channel Desk session's switches (CD-3).
    fn set_switches(&self, session: &SessionId, switches: Switches) -> Reply<()> {
        let what = format!("Changing the switches of {session}");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }

    /// Changes which session a channel Desk session looks after (CD-5): one
    /// of its channel's sessions, or `None` for the whole channel. It ends
    /// "Target gone" and does not restart a session the daemon stopped. A
    /// daemon that doesn't serve a channel's Desk answers
    /// [`DaemonError::NotServedYet`].
    fn set_target(&self, session: &SessionId, _target: Option<&SessionId>) -> Reply<()> {
        let what = format!("Changing what {session} looks after");
        Box::pin(async move { Err(DaemonError::NotServedYet { what }) })
    }

    /// The versions of a Markdown file in the session's Desk folder, newest
    /// first, and whether the folder holds changes no version has (DV-2,
    /// DV-3). `path` is relative to the folder.
    fn list_versions(
        &self,
        session: &SessionId,
        path: &str,
    ) -> Reply<crate::docs::DocVersions>;

    /// One version's text and the comments recorded with it (DV-6).
    fn read_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<crate::docs::VersionText>;

    /// Save version (DV-5): saves every change in the folder as a version.
    /// An empty message becomes "Saved by you". Refused when nothing
    /// changed.
    fn save_version(
        &self,
        session: &SessionId,
        message: &str,
    ) -> Reply<crate::docs::DocVersion>;

    /// Restore as new version (DV-8): saves unsaved changes first, then
    /// makes the version's text the file's newest. Refused when the file
    /// already has that text.
    fn restore_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<crate::docs::Restored>;

    /// A version of a Markdown file against the file as it is now, as the
    /// daemon diffs them (DV-7). Refused for a version the file doesn't have.
    fn diff_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<crate::docs::VersionDiff>;
}
