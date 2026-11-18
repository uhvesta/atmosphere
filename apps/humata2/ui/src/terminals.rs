//! Session-owned terminal views. Each (session, tab) has one
//! `terminal_replica` entity fed by that tab's link: the daemon's frames in
//! (the screen first, then output and size changes), the view's frames out.
//! Dropping the entry drops the link; the program keeps running.
//!
//! The Inbox's open item, when it is a question on its agent's screen,
//! gets one more replica of that tab, apart from the session's own: it
//! never sizes the tab and never moves the session's focus (D781).
use crate::{bridge::Ui, inbox_terminal::InboxTerminal, theme::Look};
use ghostty_vt::{Colors, Rgb};
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route, layout::FullScreen};
use humata2_clients::ImageKind;
use humata2_clients::model::{Agent, MachineKind, RequestId, SessionId, Status};
use humata2_clients::{LinkSink, SendError as LinkSendError, TerminalLink, TerminalNotice};
use humata2_session::{SessionAction, Tab};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
};
use terminal_grid::Grid as _;
use terminal_pane::{PaneEvent, TerminalPane};
use terminal_replica::{
    ClientFrame, FitPolicy, ReplicaConfig, ReplicaError, ReplicaEvent, ReplicaPhase, ReplicaSink,
    SendError, ServerFrame, ShortcutPolicy, TerminalReplica,
};
use terminal_view::Theme;
use terminal_wire::{SessionInfo, SessionState};

pub(crate) type FocusTarget = (Route, Option<Tab>, Option<FullScreen>);
pub(crate) fn focus_target(state: &AppState) -> FocusTarget {
    let tab = match &state.route {
        Route::Session(id) => state.session.views.get(id).and_then(|view| {
            view.selected(view.focused_group)
                .cloned()
                .or_else(|| view.groups.is_empty().then_some(Tab::Terminal(view.tab)))
        }),
        _ => None,
    };
    (state.route.clone(), tab, state.full_screen())
}

/// A tab sizes its program when it first shows it, while it has focus, and
/// when its own bounds change, say a side panel narrows the pane (D750). A
/// size another window set is never taken back (D271 point 3): an unfocused
/// tab resends only when its own bounds fit a size it hasn't sent.
const FIT: FitPolicy = FitPolicy::Shown;
/// A link is one connection; a new link is a new epoch of a new replica.
const EPOCH: u64 = 1;
/// The most frames a link's pump applies in one update; a full batch
/// waits for the window's next frame before the next one.
const FRAME_BATCH: usize = 256;

/// The longest a full batch waits for the window's next frame before the
/// pump goes on anyway: a window that draws no frames (hidden, on another
/// space) still takes its terminal's output, a batch at a time.
const FRAME_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

/// Ready when the window has drawn its next frame or `FRAME_WAIT` passed,
/// whichever comes first.
struct NextFrame {
    drawn: tokio::sync::oneshot::Receiver<()>,
    timer: gpui_kit::Task<()>,
}

impl std::future::Future for NextFrame {
    type Output = ();

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<()> {
        let this = &mut *self;
        // A frame drawn, or the window gone and its callback with it.
        if std::pin::Pin::new(&mut this.drawn).poll(cx).is_ready() {
            return std::task::Poll::Ready(());
        }
        std::pin::Pin::new(&mut this.timer).poll(cx)
    }
}

type Key = (SessionId, usize);

/// Where a tab's link stands.
pub(crate) enum Link {
    /// Asked for; the screen has not arrived.
    Opening,
    /// Frames can go out.
    Up(LinkSink),
    /// No more frames come. `worked` says a screen was shown before it
    /// ended; `why` is what to tell the person when none was.
    Ended { worked: bool, why: Option<String> },
}

/// The replica's way out: the link, refusing typing while the tab is not
/// writable.
struct Sink {
    link: Rc<RefCell<Link>>,
    writable: Rc<Cell<bool>>,
    /// The tab's "Reconnected" line; typing anything moves on from it.
    reconnected: Rc<Cell<Option<u32>>>,
    /// Set when typing cleared that line, so the shell redraws once.
    moved_on: Rc<Cell<bool>>,
}
impl ReplicaSink for Sink {
    fn send(&self, frame: ClientFrame) -> Result<(), SendError> {
        if matches!(frame, ClientFrame::Input { .. }) && !self.writable.get() {
            return Err(SendError::Closed);
        }
        let typed = matches!(frame, ClientFrame::Input { .. });
        let sent = match &*self.link.borrow() {
            Link::Up(sink) => sink.send(frame).map_err(|error| match error {
                LinkSendError::Closed => SendError::Closed,
                LinkSendError::Full => SendError::Full,
            }),
            _ => Err(SendError::Closed),
        };
        // Keys reach the holder here, not through InputQueued.
        if typed && sent.is_ok() && self.reconnected.take().is_some() {
            self.moved_on.set(true);
        }
        sent
    }
}

/// How long a tab stays hidden before its screen memory is freed.
const HIDDEN_FREE_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

/// Whether terminal cursors blink: not in fast mode, where an idle
/// terminal then draws nothing (FAST, D787). Free when unchanged.
fn cursor_blink(fast_mode: bool) -> bool {
    !fast_mode
}

/// The replica's memory saver for the "free hidden terminals" setting.
fn memory_saver(on: bool) -> Option<std::time::Duration> {
    on.then_some(HIDDEN_FREE_AFTER)
}

