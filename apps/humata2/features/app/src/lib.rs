//! Humata 2's root feature: which screen shows and Back (NV-1–5), the
//! overlays on top (palette, sheets, dialogs) with Escape closing the top
//! one, pane sizes, toasts, loading the workspace and following the
//! daemon's event stream, and saving where the person was so a restart
//! puts them back (PER-25, PER-26).
//!
//! Each screen is its own feature crate; this one holds their states, sends
//! them the data as it changes, and handles what they delegate.

pub mod layout;
pub mod saved;

use std::sync::Arc;
use std::time::Duration;

use humata2_board::{Board, BoardAction, BoardState};
use humata2_clients::model::{Agent, ChannelId, MachineId, Scope, SessionId};
use humata2_clients::{Clients, DaemonError, Event, Workspace};
use humata2_files::{Files, FilesAction, FilesState};
use humata2_hosts::{Hosts, HostsAction, HostsState};
use humata2_inbox::{Inbox, InboxAction, InboxState};
use humata2_palette::{Command, Palette, PaletteAction, PaletteState};
use humata2_review::{Review, ReviewAction, ReviewState};
use humata2_session::{Panel, SessionAction, SessionState, Sessions};
use humata2_settings::{Settings, SettingsAction, SettingsState};
use humata2_sidebar::{RowKey, Sidebar, SidebarAction, SidebarState};
use humata2_views::{Views, ViewsAction, ViewsState};
use serde::{Deserialize, Serialize};
use tca::{Effect, Reducer, StreamEnd};

use crate::layout::{FullScreen, Layout, PaneId};
use crate::saved::{SavedUi, Saver};

/// How long a toast shows.
pub const TOAST: Duration = Duration::from_secs(4);

/// Drafts save at most this often while they change.
pub const DRAFT_SAVE: Duration = Duration::from_secs(1);

/// How many screens Back remembers.
pub const TRAIL: usize = 50;

/// A screen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    /// What needs the person, across channels (HM-1–5).
    #[default]
    Home,
    /// The Inbox.
    Inbox,
    /// A board.
    Board(Scope),
    /// A channel's page.
    Channel(ChannelId),
    /// A session's page.
    Session(SessionId),
    /// Hosts.
    Hosts,
    /// Files and transfers.
    Files,
    /// Prepared views.
    Views,
    /// Settings.
    Settings,
}

/// Whether the workspace is there yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Load {
    /// Waiting for the daemon.
    #[default]
    Loading,
    /// Loaded.
    Ready,
    /// It couldn't load; the message says why and what to do.
    Failed(String),
}

/// The New session sheet (SE-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSession {
    /// Its channel.
    pub channel: Option<ChannelId>,
    /// Its agent.
    pub agent: Agent,
    /// Its host.
    pub machine: Option<MachineId>,
    /// Its title.
    pub title: String,
    /// Waiting for the daemon.
    pub creating: bool,
    /// Why it couldn't be made.
    pub error: Option<String>,
}

/// The New channel sheet (CH-1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewChannel {
    /// Its name.
    pub name: String,
    /// Its repositories, comma-separated as typed.
    pub repos: String,
    /// Waiting for the daemon.
    pub creating: bool,
    /// Why it couldn't be made.
    pub error: Option<String>,
}

/// Something over the screen. The top one takes the keyboard and Escape
/// closes it, putting focus back where it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// The command palette.
    Palette,
    /// The New session sheet.
    NewSession(NewSession),
    /// The New channel sheet.
    NewChannel(NewChannel),
    /// Keyboard shortcuts.
    Shortcuts,
}

/// A message at the bottom of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    /// Its id, to dismiss it.
    pub id: u64,
    /// What it says.
    pub text: String,
}

/// The whole app's state.
#[derive(Clone, Debug, PartialEq)]
pub struct AppState {
    /// Whether the workspace loaded.
    pub load: Load,
    /// What every daemon holds.
    pub workspace: Arc<Workspace>,
    /// The screen showing.
    pub route: Route,
    /// Screens Back returns to.
    pub trail: Vec<Route>,
    /// Overlays, bottom first.
    pub overlays: Vec<Overlay>,
    /// Pane sizes.
    pub layout: Layout,
    /// Each session's full-window presentation, independent of pane sizes.
    pub full_screens: std::collections::BTreeMap<SessionId, FullScreen>,
    /// A temporary navigation overlay when the sidebar is folded.
    pub navigation_overlay: bool,
    /// Toasts, oldest first.
    pub toasts: Vec<Toast>,
    /// Why saved state from the last run couldn't be used, shown once.
    pub restore_error: Option<String>,
    /// The sidebar.
    pub sidebar: SidebarState,
    /// Session pages.
    pub session: SessionState,
    /// The review panel.
    pub review: ReviewState,
    /// The Inbox.
    pub inbox: InboxState,
    /// Boards.
    pub board: BoardState,
    /// Hosts.
    pub hosts: HostsState,
    /// Files.
    pub files: FilesState,
    /// Settings.
    pub settings: SettingsState,
    /// The palette.
    pub palette: PaletteState,
    /// Prepared views.
    pub views: ViewsState,
    next_toast: u64,
    last_saved: SavedUi,
    save_generation: u64,
    save_due: bool,
}

