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
        if self.writable.get() {
            let _ = self.tx.send(Io::Input(self.key.clone(), bytes.to_vec()));
        }
    }
    fn resize(&mut self, size: Size, _: CellSize) {
        if self.writable.get() {
            let _ = self
                .tx
                .send(Io::Resize(self.key.clone(), size.cols, size.rows));
        }
    }
}
struct Entry {
    pane: Entity<TerminalPane>,
    read: usize,
    appearance: (u32, u32, u32),
    writable: Rc<Cell<bool>>,
    _events: Subscription,
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
                    view.groups.iter().enumerate().any(|(group, tabs)| {
                        (state.settings.preferences.split_groups || group == view.focused_group)
                            && tabs.tabs.get(tabs.selected) == Some(&Tab::Terminal(*tab))
                    })
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
    pub(crate) fn body(
        &self,
        session: &SessionId,
        tab: usize,
        group: usize,
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
            .flex_1()
            .min_h_0()
            .px(px(14.))
            .py(px(10.))
            .child(entry.pane.clone())
            .test_support()
            .into_any_element()
    }
}
fn font_theme(font: u8) -> Theme {
    Theme {
        font_size: f32::from(font),
        line_height: f32::from(font) + 5.,
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