struct Entry {
    replica: Entity<TerminalReplica>,
    free_hidden: bool,
    link: Rc<RefCell<Link>>,
    writable: Rc<Cell<bool>>,
    appearance: (u32, u32, u32),
    font: u8,
    /// The tab was hidden after its link ended, so showing it again is the
    /// person asking to try once more.
    hidden_since_end: bool,
    /// The line about the tab's machine coming back, shown until the person
    /// types again.
    reconnected: Rc<Cell<Option<u32>>>,
    /// How many others can type into the tab, as its link last said
    /// (SE-24); `None` until it says.
    writers: Rc<Cell<Option<u32>>>,
    /// The session's status and shell count when the link ended: a push
    /// that changes them (its start ended, a tab appeared) is another try.
    ended_with: Option<(Status, usize)>,
    _events: Subscription,
    _pump: Task<()>,
}

/// The Inbox's view of a question's screen (D781).
struct InboxEntry {
    key: Key,
    /// `None` when the replica could not be made; the view says why.
    entry: Option<Entry>,
    view: Entity<InboxTerminal>,
}

/// A terminal belongs to its session/tab, regardless of which group displays it.
pub(crate) struct Terminals {
    entries: HashMap<Key, Entry>,
    /// The Inbox's open item's screen, while it is a question on one.
    inbox: Option<InboxEntry>,
    errors: HashMap<Key, String>,
    /// How many replicas each tab has had: a replacement is a new identity.
    generations: HashMap<Key, u64>,
    /// The view that draws the terminals, told when a link ends or a
    /// tab's program does.
    shell: Rc<Cell<Option<EntityId>>>,
    /// The look of the last sync, for the lines drawn under a terminal.
    look: Option<Look>,
}

fn toast(message: String) -> AppAction {
    AppAction::Session(SessionAction::Delegate(humata2_session::Delegate::Toast(
        message,
    )))
}

fn info(key: &Key, generation: u64) -> SessionInfo {
    let title = format!("{} {}", key.0, key.1);
    SessionInfo {
        // The links number a tab's route by its index.
        id: key.1 as u64,
        uuid: format!("{}/{}/{generation}", key.0, key.1),
        automatic_title: title.clone(),
        title,
        name_override: None,
        attributes: BTreeMap::new(),
        cwd: String::new(),
        program: String::new(),
        state: SessionState::Running,
        recovery_available: false,
    }
}

impl Terminals {
    pub(crate) fn new(_: &Rc<Ui>, _: &mut App) -> Self {
        Self {
            entries: HashMap::new(),
            inbox: None,
            errors: HashMap::new(),
            generations: HashMap::new(),
            shell: Rc::new(Cell::new(None)),
            look: None,
        }
    }

