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
// REPOS (CH-6): a channel's Add a GitHub repo sheet.
mod channel_add_repo;
mod desk;
mod events;
mod find;
mod home;
mod machines;
mod new_channel;
mod new_session;
mod overlay;
mod persist;
mod pr_review;
// REPOS (RL-5): the one repo search.
mod repo_search;
// REPOS (CH-21): Add repo to this session.
mod session_add_repo;
mod restart;
mod report;
mod route;
mod toast;
mod views;

pub use new_channel::{NewChannel, NewChannelAction};
// REPOS (RL-5)
pub use repo_search::{
    ChipView, PickRow, RepoChip, RepoGroup, RepoPicker, RepoSearchAction, RepoSearchState, used_line,
};
pub use new_session::{NewSession, NewSessionAction, NewSessionPicker};
// REPOS (CH-21)
pub use session_add_repo::{SessionAddRepo, SessionAddRepoAction};
pub use overlay::Overlay;
// REPOS (CH-6)
pub use channel_add_repo::{ChannelAddRepo, ChannelAddRepoAction};
pub use report::{
    PinTarget, PinnedComment, Point, Rect, ReportAction, ReportForm, ReportKind, ReportRequest,
    ReportState, Shape, Stroke,
};
pub use home::{NeedsRow, NeedsYou, needs_you};
pub use restart::{RestartAction, RestartGroups, restart_groups};
pub use route::Route;
pub use toast::{Toast, ToastAction};
pub use views::Snapshot;

use std::sync::Arc;
use std::time::Duration;

use humata2_board::{Board, BoardAction, BoardState};
use humata2_clients::model::{ChannelId, ItemId, MachineId, Scope, SessionId, ViewId};
use humata2_clients::{Clients, DaemonError, Event, Workspace};
use humata2_desk::{Desk, DeskAction, DeskState};
use humata2_files::{Files, FilesAction, FilesState};
use humata2_find::{Find, FindAction, FindState, PinnedAction, PinnedSection, PinnedState};
use humata2_hosts::{Hosts, HostsAction, HostsState};
use humata2_inbox::{Inbox, InboxAction, InboxState};
use humata2_palette::{Palette, PaletteAction, PaletteState};
use humata2_pr_review::ask::{Ask, AskState};
pub use humata2_pr_review::ask::AskAction;
use humata2_pr_review::dialog::{Dialog, DialogAction, DialogState};
use humata2_pr_review::follower::{Follower, FollowerAction, FollowerState};
use humata2_pr_review::heads::{Heads, HeadsAction, HeadsState};
use humata2_pr_review::info::{Info, InfoAction, InfoState};
use humata2_pr_review::submit::{Submit, SubmitAction, SubmitState};
use humata2_review::comments::{Comments, CommentsAction, CommentsDelegate, CommentsState};
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
    /// Whether each session's side panel is collapsed (NV-37, SE-13). The
    /// open session's value is also in `layout.panel.collapsed`, which the
    /// views read; arriving on a session swaps it in.
    pub panel_collapsed: std::collections::BTreeMap<SessionId, bool>,
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
    /// The session's review comments, as the daemon keeps them.
    pub comments: CommentsState,
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
    /// Search over every session's conversation (LD-23, LD-24).
    pub find: FindState,
    /// The sidebar's Pinned section (NV-20 to NV-25).
    pub pinned: PinnedState,
    /// Settings.
    pub settings: SettingsState,
    /// REPOS (RL-5): the Desk's Add repo search.
    pub desk_repos: crate::repo_search::RepoSearchState,
    /// REPOS (RL-5): the New desk session form's repo search.
    pub new_desk_repos: crate::repo_search::RepoSearchState,
    /// The palette.
    pub palette: PaletteState,
    /// Prepared views.
    pub views: ViewsState,
    /// Review a pull request: the start dialog.
    pub pr_dialog: DialogState,
    /// Review a pull request: Ask.
    pub pr_ask: AskState,
    /// Review a pull request: the chat follower.
    pub pr_follower: FollowerState,
    /// Review a pull request: Submit review.
    pub pr_submit: SubmitState,
    /// Review a pull request: new commits.
    pub pr_heads: HeadsState,
    /// Review a pull request: the Info panel's switches.
    pub pr_info: InfoState,
    /// What the person had when each view opened, newest last: the view
    /// showing is the last. Revert puts back the last and shows the one
    /// under it; Keep and Close drop it.
    pub view_snapshots: Vec<(ViewId, Snapshot)>,
    /// The file tab each walkthrough opened for its step.
    pub view_files: std::collections::BTreeMap<ViewId, humata2_session::Tab>,
    /// The step each walkthrough is on; a view not here is on its first.
    pub view_step: std::collections::BTreeMap<ViewId, usize>,
    /// The Add machine dialog (frames 56 to 58).
    pub add_machine: humata2_hosts::AddMachineState,
    /// A machine's page (frame 59).
    pub machine_page: humata2_hosts::MachinePageState,
    next_toast: u64,
    /// Sessions whose file list was asked for, once, when the palette first
    /// opened with them; events keep those lists current.
    files_asked: std::collections::BTreeSet<SessionId>,
    last_saved: SavedUi,
    save_generation: u64,
    save_due: bool,
}

