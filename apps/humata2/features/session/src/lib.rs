//! A session's page (SE-6–23, FK-1–13): its terminal tabs, the side panel,
//! and the composer, a box for typed messages that wait for the agent's turn to
//! end (SE-21–23). Review comments are an explicit send: they go as soon as
//! the session takes input, whatever its turn. Every session keeps its own [`SessionView`]: tab, panel,
//! draft and queue survive switching away and a restart (PER-25).
//!
//! Live terminal output arrives as [`SessionAction::Output`] from the
//! daemon's event stream and is appended to [`SessionState::live`], so a
//! line doesn't copy the workspace.

pub mod editor;
mod close;
mod continue_in;
mod conversation;
mod document;
mod history;
mod menu;
mod message;
mod tabs;
pub use menu::{MenuItem, RenameDraft};
pub use close::{CloseAction, CloseChoice, CloseConfirm, CloseKey, CloseKind, CloseNotice, CloseThen};
pub use continue_in::ContinueDraft;
pub use conversation::{Conversation, ConversationRead, ConversationTarget};
pub use document::{DocumentEntry, DocumentMode, DocumentView, is_markdown};
pub use history::{CompareDiff, DocCompare, DocHistory, HistoryAction, Position};
use editor::{CodeMode, CodeView};
pub use message::QueuedMessage;
pub use tabs::{GroupWidth, Tab, TabGroup, group_widths};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use humata2_clients::model::{
    Agent, Answer, Archive, ForkKind, Line, MachineId, MigrateThing, Request, RequestId, Session,
    SessionId, Skill, Speaker, Turn,
};
use humata2_clients::{ArchivePlan, Clients, DaemonError, ForkOptions, PastTurn, Workspace};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

/// How many live lines a session keeps; older ones are in the daemon's
/// scrollback.
pub const LIVE_LINES: usize = 2_000;

/// The side panel beside the terminal (SE-10).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Panel {
    /// The session's changes, to review.
    Changes,
    /// Its Inbox requests.
    Inbox,
    /// Its files.
    Files,
    /// Its board items.
    Board,
    /// Events and prepared views for the session.
    Journal,
    /// What it is and where it runs.
    Info,
}

impl Panel {
    /// Every panel, in tab order.
    pub const ALL: [Self; 6] = [
        Self::Changes,
        Self::Inbox,
        Self::Files,
        Self::Board,
        Self::Journal,
        Self::Info,
    ];

    /// Its tab's title.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Changes => "Changes",
            Self::Inbox => "Inbox",
            Self::Files => "Files",
            Self::Board => "Board",
            Self::Journal => "Journal",
            Self::Info => "Info",
        }
    }
}

/// One session's page as the person left it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionView {
    /// The terminal tab: 0 is the agent, then its shells.
    pub tab: usize,
    /// One or two tab groups, migrated from the terminal selection on open.
    #[serde(default)]
    pub groups: Vec<TabGroup>,
    /// Which group receives new tabs and keyboard focus.
    #[serde(default)]
    pub focused_group: usize,
    /// The side panel, if open.
    pub panel: Option<Panel>,
    /// Whether the panel takes the whole page (RV-2).
    pub widened: bool,
    /// What the composer holds.
    pub draft: String,
    /// The outbox, first to go first. Typed text waits for the turn to end;
    /// review comments wait only for the session to take input.
    pub queue: Vec<QueuedMessage>,
    /// The selected clone and relative file path in Files.
    #[serde(default)]
    pub file: Option<(String, String)>,
    /// The Files filter, kept with this session.
    #[serde(default)]
    pub file_filter: String,
    /// Collapsed Files directories, keyed by repository and path.
    #[serde(default)]
    pub folded_files: BTreeSet<(String, String)>,
    /// Per-file document settings; read with `document`.
    #[serde(default)]
    pub documents: Vec<DocumentEntry>,
    /// Per-file editor settings for changed code files; read with `code`.
    #[serde(default)]
    pub code: Vec<editor::CodeEntry>,
    /// Where the Conversation tab is aimed, while it is open (D186).
    #[serde(default)]
    pub conversation: Option<ConversationTarget>,
}

/// A choice in the Fork dialog that lists its options under the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForkPicker {
    /// Now, or before an earlier turn (FK-3).
    From,
    /// The agent the fork runs.
    Agent,
    /// The machine the fork runs on.
    Machine,
}

/// A fork being set up, before it is made (FK-1 to FK-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkDraft {
    /// How it shares the parent's work.
    pub kind: ForkKind,
    /// Its title.
    pub title: String,
    /// The turn it starts before; `None` forks from now.
    pub before_turn: Option<u32>,
    /// The agent it runs; `None` keeps the parent's.
    pub agent: Option<Agent>,
    /// The machine it runs on; `None` keeps the parent's.
    pub machine: Option<MachineId>,
    /// The branch its clones are made on.
    pub branch: String,
    /// The parent's turns, once the picker asked for them.
    pub turns: Option<Vec<PastTurn>>,
    /// The list open under a row.
    pub picker: Option<ForkPicker>,
}

/// A move being set up (MV-1 to MV-3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MoveDraft {
    /// The machine picked; `None` until one is.
    pub to: Option<MachineId>,
    /// The move is on its way to the daemon.
    pub sending: bool,
}

/// An archive that has to ask first, because work would be lost (MV-7 to MV-9).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveConfirm {
    /// The session.
    pub session: SessionId,
    /// Its title.
    pub title: String,
    /// What archiving does.
    pub plan: ArchivePlan,
    /// The archive is on its way to the daemon.
    pub sending: bool,
    /// The agent it ends, as the close dialog names it: "Claude Code",
    /// "Copilot", "Shell" (D784).
    pub agent: String,
    /// The machine it runs on, "This Mac" for this one (D784).
    pub machine: String,
}

/// Why the daemon refused a session action, in a form the page can act on
/// (D377).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionRefused {
    /// Something on a machine is at another release than the host's, so the
    /// action wasn't done until it is migrated.
    MigrationNeeded {
        /// What is behind.
        thing: MigrateThing,
        /// The machine it is on.
        machine: MachineId,
        /// The Migrate item that fixes it, when the daemon named one.
        request: Option<RequestId>,
    },
}

impl SessionRefused {
    /// The refusal in `error`, if it is one.
    fn of(error: &DaemonError) -> Option<Self> {
        match error {
            DaemonError::MigrationNeeded { thing, machine, request } => {
                Some(Self::MigrationNeeded {
                    thing: *thing,
                    machine: machine.clone(),
                    request: request.clone(),
                })
            }
            _ => None,
        }
    }

    /// Whether `migrations` show what this refusal waits for as migrated.
    fn fixed_by(&self, migrations: &[Request]) -> bool {
        let Self::MigrationNeeded { thing, machine, request } = self;
        migrations.iter().any(|item| {
            let approved = item.answer == Some(Answer::Approved);
            match request {
                Some(id) => &item.id == id && approved,
                None => {
                    approved
                        && item
                            .migrate()
                            .is_some_and(|card| card.thing == *thing && card.machine == *machine)
                }
            }
        })
    }
}

