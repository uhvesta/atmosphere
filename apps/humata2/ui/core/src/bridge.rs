//! The one bridge between the app's tca-rust store and GPUI.
//!
//! The store lives on GPUI's thread. A view sends an action with
//! [`Ui::send`]; the store reduces it at once and its observers mark which
//! panes' parts of the state changed. Actions that effects send later are
//! reduced by the store's own task on GPUI's foreground executor, and a
//! second task re-draws the panes they changed. Each pane is redrawn only
//! when its part changed, so typing in the composer doesn't redraw the
//! sidebar, and a streaming agent doesn't redraw anything but the session.
//! Nothing polls: both tasks wait on the store.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{AnyView, App, EntityId, IntoElement, ScrollHandle, Styled as _, canvas};
use humata2_app::layout::FullScreen;
use humata2_app::{App as AppReducer, AppAction, AppState, Route};
use humata2_clients::Clients;
use humata2_clients::model::SessionId;
use humata2_session::{ConversationTarget, Tab};
use tca::{Effect, LocalStore, Observation, Reducer};
use tokio::runtime::Handle;
use tokio::sync::Notify;

/// The Inbox detail's padding, and the gap between its card, terminal and
/// hint.
pub const DETAIL_PAD: f32 = 20.;
pub const DETAIL_GAP: f32 = 12.;

/// A part of the Inbox's detail whose height the box fits under.
#[derive(Clone, Copy)]
pub enum Measured {
    /// The detail's visible height.
    Pane,
    /// The open item's card.
    Card,
    /// The hint row with Dismiss.
    Hint,
}

/// What the Inbox's detail measured at its last paint, and whether its
/// terminal shows a screen (the hint says where to answer by it).
#[derive(Default)]
pub struct InboxRoom {
    pane: Cell<f32>,
    card: Cell<f32>,
    hint: Cell<f32>,
    screen: Cell<bool>,
}