impl AppState {
    /// The daemon version the app has for `machine`: what the hosts page
    /// last read from it, else what the workspace reports. `None` when
    /// neither knows it.
    #[must_use]
    pub fn daemon_version(&self, machine: &MachineId) -> Option<String> {
        let read = self
            .hosts
            .health
            .get(machine)
            .map(|detail| detail.daemon_version.as_str());
        let reported = self
            .workspace
            .machine_health
            .get(machine)
            .map(|health| health.daemon.as_str());
        read.into_iter()
            .chain(reported)
            .find(|version| !version.is_empty())
            .map(str::to_owned)
    }

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
            panel_collapsed: std::collections::BTreeMap::new(),
            navigation_overlay: false,
            channel_pages: saved.channels.clone(),
            toasts: Vec::new(),
            report: ReportState::default(),
            restore_error,
            sidebar: SidebarState::default(),
            session: SessionState::default(),
            review: ReviewState::default(),
            comments: CommentsState::default(),
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
            find: FindState::default(),
            pinned: PinnedState::default(),
            settings: SettingsState::default(),
            desk_repos: crate::repo_search::RepoSearchState::default(),
            new_desk_repos: crate::repo_search::RepoSearchState::default(),
            palette: PaletteState::default(),
            views: ViewsState::default(),
            pr_dialog: DialogState::default(),
            pr_ask: AskState::default(),
            pr_follower: FollowerState::default(),
            pr_submit: SubmitState::default(),
            pr_heads: HeadsState::default(),
            pr_info: InfoState::default(),
            view_snapshots: Vec::new(),
            view_files: std::collections::BTreeMap::new(),
            view_step: std::collections::BTreeMap::new(),
            add_machine: humata2_hosts::AddMachineState::default(),
            machine_page: humata2_hosts::MachinePageState::default(),
            next_toast: 0,
            files_asked: std::collections::BTreeSet::new(),
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
        // Settings › Show pinned (ST-18) decides whether the Pinned section shows.
        state.pinned.shown = state.settings.preferences.show_pinned;
        state.settings.has_key = clients
            .keychain
            .has(humata2_settings::KEY_NAME)
            .unwrap_or(false);
        state.review.load_saved(saved.review);
        state.review.wrap = state.settings.preferences.wrap_diffs;
        state.palette.recent = saved.recent;
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
            comments: self.comments.saved(),
            recent: self.palette.recent.clone(),
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
    /// A session's files arrived for the palette's Files group.
    SessionFiles {
        /// The session.
        session: SessionId,
        /// Its files, or why they could not be listed.
        result: Result<Vec<humata2_clients::model::SessionFile>, DaemonError>,
    },
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
    /// The channel's page on its Desk tab (CD-2): the sidebar's Desk row.
    OpenChannelDesk(ChannelId),
    /// The channel's page on its Agent access tab (CD-3): the "Agent access"
    /// link beside a switch its rule blocks.
    OpenChannelAccess(ChannelId),
    /// New desk session for this session (CD-6), from a session's menu: the
    /// channel's New desk session dialog with the session as its target.
    NewDeskSessionFor(SessionId),
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
    /// REPOS (CH-6): the channel's Add a GitHub repo sheet changed.
    ChannelAddRepo(ChannelAddRepoAction),
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
    /// A session's review comments.
    Comments(CommentsAction),
    /// The Inbox.
    Inbox(InboxAction),
    /// A board.
    Board(BoardAction),
    /// Hosts.
    Hosts(HostsAction),
    /// Files.
    Files(FilesAction),
    /// Search over conversations.
    Find(FindAction),
    /// The Pinned section.
    Pinned(PinnedAction),
    /// Settings.
    Settings(SettingsAction),
    /// REPOS (RL-5): a repo search, named by the picker it belongs to; the
    /// answer of a picker that closed is dropped.
    RepoSearch(crate::repo_search::RepoPicker, crate::repo_search::RepoSearchAction),
    /// REPOS (CH-21): the Add repo to this session sheet.
    SessionAddRepo(crate::session_add_repo::SessionAddRepoAction),
    /// The palette.
    Palette(PaletteAction),
    /// Prepared views.
    Views(ViewsAction),
    /// Review a pull request: the start dialog.
    PrDialog(DialogAction),
    /// Review a pull request: Ask.
    PrAsk(AskAction),
    /// Review a pull request: the chat follower.
    PrFollower(FollowerAction),
    /// Review a pull request: Submit review.
    PrSubmit(SubmitAction),
    /// Review a pull request: new commits.
    PrHeads(HeadsAction),
    /// Review a pull request: the Info panel's switches.
    PrInfo(InfoAction),
    /// New desk session was clicked.
    StartNewDeskSession,
    /// The Desk.
    Desk(DeskAction),
    /// After a host restarts: what the person can do about what it found.
    Restart(RestartAction),
    /// The Add machine dialog.
    AddMachine(humata2_hosts::AddMachineAction),
    /// A machine's page.
    MachinePage(humata2_hosts::MachinePageAction),
}

