//! The command palette (KP-1–6): ⌘K opens it over whatever shows; typing
//! narrows every command, channel and session by fuzzy match, recent picks
//! first; a pasted pull request URL offers to open it (KP-5). Acting from
//! it (flow 26.22): Move session asks inline which machine, Escape steps
//! back, and the search hits sit under the entries the cursor runs over.

use std::sync::Arc;

use humata2_clients::Workspace;
use humata2_clients::marks::{PinnedList, Thing};
use humata2_clients::model::{ChannelId, Machine, MachineId, SessionId};
use humata2_settings::{Row, Section};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

/// How many recent picks are remembered.
pub const RECENT: usize = 8;

/// A GitHub pull request named by a pasted URL or `owner/repo#n` (PR-1).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PullRef {
    /// The owner, such as `acme`.
    pub owner: String,
    /// The repository.
    pub repo: String,
    /// The pull request's number.
    pub number: u32,
}

/// What a pin or VIP mark applies to: a session or a channel (NV-20).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Marked {
    /// A session.
    Session(SessionId),
    /// A channel.
    Channel(ChannelId),
}

impl Marked {
    /// The host's name for this thing, as the pins are kept.
    #[must_use]
    pub fn to_thing(&self) -> Thing {
        match self {
            Self::Session(id) => Thing::Session(id.clone()),
            Self::Channel(id) => Thing::Channel(id.clone()),
        }
    }
}

/// What the palette is asking for inline (KP-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    /// The machine to move `session` to; `query` is what was typed before
    /// Move asked, given back by Escape.
    MoveSession {
        /// The session to move.
        session: SessionId,
        /// What was typed before Move asked.
        query: String,
    },
}

/// What a palette entry does.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Command {
    /// Go to Home.
    Home,
    /// Go to the Inbox.
    Inbox,
    /// Go to the global board.
    Board,
    /// Go to Hosts.
    Hosts,
    /// Go to Files.
    Files,
    /// Go to the Desk.
    Desk,
    /// Start a Desk session.
    NewDeskSession,
    /// Go to Settings.
    Settings,
    /// Start a session.
    NewSession,
    /// Make a channel.
    NewChannel,
    /// Open every sidebar tree.
    ExpandAll,
    /// Fold every sidebar tree.
    CollapseAll,
    /// Show or hide the sidebar.
    ToggleSidebar,
    /// Fork the open session.
    Fork,
    /// Archive the open session.
    Archive,
    /// Rename the open session; the Rename dialog opens (FC-17).
    RenameSession,
    /// Continue the open session in another channel; the Continue dialog
    /// opens (CV-8).
    ContinueInChannel,
    /// Stop the open session's agent; the session stays (SE-7).
    StopAgent,
    /// Open a channel.
    Channel(ChannelId),
    /// Open a session.
    Session(SessionId),
    /// Open one of a session's terminal tabs, hidden or not (D784).
    Terminal {
        /// The session it belongs to.
        session: SessionId,
        /// Its terminal tab: 0 is the agent's, then its shells.
        tab: usize,
    },
    /// Open a pull request by number.
    Pull(u32),
    /// Open a repository path in its session's own clone.
    File {
        session: SessionId,
        repo: String,
        path: String,
    },
    /// Expand the terminal in the current session.
    TerminalFullScreen,
    /// Join the open session's two tab groups into one.
    JoinTabGroups,
    /// Start, or stop, recording a bug report (I9, D159).
    Record,
    /// Take a snapshot for a bug report, like ⇧⌘S (D159).
    Snapshot,
    /// Stop a running recording, else take a snapshot; the form opens
    /// either way (D159).
    ReportBug,
    /// Open the last bug report's form again (D159).
    OpenLastReport,
    /// Show or hide the session's side panel.
    TogglePanel,
    /// Go back to the screen before this one.
    Back,
    /// Show the keyboard shortcuts.
    ShowShortcuts,
    /// Go to Views.
    Views,
    /// Open a host's page.
    Machine(MachineId),
    /// Open a channel's board.
    ChannelBoard(ChannelId),
    /// Move the open session to another machine; the palette asks which.
    MoveSession,
    /// Move a session to a machine, as the palette asked.
    MoveTo {
        /// The session to move.
        session: SessionId,
        /// Where it goes.
        machine: MachineId,
    },
    /// Review a GitHub pull request: open the session reviewing it, or the
    /// Review a pull request dialog with it pasted (PR-1).
    ReviewPull(PullRef),
    /// Add a pasted repo to a channel: opens its Add a GitHub repo sheet
    /// with the repo in the field (RL-5, KP-4).
    AddRepoToChannel {
        /// The channel on screen.
        channel: ChannelId,
        /// `owner/name`.
        repo: String,
    },
    /// Open a Settings section.
    SettingsSection(Section),
    /// Open Settings at a row, in its section.
    SettingsRow(Row),
    /// Pin or unpin a session or channel; `on` pins it, else unpins it
    /// (NV-20).
    Pin {
        /// What is pinned.
        thing: Marked,
        /// Pin it, or unpin it.
        on: bool,
    },
    /// Mark a session or channel VIP, or clear the mark; `on` marks it
    /// VIP, else clears the mark (NV-20).
    Vip {
        /// What is marked.
        thing: Marked,
        /// VIP, or not.
        on: bool,
    },
}

