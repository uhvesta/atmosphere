//! Humata 2's root feature: which screen shows and Back (NV-1–5), the
//! overlays on top (palette, sheets, dialogs) with Escape closing the top
//! one, pane sizes, toasts, loading the workspace and following the
//! daemon's event stream, and saving where the person was so a restart
//! puts them back (PER-25, PER-26).
//!
//! Each screen is its own feature crate; this one holds their states, sends
//! them the data as it changes, and handles what they delegate.

pub mod channel;
pub mod layout;
pub mod saved;

mod commands;
mod desk;
mod events;
mod messages;
mod new_channel;
mod new_session;
mod overlay;
mod persist;
mod restart;
mod report;
mod route;
mod toast;

pub use new_channel::{NewChannel, NewChannelAction};
pub use new_session::{NewSession, NewSessionAction, NewSessionPicker};
pub use overlay::Overlay;
pub use report::{PinTarget, PinnedComment, ReportAction, ReportForm, ReportKind, ReportState};
pub use restart::{RestartAction, RestartGroups, restart_groups};
pub use route::Route;
pub use toast::{Toast, ToastAction};

use std::sync::Arc;
use std::time::Duration;

use humata2_board::{Board, BoardAction, BoardState};
use humata2_clients::model::{ChannelId, ItemId, Scope, SessionId};
use humata2_clients::{Clients, DaemonError, Event, Workspace};
use humata2_desk::{Desk, DeskAction, DeskState};
use humata2_files::{Files, FilesAction, FilesState};
use humata2_hosts::{Hosts, HostsAction, HostsState};
use humata2_inbox::{Inbox, InboxAction, InboxState};
use humata2_palette::{Palette, PaletteAction, PaletteState};
use humata2_review::{Review, ReviewAction, ReviewState};
use humata2_session::{Panel, SessionAction, SessionState, Sessions};
use humata2_settings::{Settings, SettingsAction, SettingsState};
use humata2_sidebar::{Sidebar, SidebarAction, SidebarState};
use humata2_views::{Views, ViewsAction, ViewsState};
use tca::{Effect, Reducer, StreamEnd};

use crate::layout::{FullScreen, Layout, PaneId};
use crate::saved::{SavedUi, Saver};

/// How long a toast shows.
pub const TOAST: Duration = Duration::from_secs(4);

/// Drafts save at most this often while they change.
pub const DRAFT_SAVE: Duration = Duration::from_secs(1);

/// How many screens Back remembers.
pub const TRAIL: usize = 50;

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
    /// Each channel's page tab, filters and folding, saved independently.
    pub channel_pages: std::collections::BTreeMap<ChannelId, channel::View>,
    /// Toasts, oldest first.
    pub toasts: Vec<Toast>,
    /// The bug report form and recording.
    pub report: ReportState,
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
    /// The Desk.
    pub desk: DeskState,
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
            channel_pages: saved.channels.clone(),
            toasts: Vec::new(),
            report: ReportState::default(),
            restore_error,
            sidebar: SidebarState::default(),
            session: SessionState::default(),
            review: ReviewState::default(),
            inbox: InboxState::default(),
            board: BoardState {
                scope: match &saved.route {
                    Route::Board(scope) => scope.clone(),
                    Route::Channel(id) => Scope::Channel(id.clone()),
                    _ => Scope::Global,
                },
                view: saved.board.clone(),
                ..Default::default()
            },
            hosts: HostsState {
                selected: saved.selected_host.clone(),
                ..Default::default()
            },
            desk: desk::restored(saved.desk_sort),
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
            channels: self.channel_pages.clone(),
            selected_host: self.hosts.selected.clone(),
            board: self.board.view.clone(),
            folding: self.sidebar.folding.clone(),
            preferences: self.settings.preferences.clone(),
            review: self.review.saved.clone(),
            recent: self.palette.recent.clone(),
            views: self.views.decisions.clone(),
            connected: self.hosts.connected(),
            desk_sort: self.desk.sort,
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