/// The app's reducer.
#[derive(Clone)]
pub struct App {
    clients: Clients,
    saver: Saver,
    sessions: Sessions,
    review: Review,
    comments: Comments,
    inbox: Inbox,
    board: Board,
    desk: Desk,
    hosts: Hosts,
    files: Files,
    find: Find,
    pinned: PinnedSection,
    settings: Settings,
    views: Views,
    pr_dialog: Dialog,
    pr_ask: Ask,
    pr_follower: Follower,
    pr_submit: Submit,
    pr_heads: Heads,
    pr_info: Info,
}

impl App {
    /// The app on `clients`.
    #[must_use]
    pub fn new(clients: Clients) -> Self {
        Self {
            saver: Saver::new(clients.storage.clone()),
            sessions: Sessions::new(clients.clone()),
            review: Review::new(clients.daemon.clone(), clients.clock.clone()),
            comments: Comments::new(clients.comments.clone(), clients.clock.clone()),
            inbox: Inbox::new(clients.clone()),
            board: Board::new(clients.clone()),
            desk: Desk::new(clients.desk.clone()),
            hosts: Hosts::new(clients.clone()),
            files: Files::new(clients.clone()),
            find: Find::new(clients.history.clone()),
            pinned: PinnedSection::new(clients.marks.clone()),
            settings: Settings::new(clients.clone()),
            views: Views::new(clients.views.clone()),
            pr_dialog: Dialog::new(clients.pull_requests.clone()),
            pr_ask: Ask::new(clients.daemon.clone(), clients.clock.clone()),
            pr_follower: Follower::new(clients.review_chat.clone(), clients.clock.clone()),
            pr_submit: Submit::new(clients.pull_requests.clone(), clients.comments.clone()),
            pr_heads: Heads::new(clients.pull_requests.clone()),
            pr_info: Info::new(clients.pull_requests.clone(), clients.review_chat.clone()),
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
            AppAction::Comments(action) => self
                .comments
                .reduce(&mut state.comments, action)
                .map(AppAction::Comments),
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
            AppAction::Find(action) => self
                .find
                .reduce(&mut state.find, action)
                .map(AppAction::Find),
            AppAction::Pinned(action) => self
                .pinned
                .reduce(&mut state.pinned, action)
                .map(AppAction::Pinned),
            AppAction::Settings(action) => self
                .settings
                .reduce(&mut state.settings, action)
                .map(AppAction::Settings),
            AppAction::Palette(action) => Palette
                .reduce(&mut state.palette, action)
                .map(AppAction::Palette),
            AppAction::Views(action) => self
                .views
                .reduce(&mut state.views, action)
                .map(AppAction::Views),
            AppAction::Desk(action) => self
                .desk
                .reduce(&mut state.desk, action)
                .map(AppAction::Desk),
            AppAction::PrDialog(action) => self
                .pr_dialog
                .reduce(&mut state.pr_dialog, action)
                .map(AppAction::PrDialog),
            AppAction::PrAsk(action) => self.pr_ask.reduce(&mut state.pr_ask, action).map(AppAction::PrAsk),
            AppAction::PrFollower(action) => self
                .pr_follower
                .reduce(&mut state.pr_follower, action)
                .map(AppAction::PrFollower),
            AppAction::PrSubmit(action) => self
                .pr_submit
                .reduce(&mut state.pr_submit, action)
                .map(AppAction::PrSubmit),
            AppAction::PrHeads(action) => self
                .pr_heads
                .reduce(&mut state.pr_heads, action)
                .map(AppAction::PrHeads),
            AppAction::PrInfo(action) => self
                .pr_info
                .reduce(&mut state.pr_info, action)
                .map(AppAction::PrInfo),
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
                D::OpenChannelDesk(id) => self.handle(state, AppAction::OpenChannelDesk(id.clone())),
            },
            AppAction::Session(SessionAction::Delegate(delegate)) => match delegate {
                S::Archived(id, title, frees) => {
                    // MV-15: the disk is freed when the undo window closes, not now.
                    let text = match frees {
                        Some(size) => {
                            format!("Archived “{title}”. Frees {size} when this closes.")
                        }
                        None => format!("Archived “{title}”."),
                    };
                    let notice = state.toast_for(text, &clock, toast::UNDO_TOAST);
                    if let Some(toast) = state.toasts.last_mut() {
                        toast.action = Some(ToastAction::RestoreSession(id.clone()));
                    }
                    let gone = Route::Session(id.clone());
                    let leave = if state.route == gone {
                        self.leave(state, &gone, Route::Home)
                    } else {
                        Effect::none()
                    };
                    Effect::merge([notice, leave])
                }
                S::OpenSession(id) => self.navigate(state, Route::Session(id.clone())),
                S::Toast(text) => state.toast(text.clone(), &clock),
                S::Left => {
                    // Back to the last page that isn't a session, as Back
                    // would go: what came after it leaves the trail.
                    let at = state
                        .trail
                        .iter()
                        .rposition(|route| !matches!(route, Route::Session(_)));
                    let back = at
                        .and_then(|at| state.trail.drain(at..).next())
                        .unwrap_or_default();
                    let gone = state.route.clone();
                    self.leave(state, &gone, back)
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
                I::OpenDestination(machine) => {
                    let go = self.navigate(state, Route::Files);
                    let side = state.files.active;
                    let folder = humata2_clients::Folder::home(machine.clone());
                    let show = self.child(state, AppAction::Files(FilesAction::Navigate(side, folder)));
                    Effect::merge([go, show])
                }
                I::Toast(text) => state.toast(text.clone(), &clock),
                // This Mac's daemon was replaced: load from the new one and
                // follow its events, as after any daemon restart.
                I::Reconnect => Effect::merge([self.load(), self.follow()]),
            },
            AppAction::Board(BoardAction::Delegate(delegate)) => match delegate {
                B::OpenSession(id) => self.navigate(state, Route::Session(id.clone())),
                B::Toast(text) => state.toast(text.clone(), &clock),
                B::Refresh => self.load(),
            },
            AppAction::Hosts(HostsAction::Delegate(delegate)) => match delegate {
                H::OpenSession(id) => self.navigate(state, Route::Session(id.clone())),
                H::Toast(text) => state.toast(text.clone(), &clock),
                H::RootChanged(machine, root) => {
                    let workspace = Arc::make_mut(&mut state.workspace);
                    if let Some(found) = workspace.machines.iter_mut().find(|found| &found.id == machine) {
                        found.root.clone_from(root);
                    }
                    self.show(state)
                }
            },
            AppAction::Review(ReviewAction::Delegate(R::Fetched(session, diffs))) => {
                let workspace = Arc::make_mut(&mut state.workspace);
                workspace.diffs.retain(|diff| &diff.session != session);
                workspace.diffs.extend(diffs.iter().cloned());
                // The session page opens changed files from these (§26.20).
                let shared = state.workspace.clone();
                self.child(state, AppAction::Session(SessionAction::Show(shared)))
            }
            AppAction::Review(ReviewAction::Delegate(R::HandOver(hand))) => {
                self.child(state, AppAction::Comments(CommentsAction::HandOver(hand.clone())))
            }
            AppAction::Comments(CommentsAction::Delegate(delegate)) => match delegate {
                CommentsDelegate::Toast(text) => state.toast(text.clone(), &clock),
                CommentsDelegate::ArchivedToast { undo, .. } => {
                    let notice = state.toast("Comment archived".to_owned(), &clock);
                    if let Some(toast) = state.toasts.last_mut() {
                        toast.action = Some(ToastAction::Comments(undo.clone()));
                    }
                    notice
                }
                // What is saved is compared after every action, so the
                // change saves by itself.
                CommentsDelegate::Saved(_) => Effect::none(),
                CommentsDelegate::HandedOver { session, comment } => self.child(
                    state,
                    AppAction::Review(ReviewAction::HandedOver(session.clone(), comment.clone())),
                ),
            },
            AppAction::Files(FilesAction::Delegate(F::Toast(text))) => {
                state.toast(text.clone(), &clock)
            }
            // PL-10: Open terminal here shows the session whose machine it is
            // on, with a new shell tab started in the folder.
            AppAction::Files(FilesAction::Delegate(F::OpenTerminal { session, folder })) => {
                let show = self.navigate(state, Route::Session(session.clone()));
                let open = self.child(
                    state,
                    AppAction::Session(SessionAction::OpenTerminalIn {
                        session: session.clone(),
                        folder: folder.clone(),
                    }),
                );
                Effect::merge([show, open])
            }
            AppAction::Palette(PaletteAction::Delegate(delegate)) => match delegate {
                P::Run(command) => self.run(state, command.clone()),
                P::Hit(index) => self.child(state, AppAction::Find(FindAction::Open(*index))),
                P::MoreHits => self.child(state, AppAction::Find(FindAction::More)),
                P::Closed => {
                    state
                        .overlays
                        .retain(|overlay| overlay != &Overlay::Palette);
                    Effect::none()
                }
            },
            AppAction::Find(FindAction::Delegate(delegate)) => self.find_delegate(state, delegate),
            AppAction::Pinned(PinnedAction::Delegate(delegate)) => self.pinned_delegate(state, delegate),
            AppAction::Views(ViewsAction::Delegate(delegate)) => self.views_delegate(state, delegate),
            AppAction::Desk(action) => self.desk_delegate(state, action),
            AppAction::PrAsk(AskAction::Ask { .. })
            | AppAction::PrDialog(DialogAction::Delegate(_))
            | AppAction::PrFollower(FollowerAction::Delegate(_))
            | AppAction::PrSubmit(SubmitAction::Delegate(_))
            | AppAction::PrHeads(HeadsAction::Delegate(_)) => self.pr_review_delegate(state, action),
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
        // Show pinned may have changed: the Pinned section follows it.
        state.pinned.shown = state.settings.preferences.show_pinned;
        let (count, more) = (state.find.hits.len(), !state.find.next_page.is_empty());
        let hits = if (state.palette.hits, state.palette.more_hits) == (count, more) {
            Effect::none()
        } else {
            self.child(state, AppAction::Palette(PaletteAction::Hits { count, more }))
        };
        // The machine page reads the Inbox's Migrate items (UP-1).
        let migrations = state.inbox.migrations();
        let page = if migrations == state.machine_page.migrations {
            Effect::none()
        } else {
            self.machine_page(
                state,
                humata2_hosts::MachinePageAction::Migrations(migrations.clone()),
            )
        };
        // The session page reads them too, to know when a refusal is over (D377).
        let refusal = match &state.session.refused {
            Some(_) => self.child(state, AppAction::Session(SessionAction::Migrations(migrations))),
            None => Effect::none(),
        };
        // The Add machine dialog reads This Mac's daemon Migrate items, to offer
        // Migrate This Mac when an add is blocked behind them, and to run the add
        // again once one is migrated.
        let add = if state.add_machine.open {
            let host = state
                .workspace
                .machines
                .iter()
                .find(|machine| humata2_clients::is_this_mac(machine))
                .map(|machine| machine.id.clone());
            let items: Vec<humata2_clients::model::Request> = state
                .inbox
                .migrations()
                .into_iter()
                .filter(|request| {
                    request.migrate().is_some_and(|card| {
                        card.thing == humata2_clients::model::MigrateThing::Daemon
                            && Some(&card.machine) == host.as_ref()
                    })
                })
                .collect();
            if items == state.add_machine.migrations {
                Effect::none()
            } else {
                self.add_machine(state, humata2_hosts::AddMachineAction::Migrations(items))
            }
        } else {
            Effect::none()
        };
        let save = self.persist(state);
        effect.and(hits).and(page).and(refusal).and(add).and(save)
    }
}

impl App {
    /// Whether this comments action changes a comment on a Desk file whose
    /// tab shows an old version (DV-6). Results of calls in flight, closing
    /// a box and the like are not changes made on the old version.
    fn refused_on_old_version(&self, state: &AppState, action: &CommentsAction) -> bool {
        use CommentsAction as C;
        use humata2_clients::comments::Anchor;
        let held = |session: &humata2_clients::model::SessionId| state.comments.sessions.get(session);
        let of = |session: &humata2_clients::model::SessionId, id: &humata2_clients::comments::CommentId| {
            held(session).and_then(|held| {
                held.comments
                    .iter()
                    .chain(held.archived.iter().flatten())
                    .find(|comment| &comment.id == id)
                    .map(|comment| comment.anchor.clone())
            })
        };
        let (session, anchors): (&humata2_clients::model::SessionId, Vec<Anchor>) = match action {
            C::OpenBox(session, anchor) => (session, vec![anchor.clone()]),
            C::Typed(session, _) | C::Comment(session) => (
                session,
                held(session)
                    .and_then(|held| held.editing.as_ref())
                    .map(|editing| editing.anchor.clone())
                    .into_iter()
                    .collect(),
            ),
            C::EditDraft(session, id)
            | C::Resolve(session, id)
            | C::Reopen(session, id)
            | C::Archive(session, id)
            | C::Unarchive(session, id)
            | C::Drop(session, id)
            | C::TakeBack(session, id)
            | C::SendAgain(session, id) => (session, of(session, id).into_iter().collect()),
            C::Reattach(session, id, anchor) => {
                let mut anchors = vec![anchor.clone()];
                anchors.extend(of(session, id));
                (session, anchors)
            }
            _ => return false,
        };
        anchors
            .iter()
            .any(|anchor| anchor.repo.is_empty() && state.session.doc_read_only(session, &anchor.path))
    }