impl AppState {
    /// The app as it starts: what the last run saved, restored before the
    /// first frame, waiting for the workspace. Unreadable saved state is
    /// reported once and the app starts on Home.
    #[must_use]
    pub fn launch(clients: &Clients) -> Self {
        let (saved, restore_error) = match saved::restore(clients.storage.as_ref()) {
            Ok(saved) => (saved.unwrap_or_default(), None),
            Err(error) => (SavedUi::default(), Some(error.to_string())),
        };
        let mut state = Self {
            load: Load::Loading,
            workspace: Arc::default(),
            route: saved.route.clone(),
            trail: saved.trail.clone(),
            overlays: Vec::new(),
            layout: saved.layout,
            full_screens: saved.full_screens.clone(),
            navigation_overlay: false,
            toasts: Vec::new(),
            restore_error,
            sidebar: SidebarState::default(),
            session: SessionState::default(),
            review: ReviewState::default(),
            inbox: InboxState::default(),
            board: BoardState::default(),
            hosts: HostsState::default(),
            files: FilesState::default(),
            settings: SettingsState::default(),
            palette: PaletteState::default(),
            views: ViewsState::default(),
            next_toast: 0,
            last_saved: saved.clone(),
            save_generation: 0,
            save_due: false,
        };
        state.sidebar.folding = saved.folding;
        state.session.views = saved.sessions;
        for (id, view) in &state.session.views {
            if view.widened {
                state
                    .full_screens
                    .entry(id.clone())
                    .or_insert(FullScreen::Panel);
            }
        }
        state.settings.preferences = saved.preferences;
        state.settings.has_key = clients
            .keychain
            .has(humata2_settings::KEY_NAME)
            .unwrap_or(false);
        state.review.saved = saved.review;
        state.review.wrap = state.settings.preferences.wrap_diffs;
        state.palette.recent = saved.recent;
        state.views.decisions = saved.views;
        state.hosts.previously = saved.connected;
        if let Route::Session(id) = &state.route {
            state.session.open = Some(id.clone());
        }
        state
    }

    /// The top overlay.
    #[must_use]
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlays.last()
    }

    /// What is saved now.
    #[must_use]
    pub fn saved(&self) -> SavedUi {
        SavedUi {
            route: self.route.clone(),
            trail: self.trail.clone(),
            sessions: self.session.views.clone(),
            layout: self.layout,
            full_screens: self.full_screens.clone(),
            folding: self.sidebar.folding.clone(),
            preferences: self.settings.preferences.clone(),
            review: self.review.saved.clone(),
            recent: self.palette.recent.clone(),
            views: self.views.decisions.clone(),
            connected: self.hosts.connected(),
        }
    }

    /// The open session's full-window presentation, if any.
    #[must_use]
    pub fn full_screen(&self) -> Option<FullScreen> {
        let Route::Session(id) = &self.route else {
            return None;
        };
        self.full_screens.get(id).copied().filter(|side| {
            *side != FullScreen::Panel || (self.panel().is_some() && !self.layout.panel.collapsed)
        })
    }

    /// The open session's side panel, if one shows.
    #[must_use]
    pub fn panel(&self) -> Option<Panel> {
        match &self.route {
            Route::Session(_) => self.session.view().and_then(|view| view.panel),
            _ => None,
        }
    }

    fn toast(&mut self, text: String, clock: &Arc<dyn tca::Clock>) -> Effect<AppAction> {
        self.next_toast += 1;
        let id = self.next_toast;
        self.toasts.push(Toast { id, text });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        let sleep = clock.sleep(TOAST);
        Effect::future(async move {
            sleep.await;
            AppAction::ToastExpired(id)
        })
        .cancellable(format!("toast-{id}"), true)
    }
}

/// A pane change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaneAction {
    /// Its divider was dragged so it would be this wide.
    Drag(PaneId, f32),
    /// Its collapse button was clicked.
    Toggle(PaneId),
    /// Its divider was double-clicked.
    Reset(PaneId),
}

/// A change to the New session sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewSessionAction {
    /// Pick its channel.
    Channel(ChannelId),
    /// Pick its agent.
    Agent(Agent),
    /// Pick its host.
    Machine(MachineId),
    /// Its title changed.
    Title(String),
    /// Start it.
    Create,
}