/// Where bug reporting stands, so its entries read right (D159).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReportStatus {
    /// A recording runs.
    pub recording: bool,
    /// A report was saved this run, so it can open again.
    pub last: bool,
}

/// One line of the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// What it does.
    pub command: Command,
    /// What it says.
    pub title: String,
    /// Its shortcut or kind, on the right.
    pub hint: String,
}

/// Where an entry is grouped in the list, first to last (KP-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// A pull request, pasted or numbered.
    Pull,
    /// What the session the palette opened over can do, when the query
    /// names the action rather than the session.
    ThisSession,
    /// Other sessions.
    Sessions,
    /// Files in the sessions' clones.
    Files,
    /// Channels.
    Channels,
    /// Commands, views, hosts' pages, boards and Settings.
    Commands,
    /// The machines Move session asks between.
    Machines,
}

/// The palette's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaletteState {
    /// What it searches.
    pub workspace: Arc<Workspace>,
    /// Whether it shows.
    pub open: bool,
    /// What was typed.
    pub query: String,
    /// The entries that match, best first.
    pub entries: Arc<Vec<Entry>>,
    /// The highlighted entry.
    pub cursor: usize,
    /// Recent picks, newest first; saved.
    pub recent: Vec<Command>,
    /// The session beneath the palette, for scoped file/context actions.
    pub context: Option<SessionId>,
    /// The channel whose page is beneath the palette, set by the app before
    /// it opens; the pin and VIP entries act on it when no session is open.
    pub channel: Option<ChannelId>,
    /// The host's pins as the app last had them, for the pin and VIP
    /// entries (NV-20).
    pub marks: PinnedList,
    /// Whether a session is open, so session commands apply.
    pub in_session: bool,
    /// Whether the open session has two tab groups, so they can be joined.
    pub split: bool,
    /// Bug reporting, set by the app before it opens the palette.
    pub report: ReportStatus,
    /// The question the palette asks inline, if any (KP-5).
    pub asking: Option<Ask>,
    /// How many search hits the app shows under the entries.
    pub hits: usize,
    /// Whether another page of hits can load, shown as a "more" row.
    pub more_hits: bool,
}

/// What the palette asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Do this.
    Run(Command),
    /// It closed; put focus back.
    Closed,
    /// Open the search hit at this place among the hits shown (LD-24).
    Hit(usize),
    /// Load the next page of hits; the palette stays open.
    MoreHits,
}