    #[allow(clippy::too_many_lines)]
    fn handle(&self, state: &mut AppState, action: AppAction) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match action {
            // An old version of a Desk file is read-only (DV-6): nothing
            // comments on it, whatever sent the action.
            AppAction::Comments(ref comments) if self.refused_on_old_version(state, comments) => {
                state.toast(
                    "This is an old version, so it can't be commented on. Go back to the current version to comment.".to_owned(),
                    &clock,
                )
            }
            AppAction::Launch => {
                let saved = state.last_saved.comments.clone();
                let restore = self.child(state, AppAction::Comments(CommentsAction::Restore(saved)));
                let pinned = self.child(state, AppAction::Pinned(PinnedAction::Start));
                // AGENTSET (D725): which agents the pickers offer.
                let agents = self.child(
                    state,
                    AppAction::Settings(SettingsAction::Agents(
                        humata2_settings::AgentsAction::Load,
                    )),
                );
                Effect::merge([restore, self.load(), self.follow(), pinned, agents])
            }
            AppAction::StartNewDeskSession => self.start_new_desk_session(state),
            AppAction::Session(SessionAction::Archive) => self.archive_or_close(state),
            AppAction::Session(SessionAction::StartFork(kind)) => {
                self.fork_unless_desk(state, kind)
            }
            AppAction::Loaded(Ok(workspace)) => {
                state.load = Load::Ready;
                let mut workspace = workspace;
                // A reload brings no file lists; the ones fetched stay.
                for id in &state.files_asked {
                    if let Some(files) = state.workspace.session_files.get(id) {
                        workspace.session_files.insert(id.clone(), files.clone());
                    }
                }
                let before = state.workspace.restarts.clone();
                state.workspace = Arc::new(workspace);
                let mut effects = vec![self.show(state), self.loaded_restarts(state, &before)];
                // Comments older builds saved go to the daemon once the
                // diffs are known, which tell which repository each is in.
                if !state.review.saved.legacy_queued.is_empty()
                    || !state.review.saved.legacy_drafts.is_empty()
                {
                    let saved = state.review.saved.clone();
                    effects.push(self.child(state, AppAction::Review(ReviewAction::Restore(saved))));
                }
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
                // The daemon keeps each view's state: read the offers and
                // follow them once, however often the workspace reloads.
                if !state.views.watching {
                    effects.push(self.child(state, AppAction::Views(ViewsAction::Start)));
                }
                if !state.pr_heads.watching {
                    effects.push(self.child(state, AppAction::PrHeads(HeadsAction::Start)));
                }
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
                let pinned = self.child(state, AppAction::Pinned(PinnedAction::Start));
                Effect::merge([self.load(), self.follow(), pinned])
            }
            AppAction::Navigate(route) => self.navigate(state, route),
            AppAction::Switch(route) => self.switch(state, route),
            AppAction::Back => self.go_back(state),
            AppAction::Escape => self.escape(state),
            AppAction::OpenPalette => self.open_palette(state),
            AppAction::SessionFiles { session, result } => {
                self.session_files_found(state, session, result)
            }
            AppAction::StartNewSession => self.start_new_session(state),
            AppAction::StartNewForItem(id) => self.start_new_for_item(state, id),
            AppAction::StartNewChannel => self.start_new_channel(state),
            AppAction::OpenChannelAccess(id) => {
                state.channel_pages.entry(id.clone()).or_default().tab = channel::Tab::Access;
                self.switch(state, Route::Channel(id))
            }
            AppAction::NewDeskSessionFor(id) => self.new_desk_session_for(state, &id),
            AppAction::OpenChannelDesk(id) => {
                state.channel_pages.entry(id.clone()).or_default().tab = channel::Tab::Desk;
                self.switch(state, Route::Channel(id))
            }
            AppAction::ShowShortcuts => self.show_shortcuts(state),
            AppAction::NewSession(change) => self.change_new_session(state, change),
            AppAction::NewChannel(change) => self.change_new_channel(state, change),
            // REPOS (CH-6)
            AppAction::ChannelAddRepo(change) => self.change_channel_add_repo(state, change),
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
                    self.add_machine(state, humata2_hosts::AddMachineAction::Open),
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
                        let comments =
                            self.child(state, AppAction::Comments(CommentsAction::Open(id.clone())));
                        effect
                            .and(self.child(state, AppAction::Review(ReviewAction::Open(id))))
                            .and(comments)
                    }
                    // A list that came back empty was read before the
                    // clones were on disk, or failed: opening Files reads
                    // it again. One on its way, or with files, stays.
                    (Panel::Files, Some(id)) => {
                        if state
                            .workspace
                            .session_files
                            .get(&id)
                            .is_some_and(Vec::is_empty)
                        {
                            state.files_asked.remove(&id);
                        }
                        effect.and(self.ask_files(state, id))
                    }
                    _ => effect,
                }
            }
            AppAction::Views(ViewsAction::Step { id, to }) => {
                self.step_view(state, &id, to)
            }
            AppAction::Views(action @ (ViewsAction::Keep | ViewsAction::Close)) => {
                let close = action == ViewsAction::Close;
                let (tab, under) = self.finish_view(state, close);
                let answer = self.child(state, AppAction::Views(action));
                let resume = match under {
                    Some(id) => self.child(state, AppAction::Views(ViewsAction::Resume(id))),
                    None => Effect::none(),
                };
                answer.and(tab).and(resume)
            }
            AppAction::Palette(PaletteAction::Query(query)) => {
                // The palette's query is search's too (KP-1, LD-23).
                let palette =
                    self.child(state, AppAction::Palette(PaletteAction::Query(query.clone())));
                palette.and(self.child(state, AppAction::Find(FindAction::Query(query))))
            }
            // REPOS (RL-5): a repo search.
            // REPOS (CH-21): Add repo to this session.
            AppAction::SessionAddRepo(action) => self.session_add_repo(state, action),
            AppAction::RepoSearch(picker, action) => self.repo_search(state, picker, action),
            // REPOS (ST-21): opening General reads the repo library and scope.
            action @ AppAction::Settings(SettingsAction::Section(
                humata2_settings::Section::General,
            )) => {
                let repos = self.child(
                    state,
                    AppAction::Settings(SettingsAction::Repos(
                        humata2_settings::RepositoriesAction::Open,
                    )),
                );
                let delegated = self.delegate(state, &action);
                let effect = self.child(state, action);
                effect.and(delegated).and(repos)
            }
            AppAction::Restart(change) => self.change_restart(state, change),
            AppAction::AddMachine(change) => self.add_machine(state, change),
            AppAction::MachinePage(change) => self.machine_page(state, change),
            action @ (AppAction::Sidebar(_)
            | AppAction::Session(_)
            | AppAction::Review(_)
            | AppAction::Comments(_)
            | AppAction::Inbox(_)
            | AppAction::Board(_)
            | AppAction::Hosts(_)
            | AppAction::Files(_)
            | AppAction::Find(_)
            | AppAction::Pinned(_)
            | AppAction::Settings(_)
            | AppAction::Palette(_)
            | AppAction::Views(_)
            | AppAction::PrDialog(_)
            | AppAction::PrAsk(_)
            | AppAction::PrFollower(_)
            | AppAction::PrSubmit(_)
            | AppAction::PrHeads(_)
            | AppAction::PrInfo(_)
            | AppAction::Desk(_)) => {
                let delegated = self.delegate(state, &action);
                let effect = self.child(state, action);
                effect.and(delegated)
            }
        }
    }
}
