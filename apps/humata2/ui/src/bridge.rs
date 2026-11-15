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

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::{App, EntityId};
use humata2_app::{App as AppReducer, AppAction, AppState};
use humata2_clients::Clients;
use tca::{Effect, LocalStore, Observation, Reducer};
use tokio::runtime::Handle;
use tokio::sync::Notify;

use crate::record::Recorder;

/// A part of the window that redraws on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The sidebar.
    Sidebar = 0,
    /// The screen in the middle, with the session's side panel.
    Main = 1,
    /// Overlays and toasts.
    Overlay = 2,
}

const PARTS: [Part; 3] = [Part::Sidebar, Part::Main, Part::Overlay];

/// The app's reducer, telling the recorder each action it reduces.
#[derive(Clone)]
pub struct Logged {
    app: AppReducer,
    recorder: Recorder,
}

impl Reducer for Logged {
    type State = AppState;
    type Action = AppAction;

    fn reduce(&self, state: &mut AppState, action: AppAction) -> Effect<AppAction> {
        let name = self
            .recorder
            .is_recording()
            .then(|| crate::record::action_name(&action));
        let effect = self.app.reduce(state, action);
        if let Some(name) = name {
            self.recorder.action(&name, state);
        }
        effect
    }
}

/// The store and what each pane needs to redraw.
pub struct Ui {
    store: LocalStore<Logged>,
    dirty: Rc<Cell<u8>>,
    wake: Arc<Notify>,
    panes: RefCell<[Option<EntityId>; 3]>,
    observations: RefCell<Vec<Observation>>,
    clients: Clients,
    recorder: Recorder,
}

impl Ui {
    /// The app on `clients`, its effects running on `runtime`, restored from
    /// what the last run saved. Call [`Ui::start`] once its panes exist.
    #[must_use]
    pub fn new(runtime: &Handle, clients: Clients, recorder: Recorder) -> Rc<Self> {
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
            panes: RefCell::new([None; 3]),
            observations: RefCell::new(Vec::new()),
            clients,
            recorder,
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
                    state.layout,
                    state.restore_error.clone(),
                    (
                        state.session.clone(),
                        state.review.clone(),
                        state.inbox.clone(),
                        state.board.clone(),
                    ),
                    (
                        state.hosts.clone(),
                        state.files.clone(),
                        state.settings.clone(),
                        state.views.clone(),
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
                )
            },
            move |_| changed(),
        ));
    }

    /// Redraws `id` when `part` changes.
    pub fn attach(&self, part: Part, id: EntityId) {
        self.panes.borrow_mut()[part as usize] = Some(id);
    }

    /// Starts reducing what effects send and redrawing what they change,
    /// then launches the app: loads the workspace and follows its events.
    pub fn start(self: &Rc<Self>, cx: &mut App) {
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

    /// The recorder.
    #[must_use]
    pub const fn recorder(&self) -> &Recorder {
        &self.recorder
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
