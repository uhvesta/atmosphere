//! The command palette (KP-1–6): ⌘K opens it over whatever shows; typing
//! narrows every command, channel and session by fuzzy match, recent picks
//! first; a pasted pull request URL offers to open it (KP-5).

use std::sync::Arc;

use humata2_clients::Workspace;
use humata2_clients::model::{ChannelId, SessionId};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer};

/// How many recent picks are remembered.
pub const RECENT: usize = 8;

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
    /// Open a channel.
    Channel(ChannelId),
    /// Open a session.
    Session(SessionId),
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
    /// Whether a session is open, so session commands apply.
    pub in_session: bool,
    /// Bug reporting, set by the app before it opens the palette.
    pub report: ReportStatus,
}

/// What the palette asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Do this.
    Run(Command),
    /// It closed; put focus back.
    Closed,
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
    /// The query changed.
    Query(String),
    /// ↑ or ↓.
    Move(i32),
    /// Enter: run the highlighted entry.
    Run,
    /// An entry was clicked.
    Pick(usize),
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
                    state.refresh();
                }
            }
            PaletteAction::Restore(recent) => state.recent = recent,
            PaletteAction::Open { in_session } => {
                state.open = true;
                state.in_session = in_session;
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
                    return Effect::send(PaletteAction::Delegate(Delegate::Closed));
                }
            }
            PaletteAction::Query(query) => {
                state.query = query;
                state.refresh();
            }
            PaletteAction::Move(delta) => {
                let count = state.entries.len();
                if count > 0 {
                    let next = i64::try_from(state.cursor).unwrap_or(0) + i64::from(delta);
                    let count = i64::try_from(count).unwrap_or(i64::MAX);
                    state.cursor = usize::try_from(next.rem_euclid(count)).unwrap_or(0);
                }
            }
            PaletteAction::Run => return state.pick(state.cursor),
            PaletteAction::Pick(index) => return state.pick(index),
            PaletteAction::Delegate(_) => {}
        }
        Effect::none()
    }
}

impl PaletteState {
    fn pick(&mut self, index: usize) -> Effect<PaletteAction> {
        let Some(entry) = self.entries.get(index).cloned() else {
            return Effect::none();
        };
        self.recent.retain(|command| command != &entry.command);
        self.recent.insert(0, entry.command.clone());
        self.recent.truncate(RECENT);
        self.open = false;
        Effect::concat([
            Effect::send(PaletteAction::Delegate(Delegate::Closed)),
            Effect::send(PaletteAction::Delegate(Delegate::Run(entry.command))),
        ])
    }

    fn refresh(&mut self) {
        let query = self.query.trim();
        let mut entries: Vec<(i64, Entry)> = Vec::new();
        if let Some(number) = pull_number(query) {
            entries.push((
                i64::MAX,
                Entry {
                    command: Command::Pull(number),
                    title: format!("Open pull request #{number}"),
                    hint: "Pull request".to_owned(),
                },
            ));
        }
        for entry in self.all() {
            let search = if matches!(entry.command, Command::Session(_)) {
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
                entries.push((score + recent + if direct { 10_000 } else { 0 }, entry));
            }
        }
        let rank = |entry: &Entry| match entry.command {
            Command::Session(_) => 0,
            Command::File { .. } => 1,
            Command::Channel(_) => 2,
            _ => 3,
        };
        entries.sort_by(|a, b| rank(&a.1).cmp(&rank(&b.1)).then_with(|| b.0.cmp(&a.0)));
        self.entries = Arc::new(entries.into_iter().map(|(_, entry)| entry).collect());
        self.cursor = 0;
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
        ]
        .into_iter()
        .map(|(command, title, hint)| Entry {
            command,
            title: title.to_owned(),
            hint: hint.to_owned(),
        })
        .collect();
        entries.extend(self.report_entries());
        if self.in_session {
            entries.push(Entry {
                command: Command::TerminalFullScreen,
                title: "Terminal full screen".into(),
                hint: "⇧⌘↩".into(),
            });
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
        }
        for channel in &self.workspace.channels {
            entries.push(Entry {
                command: Command::Channel(channel.id.clone()),
                title: format!("#{}", channel.name),
                hint: "Channel".to_owned(),
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