/// A change to the New channel sheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewChannelAction {
    /// Its name changed.
    Name(String),
    /// Its repositories changed.
    Repos(String),
    /// Make it.
    Create,
}

/// Everything that happens in the app.
#[derive(Clone, Debug, PartialEq)]
pub enum AppAction {
    /// The window opened: load the workspace and follow events.
    Launch,
    /// The workspace loaded, or didn't.
    Loaded(Result<Workspace, DaemonError>),
    /// Something changed on a daemon.
    Event(Event),
    /// The event stream ended.
    EventsEnded(StreamEnd<DaemonError>),
    /// Try loading again after a failure.
    Retry,
    /// Show a screen; the current one goes on the trail.
    Navigate(Route),
    /// Lateral navigation (sidebar, breadcrumb or shortcut): clear Back.
    Switch(Route),
    /// ⌘[: back to the previous screen.
    Back,
    /// Escape: close the top overlay or dialog.
    Escape,
    /// ⌘K.
    OpenPalette,
    /// ⌘N.
    StartNewSession,
    /// ⇧⌘N.
    StartNewChannel,
    /// ⌘/.
    ShowShortcuts,
    /// The New session sheet changed.
    NewSession(NewSessionAction),
    /// The New channel sheet changed.
    NewChannel(NewChannelAction),
    /// The daemon started a session, or didn't.
    SessionCreated(Result<SessionId, DaemonError>),
    /// The daemon made a channel, or didn't.
    ChannelCreated(Result<ChannelId, DaemonError>),
    /// A pane changed size.
    Pane(PaneAction),
    /// Expand this session area, or return it to the saved columns.
    ToggleFullScreen(FullScreen),
    /// Open/close navigation over a narrow window without changing pane sizes.
    ToggleNavigationOverlay,
    /// Navigation or a viewport change dismisses the temporary overlay.
    CloseNavigationOverlay,
    /// A toast's close button was clicked.
    DismissToast(u64),
    /// A toast's time ran out.
    ToastExpired(u64),
    /// The saved-state error was read.
    DismissRestoreError,
    /// Drafts changed a second ago: save them.
    SaveDue,
    /// A save finished.
    Saved(Option<String>),
    /// The sidebar.
    Sidebar(SidebarAction),
    /// A session page.
    Session(SessionAction),
    /// The review panel.
    Review(ReviewAction),
    /// The Inbox.
    Inbox(InboxAction),
    /// A board.
    Board(BoardAction),
    /// Hosts.
    Hosts(HostsAction),
    /// Files.
    Files(FilesAction),
    /// Settings.
    Settings(SettingsAction),
    /// The palette.
    Palette(PaletteAction),
    /// Prepared views.
    Views(ViewsAction),
}

/// The app's reducer.
#[derive(Clone)]
pub struct App {
    clients: Clients,
    saver: Saver,
    sessions: Sessions,
    review: Review,
    inbox: Inbox,
    board: Board,
    hosts: Hosts,
    files: Files,
    settings: Settings,
}

impl App {
    /// The app on `clients`.
    #[must_use]
    pub fn new(clients: Clients) -> Self {
        Self {
            saver: Saver::new(clients.storage.clone()),
            sessions: Sessions::new(clients.clone()),
            review: Review::new(clients.clone()),
            inbox: Inbox::new(clients.clone()),
            board: Board::new(clients.clone()),
            hosts: Hosts::new(clients.clone()),
            files: Files::new(clients.clone()),
            settings: Settings::new(clients.clone()),
            clients,
        }
    }

    fn load(&self) -> Effect<AppAction> {
        let daemon = self.clients.daemon.clone();
        Effect::future(async move { AppAction::Loaded(daemon.load().await) })
            .cancellable("load", true)
    }

    fn follow(&self) -> Effect<AppAction> {
        let daemon = self.clients.daemon.clone();
        Effect::stream(daemon.events(), AppAction::Event, AppAction::EventsEnded)
            .cancellable("events", true)
    }