impl InboxRoom {
    /// An element, drawn over `part` at its full size, that records its
    /// height at each paint and redraws once when it changed.
    pub fn measure(ui: &Rc<Ui>, part: Measured) -> impl IntoElement {
        let ui = ui.clone();
        canvas(
            move |bounds, window, _| {
                let room = ui.inbox_room();
                let cell = match part {
                    Measured::Pane => &room.pane,
                    Measured::Card => &room.card,
                    Measured::Hint => &room.hint,
                };
                let height = f32::from(bounds.size.height);
                if (cell.get() - height).abs() >= 0.5 {
                    cell.set(height);
                    window.refresh();
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// The most the box can take: the detail less its padding, the card,
    /// the hint and the gaps between them. `None` until it was measured.
    pub fn room(&self) -> Option<f32> {
        let pane = self.pane.get();
        (pane > 0.).then(|| {
            pane - 2. * DETAIL_PAD - self.card.get() - self.hint.get() - 2. * DETAIL_GAP
        })
    }

    /// Whether the open item's terminal shows its screen.
    pub fn screen(&self) -> bool {
        self.screen.get()
    }

    pub fn set_screen(&self, screen: bool) {
        self.screen.set(screen);
    }
}

pub type FocusTarget = (Route, Option<Tab>, Option<FullScreen>);

pub fn focus_target(state: &AppState) -> FocusTarget {
    let tab = match &state.route {
        Route::Session(id) => state.session.views.get(id).and_then(|view| {
            view.selected(view.focused_group)
                .cloned()
                .or_else(|| view.groups.is_empty().then_some(Tab::Terminal(view.tab)))
        }),
        Route::Channel(_) if state.active_channel_desk().is_some() => Some(Tab::Terminal(0)),
        _ => None,
    };
    (state.route.clone(), tab, state.full_screen())
}

/// The tab's scroll position, and the target it was last brought to, so the
/// hit is scrolled into view once per target and the person can scroll away.
#[derive(Default)]
pub struct ConversationScroll {
    pub handle: ScrollHandle,
    pub shown: RefCell<Option<(SessionId, ConversationTarget)>>,
}

/// Pre/post-reduce hook for recording actions without coupling `ui/core` to
/// the bug-report `Recorder`.
pub trait ActionRecorder: 'static {
    fn before_reduce(&self, action: &AppAction) -> Option<String>;
    fn after_reduce(&self, name: &str, state: &AppState);
    fn as_any(&self) -> &dyn Any;
}

/// A part of the window that redraws on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The sidebar.
    Sidebar = 0,
    /// The screen in the middle, with the session's side panel.
    Main = 1,
    /// Overlays and toasts.
    Overlay = 2,
    /// The recording dot and the bug-report sheet.
    Report = 3,
}

const PARTS: [Part; 4] = [Part::Sidebar, Part::Main, Part::Overlay, Part::Report];

/// The app's reducer, telling the recorder each action it reduces.
#[derive(Clone)]
pub struct Logged {
    app: AppReducer,
    recorder: Arc<dyn ActionRecorder>,
}

impl Reducer for Logged {
    type State = AppState;
    type Action = AppAction;

    fn reduce(&self, state: &mut AppState, action: AppAction) -> Effect<AppAction> {
        let name = self.recorder.before_reduce(&action);
        let effect = self.app.reduce(state, action);
        if let Some(name) = name {
            self.recorder.after_reduce(&name, state);
        }
        effect
    }
}

/// The store and what each pane needs to redraw.
pub struct Ui {
    store: LocalStore<Logged>,
    dirty: Rc<Cell<u8>>,
    wake: Arc<Notify>,
    panes: RefCell<[Option<EntityId>; 4]>,
    observations: RefCell<Vec<Observation>>,
    clients: Clients,
    recorder: Arc<dyn ActionRecorder>,
    /// The live terminal of the Inbox's open item, while it is a question
    /// on its agent's screen (D781): set by the shell's terminals, drawn by
    /// the Inbox's detail.
    inbox_terminal: RefCell<Option<AnyView>>,
    /// What the Inbox's detail measured, for that terminal to fit under its
    /// card, and whether it shows a screen (D781).
    inbox_room: InboxRoom,
}

impl Ui {
    /// The app on `clients`, its effects running on `runtime`, restored from
    /// what the last run saved. Call [`Ui::start`] once its panes exist.
    #[must_use]
    pub fn new(runtime: &Handle, clients: Clients, recorder: impl ActionRecorder) -> Rc<Self> {
        let recorder: Arc<dyn ActionRecorder> = Arc::new(recorder);
        let state = AppState::launch(&clients);
        let reducer = Logged {
            app: AppReducer::new(clients.clone()),
            recorder: recorder.clone(),
        };
        let store = LocalStore::on(runtime, state, reducer);
        let ui = Rc::new(Self {
            store,
            dirty: Rc::new(Cell::new(0)),
            wake: Arc::new(Notify::new()),
            panes: RefCell::new([None; 4]),
            observations: RefCell::new(Vec::new()),
            clients,
            recorder,
            inbox_terminal: RefCell::new(None),
            inbox_room: InboxRoom::default(),
        });
        ui.observe();
        ui
    }

    fn observe(&self) {
        let mut observations = self.observations.borrow_mut();
        let mark = |part: Part| {
            let dirty = self.dirty.clone();
            let wake = self.wake.clone();
            move || {
                dirty.set(dirty.get() | 1 << part as u8);
                wake.notify_one();
            }
        };
        let changed = mark(Part::Sidebar);
        observations.push(self.store.observe(
            |state: &AppState| {
                (
                    state.sidebar.clone(),
                    state.layout.sidebar,
                    state.load.clone(),
                    state.route.clone(),
                    state.navigation_overlay,
                    state.hosts.selected.clone(),
                    state.settings.preferences.show_pinned,
                    state.mass_delete.clone(),
                    state.pending_cli_logins.len(),
                )
            },
            move |_| changed(),
        ));
        let changed = mark(Part::Main);
        observations.push(self.store.observe(
            |state: &AppState| {
                (
                    state.load.clone(),
                    state.route.clone(),
                    state.trail.len(),
                    state.channel_pages.clone(),
                    (
                        state.layout,
                        state.full_screens.clone(),
                        state.navigation_overlay,
                    ),
                    state.restore_error.clone(),
                    (
                        state.session.clone(),
                        state.review.clone(),
                        state.inbox.clone(),
                        state.todo.clone(),
                    ),
                    (
                        state.hosts.clone(),
                        state.files.clone(),
                        state.settings.clone(),
                        state.views.clone(),
                        state.view_step.clone(),
                        state.desk.clone(),
                        state.machine_page.clone(),
                        state.mass_delete.clone(),
                    ),
                    // Pending `humata login` banner (Shell) and Inbox cards.
                    (
                        state.pending_cli_logins.clone(),
                        state.answering_cli_logins.clone(),
                    ),
                    // The open session's status and its machine draw the
                    // terminal's notices (Lost the connection to …, exited)
                    // and the pane's offline banner.
                    (
                        state
                            .session
                            .open
                            .as_ref()
                            .and_then(|id| state.workspace.session(id).cloned()),
                        state.workspace.machines.clone(),
                    ),
                )
            },
            move |_| changed(),
        ));
        let changed = mark(Part::Overlay);
        observations.push(self.store.observe(
            |state: &AppState| {
                (
                    state.overlays.clone(),
                    state.palette.clone(),
                    state.toasts.clone(),
                    state.session.fork.clone(),
                    state.session.moving.clone(),
                    state.session.continuing.clone(),
                    state.session.archiving.clone(),
                    state.session.renaming.clone(),
                    state.session.forking,
                    state.session.error.clone(),
                    state.desk.clone(),
                    // The machine Stop and Start confirms and the Humata
                    // folder sheet are drawn over the whole window too.
                    (
                        state.add_machine.clone(),
                        state.hosts.clone(),
                        state.machine_page.clone(),
                        // The close-tab dialog (D784).
                        state.session.closing.clone(),
                        state.mass_delete.sheet.clone(),
                    ),
                )
            },
            move |_| changed(),
        ));
        let changed = mark(Part::Report);
        observations.push(
            self.store
                .observe(|state: &AppState| state.report.clone(), move |_| changed()),
        );
    }

    /// Redraws `id` when `part` changes.
    pub fn attach(&self, part: Part, id: EntityId) {
        self.panes.borrow_mut()[part as usize] = Some(id);
    }

    /// Starts reducing what effects send and redrawing what they change,
    /// then launches the app: loads the workspace and follows its events.
    pub fn start(self: &Rc<Self>, cx: &mut App) {
        self.bind_font_zoom(cx);
        cx.foreground_executor().spawn(self.store.run()).detach();
        let ui = Rc::downgrade(self);
        let wake = self.wake.clone();
        cx.spawn(async move |cx| {
            loop {
                wake.notified().await;
                let Some(ui) = ui.upgrade() else { break };
                cx.update(|cx| ui.flush(cx));
            }
        })
        .detach();
        self.send(AppAction::Launch, cx);
    }

    /// Wires app-wide `⌘+` / `⌘-` font zoom actions to `SettingsAction::SetFontSize`.
    pub fn bind_font_zoom(self: &Rc<Self>, cx: &mut App) {
        let zoom_in = Rc::downgrade(self);
        cx.on_action(move |_: &crate::keys::ZoomIn, cx| {
            if let Some(ui) = zoom_in.upgrade() {
                let next = ui.read(|state| step_font_size(state.settings.preferences.font_size, 1));
                ui.send(AppAction::Settings(humata2_app::SettingsAction::SetFontSize(next)), cx);
            }
        });
        let zoom_out = Rc::downgrade(self);
        cx.on_action(move |_: &crate::keys::ZoomOut, cx| {
            if let Some(ui) = zoom_out.upgrade() {
                let next = ui.read(|state| step_font_size(state.settings.preferences.font_size, -1));
                ui.send(AppAction::Settings(humata2_app::SettingsAction::SetFontSize(next)), cx);
            }
        });
    }

    /// Reduces `action` now and redraws the panes it changed.
    pub fn send(&self, action: AppAction, cx: &mut App) {
        self.store.send(action);
        self.flush(cx);
    }

    /// What `read` makes of the state now.
    pub fn read<T>(&self, read: impl FnOnce(&AppState) -> T) -> T {
        self.store.with_state(read)
    }

    /// The clients the app runs on.
    #[must_use]
    pub const fn clients(&self) -> &Clients {
        &self.clients
    }

    /// The app's clock, for words that count time ([`crate::clock`]).
    #[must_use]
    pub fn now(&self) -> crate::clock::Now {
        let daemon = self.clients.daemon.clone();
        Rc::new(move || daemon.now_ms())
    }

    /// The Inbox's terminal for its open item, if it shows one (D781).
    pub fn inbox_terminal(&self) -> Option<AnyView> {
        self.inbox_terminal.borrow().clone()
    }

    /// What the Inbox's detail measured, and whether its terminal shows a
    /// screen.
    pub const fn inbox_room(&self) -> &InboxRoom {
        &self.inbox_room
    }

    /// Sets the Inbox's terminal; `None` when its open item shows none.
    pub fn set_inbox_terminal(&self, view: Option<AnyView>) {
        *self.inbox_terminal.borrow_mut() = view;
    }

    /// The action recorder, type-erased for callers outside `ui/core`.
    #[must_use]
    pub fn recorder_any(&self) -> &dyn Any {
        self.recorder.as_any()
    }

    fn flush(&self, cx: &mut App) {
        let dirty = self.dirty.replace(0);
        if dirty == 0 {
            return;
        }
        let panes = *self.panes.borrow();
        for part in PARTS {
            if dirty & 1 << part as u8 != 0
                && let Some(id) = panes[part as usize]
            {
                cx.notify(id);
            }
        }
    }
}

/// Steps `current` by `delta` (+1 or -1), clamped to `FONT_SIZES`.
#[must_use]
pub fn step_font_size(current: u8, delta: i8) -> u8 {
    let min = i16::from(*humata2_app::FONT_SIZES.start());
    let max = i16::from(*humata2_app::FONT_SIZES.end());
    (i16::from(current) + i16::from(delta)).clamp(min, max) as u8
}

#[cfg(test)]
mod tests {
    use super::step_font_size;
    use humata2_app::FONT_SIZES;

    #[test]
    fn step_font_size_moves_by_one_and_clamps_to_bounds() {
        let min = *FONT_SIZES.start();
        let max = *FONT_SIZES.end();
        assert_eq!(step_font_size(12, 1), 13);
        assert_eq!(step_font_size(12, -1), 11);
        assert_eq!(step_font_size(max, 1), max);
        assert_eq!(step_font_size(min, -1), min);
    }
}