/// The session pages' state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionState {
    /// What the pages show.
    pub workspace: Arc<Workspace>,
    /// The session on screen.
    pub open: Option<SessionId>,
    /// Each session's page, kept while the app runs and saved.
    pub views: BTreeMap<SessionId, SessionView>,
    /// Output that arrived after the session's last snapshot, per session
    /// and terminal tab.
    pub live: HashMap<(SessionId, usize), Arc<Vec<Line>>>,
    /// The group whose plus menu is open.
    pub terminal_menu: Option<usize>,
    /// The path of the clone whose Info row menu is open (PL-10).
    pub clone_menu: Option<String>,
    /// Whether the open session's waiting messages are expanded.
    pub queue_expanded: bool,
    /// Queues awaiting explicit retry after a failed delivery.
    pub failed_queues: BTreeSet<SessionId>,
    /// Whether a new terminal is being started.
    pub opening_terminal: bool,
    /// The change the editor's cursor is on, per session, repository and
    /// path: an index into the changes the editor lists. Not saved.
    pub changes_at: BTreeMap<(SessionId, String, String), usize>,
    /// Loaded text files, keyed by session/repository/path.
    pub file_contents: BTreeMap<(SessionId, String, String), Result<String, DaemonError>>,
    /// Each session's Conversation tab entries, read from the host's mirror.
    pub conversations: BTreeMap<SessionId, Conversation>,
    /// The versions of each open Markdown file of a Desk folder that keeps
    /// them, by session and path (DV-3). Read through `history`.
    pub histories: BTreeMap<(SessionId, String), DocHistory>,
    /// The old version Compare with current was asked for (DV-7); the diff
    /// view takes it as its base.
    pub compare: Option<DocCompare>,
    /// Whether a message is on its way.
    pub sending: bool,
    /// A fork request is waiting for the client.
    pub forking: bool,
    /// The fork dialog, if open (FK-1).
    pub fork: Option<ForkDraft>,
    /// The Move dialog, if open (MV-1).
    pub moving: Option<MoveDraft>,
    /// The Continue in another channel dialog, if open (CV-8).
    pub continuing: Option<ContinueDraft>,
    /// The Archive dialog, when the session holds unpushed work (MV-8).
    pub archiving: Option<ArchiveConfirm>,
    /// The header's ⋯ menu is open (SE-7).
    pub menu: bool,
    /// The Rename dialog, if open (FC-17).
    pub renaming: Option<RenameDraft>,
    /// What went wrong last, for the page's error banner.
    pub error: Option<String>,
    /// What the daemon last refused the open session until a migration is
    /// done; the page draws a Migrate-first notice from it.
    pub refused: Option<SessionRefused>,
    /// The close dialog of a terminal tab, if open (D784).
    pub closing: Option<CloseConfirm>,
    /// "Always keep running without asking": closing a terminal tab hides
    /// it without the dialog (D784).
    pub always_keep_running: bool,
    /// The session the close dialog's End for good is archiving, so its
    /// toast's Undo restores it as Stopped (D784).
    pub ending_for_good: Option<SessionId>,
}

impl SessionState {
    /// Records why a session action failed: a refusal the page draws as its
    /// own notice, or an error for the banner.
    fn fail(&mut self, error: &DaemonError) {
        match SessionRefused::of(error) {
            Some(refused) => self.refused = Some(refused),
            None => self.error = Some(error.to_string()),
        }
    }

    /// The change a session's editor is on for a file, if it has a cursor.
    #[must_use]
    pub fn change(&self, session: &SessionId, repo: &str, path: &str) -> Option<usize> {
        self.changes_at
            .get(&(session.clone(), repo.to_owned(), path.to_owned()))
            .copied()
    }

    /// The page of the open session.
    #[must_use]
    pub fn view(&self) -> Option<&SessionView> {
        self.open.as_ref().and_then(|id| self.views.get(id))
    }

    fn view_mut(&mut self) -> Option<&mut SessionView> {
        let id = self.open.clone()?;
        Some(self.views.entry(id).or_default())
    }

    /// The skills the `/` menu offers for the open draft, if it is a
    /// command being typed.
    #[must_use]
    pub fn suggestions(&self) -> Vec<&Skill> {
        let Some(draft) = self.view().map(|view| view.draft.as_str()) else {
            return Vec::new();
        };
        let Some(typed) = draft.strip_prefix('/') else {
            return Vec::new();
        };
        if typed.contains(char::is_whitespace) {
            return Vec::new();
        }
        self.workspace
            .skills
            .iter()
            .filter(|skill| skill.name.starts_with(typed))
            .collect()
    }
}

/// What the session pages ask their parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open this session, such as a new fork.
    OpenSession(SessionId),
    /// Say this in a toast.
    Toast(String),
    /// An archive succeeded; the parent can offer Undo. Last comes the disk
    /// it frees when the undo window closes, when the machine said.
    Archived(SessionId, String, Option<String>),
    /// The open session was archived: leave its page.
    Left,
    /// Show this toast from the close dialog (D784); its button sends
    /// [`SessionAction::Close`] with [`CloseAction::Then`].
    CloseNotice(CloseNotice),
    /// End for good archived this session: like [`Delegate::Archived`], but
    /// the toast says so and its Undo is [`CloseThen::RestoreStopped`].
    EndedForGood(SessionId, String),
}