    /// Hands the new workspace to every feature.
    fn show(&self, state: &mut AppState) -> Effect<AppAction> {
        let workspace = &state.workspace;
        Effect::merge([
            self.child(
                state,
                AppAction::Sidebar(SidebarAction::Show(workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Session(SessionAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Review(ReviewAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Inbox(InboxAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Board(BoardAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Hosts(HostsAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Files(FilesAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Palette(PaletteAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Views(ViewsAction::Show(state.workspace.clone())),
            ),
        ])
    }

    /// Runs a feature's reducer on its part of the state.
    fn child(&self, state: &mut AppState, action: AppAction) -> Effect<AppAction> {
        match action {
            AppAction::Sidebar(action) => Sidebar
                .reduce(&mut state.sidebar, action)
                .map(AppAction::Sidebar),
            AppAction::Session(action) => self
                .sessions
                .reduce(&mut state.session, action)
                .map(AppAction::Session),
            AppAction::Review(action) => self
                .review
                .reduce(&mut state.review, action)
                .map(AppAction::Review),
            AppAction::Inbox(action) => self
                .inbox
                .reduce(&mut state.inbox, action)
                .map(AppAction::Inbox),
            AppAction::Board(action) => self
                .board
                .reduce(&mut state.board, action)
                .map(AppAction::Board),
            AppAction::Hosts(action) => self
                .hosts
                .reduce(&mut state.hosts, action)
                .map(AppAction::Hosts),
            AppAction::Files(action) => self
                .files
                .reduce(&mut state.files, action)
                .map(AppAction::Files),
            AppAction::Settings(action) => self
                .settings
                .reduce(&mut state.settings, action)
                .map(AppAction::Settings),
            AppAction::Palette(action) => Palette
                .reduce(&mut state.palette, action)
                .map(AppAction::Palette),
            AppAction::Views(action) => {
                Views.reduce(&mut state.views, action).map(AppAction::Views)
            }
            _ => Effect::none(),
        }
    }

    fn navigate(&self, state: &mut AppState, route: Route) -> Effect<AppAction> {
        if state.route != route {
            let previous = std::mem::replace(&mut state.route, route);
            state.trail.push(previous);
            if state.trail.len() > TRAIL {
                state.trail.remove(0);
            }
        }
        self.arrive(state)
    }

    fn switch(&self, state: &mut AppState, route: Route) -> Effect<AppAction> {
        state.route = route;
        state.navigation_overlay = false;
        state.trail.clear();
        self.arrive(state)
    }

    /// Tells the features what the route now shows.
    fn arrive(&self, state: &mut AppState) -> Effect<AppAction> {
        let current = match &state.route {
            Route::Session(id) => Some(RowKey::Session(id.clone())),
            Route::Channel(id) => Some(RowKey::Channel(id.clone())),
            _ => None,
        };
        let mut effects = vec![self.child(
            state,
            AppAction::Sidebar(SidebarAction::SetCurrent(current)),
        )];
        match state.route.clone() {
            Route::Session(id) => {
                effects.push(self.child(state, AppAction::Views(ViewsAction::Return(id.clone()))));
                effects
                    .push(self.child(state, AppAction::Session(SessionAction::Open(id.clone()))));
                effects.push(self.child(state, AppAction::Review(ReviewAction::Open(id))));
            }
            Route::Inbox => {
                state.session.open = None;
                effects.push(self.child(
                    state,
                    AppAction::Inbox(InboxAction::SetFilter(humata2_inbox::Filter::All)),
                ));
            }
            Route::Board(scope) => {
                effects.push(self.child(state, AppAction::Board(BoardAction::SetScope(scope))));
            }
            _ => state.session.open = None,
        }
        Effect::merge(effects)
    }

    fn run(&self, state: &mut AppState, command: Command) -> Effect<AppAction> {
        match command {
            Command::Home => self.navigate(state, Route::Home),
            Command::Inbox => self.navigate(state, Route::Inbox),
            Command::Board => self.navigate(state, Route::Board(Scope::Global)),
            Command::Hosts => self.navigate(state, Route::Hosts),
            Command::Files => self.navigate(state, Route::Files),
            Command::Settings => self.navigate(state, Route::Settings),
            Command::NewSession => self.reduce(state, AppAction::StartNewSession),
            Command::NewChannel => self.reduce(state, AppAction::StartNewChannel),
            Command::ExpandAll => self.child(state, AppAction::Sidebar(SidebarAction::ExpandAll)),
            Command::CollapseAll => {
                self.child(state, AppAction::Sidebar(SidebarAction::CollapseAll))
            }
            Command::ToggleSidebar => {
                state.layout.toggle(PaneId::Sidebar);
                Effect::none()
            }
            Command::Fork => self.child(
                state,
                AppAction::Session(SessionAction::StartFork(
                    humata2_clients::model::ForkKind::Worktree,
                )),
            ),
            Command::Archive => self.child(state, AppAction::Session(SessionAction::Archive)),
            Command::Channel(id) => self.navigate(state, Route::Channel(id)),
            Command::Session(id) => self.navigate(state, Route::Session(id)),
            Command::Pull(number) => {
                let found = state
                    .workspace
                    .pulls
                    .iter()
                    .find(|pull| pull.number == number);
                match found.and_then(|pull| pull.session.clone()) {
                    Some(session) => self.navigate(state, Route::Session(session)),
                    None => {
                        let clock = self.clients.clock.clone();
                        state.toast(
                            format!(
                                "No session here opened pull request #{number}. Open it on GitHub, or start a session to review it."
                            ),
                            &clock,
                        )
                    }
                }
            }
            Command::TogglePanel => {
                state.layout.toggle(PaneId::Panel);
                Effect::none()
            }
            Command::Record => {
                let clock = self.clients.clock.clone();
                state.toast(
                    "Recording starts from the desktop app's --record flag for now.".to_owned(),
                    &clock,
                )
            }
        }
    }

    /// Handles what a feature asked for.
    fn delegate(&self, state: &mut AppState, action: &AppAction) -> Effect<AppAction> {
        use humata2_board::Delegate as B;
        use humata2_files::Delegate as F;
        use humata2_hosts::Delegate as H;
        use humata2_inbox::Delegate as I;
        use humata2_palette::Delegate as P;
        use humata2_review::Delegate as R;
        use humata2_session::Delegate as S;
        use humata2_sidebar::Delegate as D;
        let clock = self.clients.clock.clone();
        match action {
            AppAction::Sidebar(SidebarAction::Delegate(delegate)) => match delegate {
                D::OpenChannel(id) => self.switch(state, Route::Channel(id.clone())),
                D::OpenSession(id) => self.switch(state, Route::Session(id.clone())),
            },
            AppAction::Session(SessionAction::Delegate(delegate)) => match delegate {
                S::OpenSession(id) => self.navigate(state, Route::Session(id.clone())),
                S::Toast(text) => state.toast(text.clone(), &clock),
                S::Left => {
                    let back = state
                        .trail
                        .iter()
                        .rev()
                        .find(|route| !matches!(route, Route::Session(_)))
                        .cloned()
                        .unwrap_or_default();
                    self.navigate(state, back)
                }
            },
            AppAction::Inbox(InboxAction::Delegate(delegate)) => match delegate {
                I::OpenSession(id) => self.navigate(state, Route::Session(id.clone())),
                I::Toast(text) => state.toast(text.clone(), &clock),
            },
            AppAction::Board(BoardAction::Delegate(delegate)) => match delegate {
                B::OpenSession(id) => self.navigate(state, Route::Session(id.clone())),
                B::Toast(text) => state.toast(text.clone(), &clock),
            },
            AppAction::Hosts(HostsAction::Delegate(delegate)) => match delegate {
                H::OpenSession(id) => self.navigate(state, Route::Session(id.clone())),
                H::Toast(text) => state.toast(text.clone(), &clock),
            },
            AppAction::Review(ReviewAction::Delegate(R::Toast(text)))
            | AppAction::Files(FilesAction::Delegate(F::Toast(text))) => {
                state.toast(text.clone(), &clock)
            }
            AppAction::Palette(PaletteAction::Delegate(delegate)) => match delegate {
                P::Run(command) => self.run(state, command.clone()),
                P::Closed => {
                    state
                        .overlays
                        .retain(|overlay| overlay != &Overlay::Palette);
                    Effect::none()
                }
            },
            _ => Effect::none(),
        }
    }

    fn apply(&self, state: &mut AppState, event: Event) -> Effect<AppAction> {
        let workspace = Arc::make_mut(&mut state.workspace);
        let mut snapshot = None;
        match event {
            Event::Output { session, tab, line } => {
                return self.child(
                    state,
                    AppAction::Session(SessionAction::Output { session, tab, line }),
                );
            }
            Event::Session(session) => {
                snapshot = Some(session.id.clone());
                upsert(&mut workspace.sessions, session, |a, b| a.id == b.id);
            }
            Event::Request(request) => {
                upsert(&mut workspace.requests, request, |a, b| a.id == b.id);
            }
            Event::Machine(machine) => {
                upsert(&mut workspace.machines, machine, |a, b| a.id == b.id);
            }
            Event::Transfer(transfer) => {
                upsert(&mut workspace.transfers, transfer, |a, b| a.id == b.id);
            }
            Event::Resync => return self.load(),
        }
        let mut effects = vec![self.show(state)];
        if let Some(id) = snapshot {
            effects.push(self.child(state, AppAction::Session(SessionAction::Snapshot(id))));
        }
        Effect::merge(effects)
    }

    /// Saves where the person is if it changed: at once for a structural
    /// change, within a second for drafts.
    fn persist(&self, state: &mut AppState) -> Effect<AppAction> {
        if state.load != Load::Ready {
            return Effect::none();
        }
        let now = state.saved();
        if now == state.last_saved {
            return Effect::none();
        }
        if now.structure() == state.last_saved.structure() {
            if state.save_due {
                return Effect::none();
            }
            state.save_due = true;
            let sleep = self.clients.clock.sleep(DRAFT_SAVE);
            return Effect::future(async move {
                sleep.await;
                AppAction::SaveDue
            });
        }
        self.save(state, now)
    }

    fn save(&self, state: &mut AppState, now: SavedUi) -> Effect<AppAction> {
        state.save_generation += 1;
        let generation = state.save_generation;
        state.last_saved = now.clone();
        let saver = self.saver.clone();
        Effect::future(async move {
            AppAction::Saved(
                saver
                    .save(generation, &now)
                    .err()
                    .map(|error| error.to_string()),
            )
        })
    }
}

fn upsert<T>(list: &mut Vec<T>, value: T, same: impl Fn(&T, &T) -> bool) {
    match list.iter_mut().find(|found| same(found, &value)) {
        Some(found) => *found = value,
        None => list.push(value),
    }
}

impl Reducer for App {
    type State = AppState;
    type Action = AppAction;

    fn reduce(&self, state: &mut AppState, action: AppAction) -> Effect<AppAction> {
        let effect = self.handle(state, action);
        let save = self.persist(state);
        effect.and(save)
    }
}

impl App {
    #[allow(clippy::too_many_lines)]
    fn handle(&self, state: &mut AppState, action: AppAction) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match action {
            AppAction::Launch => Effect::merge([self.load(), self.follow()]),
            AppAction::Loaded(Ok(workspace)) => {
                state.load = Load::Ready;
                state.workspace = Arc::new(workspace);
                let mut effects = vec![self.show(state)];
                let gone = match &state.route {
                    Route::Session(id) => state.workspace.session(id).is_none_or(|s| s.archived),
                    Route::Channel(id) => state.workspace.channel(id).is_none(),
                    _ => false,
                };
                if gone {
                    state.route = Route::Home;
                    effects.push(state.toast(
                        "What you had open last time is gone, so Humata opened Home.".to_owned(),
                        &clock,
                    ));
                }
                effects.push(self.arrive(state));
                Effect::merge(effects)
            }
            AppAction::Loaded(Err(error)) => {
                state.load = Load::Failed(error.to_string());
                Effect::none()
            }
            AppAction::Event(event) => self.apply(state, event),
            AppAction::EventsEnded(end) => {
                let text = match end {
                    StreamEnd::Finished => {
                        "The daemon stopped sending updates. Reconnecting.".to_owned()
                    }
                    StreamEnd::Failed(error) => {
                        format!("Lost updates from the daemon: {error}. Reconnecting.")
                    }
                };
                Effect::merge([state.toast(text, &clock), self.load(), self.follow()])
            }
            AppAction::Retry => {
                state.load = Load::Loading;
                Effect::merge([self.load(), self.follow()])
            }
            AppAction::Navigate(route) => self.navigate(state, route),
            AppAction::Switch(route) => self.switch(state, route),
            AppAction::Back => match state.trail.pop() {
                Some(route) => {
                    state.route = route;
                    self.arrive(state)
                }
                None => Effect::none(),
            },
            AppAction::Escape => {
                if let Some(top) = state.overlays.pop() {
                    if top == Overlay::Palette {
                        state.palette.open = false;
                    }
                    return Effect::none();
                }
                if state.navigation_overlay {
                    state.navigation_overlay = false;
                    return Effect::none();
                }
                if state.session.terminal_menu.is_some() {
                    return self
                        .child(state, AppAction::Session(SessionAction::TerminalMenu(None)));
                }
                if state.session.fork.is_some() {
                    return self.child(state, AppAction::Session(SessionAction::CancelFork));
                }
                if state.inbox.confirm.is_some() {
                    return self.child(state, AppAction::Inbox(InboxAction::CancelBulk));
                }
                if state.inbox.reply.is_some() {
                    return self.child(state, AppAction::Inbox(InboxAction::CancelReply));
                }
                if state.board.new_title.is_some() {
                    return self.child(state, AppAction::Board(BoardAction::CancelNew));
                }
                if state.board.drawer.is_some() && matches!(state.route, Route::Board(_)) {
                    return self.child(state, AppAction::Board(BoardAction::Close));
                }
                if state.review.draft().is_some() && state.panel() == Some(Panel::Changes) {
                    return self.child(state, AppAction::Review(ReviewAction::CancelComment));
                }
                Effect::none()
            }
            AppAction::OpenPalette => {
                if state.overlay() == Some(&Overlay::Palette) {
                    return Effect::none();
                }
                state.overlays.push(Overlay::Palette);
                let in_session = matches!(state.route, Route::Session(_));
                self.child(
                    state,
                    AppAction::Palette(PaletteAction::Open { in_session }),
                )
            }
            AppAction::StartNewSession => {
                let channel = match &state.route {
                    Route::Channel(id) => Some(id.clone()),
                    Route::Session(id) => state.workspace.session(id).map(|s| s.channel.clone()),
                    _ => state.workspace.channels.first().map(|c| c.id.clone()),
                };
                let machine = state
                    .workspace
                    .machines
                    .iter()
                    .find(|machine| machine.online)
                    .map(|machine| machine.id.clone());
                state.overlays.push(Overlay::NewSession(NewSession {
                    channel,
                    agent: state.settings.preferences.default_agent,
                    machine,
                    title: String::new(),
                    creating: false,
                    error: None,
                }));
                Effect::none()
            }
            AppAction::StartNewChannel => {
                state
                    .overlays
                    .push(Overlay::NewChannel(NewChannel::default()));
                Effect::none()
            }
            AppAction::ShowShortcuts => {
                if state.overlay() != Some(&Overlay::Shortcuts) {
                    state.overlays.push(Overlay::Shortcuts);
                }
                Effect::none()
            }
            AppAction::NewSession(change) => {
                let Some(Overlay::NewSession(sheet)) = state.overlays.last_mut() else {
                    return Effect::none();
                };
                match change {
                    NewSessionAction::Channel(id) => sheet.channel = Some(id),
                    NewSessionAction::Agent(agent) => sheet.agent = agent,
                    NewSessionAction::Machine(id) => sheet.machine = Some(id),
                    NewSessionAction::Title(title) => sheet.title = title,
                    NewSessionAction::Create => {
                        let (Some(channel), Some(machine)) =
                            (sheet.channel.clone(), sheet.machine.clone())
                        else {
                            sheet.error = Some(
                                "Pick a channel and a host first. Add a host on the Hosts page if none is listed.".to_owned(),
                            );
                            return Effect::none();
                        };
                        let title = if sheet.title.trim().is_empty() {
                            format!("New {} session", sheet.agent.name())
                        } else {
                            sheet.title.trim().to_owned()
                        };
                        sheet.creating = true;
                        sheet.error = None;
                        let agent = sheet.agent;
                        let daemon = self.clients.daemon.clone();
                        return Effect::future(async move {
                            AppAction::SessionCreated(
                                daemon
                                    .create_session(&channel, agent, &machine, &title)
                                    .await,
                            )
                        });
                    }
                }
                Effect::none()
            }
            AppAction::NewChannel(change) => {
                let Some(Overlay::NewChannel(sheet)) = state.overlays.last_mut() else {
                    return Effect::none();
                };
                match change {
                    NewChannelAction::Name(name) => sheet.name = name,
                    NewChannelAction::Repos(repos) => sheet.repos = repos,
                    NewChannelAction::Create => {
                        let name = sheet.name.trim().trim_start_matches('#').to_owned();
                        if name.is_empty() {
                            sheet.error = Some("Give the channel a name.".to_owned());
                            return Effect::none();
                        }
                        let repos = sheet
                            .repos
                            .split(',')
                            .map(str::trim)
                            .filter(|repo| !repo.is_empty())
                            .map(str::to_owned)
                            .collect();
                        sheet.creating = true;
                        sheet.error = None;
                        let machines = state
                            .workspace
                            .machines
                            .iter()
                            .filter(|machine| machine.online)
                            .map(|machine| machine.id.clone())
                            .take(1)
                            .collect();
                        let daemon = self.clients.daemon.clone();
                        return Effect::future(async move {
                            AppAction::ChannelCreated(
                                daemon.create_channel(&name, repos, machines).await,
                            )
                        });
                    }
                }
                Effect::none()
            }
            AppAction::SessionCreated(result) => {
                let sheet = state
                    .overlays
                    .iter_mut()
                    .rev()
                    .find_map(|overlay| match overlay {
                        Overlay::NewSession(sheet) => Some(sheet),
                        _ => None,
                    });
                match result {
                    Ok(id) => {
                        state
                            .overlays
                            .retain(|overlay| !matches!(overlay, Overlay::NewSession(_)));
                        self.navigate(state, Route::Session(id))
                    }
                    Err(error) => {
                        if let Some(sheet) = sheet {
                            sheet.creating = false;
                            sheet.error = Some(error.to_string());
                        }
                        Effect::none()
                    }
                }
            }
            AppAction::ChannelCreated(result) => {
                let sheet = state
                    .overlays
                    .iter_mut()
                    .rev()
                    .find_map(|overlay| match overlay {
                        Overlay::NewChannel(sheet) => Some(sheet),
                        _ => None,
                    });
                match result {
                    Ok(id) => {
                        state
                            .overlays
                            .retain(|overlay| !matches!(overlay, Overlay::NewChannel(_)));
                        self.navigate(state, Route::Channel(id))
                    }
                    Err(error) => {
                        if let Some(sheet) = sheet {
                            sheet.creating = false;
                            sheet.error = Some(error.to_string());
                        }
                        Effect::none()
                    }
                }
            }
            AppAction::ToggleNavigationOverlay => {
                state.navigation_overlay = !state.navigation_overlay;
                Effect::none()
            }
            AppAction::CloseNavigationOverlay => {
                state.navigation_overlay = false;
                Effect::none()
            }
            AppAction::ToggleFullScreen(side) => {
                if let Route::Session(id) = &state.route {
                    if side == FullScreen::Panel && state.panel().is_none() {
                        return Effect::none();
                    }
                    let next = if state.full_screen() == Some(side) {
                        None
                    } else {
                        Some(side)
                    };
                    if let Some(next) = next {
                        state.full_screens.insert(id.clone(), next);
                    } else {
                        state.full_screens.remove(id);
                    }
                    if side == FullScreen::Panel {
                        state.layout.panel.collapsed = false;
                    }
                    state.session.views.entry(id.clone()).or_default().widened =
                        next == Some(FullScreen::Panel);
                    state.navigation_overlay = false;
                }
                Effect::none()
            }
            AppAction::Session(SessionAction::ToggleWiden) => {
                self.handle(state, AppAction::ToggleFullScreen(FullScreen::Panel))
            }
            AppAction::Pane(change) => {
                if change == PaneAction::Toggle(PaneId::Panel)
                    && state.panel().is_none()
                    && matches!(state.route, Route::Session(_))
                {
                    return self.handle(
                        state,
                        AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
                    );
                }
                match change {
                    PaneAction::Drag(pane, width) => state.layout.drag(pane, width),
                    PaneAction::Toggle(PaneId::Panel)
                        if state.full_screen() == Some(FullScreen::Terminal) =>
                    {
                        if let Some(id) = &state.session.open {
                            state.full_screens.remove(id);
                        }
                        state.layout.panel.collapsed = false;
                    }
                    PaneAction::Toggle(pane) => state.layout.toggle(pane),
                    PaneAction::Reset(pane) => state.layout.reset(pane),
                }
                Effect::none()
            }
            AppAction::DismissToast(id) => {
                state.toasts.retain(|toast| toast.id != id);
                Effect::cancel(format!("toast-{id}"))
            }
            AppAction::ToastExpired(id) => {
                state.toasts.retain(|toast| toast.id != id);
                Effect::none()
            }
            AppAction::DismissRestoreError => {
                state.restore_error = None;
                Effect::none()
            }
            AppAction::SaveDue => {
                state.save_due = false;
                let now = state.saved();
                if now == state.last_saved {
                    return Effect::none();
                }
                self.save(state, now)
            }
            AppAction::Saved(error) => match error {
                Some(error) => state.toast(error, &clock),
                None => Effect::none(),
            },
            AppAction::Settings(SettingsAction::ToggleWrap) => {
                let effect = self.child(state, AppAction::Settings(SettingsAction::ToggleWrap));
                state.review.wrap = state.settings.preferences.wrap_diffs;
                effect
            }
            AppAction::Session(SessionAction::SetPanel(None)) => {
                if let Some(id) = &state.session.open {
                    if state.full_screens.get(id) == Some(&FullScreen::Panel) {
                        state.full_screens.remove(id);
                    }
                }
                self.child(state, AppAction::Session(SessionAction::SetPanel(None)))
            }
            AppAction::Session(SessionAction::SetPanel(Some(panel))) => {
                if let Some(id) = &state.session.open {
                    if state.full_screens.get(id) == Some(&FullScreen::Terminal) {
                        state.full_screens.remove(id);
                    }
                }
                let effect = self.child(
                    state,
                    AppAction::Session(SessionAction::SetPanel(Some(panel))),
                );
                if state.layout.panel.collapsed {
                    state.layout.panel.collapsed = false;
                }
                match (panel, state.session.open.clone()) {
                    (Panel::Changes, Some(id)) => {
                        effect.and(self.child(state, AppAction::Review(ReviewAction::Open(id))))
                    }
                    _ => effect,
                }
            }
            AppAction::Views(action @ (ViewsAction::Accept(_) | ViewsAction::Open(_))) => {
                let (ViewsAction::Accept(title) | ViewsAction::Open(title)) = &action else {
                    return Effect::none();
                };
                let title = title.clone();
                let session = state
                    .workspace
                    .views
                    .iter()
                    .find(|v| v.title == title)
                    .map(|v| v.session.clone());
                let effect = self.child(state, AppAction::Views(action));
                if let Some(id) = session {
                    let arrive = self.navigate(state, Route::Session(id));
                    effect
                        .and(arrive)
                        .and(self.child(state, AppAction::Session(SessionAction::OpenView(title))))
                } else {
                    effect
                }
            }
            action @ (AppAction::Sidebar(_)
            | AppAction::Session(_)
            | AppAction::Review(_)
            | AppAction::Inbox(_)
            | AppAction::Board(_)
            | AppAction::Hosts(_)
            | AppAction::Files(_)
            | AppAction::Settings(_)
            | AppAction::Palette(_)
            | AppAction::Views(_)) => {
                let delegated = self.delegate(state, &action);
                let effect = self.child(state, action);
                effect.and(delegated)
            }
        }
    }
}