/// What happens in the palette.
#[derive(Clone, Debug, PartialEq)]
pub enum PaletteAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Saved recent picks came back at launch.
    Restore(Vec<Command>),
    /// ⌘K: open, with session commands if one is open.
    Open {
        /// Whether a session is open.
        in_session: bool,
    },
    /// The session under the palette changed.
    Context(Option<SessionId>),
    /// Escape or a click outside.
    Close,
    /// Escape while the palette shows: steps back from an inline question,
    /// else closes it.
    Escape,
    /// The query changed.
    Query(String),
    /// ↑ or ↓.
    Move(i32),
    /// Enter: run the highlighted entry.
    Run,
    /// An entry was clicked.
    Pick(usize),
    /// The search hits under the entries changed.
    Hits {
        /// How many hits there are.
        count: usize,
        /// Whether another page can load.
        more: bool,
    },
    /// Run this command as if its entry were picked (⌥⌘M, G3).
    Begin(Command),
    /// For the parent.
    Delegate(Delegate),
}

/// The palette's reducer.
#[derive(Clone, Copy, Debug, Default)]
pub struct Palette;

impl Reducer for Palette {
    type State = PaletteState;
    type Action = PaletteAction;

    fn reduce(&self, state: &mut PaletteState, action: PaletteAction) -> Effect<PaletteAction> {
        match action {
            PaletteAction::Show(workspace) => {
                state.workspace = workspace;
                if state.open {
                    state.refresh_keeping_cursor();
                }
            }
            PaletteAction::Restore(recent) => state.recent = recent,
            PaletteAction::Open { in_session } => {
                state.open = true;
                state.in_session = in_session;
                state.asking = None;
                state.query.clear();
                state.refresh();
            }
            PaletteAction::Context(session) => {
                state.context = session;
                if state.open {
                    state.refresh();
                }
            }
            PaletteAction::Close => {
                if state.open {
                    state.open = false;
                    state.asking = None;
                    return Effect::send(PaletteAction::Delegate(Delegate::Closed));
                }
            }
            PaletteAction::Escape => match state.asking.take() {
                Some(Ask::MoveSession { query, .. }) => {
                    state.query = query;
                    state.refresh();
                }
                None => return self.reduce(state, PaletteAction::Close),
            },
            PaletteAction::Query(query) => {
                state.query = query;
                state.refresh();
            }
            PaletteAction::Move(delta) => {
                let count = state.rows();
                if count > 0 {
                    let next = i64::try_from(state.cursor).unwrap_or(0) + i64::from(delta);
                    let count = i64::try_from(count).unwrap_or(i64::MAX);
                    state.cursor = usize::try_from(next.rem_euclid(count)).unwrap_or(0);
                }
            }
            PaletteAction::Run => return state.pick(state.cursor),
            PaletteAction::Pick(index) => return state.pick(index),
            PaletteAction::Hits { count, more } => {
                state.hits = count;
                state.more_hits = more;
                let total = state.rows();
                state.cursor = if total == 0 {
                    0
                } else {
                    state.cursor.min(total - 1)
                };
            }
            PaletteAction::Begin(command) => return state.pick_command(command),
            PaletteAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

impl PaletteState {
    /// Whether Move is asking for a machine and none can take the session:
    /// no other machine it may go to is online, whatever is typed (MV-1).
    #[must_use]
    pub fn no_machine_to_move_to(&self) -> bool {
        match &self.asking {
            Some(Ask::MoveSession { session, .. }) => self.machine_entries(session, "").is_empty(),
            None => false,
        }
    }

    /// How many rows the cursor walks: the entries, then the hits and the
    /// "more" row, which are not shown while the palette asks inline.
    fn rows(&self) -> usize {
        let hits = if self.asking.is_some() {
            0
        } else {
            self.hits + usize::from(self.more_hits)
        };
        self.entries.len() + hits
    }

    fn pick(&mut self, index: usize) -> Effect<PaletteAction> {
        if let Some(entry) = self.entries.get(index) {
            let command = entry.command.clone();
            return self.pick_command(command);
        }
        if self.asking.is_some() {
            return Effect::none();
        }
        let Some(hit) = index.checked_sub(self.entries.len()) else {
            return Effect::none();
        };
        if hit < self.hits {
            self.open = false;
            return Effect::concat([
                Effect::send(PaletteAction::Delegate(Delegate::Closed)),
                Effect::send(PaletteAction::Delegate(Delegate::Hit(hit))),
            ]);
        }
        if hit == self.hits && self.more_hits {
            return Effect::send(PaletteAction::Delegate(Delegate::MoreHits));
        }
        Effect::none()
    }

    /// Runs a command, except that Move session asks which machine first
    /// and a machine move is not a recent pick.
    fn pick_command(&mut self, command: Command) -> Effect<PaletteAction> {
        if command == Command::MoveSession
            && let Some(session) = self.context.clone()
        {
            self.asking = Some(Ask::MoveSession {
                session,
                query: std::mem::take(&mut self.query),
            });
            self.refresh();
            return Effect::none();
        }
        if !matches!(
            command,
            Command::MoveTo { .. }
                | Command::Pin { .. }
                | Command::Vip { .. }
                | Command::AddRepoToChannel { .. }
        ) {
            self.recent.retain(|recent| recent != &command);
            self.recent.insert(0, command.clone());
            self.recent.truncate(RECENT);
        }
        self.open = false;
        self.asking = None;
        Effect::concat([
            Effect::send(PaletteAction::Delegate(Delegate::Closed)),
            Effect::send(PaletteAction::Delegate(Delegate::Run(command))),
        ])
    }

    /// Lists the entries again for a changed workspace, with the cursor
    /// still on the row it was on: a session that changes state while the
    /// palette is open doesn't send the person back to the top. A row that
    /// is gone puts the cursor on the first.
    fn refresh_keeping_cursor(&mut self) {
        let before = self.entries.clone();
        let cursor = self.cursor;
        self.refresh();
        self.cursor = match before.get(cursor) {
            Some(entry) => self
                .entries
                .iter()
                .position(|found| found.command == entry.command)
                .unwrap_or(0),
            // A hit row: the hits follow the entries, so it moves with them.
            None => cursor
                .checked_sub(before.len())
                .map(|hit| self.entries.len() + hit)
                .filter(|&row| row < self.rows())
                .unwrap_or(0),
        };
    }

    fn refresh(&mut self) {
        let query = self.query.trim();
        let asking = self
            .asking
            .as_ref()
            .map(|Ask::MoveSession { session, .. }| session.clone());
        let scored = match &asking {
            Some(session) => self.machine_entries(session, query),
            None => self.command_entries(query),
        };
        let mut ranked: Vec<(Group, i64, Entry)> = scored
            .into_iter()
            .map(|(score, entry)| (self.group(&entry), score, entry))
            .collect();
        ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
        self.entries = Arc::new(ranked.into_iter().map(|(_, _, entry)| entry).collect());
        self.cursor = 0;
    }

    /// The machines Move session can send `session` to: online, not where
    /// it runs, and matching the query (MV-1). A Desk session may go to any
    /// host (DK-3).
    fn machine_entries(&self, session: &SessionId, query: &str) -> Vec<(i64, Entry)> {
        let Some(current) = self.workspace.session(session) else {
            return Vec::new();
        };
        // A Desk session works in the same place on every machine, and a
        // review's channel is Humata's own (PR-21): the move gives the machine
        // it, so any online machine may be picked.
        let any_machine = humata2_clients::desk::is_desk(current) || current.pull_request_review;
        let candidates: Vec<&Machine> = if any_machine {
            self.workspace.machines.iter().collect()
        } else {
            self.workspace.channel_machines(&current.channel).collect()
        };
        candidates
            .into_iter()
            .filter(|machine| machine.online && machine.id != current.machine)
            .filter_map(|machine| {
                let score = fuzzy(query, &machine.name)?;
                let entry = Entry {
                    command: Command::MoveTo {
                        session: session.clone(),
                        machine: machine.id.clone(),
                    },
                    title: machine.name.clone(),
                    hint: machine.detail.clone(),
                };
                Some((score, entry))
            })
            .collect()
    }

    /// What a pasted repo (`owner/name` or a GitHub address) offers: adding it
    /// to the channel on screen, and the sessions already in it (KP-4). A pull
    /// request address is not a repo here.
    fn repo_entries(&self, query: &str) -> Vec<(i64, Entry)> {
        if pull_ref(query).is_some() || pull_number(query).is_some() {
            return Vec::new();
        }
        let Ok(repo) = humata2_clients::github::parse_repo_text(query) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        if let Some(channel) = &self.channel {
            found.push((
                i64::MAX,
                Entry {
                    command: Command::AddRepoToChannel {
                        channel: channel.clone(),
                        repo: repo.clone(),
                    },
                    title: format!("Add {repo} to this channel\u{2026}"),
                    hint: "Repo".to_owned(),
                },
            ));
        }
        let working = self
            .workspace
            .sessions
            .iter()
            .filter(|session| !session.archived && session.repo.eq_ignore_ascii_case(&repo))
            .take(5);
        for session in working {
            found.push((
                i64::MAX - 1,
                Entry {
                    command: Command::Session(session.id.clone()),
                    title: session.title.clone(),
                    hint: repo.clone(),
                },
            ));
        }
        found
    }

    /// Every command, channel, session and file the query matches, with
    /// its score; the pull request entries come first when the query is one.
    fn command_entries(&self, query: &str) -> Vec<(i64, Entry)> {
        let mut scored: Vec<(i64, Entry)> = Vec::new();
        // PR-1 (reverses D197): a pasted pull request offers Review this PR at
        // once, from the parsed text. The app opens the session already
        // reviewing it, else the Review a pull request dialog.
        if let Some(pull) = pull_ref(query) {
            let title = format!(
                "Review this PR \u{b7} {}/{} #{}",
                pull.owner, pull.repo, pull.number
            );
            scored.push((
                i64::MAX,
                Entry {
                    command: Command::ReviewPull(pull),
                    title,
                    hint: "Pull request".to_owned(),
                },
            ));
        }
        scored.extend(self.repo_entries(query));
        if let Some(number) = pull_number(query) {
            scored.push((
                i64::MAX,
                Entry {
                    command: Command::Pull(number),
                    title: format!("Open pull request #{number}"),
                    hint: "Pull request".to_owned(),
                },
            ));
        }
        for entry in self.all() {
            // A terminal is searched on its title and machine (its hint),
            // never its session's status word.
            let search = if matches!(entry.command, Command::Session(_) | Command::Terminal { .. }) {
                format!("{} {}", entry.title, entry.hint)
            } else {
                entry.title.clone()
            };
            let contextual = matches!(
                entry.command,
                Command::TogglePanel | Command::TerminalFullScreen
            ) && self
                .context
                .as_ref()
                .and_then(|id| self.workspace.session(id))
                .is_some_and(|session| fuzzy(query, &session.title).is_some());
            if let Some(score) = fuzzy(query, &search).or_else(|| contextual.then_some(0)) {
                let recent = self
                    .recent
                    .iter()
                    .position(|command| command == &entry.command)
                    .map_or(0, |at| 1_000 - i64::try_from(at).unwrap_or(0));
                let direct = fuzzy(query, &entry.title).is_some();
                // A session's terminals sit just below the session (D784).
                let terminal = if matches!(entry.command, Command::Terminal { .. }) { 100 } else { 0 };
                scored.push((score + recent + if direct { 10_000 } else { 0 } - terminal, entry));
            }
        }
        scored
    }

    /// The group an entry sits in (G4). A command the query names, that
    /// acts on the session the palette opened over, is "this session" when
    /// the session's own title does not match the query (KP-3).
    #[must_use]
    pub fn group(&self, entry: &Entry) -> Group {
        match entry.command {
            Command::ReviewPull(_) | Command::Pull(_) => Group::Pull,
            Command::MoveTo { .. } => Group::Machines,
            Command::Session(_) | Command::Terminal { .. } => Group::Sessions,
            Command::File { .. } => Group::Files,
            Command::Channel(_) => Group::Channels,
            _ if self.promoted(entry) => Group::ThisSession,
            _ => Group::Commands,
        }
    }

    /// Whether `entry` is one of the session actions the query asks for by
    /// name, so it sorts ahead of the sessions it would otherwise follow.
    fn promoted(&self, entry: &Entry) -> bool {
        let query = self.query.trim();
        matches!(
            entry.command,
            Command::MoveSession
                | Command::Fork
                | Command::Archive
                | Command::RenameSession
                | Command::ContinueInChannel
                | Command::StopAgent
                | Command::TerminalFullScreen
                | Command::TogglePanel
        ) && !query.is_empty()
            && fuzzy(query, &entry.title).is_some()
            && self
                .context
                .as_ref()
                .and_then(|id| self.workspace.session(id))
                .is_none_or(|session| fuzzy(query, &session.title).is_none())
    }

    /// The pin and VIP entries for the session beneath the palette, or for
    /// the channel whose page it opened over (NV-20). Each says what it will
    /// do, from the marks the app handed over.
    fn mark_entries(&self) -> Vec<Entry> {
        let (thing, workspace_vip, subject) = match (&self.context, &self.channel) {
            (Some(id), _) => {
                let Some(session) = self.workspace.session(id) else {
                    return Vec::new();
                };
                (Marked::Session(id.clone()), session.vip, "this session")
            }
            (None, Some(channel)) => (Marked::Channel(channel.clone()), false, "this channel"),
            (None, None) => return Vec::new(),
        };
        let pinned = self
            .marks
            .items
            .iter()
            .find(|pinned| pinned.thing == thing.to_thing());
        let vip = workspace_vip || pinned.is_some_and(|pinned| pinned.vip);
        let (pin, pin_title) = match pinned {
            Some(_) => (false, format!("Unpin {subject}")),
            None => (true, format!("Pin {subject}")),
        };
        let (mark, mark_title) = if vip {
            (false, format!("Clear {subject}'s VIP mark"))
        } else {
            (true, format!("Mark {subject} VIP"))
        };
        [
            (
                Command::Pin {
                    thing: thing.clone(),
                    on: pin,
                },
                pin_title,
            ),
            (
                Command::Vip {
                    thing,
                    on: mark,
                },
                mark_title,
            ),
        ]
        .into_iter()
        .map(|(command, title)| Entry {
            command,
            title,
            hint: String::new(),
        })
        .collect()
    }

    fn report_entries(&self) -> Vec<Entry> {
        let record = if self.report.recording {
            "Stop recording"
        } else {
            "Start recording a bug report"
        };
        let mut entries = vec![
            (Command::Record, record, "⇧⌘R"),
            (Command::Snapshot, "Take a snapshot for a bug report", "⇧⌘S"),
            (Command::ReportBug, "Report a bug…", ""),
        ];
        if self.report.last {
            entries.push((Command::OpenLastReport, "Open the last bug report", ""));
        }
        entries
            .into_iter()
            .map(|(command, title, hint)| Entry {
                command,
                title: title.to_owned(),
                hint: hint.to_owned(),
            })
            .collect()
    }

    fn all(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = [
            (Command::Home, "Go to Home", "⌘1"),
            (Command::Inbox, "Go to Inbox", "⌘2"),
            (Command::Board, "Go to Board", "⌘3"),
            (Command::Hosts, "Go to Hosts", "⌘4"),
            (Command::Files, "Go to Files", "⌘5"),
            (Command::Desk, "Go to Desk", ""),
            (Command::NewDeskSession, "New desk session…", ""),
            (Command::Settings, "Open Settings", "⌘,"),
            (Command::NewSession, "New session…", "⌘N"),
            (Command::NewChannel, "New channel…", "⇧⌘N"),
            (Command::ExpandAll, "Expand all", "⌥⌘→"),
            (Command::CollapseAll, "Collapse all", "⌥⌘←"),
            (Command::ToggleSidebar, "Toggle sidebar", "⌘0"),
            (Command::TogglePanel, "Toggle side panel", "⌥⌘0"),
            (Command::Back, "Go back", "⌘["),
            (Command::ShowShortcuts, "Keyboard shortcuts", "⌘/"),
            (Command::Views, "Go to Views", ""),
        ]
        .into_iter()
        .map(|(command, title, hint)| Entry {
            command,
            title: title.to_owned(),
            hint: hint.to_owned(),
        })
        .collect();
        entries.extend(self.report_entries());
        entries.extend(self.mark_entries());
        if self.in_session {
            entries.push(Entry {
                command: Command::TerminalFullScreen,
                title: "Terminal full screen".into(),
                hint: "⇧⌘↩".into(),
            });
            if self.split {
                entries.push(Entry {
                    command: Command::JoinTabGroups,
                    title: "Join tab groups".into(),
                    hint: String::new(),
                });
            }
            // A Desk session has no Fork, and its Archive is Close (DK-11,
            // DK-12).
            let on_desk = self
                .context
                .as_ref()
                .and_then(|id| self.workspace.session(id))
                .is_some_and(humata2_clients::desk::is_desk);
            if !on_desk {
                entries.push(Entry {
                    command: Command::Fork,
                    title: self
                        .context
                        .as_ref()
                        .and_then(|id| self.workspace.session(id))
                        .map_or_else(
                            || "Fork this session…".into(),
                            |session| format!("Fork “{}”", session.title),
                        ),
                    hint: "⇧⌘F".to_owned(),
                });
            }
            entries.push(Entry {
                command: Command::Archive,
                title: if on_desk {
                    "Close this session"
                } else {
                    "Archive this session"
                }
                .to_owned(),
                hint: String::new(),
            });
            let live = self
                .context
                .as_ref()
                .and_then(|id| self.workspace.session(id))
                .is_some_and(|session| !session.archived);
            let open = self.context.as_ref().and_then(|id| self.workspace.session(id));
            if live {
                entries.push(Entry {
                    command: Command::MoveSession,
                    title: "Move session to another machine…".to_owned(),
                    hint: "⌥⌘M".to_owned(),
                });
                entries.push(Entry {
                    command: Command::RenameSession,
                    title: "Rename this session…".to_owned(),
                    hint: String::new(),
                });
            }
            // CV-8: the Desk isn't a channel, so a Desk session has no
            // Continue, as its session menu has none.
            if live && !on_desk {
                entries.push(Entry {
                    command: Command::ContinueInChannel,
                    title: "Continue in another channel…".to_owned(),
                    hint: String::new(),
                });
            }
            if live
                && open.is_some_and(|session| {
                    !matches!(session.status, humata2_clients::model::Status::Exited { .. })
                })
            {
                entries.push(Entry {
                    command: Command::StopAgent,
                    title: "Stop this session's agent".to_owned(),
                    hint: String::new(),
                });
            }
        }
        for channel in &self.workspace.channels {
            entries.push(Entry {
                command: Command::Channel(channel.id.clone()),
                title: format!("#{}", channel.name),
                hint: "Channel".to_owned(),
            });
        }
        for machine in &self.workspace.machines {
            entries.push(Entry {
                command: Command::Machine(machine.id.clone()),
                title: machine.name.clone(),
                hint: "Machine".to_owned(),
            });
        }
        for channel in &self.workspace.channels {
            entries.push(Entry {
                command: Command::ChannelBoard(channel.id.clone()),
                title: format!("#{} board", channel.name),
                hint: "Board".to_owned(),
            });
        }
        for section in Section::ALL {
            entries.push(Entry {
                command: Command::SettingsSection(section),
                title: format!("Settings › {}", section.title()),
                hint: "Settings".to_owned(),
            });
        }
        for row in Row::ALL {
            entries.push(Entry {
                command: Command::SettingsRow(row),
                title: format!("Settings › {}", row.title()),
                hint: row.section().title().to_owned(),
            });
        }
        for session in self
            .workspace
            .sessions
            .iter()
            .filter(|session| !session.archived)
        {
            entries.push(Entry {
                command: Command::Session(session.id.clone()),
                title: session.title.clone(),
                hint: session
                    .parent
                    .as_ref()
                    .and_then(|id| self.workspace.session(id))
                    .map_or_else(
                        || {
                            format!(
                                "{} · {}",
                                if humata2_clients::desk::is_desk(session) {
                                    "Desk".to_owned()
                                } else {
                                    format!("# {}", session.channel)
                                },
                                session.status.label().to_lowercase()
                            )
                        },
                        |parent| format!("fork of {}", parent.title),
                    ),
            });
        }
        // D784: every terminal tab of a session, hidden or not, so a tab
        // closed with Keep running is found again.
        for session in self.workspace.sessions.iter().filter(|session| !session.archived) {
            let machine = self
                .workspace
                .machines
                .iter()
                .find(|machine| machine.id == session.machine)
                .map_or_else(|| humata2_clients::machine_names::name_of(&session.machine), |machine| machine.name.clone());
            for (at, shell) in session.shells.iter().enumerate() {
                let tab = at + 1;
                let program = session
                    .terminal_agents
                    .get(&tab)
                    .map_or_else(|| shell.clone(), |agent| agent.name().to_owned());
                entries.push(Entry {
                    command: Command::Terminal {
                        session: session.id.clone(),
                        tab,
                    },
                    title: format!("{program} in {}", session.title),
                    hint: machine.clone(),
                });
            }
        }
        for (session, files) in &self.workspace.session_files {
            if self.context.as_ref().is_some_and(|id| id != session) {
                continue;
            }
            let Some(owner) = self.workspace.session(session).filter(|s| !s.archived) else {
                continue;
            };
            for file in files.iter().filter(|file| !file.folder) {
                entries.push(Entry {
                    command: Command::File {
                        session: session.clone(),
                        repo: file.repo.clone(),
                        path: file.path.clone(),
                    },
                    title: format!(
                        "{}/{}",
                        file.repo.rsplit('/').next().unwrap_or(&file.repo),
                        file.path
                    ),
                    hint: owner.title.clone(),
                });
            }
        }
        entries
    }
}

/// How well `query` matches `text` as a subsequence, ignoring case; higher
/// is better, `None` when it doesn't match.
#[must_use]
pub fn fuzzy(query: &str, text: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0_i64;
    let mut at = 0;
    let mut last: Option<usize> = None;
    for wanted in query.to_lowercase().chars() {
        let found = text[at..].iter().position(|&c| c == wanted)? + at;
        score += 10;
        if last.is_some_and(|last| last + 1 == found) {
            score += 15;
        }
        if found == 0 || text.get(found - 1).is_some_and(|c| !c.is_alphanumeric()) {
            score += 20;
        }
        last = Some(found);
        at = found + 1;
    }
    Some(score - i64::try_from(text.len()).unwrap_or(0))
}

/// The pull request number in a pasted URL such as
/// `https://github.com/org/repo/pull/412`, or `#412`.
#[must_use]
pub fn pull_number(text: &str) -> Option<u32> {
    if let Some(number) = text.strip_prefix('#') {
        return number.parse().ok();
    }
    let (_, rest) = text.split_once("/pull/")?;
    rest.split(['/', '#', '?']).next()?.parse().ok()
}

/// The pull request a pasted GitHub URL (with or without `https://`, and
/// with a trailing `/files`, `?…` or `#…`) or an `owner/repo#n` names (PR-1).
#[must_use]
pub fn pull_ref(text: &str) -> Option<PullRef> {
    let text = text.trim();
    let without_scheme = text
        .strip_prefix("https://")
        .or_else(|| text.strip_prefix("http://"))
        .unwrap_or(text);
    if let Some(path) = without_scheme.strip_prefix("github.com/") {
        let path = path.split(['?', '#']).next()?;
        let mut parts = path.split('/');
        let owner = parts.next()?;
        let repo = parts.next()?;
        if parts.next()? != "pull" {
            return None;
        }
        let number = parts.next()?.parse().ok()?;
        return named_pull(owner, repo, number);
    }
    let (names, number) = text.split_once('#')?;
    let (owner, repo) = names.split_once('/')?;
    named_pull(owner, repo, number.parse().ok()?)
}

/// A pull request whose owner and repository are plain names.
fn named_pull(owner: &str, repo: &str, number: u32) -> Option<PullRef> {
    let plain = |part: &str| {
        !part.is_empty()
            && !part.contains(['/', '#', '?'])
            && !part.chars().any(char::is_whitespace)
    };
    (plain(owner) && plain(repo)).then(|| PullRef {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number,
    })
}