    /// A replica for `key` and the task that opens its link and feeds it.
    /// `inbox`: the Inbox's view of a question's screen, which never sizes
    /// the tab and leaves the session's focus alone (D781).
    #[allow(clippy::too_many_arguments)]
    fn open(
        &mut self,
        key: Key,
        ui: &Rc<Ui>,
        look: Look,
        font: u8,
        free_hidden: bool,
        inbox: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entry, ReplicaError> {
        let generation = self.generations.entry(key.clone()).or_default();
        *generation += 1;
        let session = info(&key, *generation);
        let link = Rc::new(RefCell::new(Link::Opening));
        let writable = Rc::new(Cell::new(false));
        let reconnected = Rc::new(Cell::new(None));
        let moved_on = Rc::new(Cell::new(false));
        let sink = Rc::new(Sink {
            link: link.clone(),
            writable: writable.clone(),
            reconnected: reconnected.clone(),
            moved_on: moved_on.clone(),
        });
        let config = ReplicaConfig {
            session: session.clone(),
            theme: font_theme(font),
            memory_saver: memory_saver(free_hidden),
            shortcuts: ShortcutPolicy::default(),
            fit: if inbox { FitPolicy::Never } else { FIT },
            default_colors: Some(colors(look)),
        };
        let replica = TerminalReplica::create(config, sink, window, cx)?;
        let writers = Rc::new(Cell::new(None));
        let events = {
            let ui = ui.clone();
            let key = key.clone();
            let reconnected = reconnected.clone();
            let shell = self.shell.clone();
            cx.subscribe(&replica, move |replica, event: &ReplicaEvent, cx| match event {
                // Typing again is moving on from the reconnect.
                ReplicaEvent::InputQueued(_) => {
                    if reconnected.take().is_some()
                        && let Some(shell) = shell.get()
                    {
                        cx.notify(shell);
                    }
                }
                ReplicaEvent::Focused { .. } if !inbox => {
                    let ui = ui.clone();
                    let (session, tab) = key.clone();
                    // Focus can change during a render that is reading the store.
                    cx.defer(move |cx| {
                        ui.send(
                            AppAction::Session(SessionAction::FocusTerminal { session, tab }),
                            cx,
                        )
                    });
                }
                ReplicaEvent::Pane(PaneEvent::SplitRequested) => ui.send(
                    AppAction::Session(SessionAction::OpenTerminal {
                        agent: Agent::Shell,
                        split: true,
                    }),
                    cx,
                ),
                ReplicaEvent::Pane(PaneEvent::ClipboardWrite { text, .. }) => {
                    ui.clients().clipboard.copy(text)
                }
                ReplicaEvent::Pane(PaneEvent::InputRefused) => {
                    let phase = replica.read(cx).state().phase;
                    let (session, tab) = key.clone();
                    let why = ui.read(|state| refusal(state, &session, tab, phase));
                    ui.send(toast(why), cx);
                }
                ReplicaEvent::Pane(PaneEvent::OpenLink(uri)) => {
                    if openable(uri) {
                        cx.open_url(uri);
                    } else {
                        ui.send(
                            toast(
                                "Humata opens only web and mail links from a terminal, so this link stays closed. Select and copy it to open it yourself, if you trust where it goes."
                                    .into(),
                            ),
                            cx,
                        );
                    }
                }
                ReplicaEvent::Pane(PaneEvent::PasteFiles(paths)) => {
                    let session = key.0.clone();
                    // None: the session is on this Mac. Some(None): it is gone.
                    let elsewhere = ui.read(|state| {
                        (!on_this_mac(state, &session)).then(|| {
                            state.workspace.session(&session).map(|found| found.machine.clone())
                        })
                    });
                    if let Some(machine) = elsewhere {
                        // The paths don't exist on that machine: the files go
                        // to the session's pastes there first (D335).
                        let Some(machine) = machine else {
                            return;
                        };
                        let ui = ui.clone();
                        let paths = paths.clone();
                        cx.spawn(async move |cx| {
                            let pasted = ui
                                .clients()
                                .file_manager
                                .paste_files(paths, &machine, &session)
                                .await;
                            cx.update(|cx| match pasted {
                                Ok(there) => {
                                    let words: Vec<_> =
                                        there.iter().map(|path| shell_word(path)).collect();
                                    paste_into(&replica, words.join(" "), cx);
                                }
                                Err(error) => ui.send(
                                    toast(format!("The files could not be pasted: {error}")),
                                    cx,
                                ),
                            });
                        })
                        .detach();
                    } else if let Some(words) = paths
                        .iter()
                        .map(|path| path.to_str().map(shell_word))
                        .collect::<Option<Vec<_>>>()
                    {
                        paste_into(&replica, words.join(" "), cx);
                    } else {
                        ui.send(
                            toast(
                                "A pasted file's name isn't valid UTF-8, so its path can't be typed. Rename the file, then paste it again."
                                    .into(),
                            ),
                            cx,
                        );
                    }
                }
                ReplicaEvent::Pane(PaneEvent::PasteImage { format, bytes }) => {
                    let Some(kind) = image_kind(*format) else {
                        ui.send(
                            toast(
                                "This image's format can't be pasted into a terminal. Save it as PNG, JPEG, GIF, WebP or TIFF and paste that instead."
                                    .into(),
                            ),
                            cx,
                        );
                        return;
                    };
                    let ui = ui.clone();
                    let session = key.0.clone();
                    let bytes = bytes.clone();
                    cx.spawn(async move |cx| {
                        let pasted = ui
                            .clients()
                            .terminals
                            .paste_image(&session, kind, bytes)
                            .await;
                        cx.update(|cx| match pasted {
                            Ok(path) => paste_into(&replica, shell_word(&path), cx),
                            Err(error) => ui.send(
                                toast(format!("The image could not be pasted: {error}")),
                                cx,
                            ),
                        });
                    })
                    .detach();
                }
                ReplicaEvent::Pane(PaneEvent::Failed(error)) => {
                    ui.send(toast(error.to_string()), cx)
                }
                ReplicaEvent::Failed(error) => ui.send(toast(error.to_string()), cx),
                ReplicaEvent::InputUncertain(_) => ui.send(
                    toast(
                        "Something you typed may not have reached the terminal. Check what it shows before typing it again."
                            .into(),
                    ),
                    cx,
                ),
                // The bell, a program's title and its working folder wait on
                // their decisions (D338, D424). The pane itself asks before
                // a program reads the clipboard.
                _ => {}
            })
        };
        let pump = {
            let ui = ui.clone();
            let replica = replica.clone();
            let link = link.clone();
            let shell = self.shell.clone();
            let reconnected = reconnected.clone();
            let moved_on = moved_on.clone();
            let writers = writers.clone();
            let handle = window.window_handle();
            let (id, tab) = key;
            cx.spawn(async move |cx| {
                let linked = if inbox {
                    ui.clients().terminals.link_unsized(&id, tab)
                } else {
                    ui.clients().terminals.link(&id, tab)
                };
                let (worked, why) = match linked.await {
                    Err(error) => (false, Some(error.to_string())),
                    Ok(TerminalLink {
                        mut frames,
                        mut notices,
                        sink,
                    }) => {
                        *link.borrow_mut() = Link::Up(sink);
                        let mut worked = false;
                        let mut refusal_open = false;
                        let bound = cx.update_window(handle, |_, _, cx| {
                            replica.update(cx, |replica, cx| {
                                replica.bind_connection(EPOCH, session, cx)
                            })
                        });
                        let why = match bound {
                            Ok(Ok(())) => None,
                            Ok(Err(error)) => Some(error.to_string()),
                            Err(_) => Some("The window closed.".to_owned()),
                        };
                        while why.is_none() {
                            let frame = tokio::select! {
                                frame = frames.recv() => frame,
                                Some(notice) = notices.recv() => {
                                    match notice {
                                        TerminalNotice::Reconnected { caught_up_lines } => {
                                            reconnected.set(Some(caught_up_lines));
                                        }
                                        TerminalNotice::Writers { others } => {
                                            writers.set(Some(others));
                                        }
                                        TerminalNotice::Refused(error) => {
                                            // Tell the session page what to migrate
                                            // first; the next write that lands
                                            // takes it away again.
                                            refusal_open = true;
                                            cx.update(|cx| {
                                                ui.send(
                                                    AppAction::Session(SessionAction::Typed(
                                                        Err(error),
                                                    )),
                                                    cx,
                                                )
                                            });
                                        }
                                    }
                                    if let Some(shell) = shell.get() {
                                        cx.update(|cx| cx.notify(shell));
                                    }
                                    continue;
                                }
                            };
                            let Some(frame) = frame else {
                                break;
                            };
                            // What is already queued behind it goes in the
                            // same update: a program that floods its output
                            // (`yes`) costs one update per batch, not one
                            // per frame.
                            let mut batch = vec![frame];
                            while batch.len() < FRAME_BATCH {
                                match frames.try_recv() {
                                    Ok(frame) => batch.push(frame),
                                    Err(_) => break,
                                }
                            }
                            let full = batch.len() == FRAME_BATCH;
                            let mut accepted = false;
                            let mut redraw = false;
                            let applied = cx.update_window(handle, |_, window, cx| {
                                replica.update(cx, |replica, cx| {
                                    let mut outcomes = Vec::with_capacity(batch.len());
                                    for frame in batch {
                                        let screen = matches!(frame, ServerFrame::Snapshot { .. });
                                        // A line written, or the right to type
                                        // given: the refusal this view told no
                                        // longer holds.
                                        accepted |= matches!(
                                            frame,
                                            ServerFrame::InputWritten { .. }
                                                | ServerFrame::Control { controller: true, .. }
                                        );
                                        // The line under the output changes with
                                        // these: the shell redraws for them. So
                                        // does typing that moved on from
                                        // "Reconnected".
                                        redraw |= screen
                                            || matches!(frame, ServerFrame::Exited { .. })
                                            || (matches!(frame, ServerFrame::InputWritten { .. })
                                                && moved_on.take());
                                        outcomes.push((screen, replica.apply(frame, EPOCH, window, cx)));
                                    }
                                    outcomes
                                })
                            });
                            if accepted && refusal_open {
                                refusal_open = false;
                                cx.update(|cx| {
                                    ui.send(AppAction::Session(SessionAction::Typed(Ok(()))), cx)
                                });
                            }
                            if redraw && let Some(shell) = shell.get() {
                                cx.update(|cx| cx.notify(shell));
                            }
                            let Ok(outcomes) = applied else { break };
                            for (screen, outcome) in outcomes {
                                match outcome {
                                    Ok(()) => worked |= screen,
                                    Err(error) => {
                                        let message = error.to_string();
                                        cx.update(|cx| ui.send(toast(message), cx));
                                    }
                                }
                            }
                            // A flood never leaves the channel empty, so the
                            // next receive would be ready at once and this
                            // loop would keep the main thread: after a full
                            // batch it waits for the window to draw it, so
                            // keys, clicks and frames go between batches and
                            // a flood costs one batch a frame.
                            if full {
                                let (drawn, wait) = tokio::sync::oneshot::channel();
                                let asked = cx.update_window(handle, |_, window, _| {
                                    window.on_next_frame(move |_, _| {
                                        let _ = drawn.send(());
                                    });
                                });
                                if asked.is_err() {
                                    break;
                                }
                                let timer = cx.background_executor().timer(FRAME_WAIT);
                                NextFrame { drawn: wait, timer }.await;
                            }
                        }
                        (worked, why)
                    }
                };
                *link.borrow_mut() = Link::Ended { worked, why };
                if let Some(shell) = shell.get() {
                    cx.update(|cx| cx.notify(shell));
                }
            })
        };
        Ok(Entry {
            replica,
            free_hidden,
            link,
            writable,
            appearance: (look.term_text, look.term, look.term_dim),
            font,
            hidden_since_end: false,
            reconnected,
            writers,
            ended_with: None,
            _events: events,
            _pump: pump,
        })
    }

    /// Opens, updates and drops the views to match `state`. Returns whether
    /// it dropped the Inbox's terminal while it had focus, so the shell
    /// takes focus back for the list (D781).
    pub(crate) fn sync(
        &mut self,
        ui: &Rc<Ui>,
        state: &AppState,
        look: Look,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        self.shell.set(Some(window.current_view()));
        self.look = Some(look);
        // Closing a tab hides its view, retaining the program and scrollback.
        self.entries
            .retain(|(id, _), _| state.workspace.session(id).is_some());
        // A link that ended after showing a screen is opened again; one that
        // never did waits until the tab is shown again or a push changes
        // its session, so a refusal is not retried on every redraw. While
        // the machine is out of reach the last screen stays.
        let mut reopen = Vec::new();
        for (key, entry) in &mut self.entries {
            let (ended, worked) = match &*entry.link.borrow() {
                Link::Ended { worked, .. } => (true, *worked),
                _ => (false, false),
            };
            if !ended {
                continue;
            }
            let shown = is_visible(state, &key.0, key.1);
            if !shown {
                entry.hidden_since_end = true;
            }
            let Some(session) = state.workspace.session(&key.0) else {
                continue;
            };
            let now = (session.status, session.shells.len());
            let changed = *entry.ended_with.get_or_insert(now) != now;
            if is_connected(state, &key.0)
                && (worked || (shown && (entry.hidden_since_end || changed)))
            {
                reopen.push(key.clone());
            }
        }
        for key in reopen {
            self.entries.remove(&key);
        }
        let font = state.settings.preferences.font_size;
        let free_hidden = state.settings.preferences.free_hidden_terminals;
        let option_as_alt = state.settings.preferences.option_as_alt;
        let blink = cursor_blink(state.settings.preferences.fast_mode);
        if let Route::Session(id) = &state.route {
            if let Some(session) = state.workspace.session(id) {
                for tab in 0..=session.shells.len() {
                    let key = (id.clone(), tab);
                    if !self.entries.contains_key(&key) && !self.errors.contains_key(&key) {
                        match self.open(key.clone(), ui, look, font, free_hidden, false, window, cx) {
                            Ok(entry) => {
                                self.entries.insert(key, entry);
                            }
                            Err(source) => {
                                self.errors.insert(key, format!("The terminal could not start: {source}. Reopen this session to retry."));
                            }
                        }
                    }
                }
            }
        }
        for ((id, tab), entry) in &mut self.entries {
            let Some(session) = state.workspace.session(id) else {
                continue;
            };
            let visible = is_visible(state, id, *tab);
            let connected = is_connected(state, id);
            entry
                .writable
                .set(connected && (*tab != 0 || !session.status.agent_ended()));
            let appearance = (look.term_text, look.term, look.term_dim);
            let changed_appearance = entry.appearance != appearance;
            entry.appearance = appearance;
            let changed_font = entry.font != font;
            entry.font = font;
            let changed_saver = entry.free_hidden != free_hidden;
            entry.free_hidden = free_hidden;
            let failed = entry.replica.update(cx, |replica, cx| {
                // A hidden replica sends nothing; showing it again asks for a fresh fit.
                let _ = replica.set_visible(visible, cx);
                if changed_saver {
                    // Switching it on starts the 30 s count for tabs hidden now;
                    // switching it off keeps every screen from here on.
                    let _ = replica.set_memory_saver(memory_saver(free_hidden), cx);
                }
                if let Some(pane) = replica.pane().cloned() {
                    pane.update(cx, |pane, cx| {
                        if !connected {
                            pane.set_connected(false, cx);
                        }
                        // Free when unchanged; a pane made since the last
                        // pass takes the setting here.
                        pane.set_option_as_alt(option_as_alt, cx);
                        pane.view().update(cx, |view, cx| view.set_cursor_blink(blink, cx));
                        if changed_font {
                            pane.view()
                                .update(cx, |view, cx| view.set_theme(font_theme(font), cx));
                        }
                    });
                }
                if changed_appearance {
                    replica.set_default_colors(Some(colors(look)), cx).err()
                } else {
                    None
                }
            });
            if let Some(source) = failed {
                self.errors.insert((id.clone(), *tab), format!("The terminal theme could not be applied: {source}. Change appearance again to retry."));
            }
        }
        let lost_focus = self.sync_inbox(ui, state, look, window, cx);
        ui.set_inbox_terminal(self.inbox.as_ref().map(|inbox| inbox.view.clone().into()));
        let screen = self
            .inbox
            .as_ref()
            .is_some_and(|inbox| inbox.view.read(cx).shows_screen(cx));
        ui.inbox_room().set_screen(screen);
        lost_focus
    }

    /// Links the Inbox's open item's screen while it is a question on one,
    /// and only that one (D781). Returns whether the view it dropped had
    /// focus.
    fn sync_inbox(
        &mut self,
        ui: &Rc<Ui>,
        state: &AppState,
        look: Look,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let wanted = if state.route == Route::Inbox {
            state.inbox.terminal()
        } else {
            None
        };
        // A link that ended after showing a screen opens again once the
        // machine answers; one that never showed one stays with its reason.
        let ended_after_screen = self.inbox.as_ref().is_some_and(|inbox| {
            inbox.entry.as_ref().is_some_and(|entry| {
                matches!(&*entry.link.borrow(), Link::Ended { worked: true, .. })
            }) && is_connected(state, &inbox.key.0)
        });
        let moved = self.inbox.as_ref().map(|inbox| &inbox.key) != wanted.as_ref();
        let mut lost_focus = false;
        if (ended_after_screen || moved)
            && let Some(entry) = self.inbox.take().and_then(|gone| gone.entry)
        {
            lost_focus = entry.replica.read(cx).pane().is_some_and(|pane| {
                pane.read(cx).focus_handle(cx).contains_focused(window, cx)
            });
        }
        let Some(key) = wanted else {
            return lost_focus;
        };
        let font = state.settings.preferences.font_size;
        let request = state
            .inbox
            .shown()
            .into_iter()
            .find(|request| {
                request.session == key.0
                    && request.on_screen == Some(key.1)
                    && request.answer.is_none()
            })
            .map(|request| request.id.clone())
            .unwrap_or_else(|| RequestId::new(""));
        if self.inbox.is_none() {
            let opened = self.open(key.clone(), ui, look, font, false, true, window, cx);
            let (entry, why) = match opened {
                Ok(entry) => (Some(entry), None),
                // Nothing to link: the reason stays until another item opens.
                Err(source) => (None, Some(format!("the terminal could not start ({source})"))),
            };
            let shown = entry
                .as_ref()
                .map(|entry| (entry.replica.clone(), entry.link.clone()));
            let view = InboxTerminal::new(ui.clone(), request.clone(), shown, why, cx);
            self.inbox = Some(InboxEntry { key: key.clone(), entry, view });
        }
        let Some(inbox) = &mut self.inbox else {
            return lost_focus;
        };
        if let Some(entry) = &mut inbox.entry {
            let connected = is_connected(state, &key.0);
            let exited = key.1 == 0
                && state
                    .workspace
                    .session(&key.0)
                    .is_some_and(|session| session.status.agent_ended());
            entry.writable.set(connected && !exited);
            let appearance = (look.term_text, look.term, look.term_dim);
            let changed_appearance = entry.appearance != appearance;
            entry.appearance = appearance;
            let option_as_alt = state.settings.preferences.option_as_alt;
            let blink = cursor_blink(state.settings.preferences.fast_mode);
            entry.replica.update(cx, |replica, cx| {
                let _ = replica.set_visible(true, cx);
                if let Some(pane) = replica.pane().cloned() {
                    pane.update(cx, |pane, cx| {
                        if !connected {
                            pane.set_connected(false, cx);
                        }
                        pane.set_option_as_alt(option_as_alt, cx);
                        pane.view().update(cx, |view, cx| view.set_cursor_blink(blink, cx));
                    });
                }
                if changed_appearance {
                    // A refused color change keeps the screen as it was.
                    let _ = replica.set_default_colors(Some(colors(look)), cx);
                }
            });
        }
        inbox.view.update(cx, |view, cx| view.update(request, font, look, cx));
        lost_focus
    }

    /// Focuses the Inbox's terminal: a click in it, or Tab from the list.
    /// Returns whether it took focus: false when there is none or it has
    /// focus already.
    pub(crate) fn focus_inbox(&self, window: &mut Window, cx: &mut App) -> bool {
        let Some(entry) = self.inbox.as_ref().and_then(|inbox| inbox.entry.as_ref()) else {
            return false;
        };
        let focused = entry.replica.read(cx).pane().is_some_and(|pane| {
            pane.read(cx).focus_handle(cx).contains_focused(window, cx)
        });
        if focused {
            return false;
        }
        entry
            .replica
            .update(cx, |replica, cx| replica.focus_input(window, cx));
        true
    }

    pub(crate) fn focus(
        &self,
        session: &SessionId,
        tab: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if let Some(entry) = self.entries.get(&(session.clone(), tab)) {
            // Before the screen arrives this waits for it, then takes focus.
            entry
                .replica
                .update(cx, |replica, cx| replica.focus_input(window, cx));
            true
        } else {
            false
        }
    }

    /// How many others can type into a tab, as its link last said (SE-24);
    /// `None` before it has said.
    pub(crate) fn other_writers(&self, session: &SessionId, tab: usize) -> Option<u32> {
        self.entries
            .get(&(session.clone(), tab))
            .and_then(|entry| entry.writers.get())
    }

    /// The text input of a tab's view, as the platform's input method
    /// reaches it; `None` while the tab has no screen.
    pub(crate) fn input_handler(
        &self,
        session: &SessionId,
        tab: usize,
        bounds: Bounds<Pixels>,
        cx: &App,
    ) -> Option<ElementInputHandler<terminal_view::TerminalView<ghostty_vt::Terminal>>> {
        let entry = self.entries.get(&(session.clone(), tab))?;
        let pane = entry.replica.read(cx).pane()?;
        Some(ElementInputHandler::new(bounds, pane.read(cx).view().clone()))
    }

    /// Shows or hides the pane's built-in disconnected banner.
    pub(crate) fn set_offline_banner(
        &self,
        session: &SessionId,
        tab: usize,
        show: bool,
        cx: &mut App,
    ) {
        let pane = self
            .entries
            .get(&(session.clone(), tab))
            .and_then(|entry| entry.replica.read(cx).pane().cloned());
        if let Some(pane) = pane {
            pane.update(cx, |pane, cx| pane.set_offline_banner(show, cx));
        }
    }

    /// The terminal body; `notice` reads as part of the output, under the
    /// terminal's last row of content.
    pub(crate) fn body(
        &self,
        session: &SessionId,
        tab: usize,
        group: usize,
        notice: Option<AnyElement>,
        cx: &mut App,
    ) -> AnyElement {
        let key = (session.clone(), tab);
        if let Some(error) = self.errors.get(&key) {
            return failed(error.clone());
        }
        let Some(entry) = self.entries.get(&key) else {
            return div().into_any_element();
        };
        let pane = entry.replica.read(cx).pane().cloned();
        if pane.is_none()
            && let Link::Ended {
                why: Some(why), ..
            } = &*entry.link.borrow()
        {
            return failed(format!(
                "The terminal could not open: {}. Select the tab again to retry.",
                why.trim_end_matches('.')
            ));
        }
        // Expose the actual rendered grid to accessibility and headless flows.
        let text = pane
            .as_ref()
            .map(|pane| grid_text(pane, cx))
            .unwrap_or_default();
        // Tab 0's program has its own line, with Resume; a shell tab's says
        // its shell ended.
        let exited = if tab == 0 {
            None
        } else {
            entry.replica.read(cx).state().exit_code
        };
        let notice =
            notice.or_else(|| exited.zip(self.look).map(|(code, look)| exit_line(code, look)));
        let notice = notice.or_else(|| {
            entry
                .reconnected
                .get()
                .zip(self.look)
                .map(|(reconnected, look)| reconnected_line(reconnected, look))
        });
        let notice = notice.map(|notice| {
            let top = pane.as_ref().map_or(px(18.), |pane| {
                let pane = pane.read(cx);
                px(10.) + pane.line_height(cx) * pane.content_rows(cx) as f32 + px(8.)
            });
            div()
                .absolute()
                .top(top)
                .left(px(14.))
                .right(px(14.))
                .child(notice)
        });
        div()
            .id(if group == 0 {
                "terminal-area"
            } else {
                "terminal-secondary"
            })
            .role(Role::Log)
            .aria_label("Terminal output")
            .aria_description(text.clone())
            .test_text(text)
            .relative()
            .flex_1()
            .min_h_0()
            .px(px(14.))
            .py(px(10.))
            // Escape is the app's Dismiss everywhere, a text box included,
            // so the find bar's own Escape never runs: close find here, on
            // the way up, and leave every other Escape to the Shell.
            .on_action(move |_: &crate::keys::Dismiss, window, cx| {
                match pane.as_ref().filter(|pane| pane.read(cx).is_find_open()) {
                    Some(pane) => pane.update(cx, |pane, cx| pane.close_find(window, cx)),
                    None => cx.propagate(),
                }
            })
            .child(entry.replica.clone())
            .children(notice)
            .test_support()
            .into_any_element()
    }
}

/// Why the tab shows no terminal, in its place. It is an alert, so a screen
/// reader and a headless flow both read the reason.
fn failed(message: String) -> AnyElement {
    div()
        .id("terminal-error")
        .role(Role::Alert)
        .aria_label(message.clone())
        .p(px(14.))
        .child(message)
        .test_support()
        .into_any_element()
}

/// The line under a tab's output once its machine is back (SE-16). With no
/// count it does not claim one.
fn reconnected_line(caught_up_lines: u32, look: Look) -> AnyElement {
    let text = match caught_up_lines {
        0 => "Reconnected. What the program printed while it was away is shown above.".to_owned(),
        1 => "Reconnected; caught up 1 line of output.".to_owned(),
        lines => format!("Reconnected; caught up {lines} lines of output."),
    };
    div()
        .id("terminal-reconnected")
        .aria_label(text.clone())
        .w_full()
        .px(px(10.))
        .py(px(7.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .text_size(px(12.))
        .child(text)
        .test_support()
        .into_any_element()
}

/// The line under a shell tab's output once its shell has ended.
fn exit_line(code: i32, look: Look) -> AnyElement {
    let text = if code < 0 {
        "The shell ended. Its output stays here; open a new terminal to run more commands."
            .to_owned()
    } else {
        format!(
            "The shell exited with code {code}. Its output stays here; open a new terminal to run more commands."
        )
    };
    div()
        .id("terminal-exited")
        .aria_label(text.clone())
        .w_full()
        .px(px(10.))
        .py(px(7.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .text_size(px(12.))
        .child(text)
        .test_support()
        .into_any_element()
}

/// The rows the grid shows, one per line.
pub(crate) fn grid_text(pane: &Entity<TerminalPane>, cx: &mut App) -> String {
    pane.read(cx).view().clone().update(cx, |view, _| {
        let grid = view.grid_mut();
        grid.sync();
        (0..grid.size().rows)
            .filter_map(|row| grid.row(row))
            .map(|row| {
                row.cells
                    .iter()
                    .map(|c| c.text.as_str())
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// Whether a group of the open session shows this tab right now.
fn is_visible(state: &AppState, id: &SessionId, tab: usize) -> bool {
    state.route == Route::Session(id.clone())
        && state.full_screen() != Some(FullScreen::Panel)
        && state.session.views.get(id).is_some_and(|view| {
            if view.groups.is_empty() {
                return view.tab == tab;
            }
            view.groups
                .iter()
                .any(|tabs| tabs.tabs.get(tabs.selected) == Some(&Tab::Terminal(tab)))
        })
}

/// Whether the session's machine is reachable and the session not offline.
fn is_connected(state: &AppState, id: &SessionId) -> bool {
    state.workspace.session(id).is_some_and(|session| {
        session.status != Status::Offline
            && state
                .workspace
                .machine(&session.machine)
                .is_some_and(|m| m.online)
    })
}

/// The cell is the font's advance rounded up to a whole pixel, so 12 pt Menlo
/// (7.2 px) would become 8 px, a tenth wider than the design. Half a point
/// smaller lands on 7 px; the line pitch stays 18 at the default setting.
pub(crate) fn font_theme(font: u8) -> Theme {
    let size = f32::from(font) - 0.5;
    Theme {
        font_size: size,
        line_height: size + 6.5,
        ..Default::default()
    }
}
/// Why a tab took none of what was typed, and what to do, for the toast that
/// says so.
fn refusal(state: &AppState, session: &SessionId, tab: usize, phase: ReplicaPhase) -> String {
    let moving = state
        .workspace
        .session(session)
        .and_then(|found| found.moving.as_ref());
    let reach = if let Some(moving) = moving {
        let name = state
            .workspace
            .machine(&moving.to)
            .map_or_else(
                || moving.to.as_str().to_owned(),
                |machine| machine.name.clone(),
            );
        Reach::Moving(name)
    } else if is_connected(state, session) {
        Reach::Here
    } else {
        Reach::OutOfReach
    };
    refused_because(&reach, tab, phase)
}

/// Where a tab's session is, for why what was typed into it was not sent.
enum Reach {
    /// On a machine that answers.
    Here,
    /// On a machine that does not answer now.
    OutOfReach,
    /// Moving to the machine named (MV-4).
    Moving(String),
}

/// Why typing into a tab was not sent, from where its session is and the
/// tab's phase.
fn refused_because(reach: &Reach, tab: usize, phase: ReplicaPhase) -> String {
    match reach {
        Reach::Moving(name) => {
            return format!(
                "This session is moving to {name}, so what you typed was not sent. Type it again once the move is done and its screen shows."
            );
        }
        Reach::OutOfReach => {
            return "This tab's machine is out of reach, so what you typed was not sent. Its program keeps running; type it again once the machine is back.".to_owned();
        }
        Reach::Here => {}
    }
    let why = match phase {
        ReplicaPhase::Exited if tab == 0 => {
            "This tab's program has exited, so what you typed was not sent. Resume the session to type into it again."
        }
        ReplicaPhase::Exited => {
            "This tab's shell has exited, so what you typed was not sent. Open a new terminal to run more commands."
        }
        ReplicaPhase::ReadOnly => {
            "Another window controls this tab, so it is read-only here and what you typed was not sent. Type in that window instead."
        }
        ReplicaPhase::Unavailable => {
            "This tab's program is not running, so what you typed was not sent. Reopen the session to start it again."
        }
        ReplicaPhase::Disconnected | ReplicaPhase::Restoring => {
            "This tab is still getting its screen back, so what you typed was not sent. Type it again once the screen shows."
        }
        _ => {
            "This tab is not taking typing right now, so what you typed was not sent. Click the terminal and type it again."
        }
    };
    why.to_owned()
}

/// Whether a link in a terminal opens when clicked: web and mail links only,
/// so output cannot open an app or a file on this Mac.
fn openable(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    ["http", "https", "mailto"]
        .iter()
        .any(|known| scheme.eq_ignore_ascii_case(known))
        && !rest.is_empty()
        && !uri.chars().any(char::is_control)
}

/// `path` as one shell word: as it is when every character is plain, else
/// in single quotes, so the shell expands nothing in it.
fn shell_word(path: &str) -> String {
    let plain = !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+,:@".contains(c));
    if plain {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', "'\\''"))
    }
}

fn image_kind(format: ImageFormat) -> Option<ImageKind> {
    match format {
        ImageFormat::Png => Some(ImageKind::Png),
        ImageFormat::Jpeg => Some(ImageKind::Jpeg),
        ImageFormat::Gif => Some(ImageKind::Gif),
        ImageFormat::Webp => Some(ImageKind::Webp),
        ImageFormat::Tiff => Some(ImageKind::Tiff),
        _ => None,
    }
}

/// Whether `session` runs on this Mac, where pasted files' paths exist.
fn on_this_mac(state: &AppState, session: &SessionId) -> bool {
    state
        .workspace
        .session(session)
        .and_then(|session| state.workspace.machine(&session.machine))
        .is_some_and(|machine| machine.kind == MachineKind::ThisMac)
}

/// Pastes `text` into the tab as if from the clipboard: bracketed when the
/// program asks, and held for a yes when it is several lines.
fn paste_into(replica: &Entity<TerminalReplica>, text: String, cx: &mut App) {
    if let Some(pane) = replica.read(cx).pane().cloned() {
        pane.update(cx, |pane, cx| pane.paste_text(text, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // `gpui_kit::*` brings GPUI's `test` macro in, which shadows the std one.
    use ::core::prelude::v1::test;

    #[test]
    fn only_web_and_mail_links_open() {
        for uri in [
            "https://example.com/a?b=c",
            "HTTP://example.com",
            "mailto:dev@example.com",
        ] {
            assert!(openable(uri), "{uri} opens");
        }
        for uri in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ssh://dev@box",
            "x-apple.systempreferences:com.apple.preference",
            "https:",
            "no scheme",
            "https://example.com/\u{7}bell",
        ] {
            assert!(!openable(uri), "{uri} stays closed");
        }
    }

    /// MV-4: typing into a session that is moving says where it is going,
    /// never that its machine is out of reach.
    #[test]
    fn typing_during_a_move_names_where_the_session_is_going() {
        let moving = Reach::Moving("studio".to_owned());
        for phase in [
            ReplicaPhase::Disconnected,
            ReplicaPhase::Restoring,
            ReplicaPhase::Live,
        ] {
            for tab in [0, 1] {
                let why = refused_because(&moving, tab, phase);
                assert!(why.contains("studio"), "{phase:?} names the target");
                assert_ne!(why, refused_because(&Reach::OutOfReach, tab, phase));
            }
        }
    }

    #[test]
    fn a_pasted_path_is_one_shell_word() {
        assert_eq!(shell_word("/tmp/shot.png"), "/tmp/shot.png");
        assert_eq!(shell_word("/tmp/a b.png"), "'/tmp/a b.png'");
        assert_eq!(shell_word("/tmp/it's.png"), "'/tmp/it'\\''s.png'");
        for risky in ["~/x", "/tmp/$HOME", "/tmp/a;rm", "/tmp/*", "=ls", ""] {
            let word = shell_word(risky);
            assert!(word.starts_with('\'') && word.ends_with('\''), "{risky} is quoted");
        }
    }
}

fn colors(look: Look) -> Colors {
    let mut colors = Colors {
        foreground: Rgb::hex(look.term_text),
        background: Rgb::hex(look.term),
        ..Default::default()
    };
    colors.palette[8] = Rgb::hex(look.term_dim);
    colors.palette[2] = Rgb::hex(0x30d158);
    colors
}
