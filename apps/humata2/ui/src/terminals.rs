//! Session-owned terminal entities and their ordered, nonblocking client bridge.
use crate::{bridge::Ui, theme::Look};
use ghostty_vt::{Colors, Grid, Rgb, Size, Terminal};
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route, layout::FullScreen};
use humata2_clients::model::{Agent, SessionId, Status};
use humata2_session::{SessionAction, Tab};
use std::{cell::Cell, collections::HashMap, rc::Rc};
use terminal_pane::{PaneEvent, Pty, TerminalPane};
use terminal_view::{CellSize, Theme};
use tokio::sync::mpsc;

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

type Key = (SessionId, usize);
enum Io {
    Input(Key, Vec<u8>),
    Resize(Key, u16, u16),
}
struct ClientPty {
    key: Key,
    tx: mpsc::UnboundedSender<Io>,
    writable: Rc<Cell<bool>>,
}
impl Pty for ClientPty {
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.write_accepted(bytes);
    }
    fn write_accepted(&mut self, bytes: &[u8]) -> bool {
        self.writable.get()
            && self
                .tx
                .send(Io::Input(self.key.clone(), bytes.to_vec()))
                .is_ok()
    }
    fn resize(&mut self, size: Size, _: CellSize) {
        if self.writable.get() {
            let _ = self
                .tx
                .send(Io::Resize(self.key.clone(), size.cols, size.rows));
        }
    }
}
struct FocusObserver {
    _subscription: Subscription,
}
struct Entry {
    pane: Entity<TerminalPane>,
    read: usize,
    appearance: (u32, u32, u32),
    writable: Rc<Cell<bool>>,
    _events: Subscription,
    _focus_observer: Entity<FocusObserver>,
}
/// A terminal belongs to its session/tab, regardless of which group displays it.
pub(crate) struct Terminals {
    entries: HashMap<Key, Entry>,
    errors: HashMap<Key, String>,
    tx: mpsc::UnboundedSender<Io>,
}
impl Terminals {
    pub(crate) fn new(ui: &Rc<Ui>, cx: &mut App) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ui = ui.clone();
        // A single consumer preserves the order of keys, pastes and resizes.
        cx.spawn(async move |cx| {
            while let Some(io) = rx.recv().await {
                let result = match io {
                    Io::Input((session, tab), bytes) => {
                        ui.clients().terminals.input(&session, tab, bytes).await
                    }
                    Io::Resize((session, tab), cols, rows) => {
                        ui.clients()
                            .terminals
                            .resize(&session, tab, cols, rows)
                            .await
                    }
                };
                if let Err(error) = result {
                    cx.update(|cx| {
                        ui.send(AppAction::Session(SessionAction::TerminalFailed(error)), cx)
                    });
                }
            }
        })
        .detach();
        Self {
            entries: HashMap::new(),
            errors: HashMap::new(),
            tx,
        }
    }
    fn create(
        &self,
        key: Key,
        ui: &Rc<Ui>,
        look: Look,
        font: u8,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entry, ghostty_vt::Error> {
        let mut terminal = Terminal::new(100, 40, 10_000)?;
        terminal.set_default_colors(&colors(look))?;
        let writable = Rc::new(Cell::new(true));
        let pty = ClientPty {
            key: key.clone(),
            tx: self.tx.clone(),
            writable: writable.clone(),
        };
        let pane = cx.new(|cx| TerminalPane::new(terminal, pty, font_theme(font), window, cx));
        let focus = pane.read(cx).focus_handle(cx);
        let focused_ui = ui.clone();
        let focus_key = key.clone();
        let focus_observer = cx.new(|cx| {
            let subscription = cx.on_focus(&focus, window, move |_: &mut FocusObserver, _, cx| {
                let ui = focused_ui.clone();
                let (session, tab) = focus_key.clone();
                // Focus can change during a render that is reading the store.
                cx.defer(move |cx| {
                    ui.send(
                        AppAction::Session(SessionAction::FocusTerminal { session, tab }),
                        cx,
                    )
                });
            });
            FocusObserver {
                _subscription: subscription,
            }
        });
        let ui = ui.clone();
        let events = cx.subscribe(&pane, move |_, event: &PaneEvent, cx| match event {
            PaneEvent::SplitRequested => ui.send(
                AppAction::Session(SessionAction::OpenTerminal {
                    agent: Agent::Shell,
                    split: true,
                }),
                cx,
            ),
            PaneEvent::ClipboardWrite { text, .. } => ui.clients().clipboard.copy(text),
            PaneEvent::Failed(error) => ui.send(
                AppAction::Session(SessionAction::Delegate(humata2_session::Delegate::Toast(
                    error.to_string(),
                ))),
                cx,
            ),
            _ => {}
        });
        Ok(Entry {
            pane,
            read: 0,
            appearance: (look.term_text, look.term, look.term_dim),
            writable,
            _events: events,
            _focus_observer: focus_observer,
        })
    }
    pub(crate) fn sync(
        &mut self,
        ui: &Rc<Ui>,
        state: &AppState,
        look: Look,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Route::Session(id) = &state.route {
            if let Some(session) = state.workspace.session(id) {
                for tab in 0..=session.shells.len() {
                    let key = (id.clone(), tab);
                    if !self.entries.contains_key(&key) && !self.errors.contains_key(&key) {
                        match self.create(
                            key.clone(),
                            ui,
                            look,
                            state.settings.preferences.font_size,
                            window,
                            cx,
                        ) {
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
        // Closing a tab hides its view, retaining the program and scrollback.
        self.entries
            .retain(|(id, _), _| state.workspace.session(id).is_some());
        for ((id, tab), entry) in &mut self.entries {
            let Some(session) = state.workspace.session(id) else {
                continue;
            };
            let visible = state.route == Route::Session(id.clone())
                && state.full_screen() != Some(FullScreen::Panel)
                && state.session.views.get(id).is_some_and(|view| {
                    if view.groups.is_empty() {
                        return view.tab == *tab;
                    }
                    view.groups
                        .iter()
                        .any(|tabs| tabs.tabs.get(tabs.selected) == Some(&Tab::Terminal(*tab)))
                });
            let connected = session.status != Status::Offline
                && state
                    .workspace
                    .machine(&session.machine)
                    .is_some_and(|m| m.online);
            entry
                .writable
                .set(connected && (*tab != 0 || !matches!(session.status, Status::Exited { .. })));
            let bytes = state
                .session
                .terminal_bytes
                .get(&(id.clone(), *tab))
                .map(|b| b.as_slice())
                .or_else(|| {
                    state
                        .workspace
                        .terminal_bytes
                        .get(&(id.clone(), *tab))
                        .map(Vec::as_slice)
                })
                .unwrap_or(&[]);
            let appearance = (look.term_text, look.term, look.term_dim);
            let changed_appearance = entry.appearance != appearance;
            entry.appearance = appearance;
            entry.pane.update(cx, |pane, cx| {
                pane.set_connected(connected, cx);
                pane.set_visible(visible, cx);
                if bytes.len() > entry.read { pane.feed(&bytes[entry.read..], cx); entry.read = bytes.len(); }
                pane.view().update(cx, |view, cx| {
                    view.set_theme(font_theme(state.settings.preferences.font_size), cx);
                    let palette = colors(look);
                    if changed_appearance {
                        if let Err(source) = view.grid_mut().set_default_colors(&palette) {
                            self.errors.insert((id.clone(), *tab), format!("The terminal theme could not be applied: {source}. Change appearance again to retry."));
                        }
                        cx.notify();
                    }
                });
            });
        }
    }
    pub(crate) fn focus(
        &self,
        session: &SessionId,
        tab: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if let Some(entry) = self.entries.get(&(session.clone(), tab)) {
            let focus = entry.pane.read(cx).focus_handle(cx);
            window.focus(&focus, cx);
            true
        } else {
            false
        }
    }

    /// Shows or hides the pane's built-in disconnected banner.
    pub(crate) fn set_offline_banner(
        &self,
        session: &SessionId,
        tab: usize,
        show: bool,
        cx: &mut App,
    ) {
        if let Some(entry) = self.entries.get(&(session.clone(), tab)) {
            entry
                .pane
                .update(cx, |pane, cx| pane.set_offline_banner(show, cx));
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
            return div().p(px(14.)).child(error.clone()).into_any_element();
        }
        let Some(entry) = self.entries.get(&key) else {
            return div().into_any_element();
        };
        // Expose the actual rendered grid to accessibility and headless flows.
        let text = entry.pane.read(cx).view().clone().update(cx, |view, _| {
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
        });
        let notice = notice.map(|notice| {
            let pane = entry.pane.read(cx);
            let line = pane.line_height(cx);
            let top = px(10.) + line * pane.content_rows(cx) as f32 + px(8.);
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
            .child(entry.pane.clone())
            .children(notice)
            .test_support()
            .into_any_element()
    }
}
/// The cell is the font's advance rounded up to a whole pixel, so 12 pt Menlo
/// (7.2 px) would become 8 px, a tenth wider than the design. Half a point
/// smaller lands on 7 px; the line pitch stays 18 at the default setting.
fn font_theme(font: u8) -> Theme {
    let size = f32::from(font) - 0.5;
    Theme {
        font_size: size,
        line_height: size + 6.5,
        ..Default::default()
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