/// Everything that happens in the app.
#[derive(Clone, Debug, PartialEq)]
pub enum AppAction {
    /// The window opened: load the workspace and follow events.
    Launch,
    /// The workspace loaded, or didn't.
    Loaded(Result<Workspace, DaemonError>),
    /// The bug report form and recording.
    Report(ReportAction),
    /// Undo the action offered by this toast.
    UndoToast(u64),
    /// A toast undo finished.
    Undone {
        toast: u64,
        result: Result<(), DaemonError>,
    },
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
    /// Change the current channel's tab, filters or tree folds.
    Channel(channel::Action),
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
    /// Start a session for this board item.
    StartNewForItem(ItemId),
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
    /// New desk session was clicked.
    StartNewDeskSession,
    /// The Desk.
    Desk(DeskAction),
    /// After a host restarts: what the person can do about what it found.
    Restart(RestartAction),
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
    desk: Desk,
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
            review: Review,
            inbox: Inbox::new(clients.clone()),
            board: Board::new(clients.clone()),
            desk: Desk::new(clients.desk.clone()),
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
            AppAction::Desk(action) => self
                .desk
                .reduce(&mut state.desk, action)
                .map(AppAction::Desk),
            _ => Effect::none(),
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
                S::CommentsSent(_, count) => state.toast(
                    format!(
                        "Sent {count} comment{} to the agent.",
                        if *count == 1 { "" } else { "s" }
                    ),
                    &clock,
                ),
                S::CommentsUnqueued(id, comments) => self.child(
                    state,
                    AppAction::Review(ReviewAction::Unqueued(id.clone(), comments.clone())),
                ),
                S::Archived(id, title) => {
                    let notice = state.toast(format!("Archived “{title}”"), &clock);
                    if let Some(toast) = state.toasts.last_mut() {
                        toast.action = Some(ToastAction::RestoreSession(id.clone()));
                    }
                    let leave = if state.route == Route::Session(id.clone()) {
                        self.navigate(state, Route::Home)
                    } else {
                        Effect::none()
                    };
                    Effect::merge([notice, leave])
                }
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
                I::OpenRequest { session, request } => {
                    let go = self.navigate(state, Route::Session(session.clone()));
                    state.inbox.selected = Some(request.clone());
                    let panel = self.handle(
                        state,
                        AppAction::Session(SessionAction::SetPanel(Some(Panel::Inbox))),
                    );
                    Effect::merge([go, panel])
                }
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
            AppAction::Review(ReviewAction::Delegate(R::QueueComments(id, comments))) => {
                self.queue_comments(state, id.clone(), comments.clone())
            }
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
            AppAction::Desk(action) => self.desk_delegate(state, action),
            _ => Effect::none(),
        }
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
            AppAction::StartNewDeskSession => self.start_new_desk_session(state),
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
            AppAction::EventsEnded(end) => self.events_ended(state, end),
            AppAction::Report(action) => self.report(state, action),
            AppAction::UndoToast(id) => self.undo_toast(state, id),
            AppAction::Undone { toast, result } => self.toast_undone(state, toast, result),
            AppAction::Retry => {
                state.load = Load::Loading;
                Effect::merge([self.load(), self.follow()])
            }
            AppAction::Navigate(route) => self.navigate(state, route),
            AppAction::Switch(route) => self.switch(state, route),
            AppAction::Back => self.go_back(state),
            AppAction::Escape => self.escape(state),
            AppAction::OpenPalette => self.open_palette(state),
            AppAction::StartNewSession => self.start_new_session(state),
            AppAction::StartNewForItem(id) => self.start_new_for_item(state, id),
            AppAction::StartNewChannel => self.start_new_channel(state),
            AppAction::ShowShortcuts => self.show_shortcuts(state),
            AppAction::NewSession(change) => self.change_new_session(state, change),
            AppAction::NewChannel(change) => self.change_new_channel(state, change),
            AppAction::SessionCreated(result) => self.session_created(state, result),
            AppAction::ChannelCreated(result) => self.channel_created(state, result),
            AppAction::ToggleNavigationOverlay => {
                state.navigation_overlay = !state.navigation_overlay;
                Effect::none()
            }
            AppAction::CloseNavigationOverlay => {
                state.navigation_overlay = false;
                Effect::none()
            }
            AppAction::Hosts(HostsAction::StartAdd) => {
                let navigate = self.switch(state, Route::Hosts);
                Effect::merge([
                    navigate,
                    self.child(state, AppAction::Hosts(HostsAction::StartAdd)),
                ])
            }
            AppAction::Hosts(HostsAction::Select(id)) => {
                let navigate = self.switch(state, Route::Hosts);
                Effect::merge([
                    navigate,
                    self.child(state, AppAction::Hosts(HostsAction::Select(id))),
                ])
            }
            AppAction::Board(BoardAction::SetScope(scope)) => {
                if matches!(state.route, Route::Board(_)) {
                    state.route = Route::Board(scope.clone());
                }
                self.child(state, AppAction::Board(BoardAction::SetScope(scope)))
            }
            AppAction::Board(BoardAction::GlobalOnly(only)) => {
                if matches!(state.route, Route::Board(_)) {
                    state.route = Route::Board(Scope::Global);
                }
                self.child(state, AppAction::Board(BoardAction::GlobalOnly(only)))
            }
            AppAction::Channel(action) => {
                let Route::Channel(id) = state.route.clone() else {
                    return Effect::none();
                };
                let view = state.channel_pages.entry(id.clone()).or_default();
                view.change(action, &state.workspace, &id);
                match view.tab {
                    channel::Tab::Inbox => self.child(
                        state,
                        AppAction::Inbox(InboxAction::SetFilter(humata2_inbox::Filter::Channel(
                            id,
                        ))),
                    ),
                    channel::Tab::Board => self.child(
                        state,
                        AppAction::Board(BoardAction::SetScope(Scope::Channel(id))),
                    ),
                    _ => Effect::none(),
                }
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
            AppAction::DismissToast(id) => self.dismiss_toast(state, id),
            AppAction::ToastExpired(id) => self.toast_expired(state, id),
            AppAction::DismissRestoreError => {
                state.restore_error = None;
                Effect::none()
            }
            AppAction::SaveDue => self.save_if_due(state),
            AppAction::Saved(error) => self.save_finished(state, error),
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
            AppAction::Restart(change) => self.change_restart(state, change),
            action @ (AppAction::Sidebar(_)
            | AppAction::Session(_)
            | AppAction::Review(_)
            | AppAction::Inbox(_)
            | AppAction::Board(_)
            | AppAction::Hosts(_)
            | AppAction::Files(_)
            | AppAction::Settings(_)
            | AppAction::Palette(_)
            | AppAction::Views(_)
            | AppAction::Desk(_)) => {
                let delegated = self.delegate(state, &action);
                let effect = self.child(state, action);
                effect.and(delegated)
            }
        }
    }
}