/// What happens on a session's page.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Saved pages came back at launch.
    Restore(BTreeMap<SessionId, SessionView>),
    /// Show this session.
    Open(SessionId),
    /// A line arrived for a session's tab.
    Output {
        /// The session.
        session: SessionId,
        /// The terminal tab.
        tab: usize,
        /// The line.
        line: Line,
    },
    /// A new snapshot of the session arrived, holding the agent tab's
    /// output so far.
    Snapshot(SessionId),
    /// A terminal tab was picked.
    SelectTab(usize),
    /// Pick a tab in a particular group.
    SelectGroupTab { group: usize, index: usize },
    /// A visible terminal gained keyboard focus, including by clicking its grid.
    FocusTerminal { session: SessionId, tab: usize },
    /// Open or close a group's plus menu.
    TerminalMenu(Option<usize>),
    /// Start/attach a program in the focused group or in the second group.
    OpenTerminal { agent: Agent, split: bool },
    /// The new terminal is ready, even if the person changed sessions meanwhile.
    TerminalOpened {
        session: SessionId,
        group: usize,
        split: bool,
        result: Result<(usize, Session), DaemonError>,
    },
    /// Open or close the menu of the Info clone row at this absolute path.
    CloneMenu(Option<String>),
    /// Copy an absolute path (PL-10).
    CopyClonePath(String),
    /// Show an absolute path on This Mac in Finder (PL-10).
    RevealClone(String),
    /// Finder could not show it; the message says why.
    RevealFailed(String),
    /// Start a shell tab of `session`, in `folder`, an absolute path on its
    /// machine (PL-10, Open terminal here).
    OpenTerminalIn { session: SessionId, folder: String },
    /// Open the selected Files path as a tab.
    OpenFile,
    /// Text came back from the session's host.
    FileLoaded {
        session: SessionId,
        repo: String,
        path: String,
        result: Result<String, DaemonError>,
    },
    /// A search hit was picked: show this session's Conversation tab in its
    /// focused group, aimed at the hit, and read around it (D186).
    OpenConversation { session: SessionId, turn: u32, seq: u64 },
    /// The mirror answered a Conversation read. Ignored unless `target` is
    /// still the session's target.
    ConversationLoaded {
        session: SessionId,
        target: ConversationTarget,
        read: ConversationRead,
        first: u32,
        last: u32,
        result: Result<Vec<humata2_clients::history::Turn>, DaemonError>,
    },
    /// Show earlier turns in the open session's Conversation tab.
    ShowEarlierTurns,
    /// Show later turns in the open session's Conversation tab.
    ShowLaterTurns,
    /// Repeat the open session's failed Conversation read.
    RetryConversation,
    /// Open an opted-in prepared view in this session, by its id (the
    /// `ViewId`'s string).
    OpenView(String),
    /// Close a tab; its process keeps running.
    CloseTab { group: usize, index: usize },
    /// The close dialog of a terminal tab (D784).
    Close(CloseAction),
    /// Move a tab into the other group: with one group it makes the second,
    /// beside the first; from the second it joins the first. The moved tab
    /// is selected and its group focused. A lone tab in the only group stays.
    MoveTab { group: usize, index: usize },
    /// Join a group's tabs into the other group.
    CloseGroup(usize),
    /// A panel tab was picked, or the open one closed with `None`.
    SetPanel(Option<Panel>),
    /// The panel was widened or narrowed again.
    ToggleWiden,
    /// Filter this session's repository paths.
    FilterFiles(String),
    /// Show a document in this mode; remembered per file in this session.
    DocumentMode {
        /// The repository containing the file.
        repo: String,
        /// Its path relative to the clone.
        path: String,
        /// How to show it.
        mode: DocumentMode,
    },
    /// Make a document's page follow its source, or stop.
    LinkScrolling {
        /// The repository containing the file.
        repo: String,
        /// Its path relative to the clone.
        path: String,
        /// Whether the page follows the source.
        linked: bool,
    },
    /// Show a changed file in this editor mode; remembered per file in this session.
    CodeMode {
        /// The repository containing the file.
        repo: String,
        /// Its path relative to the clone.
        path: String,
        /// How to show it.
        mode: CodeMode,
    },
    /// Show or hide the inline changes of a file in Full file mode (RV-22).
    ShowChanges {
        /// The repository containing the file.
        repo: String,
        /// Its path relative to the clone.
        path: String,
        /// Whether the changes are marked.
        shown: bool,
    },
    /// Open the fold of unchanged lines that starts at this new line (RV-18).
    Unfold {
        /// The repository containing the file.
        repo: String,
        /// Its path relative to the clone.
        path: String,
        /// The new line number the fold starts at.
        start: u32,
    },
    /// Step the editor's cursor to the next or previous change, wrapping
    /// around (RV-19). `count` is how many changes the editor lists; zero
    /// removes the cursor.
    StepChange {
        /// The repository containing the file.
        repo: String,
        /// Its path relative to the clone.
        path: String,
        /// Whether to go forward.
        forward: bool,
        /// How many changes there are.
        count: usize,
    },
    /// Select a file in this session's clones.
    SelectFile {
        /// The repository containing it.
        repo: String,
        /// Its path relative to the clone.
        path: String,
    },
    /// Expand or collapse a repository/directory in Files.
    ToggleFolder {
        /// The repository containing it.
        repo: String,
        /// Its path, or empty for the clone's root.
        path: String,
    },
    /// The composer's text changed.
    Draft(String),
    /// A `/` suggestion was picked.
    PickSkill(String),
    /// Enter in the composer: send now if the turn ended, else queue.
    Submit,
    /// Send the first queued message even though the turn may not have
    /// ended (SE-23).
    SendNow,
    /// Send a particular waiting message, even during a turn.
    SendQueued(usize),
    /// Expand or collapse the waiting-message list.
    ShowQueue(bool),
    /// Take a queued message back into the composer to edit it.
    EditQueued(usize),
    /// Drop a queued message.
    RemoveQueued(usize),
    /// The daemon took a message, or didn't.
    Sent {
        session: SessionId,
        index: usize,
        message: QueuedMessage,
        result: Result<(), DaemonError>,
    },
    /// Start an exited session again on its conversation (SE-14).
    Resume,
    /// Open the fork dialog.
    StartFork(ForkKind),
    /// Choose how the fork shares code and conversation.
    ForkKind(ForkKind),
    /// The fork dialog's title changed.
    ForkTitle(String),
    /// Make the fork.
    ConfirmFork,
    /// Close the fork dialog.
    CancelFork,
    /// Fork before this turn, or from now (FK-3).
    ForkBeforeTurn(Option<u32>),
    /// Run the fork's agent here.
    ForkAgent(Agent),
    /// Run the fork on this machine.
    ForkMachine(MachineId),
    /// Name the fork's branch.
    ForkBranch(String),
    /// Open or close a list under a row of the dialog.
    ForkPicker(Option<ForkPicker>),
    /// The parent's turns arrived.
    ForkTurns(Result<Vec<PastTurn>, DaemonError>),
    /// Open the Move dialog for the open session.
    StartMove,
    /// Pick the machine to move to.
    MovePick(MachineId),
    /// Move the session.
    ConfirmMove,
    /// Close the Move dialog.
    CancelMove,
    /// The daemon moved the session, or didn't.
    Moved(Result<(), DaemonError>),
    /// Open the Continue in another channel dialog for the open session
    /// (CV-8).
    StartContinue,
    /// Pick the channel to continue in.
    ContinuePick(humata2_clients::model::ChannelId),
    /// Open or close the "Continue from" list; opening it asks for the
    /// session's turns.
    ContinueTurnPicker(bool),
    /// The session's turns arrived.
    ContinueTurns(Result<Vec<PastTurn>, DaemonError>),
    /// Continue from before this turn, or from now with `None` (CV-2).
    ContinueBeforeTurn(Option<u32>),
    /// Start the new session.
    ConfirmContinue,
    /// Close the dialog.
    CancelContinue,
    /// The daemon started the new session, or didn't.
    Continued(Result<SessionId, DaemonError>),
    /// The daemon said what archiving would lose.
    ArchivePlanned {
        session: SessionId,
        title: String,
        result: Result<ArchivePlan, DaemonError>,
    },
    /// Archive after the dialog: with `true`, commit, push and open pull
    /// requests first; with `false`, delete the unpushed work.
    ConfirmArchive(bool),
    /// Close the Archive dialog.
    CancelArchive,
    /// The daemon made the fork, or didn't.
    Forked(Result<SessionId, DaemonError>),
    /// The archive request finished.
    Archived {
        session: SessionId,
        title: String,
        result: Result<Archive, DaemonError>,
    },
    /// Archive the open session.
    Archive,
    /// Open or close the header's ⋯ menu (SE-7).
    Menu(bool),
    /// An item of the ⋯ menu was picked; the menu closes.
    MenuPick(MenuItem),
    /// Open the Rename dialog on the open session's title (FC-17).
    StartRename,
    /// The title typed in the Rename dialog.
    RenameTitle(String),
    /// Whether the branch is renamed from the new title too.
    RenameBranch(bool),
    /// Rename the open session as the dialog says.
    ConfirmRename,
    /// Close the Rename dialog without renaming.
    CancelRename,
    /// The rename finished; on failure the dialog stays with the error.
    Renamed(Result<(), DaemonError>),
    /// Stop the open session's agent; the session stays.
    StopAgent,
    /// Pin or unpin the open session.
    TogglePin,
    /// Mark or unmark the open session very important.
    ToggleVip,
    /// A request finished; on failure its message is shown.
    Done(Result<(), DaemonError>),
    /// A line typed into a terminal, or a writable attach to one, was
    /// accepted or refused (D377). Only a "Migrate first" refusal is kept
    /// here; the terminal view says any other failure itself.
    Typed(Result<(), DaemonError>),
    /// The Inbox's Migrate items changed: a refusal they show migrated is
    /// over.
    Migrations(Vec<Request>),
    /// Hide the error banner.
    DismissError,
    /// The History strip and the version bar of a Markdown tab (DV-3 to DV-8).
    History(HistoryAction),
    /// For the parent.
    Delegate(Delegate),
}

/// The session pages' reducer.
#[derive(Clone)]
pub struct Sessions {
    clients: Clients,
    /// Messages queued so far, so keys made at one instant differ.
    queued: Arc<std::sync::atomic::AtomicU64>,
}

impl Sessions {
    /// Session pages that talk to `clients`.
    #[must_use]
    pub fn new(clients: Clients) -> Self {
        Self {
            clients,
            queued: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    fn send(
        &self,
        session: SessionId,
        index: usize,
        message: QueuedMessage,
    ) -> Effect<SessionAction> {
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            let text = message.text();
            let result = match message.key() {
                Some(key) => daemon.send_keyed(&session, &text, key).await,
                None => daemon.send(&session, &text).await,
            };
            SessionAction::Sent {
                session,
                index,
                message,
                result,
            }
        })
    }

    fn read_file(&self, session: SessionId, repo: String, path: String) -> Effect<SessionAction> {
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            let result = daemon.read_file(&session, &repo, &path).await;
            SessionAction::FileLoaded {
                session,
                repo,
                path,
                result,
            }
        })
    }

    fn read_conversation(
        &self,
        session: SessionId,
        target: ConversationTarget,
        read: ConversationRead,
        first: u32,
        last: u32,
    ) -> Effect<SessionAction> {
        let history = self.clients.history.clone();
        Effect::future(async move {
            let result = history.read(&session, first, last).await;
            SessionAction::ConversationLoaded {
                session,
                target,
                read,
                first,
                last,
                result,
            }
        })
    }

    /// Starts the Around read of a new target: its tab's entries come from
    /// the mirror, and a reply for any older target is ignored.
    fn start_conversation(
        &self,
        state: &mut SessionState,
        session: SessionId,
        target: ConversationTarget,
    ) -> Effect<SessionAction> {
        let mut conversation = Conversation::new(target);
        conversation.loading = Some(ConversationRead::Around);
        let (first, last) = (conversation.first, conversation.last);
        state.conversations.insert(session.clone(), conversation);
        self.read_conversation(session, target, ConversationRead::Around, first, last)
    }

    /// Reads the open session's next Earlier or Later turns, unless a read
    /// is already on its way or there is nothing that way.
    fn step_conversation(&self, state: &mut SessionState, read: ConversationRead) -> Effect<SessionAction> {
        let Some(session) = state.open.clone() else {
            return Effect::none();
        };
        let Some(conversation) = state.conversations.get_mut(&session) else {
            return Effect::none();
        };
        if conversation.loading.is_some() || (read == ConversationRead::Later && !conversation.can_show_later()) {
            return Effect::none();
        }
        let Some((first, last)) = conversation.range(read) else {
            return Effect::none();
        };
        conversation.loading = Some(read);
        let target = conversation.target;
        self.read_conversation(session, target, read, first, last)
    }

    /// Archives `session`, then reports how it went.
    fn archive(&self, session: SessionId, title: String, push_first: bool) -> Effect<SessionAction> {
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            let result = daemon.archive(&session, push_first).await;
            SessionAction::Archived {
                session,
                title,
                result,
            }
        })
    }

    fn done(
        &self,
        work: impl FnOnce(&dyn humata2_clients::Daemon) -> humata2_clients::Reply<()>,
    ) -> Effect<SessionAction> {
        let future = work(self.clients.daemon.as_ref());
        Effect::future(async move { SessionAction::Done(future.await) })
    }

    /// Sends the first queued message of `session`. Typed text is the wait
    /// mode and goes only when a hook-backed agent's turn ended. Never sets
    /// an error, because it runs on every refresh.
    fn flush(&self, state: &mut SessionState, session: &SessionId) -> Effect<SessionAction> {
        if state.sending || state.failed_queues.contains(session) {
            return Effect::none();
        }
        let Some(workspace_session) = state.workspace.session(session) else {
            return Effect::none();
        };
        if !workspace_session.status.accepts_input() {
            return Effect::none();
        }
        let ended = workspace_session.turn == Turn::Ended
            && matches!(
                workspace_session.agent,
                Agent::Claude | Agent::Copilot | Agent::OpenCode
            );
        if !ended || state.views.get(session).is_none_or(|view| view.queue.is_empty()) {
            return Effect::none();
        }
        self.send_queued(state, session, 0)
    }

    fn send_queued(
        &self,
        state: &mut SessionState,
        session: &SessionId,
        index: usize,
    ) -> Effect<SessionAction> {
        if state.sending {
            return Effect::none();
        }
        if !state
            .views
            .get(session)
            .is_some_and(|view| index < view.queue.len())
        {
            return Effect::none();
        }
        if !state
            .workspace
            .session(session)
            .is_some_and(|s| s.status.accepts_input())
        {
            state.error = Some("This session cannot take a message. Reconnect its machine or resume the agent, then send again.".into());
            return Effect::none();
        }
        // Keep the saved message until its acknowledgement. Closing the app
        // while the client is pending must not discard it.
        let Some(text) = state
            .views
            .get(session)
            .and_then(|view| view.queue.get(index))
            .cloned()
        else {
            return Effect::none();
        };
        state.failed_queues.remove(session);
        state.error = None;
        state.sending = true;
        self.send(session.clone(), index, text)
    }
}

impl Reducer for Sessions {
    type State = SessionState;
    type Action = SessionAction;

    #[allow(clippy::too_many_lines)]
    fn reduce(&self, state: &mut SessionState, action: SessionAction) -> Effect<SessionAction> {
        match action {
            action @ (SessionAction::Menu(_)
            | SessionAction::MenuPick(_)
            | SessionAction::StartRename
            | SessionAction::RenameTitle(_)
            | SessionAction::RenameBranch(_)
            | SessionAction::ConfirmRename
            | SessionAction::CancelRename
            | SessionAction::Renamed(_)
            | SessionAction::StopAgent) => return self.menu(state, action),
            action @ (SessionAction::StartContinue
            | SessionAction::ContinuePick(_)
            | SessionAction::ContinueTurnPicker(_)
            | SessionAction::ContinueTurns(_)
            | SessionAction::ContinueBeforeTurn(_)
            | SessionAction::ConfirmContinue
            | SessionAction::CancelContinue
            | SessionAction::Continued(_)) => return self.continue_in(state, action),
            SessionAction::History(action) => return self.history(state, action),
            SessionAction::Close(action) => return self.close(state, action),
            SessionAction::Show(workspace) => {
                state.workspace = workspace;
                let waiting: Vec<_> = state
                    .views
                    .iter()
                    .filter(|(_, view)| !view.queue.is_empty())
                    .map(|(id, _)| id.clone())
                    .collect();
                let mut effects: Vec<_> = waiting.iter().map(|id| self.flush(state, id)).collect();
                effects.extend(self.sync_histories(state));
                return Effect::merge(effects);
            }
            SessionAction::Restore(views) => state.views = views,
            SessionAction::Open(id) => {
                let shells = state.workspace.session(&id).map_or(0, |s| s.shells.len());
                state
                    .views
                    .entry(id.clone())
                    .or_default()
                    .ensure_groups(shells);
                state.terminal_menu = None;
                state.queue_expanded = false;
                state.open = Some(id.clone());
                state.fork = None;
                state.moving = None;
                state.continuing = None;
                state.archiving = None;
                state.menu = false;
                state.renaming = None;
                state.error = None;
                let unread = state
                    .workspace
                    .session(&id)
                    .is_some_and(|session| session.unread > 0 || session.recovery.is_some());
                let mut effects = Vec::new();
                if unread {
                    effects.push(self.done(|daemon| daemon.mark_read(&id)));
                }
                for tab in state
                    .views
                    .get(&id)
                    .into_iter()
                    .flat_map(|v| v.groups.iter())
                    .flat_map(|g| g.tabs.iter())
                {
                    if let Tab::File { repo, path } = tab {
                        effects.push(self.read_file(id.clone(), repo.clone(), path.clone()));
                    }
                }
                // A saved Conversation tab keeps its target, not its entries: read them again.
                let held = state.views.get(&id).and_then(|view| {
                    let shown = view.groups.iter().any(|g| g.tabs.contains(&Tab::Conversation));
                    view.conversation.filter(|_| shown)
                });
                if let Some(target) = held
                    && !state.conversations.get(&id).is_some_and(|c| c.target == target)
                {
                    effects.push(self.start_conversation(state, id.clone(), target));
                }
                return Effect::merge(effects);
            }
            SessionAction::Output { session, tab, line } => {
                let lines = state.live.entry((session, tab)).or_default();
                let lines = Arc::make_mut(lines);
                lines.push(line);
                if lines.len() > LIVE_LINES {
                    let extra = lines.len() - LIVE_LINES;
                    lines.drain(..extra);
                }
            }
            SessionAction::Snapshot(session) => {
                state.live.remove(&(session, 0));
            }
            SessionAction::SelectTab(tab) => {
                let shells = open_session(state).map_or(0, |s| s.shells.len());
                if let Some(view) = state.view_mut() {
                    view.ensure_groups(shells);
                    let tab = Tab::Terminal(tab.min(shells));
                    let group = view
                        .groups
                        .iter()
                        .position(|g| g.tabs.contains(&tab))
                        .unwrap_or(view.focused_group);
                    view.show_tab(tab, group, false);
                }
            }
            SessionAction::SelectGroupTab { group, index } => {
                let mut file = None;
                if let Some(view) = state.view_mut()
                    && let Some(g) = view.groups.get_mut(group)
                    && index < g.tabs.len()
                {
                    g.selected = index;
                    view.focused_group = group;
                    match &g.tabs[index] {
                        Tab::Terminal(tab) => view.tab = *tab,
                        Tab::File { repo, path } => file = Some((repo.clone(), path.clone())),
                        Tab::View(_) | Tab::Conversation => {}
                    }
                }
                state.terminal_menu = None;
                if let (Some(id), Some((repo, path))) = (state.open.clone(), file) {
                    return self.read_file(id, repo, path);
                }
            }
            SessionAction::FocusTerminal { session, tab } => {
                if state.open.as_ref() == Some(&session)
                    && let Some(view) = state.views.get_mut(&session)
                {
                    if let Some(group) = view.groups.iter().position(|group| {
                        group.tabs.get(group.selected) == Some(&Tab::Terminal(tab))
                    }) {
                        view.focused_group = group;
                        view.tab = tab;
                    }
                }
            }
            SessionAction::TerminalMenu(group) => state.terminal_menu = group,
            SessionAction::OpenTerminal { agent, split } => {
                if state.opening_terminal {
                    return Effect::none();
                }
                let Some(session) = state.open.clone() else {
                    return Effect::none();
                };
                let group = state
                    .terminal_menu
                    .or_else(|| state.view().map(|v| v.focused_group))
                    .unwrap_or(0);
                state.terminal_menu = None;
                state.opening_terminal = true;
                let terminals = self.clients.terminals.clone();
                return Effect::future(async move {
                    let result = terminals.open(&session, agent).await;
                    SessionAction::TerminalOpened {
                        session,
                        group,
                        split,
                        result,
                    }
                });
            }
            SessionAction::TerminalOpened {
                session,
                group,
                split,
                result,
            } => {
                state.opening_terminal = false;
                match result {
                    Ok((tab, snapshot)) => {
                        let shells = snapshot.shells.len();
                        if let Some(existing) = Arc::make_mut(&mut state.workspace)
                            .sessions
                            .iter_mut()
                            .find(|s| s.id == session)
                        {
                            *existing = snapshot;
                        }
                        let view = state.views.entry(session).or_default();
                        view.ensure_groups(shells);
                        view.show_tab(Tab::Terminal(tab), group, split);
                        state.refused = None;
                    }
                    Err(error) => state.fail(&error),
                }
            }
            SessionAction::CloneMenu(path) => state.clone_menu = path,
            SessionAction::CopyClonePath(path) => {
                state.clone_menu = None;
                self.clients.clipboard.copy(&path);
                return Effect::send(SessionAction::Delegate(Delegate::Toast("Copied the path".to_owned())));
            }
            SessionAction::RevealClone(path) => {
                state.clone_menu = None;
                let reveal = self.clients.reveal.clone();
                return Effect::run(move |send| async move {
                    if let Err(error) = reveal.reveal(&path) {
                        send.send(SessionAction::RevealFailed(error.to_string()));
                    }
                });
            }
            SessionAction::RevealFailed(message) => state.error = Some(message),
            SessionAction::OpenTerminalIn { session, folder } => {
                state.clone_menu = None;
                let group = state.views.get(&session).map_or(0, |view| view.focused_group);
                let terminals = self.clients.terminals.clone();
                return Effect::future(async move {
                    let result = terminals.open_in(&session, Agent::Shell, &folder).await;
                    SessionAction::TerminalOpened {
                        session,
                        group,
                        split: false,
                        result,
                    }
                });
            }
            SessionAction::OpenFile => {
                let Some(session) = state.open.clone() else {
                    return Effect::none();
                };
                let shells = open_session(state).map_or(0, |s| s.shells.len());
                let Some(view) = state.view_mut() else {
                    return Effect::none();
                };
                let Some((repo, path)) = view.file.clone() else {
                    return Effect::none();
                };
                view.ensure_groups(shells);
                view.show_tab(
                    Tab::File {
                        repo: repo.clone(),
                        path: path.clone(),
                    },
                    view.focused_group,
                    false,
                );
                return self.read_file(session, repo, path);
            }
            SessionAction::FileLoaded {
                session,
                repo,
                path,
                result,
            } => {
                let key = (session, repo, path);
                let changed = state.file_contents.get(&key) != Some(&result);
                state.file_contents.insert(key.clone(), result);
                let (session, repo, path) = key;
                return self.file_loaded_history(state, &session, &repo, &path, changed);
            }
            SessionAction::OpenConversation { session, turn, seq } => {
                let target = ConversationTarget { turn, seq };
                let shells = state.workspace.session(&session).map_or(0, |s| s.shells.len());
                let view = state.views.entry(session.clone()).or_default();
                view.ensure_groups(shells);
                view.conversation = Some(target);
                view.show_tab(Tab::Conversation, view.focused_group, false);
                return self.start_conversation(state, session, target);
            }
            SessionAction::ConversationLoaded {
                session,
                target,
                read,
                first,
                last,
                result,
            } => {
                let Some(conversation) = state.conversations.get_mut(&session) else {
                    return Effect::none();
                };
                if conversation.target != target {
                    return Effect::none();
                }
                match result {
                    Ok(entries) => conversation.merge(read, first, last, entries),
                    Err(error) => {
                        conversation.loading = None;
                        conversation.error = Some(error.to_string());
                        conversation.failed = Some((read, first, last));
                    }
                }
            }
            SessionAction::ShowEarlierTurns => return self.step_conversation(state, ConversationRead::Earlier),
            SessionAction::ShowLaterTurns => return self.step_conversation(state, ConversationRead::Later),
            SessionAction::RetryConversation => {
                let Some(session) = state.open.clone() else {
                    return Effect::none();
                };
                let Some(conversation) = state.conversations.get_mut(&session) else {
                    return Effect::none();
                };
                let Some((read, first, last)) = conversation.failed else {
                    return Effect::none();
                };
                if conversation.loading.is_some() {
                    return Effect::none();
                }
                conversation.error = None;
                conversation.loading = Some(read);
                let target = conversation.target;
                return self.read_conversation(session, target, read, first, last);
            }
            SessionAction::OpenView(id) => {
                let shells = open_session(state).map_or(0, |s| s.shells.len());
                if let Some(view) = state.view_mut() {
                    view.ensure_groups(shells);
                    view.show_tab(Tab::View(id), view.focused_group, false);
                }
            }
            SessionAction::CloseTab { group, index } => {
                let split = state.view().is_some_and(|v| v.groups.len() == 2);
                let closing_conversation = state
                    .view()
                    .and_then(|v| v.groups.get(group))
                    .and_then(|g| g.tabs.get(index))
                    == Some(&Tab::Conversation);
                if let Some(view) = state.view_mut()
                    && let Some(g) = view.groups.get_mut(group)
                    && index < g.tabs.len()
                {
                    if g.tabs.len() == 1 && split {
                        view.groups.remove(group);
                        view.focused_group = 0;
                        if let Some(Tab::Terminal(tab)) = view.selected(0) {
                            view.tab = *tab;
                        }
                    } else if g.tabs.len() == 1 {
                        g.tabs[0] = Tab::Terminal(0);
                        g.selected = 0;
                        view.tab = 0;
                    } else {
                        g.tabs.remove(index);
                        if index < g.selected {
                            g.selected -= 1;
                        }
                        g.selected = g.selected.min(g.tabs.len() - 1);
                        if let Some(Tab::Terminal(tab)) = view.selected(view.focused_group) {
                            view.tab = *tab;
                        }
                    }
                }
                if closing_conversation {
                    if let Some(view) = state.view_mut() {
                        view.conversation = None;
                    }
                    if let Some(id) = state.open.clone() {
                        state.conversations.remove(&id);
                    }
                }
            }
            SessionAction::MoveTab { group, index } => {
                let shells = open_session(state).map_or(0, |s| s.shells.len());
                if let Some(view) = state.view_mut() {
                    view.ensure_groups(shells);
                    let Some(tab) = view.groups.get(group).and_then(|g| g.tabs.get(index)).cloned() else {
                        return Effect::none();
                    };
                    let only = view.groups.len() == 1 && view.groups[0].tabs.len() == 1;
                    if !only {
                        // `show_tab`'s split always targets the second group,
                        // so the way back names the first.
                        if group == 0 {
                            view.show_tab(tab, 0, true);
                        } else {
                            view.show_tab(tab, 0, false);
                        }
                    }
                }
                state.terminal_menu = None;
            }
            SessionAction::CloseGroup(group) => {
                if let Some(view) = state.view_mut() {
                    view.close_group(group);
                }
            }
            SessionAction::SetPanel(panel) => {
                if let Some(view) = state.view_mut() {
                    view.panel = panel;
                    if panel.is_none() {
                        view.widened = false;
                    }
                }
            }
            SessionAction::ToggleWiden => {
                if let Some(view) = state.view_mut()
                    && view.panel.is_some()
                {
                    view.widened = !view.widened;
                }
            }
            SessionAction::FilterFiles(filter) => {
                if let Some(view) = state.view_mut() {
                    view.file_filter = filter;
                }
            }
            SessionAction::DocumentMode { repo, path, mode } => {
                if let Some(view) = state.view_mut() {
                    let current = view.document(&repo, &path);
                    view.set_document(&repo, &path, DocumentView { mode, ..current });
                }
            }
            SessionAction::LinkScrolling { repo, path, linked } => {
                if let Some(view) = state.view_mut() {
                    let current = view.document(&repo, &path);
                    view.set_document(&repo, &path, DocumentView { linked, ..current });
                }
            }
            SessionAction::CodeMode { repo, path, mode } => {
                if let Some(view) = state.view_mut() {
                    let current = view.code(&repo, &path);
                    view.set_code(&repo, &path, CodeView { mode, ..current });
                }
            }
            SessionAction::ShowChanges { repo, path, shown } => {
                if let Some(view) = state.view_mut() {
                    let current = view.code(&repo, &path);
                    view.set_code(
                        &repo,
                        &path,
                        CodeView {
                            changes: shown,
                            ..current
                        },
                    );
                }
            }
            SessionAction::Unfold { repo, path, start } => {
                if let Some(view) = state.view_mut() {
                    let mut current = view.code(&repo, &path);
                    current.unfolded.insert(start);
                    view.set_code(&repo, &path, current);
                }
            }
            SessionAction::StepChange {
                repo,
                path,
                forward,
                count,
            } => {
                if let Some(session) = state.open.clone() {
                    let key = (session, repo, path);
                    if count == 0 {
                        state.changes_at.remove(&key);
                    } else {
                        let next = match state.changes_at.get(&key) {
                            None if forward => 0,
                            None => count - 1,
                            Some(&at) if forward => (at + 1) % count,
                            Some(&at) => (at + count - 1) % count,
                        };
                        state.changes_at.insert(key, next);
                    }
                }
            }
            SessionAction::SelectFile { repo, path } => {
                // The Files tree, or the diff when the tree isn't listed yet.
                let exists = state.open.as_ref().is_some_and(|id| {
                    let listed = state.workspace.session_files.get(id).is_some_and(|files| {
                        files
                            .iter()
                            .any(|file| !file.folder && file.repo == repo && file.path == path)
                    });
                    listed
                        || state.workspace.diffs.iter().any(|diff| {
                            &diff.session == id
                                && diff.repo == repo
                                && diff.files.iter().any(|file| file.path == path)
                        })
                });
                if exists && let Some(view) = state.view_mut() {
                    view.file = Some((repo, path));
                }
            }
            SessionAction::ToggleFolder { repo, path } => {
                if let Some(view) = state.view_mut() {
                    let key = (repo, path);
                    if !view.folded_files.remove(&key) {
                        view.folded_files.insert(key);
                    }
                }
            }
            SessionAction::Draft(text) => {
                if let Some(view) = state.view_mut() {
                    view.draft = text;
                }
            }
            SessionAction::PickSkill(name) => {
                if let Some(view) = state.view_mut() {
                    view.draft = format!("/{name} ");
                }
            }
            SessionAction::Submit => {
                let Some(id) = state.open.clone() else {
                    return Effect::none();
                };
                let Some(session) = state.workspace.session(&id) else {
                    return Effect::none();
                };
                if !session.status.accepts_input() {
                    state.error = Some(format!(
                        "{} isn't running, so it can't take a message. Resume it first, then send again.",
                        session.title
                    ));
                    return Effect::none();
                }
                let Some(view) = state.view_mut() else {
                    return Effect::none();
                };
                let text = view.draft.trim().to_owned();
                if text.is_empty() {
                    return Effect::none();
                }
                view.draft.clear();
                let position = view.queue.len();
                let serial = self
                    .queued
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let now = self.clients.clock.now();
                view.queue
                    .push(QueuedMessage::new(&id, position, text, now, serial));
                return self.flush(state, &id);
            }
            SessionAction::SendNow | SessionAction::SendQueued(_) => {
                let Some(id) = state.open.clone() else {
                    return Effect::none();
                };
                let index = match action {
                    SessionAction::SendQueued(index) => index,
                    _ => 0,
                };
                return self.send_queued(state, &id, index);
            }
            SessionAction::ShowQueue(show) => state.queue_expanded = show,
            SessionAction::EditQueued(index) => {
                if state.sending {
                    return Effect::none();
                }
                if let Some(view) = state.view_mut()
                    && index < view.queue.len()
                {
                    let text = view.queue.remove(index).text();
                    let emptied = view.queue.is_empty();
                    if view.draft.trim().is_empty() {
                        view.draft = text;
                    } else {
                        view.draft = format!("{text}\n{}", view.draft);
                    }
                    if emptied && let Some(id) = state.open.clone() {
                        state.failed_queues.remove(&id);
                    }
                }
            }
            SessionAction::RemoveQueued(index) => {
                if state.sending {
                    return Effect::none();
                }
                if let Some(view) = state.view_mut()
                    && index < view.queue.len()
                {
                    view.queue.remove(index);
                    // Nothing failed is left: a new send goes at once.
                    if view.queue.is_empty()
                        && let Some(id) = state.open.clone()
                    {
                        state.failed_queues.remove(&id);
                    }
                }
            }
            SessionAction::Sent {
                session,
                index,
                message,
                result,
            } => {
                state.sending = false;
                match result {
                    // The daemon echoes the message as output.
                    Ok(()) => {
                        state.refused = None;
                        if let Some(view) = state.views.get_mut(&session)
                            && view.queue.get(index) == Some(&message)
                        {
                            view.queue.remove(index);
                        }
                        let waiting: Vec<_> = state.views.keys().cloned().collect();
                        let effects: Vec<_> =
                            waiting.iter().map(|id| self.flush(state, id)).collect();
                        return Effect::merge(effects);
                    }
                    Err(error) => {
                        state.failed_queues.insert(session.clone());
                        state.fail(&error);
                    }
                }
            }
            SessionAction::Resume => {
                // D784: an archived session comes back only by Restore.
                if let Some(id) = state.open.clone()
                    && !open_session(state).is_some_and(|session| session.archived)
                {
                    return self.done(|daemon| daemon.resume(&id));
                }
            }
            SessionAction::StartFork(kind) => {
                if state.forking {
                    return Effect::none();
                }
                let Some(session) = open_session(state) else {
                    return Effect::none();
                };
                let title = format!("{} 2", session.title);
                let branch = suggested_branch(session);
                state.fork = Some(ForkDraft {
                    kind,
                    title,
                    before_turn: None,
                    agent: None,
                    machine: None,
                    branch,
                    turns: None,
                    picker: None,
                });
                state.error = None;
            }
            SessionAction::ForkKind(kind) => {
                if !state.forking
                    && let Some(fork) = &mut state.fork
                {
                    fork.kind = kind;
                    if kind == ForkKind::Shared {
                        // A fresh agent has no turns to cut at.
                        fork.before_turn = None;
                        if fork.picker == Some(ForkPicker::From) {
                            fork.picker = None;
                        }
                    }
                }
            }
            SessionAction::ForkTitle(title) => {
                if !state.forking
                    && let Some(fork) = &mut state.fork
                {
                    fork.title = title;
                }
            }
            SessionAction::ForkBeforeTurn(turn) => {
                if !state.forking
                    && let Some(fork) = &mut state.fork
                {
                    fork.before_turn = turn;
                    fork.picker = None;
                }
            }
            SessionAction::ForkAgent(agent) => {
                let parent = open_session(state).map(|session| session.agent);
                if !state.forking
                    && let Some(fork) = &mut state.fork
                {
                    fork.agent = (Some(agent) != parent).then_some(agent);
                    fork.picker = None;
                }
            }
            SessionAction::ForkMachine(machine) => {
                let parent = open_session(state).map(|session| session.machine.clone());
                if !state.forking
                    && let Some(fork) = &mut state.fork
                {
                    fork.machine = (Some(&machine) != parent.as_ref()).then_some(machine);
                    fork.picker = None;
                }
            }
            SessionAction::ForkBranch(branch) => {
                if !state.forking
                    && let Some(fork) = &mut state.fork
                {
                    fork.branch = branch;
                }
            }
            SessionAction::ForkPicker(picker) => {
                let Some(id) = state.open.clone() else {
                    return Effect::none();
                };
                if state.forking {
                    return Effect::none();
                }
                let Some(fork) = &mut state.fork else {
                    return Effect::none();
                };
                fork.picker = if fork.picker == picker { None } else { picker };
                if fork.picker == Some(ForkPicker::From) && fork.turns.is_none() {
                    let daemon = self.clients.daemon.clone();
                    return Effect::future(async move {
                        SessionAction::ForkTurns(daemon.list_turns(&id).await)
                    });
                }
            }
            SessionAction::ForkTurns(result) => {
                if let Some(fork) = &mut state.fork {
                    match result {
                        Ok(turns) => fork.turns = Some(turns),
                        // A daemon that can't list turns forks from now only.
                        Err(DaemonError::NotServedYet { .. }) => fork.turns = Some(Vec::new()),
                        Err(error) => state.error = Some(error.to_string()),
                    }
                }
            }
            SessionAction::ConfirmFork => {
                if state.forking {
                    return Effect::none();
                }
                let (Some(id), Some(fork)) = (state.open.clone(), state.fork.clone()) else {
                    return Effect::none();
                };
                if fork.title.trim().is_empty() {
                    state.error = Some("Give the fork a title before starting it.".into());
                    return Effect::none();
                }
                state.forking = true;
                state.error = None;
                let daemon = self.clients.daemon.clone();
                let options = ForkOptions {
                    kind: fork.kind,
                    title: fork.title.trim().to_owned(),
                    before_turn: fork.before_turn,
                    agent: fork.agent,
                    machine: fork.machine,
                    branch: fork.branch.trim().to_owned(),
                };
                return Effect::future(async move {
                    SessionAction::Forked(daemon.fork(&id, options).await)
                });
            }
            SessionAction::CancelFork => {
                if !state.forking {
                    state.fork = None;
                }
            }
            SessionAction::Forked(result) => {
                state.forking = false;
                match result {
                    Ok(id) => {
                        state.fork = None;
                        return Effect::send(SessionAction::Delegate(Delegate::OpenSession(id)));
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
            SessionAction::StartMove => {
                if open_session(state).is_some() {
                    state.moving = Some(MoveDraft::default());
                    state.error = None;
                }
            }
            SessionAction::MovePick(machine) => {
                let from = open_session(state).map(|session| session.machine.clone());
                let choosable = from.as_ref() != Some(&machine)
                    && state
                        .workspace
                        .machine(&machine)
                        .is_some_and(|found| found.online);
                if choosable
                    && let Some(draft) = &mut state.moving
                    && !draft.sending
                {
                    draft.to = Some(machine);
                }
            }
            SessionAction::ConfirmMove => {
                let (Some(id), Some(draft)) = (state.open.clone(), &mut state.moving) else {
                    return Effect::none();
                };
                let Some(to) = draft.to.clone() else {
                    return Effect::none();
                };
                if draft.sending {
                    return Effect::none();
                }
                draft.sending = true;
                state.error = None;
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    SessionAction::Moved(daemon.move_session(&id, &to).await)
                });
            }
            SessionAction::CancelMove => {
                if state.moving.as_ref().is_some_and(|draft| !draft.sending) {
                    state.moving = None;
                }
            }
            SessionAction::Moved(result) => match result {
                Ok(()) => state.moving = None,
                Err(error) => {
                    if let Some(draft) = &mut state.moving {
                        draft.sending = false;
                    }
                    state.error = Some(error.to_string());
                }
            },
            SessionAction::Archive => {
                if let Some(id) = state.open.clone() {
                    let title = state
                        .workspace
                        .session(&id)
                        .map_or_else(String::new, |s| s.title.clone());
                    let daemon = self.clients.daemon.clone();
                    return Effect::future(async move {
                        let result = daemon.plan_archive(&id).await;
                        SessionAction::ArchivePlanned {
                            session: id,
                            title,
                            result,
                        }
                    });
                }
            }
            SessionAction::ArchivePlanned {
                session,
                title,
                result,
            } => match result {
                Ok(plan) if plan.unpushed.is_empty() && plan.unsubmitted_drafts == 0 => {
                    return self.archive(session, title, false);
                }
                Ok(plan) => {
                    let (agent, machine) = close::agent_and_machine(state, &session);
                    state.archiving = Some(ArchiveConfirm {
                        session,
                        title,
                        plan,
                        sending: false,
                        agent,
                        machine,
                    });
                    state.error = None;
                }
                Err(error) => {
                    state.ending_for_good = None;
                    state.error = Some(error.to_string());
                }
            },
            SessionAction::ConfirmArchive(push_first) => {
                let Some(confirm) = &mut state.archiving else {
                    return Effect::none();
                };
                if confirm.sending {
                    return Effect::none();
                }
                confirm.sending = true;
                state.error = None;
                let (session, title) = (confirm.session.clone(), confirm.title.clone());
                return self.archive(session, title, push_first);
            }
            SessionAction::CancelArchive => {
                if state.archiving.as_ref().is_some_and(|confirm| !confirm.sending) {
                    state.archiving = None;
                    state.ending_for_good = None;
                }
            }
            SessionAction::Archived {
                session,
                title,
                result,
            } => match result {
                Ok(archive) => {
                    state.archiving = None;
                    if state.ending_for_good.as_ref() == Some(&session) {
                        state.ending_for_good = None;
                        let text = close::ended_text(&title, &close::machine_of(state, &session));
                        if state.open.as_ref() == Some(&session) {
                            state.open = None;
                        }
                        return Effect::send(SessionAction::Delegate(Delegate::EndedForGood(session, text)));
                    }
                    if state.open.as_ref() == Some(&session) {
                        state.open = None;
                    }
                    return Effect::send(SessionAction::Delegate(Delegate::Archived(
                        session,
                        title,
                        archive.frees,
                    )));
                }
                Err(error) => {
                    if let Some(confirm) = &mut state.archiving {
                        confirm.sending = false;
                    } else {
                        state.ending_for_good = None;
                    }
                    state.error = Some(error.to_string());
                }
            },
            SessionAction::TogglePin => {
                if let Some(session) = open_session(state) {
                    let (id, pinned) = (session.id.clone(), !session.pinned);
                    return self.done(|daemon| daemon.set_pinned(&id, pinned));
                }
            }
            SessionAction::ToggleVip => {
                if let Some(session) = open_session(state) {
                    let (id, vip) = (session.id.clone(), !session.vip);
                    return self.done(|daemon| daemon.set_vip(&id, vip));
                }
            }
            SessionAction::Done(result) => match result {
                Ok(()) => state.refused = None,
                Err(error) => state.fail(&error),
            },
            SessionAction::Typed(result) => match result {
                Ok(()) => state.refused = None,
                Err(error) => {
                    if let Some(refused) = SessionRefused::of(&error) {
                        state.refused = Some(refused);
                    }
                }
            },
            SessionAction::Migrations(migrations) => {
                if state.refused.as_ref().is_some_and(|refused| refused.fixed_by(&migrations)) {
                    state.refused = None;
                }
            }
            SessionAction::DismissError => state.error = None,
            SessionAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

/// The branch a fork of `session` would start on: beside the parent's, named
/// for its title, with a number so it is a new one (FC-15).
fn suggested_branch(session: &Session) -> String {
    let slug = session
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let prefix = session
        .branch
        .rsplit_once('/')
        .map_or("humata", |(prefix, _)| prefix);
    format!("{prefix}/{slug}-2")
}

fn open_session(state: &SessionState) -> Option<&humata2_clients::model::Session> {
    state
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
}

/// The lines a terminal tab shows: for the agent's tab the last snapshot's
/// transcript, then live output.
#[must_use]
pub fn lines(state: &SessionState, session: &SessionId, tab: usize) -> Vec<Line> {
    let mut lines = if tab == 0 {
        state
            .workspace
            .session(session)
            .map(|session| session.transcript.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if let Some(live) = state.live.get(&(session.clone(), tab)) {
        lines.extend(live.iter().cloned());
    }
    lines
}

/// A line from the person, as the terminal echoes it.
#[must_use]
pub fn echo(text: &str) -> Line {
    Line {
        from: Speaker::You,
        text: text.to_owned(),
    }
}
