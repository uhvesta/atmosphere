//! A small real-terminal test host. Session and embed crates have no dependency on this app.
mod appearance;
use appearance::Look;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    component::input::{Input, InputState},
    prelude::*,
    *,
};
use std::{collections::BTreeMap, time::Duration};
use terminal_embed::EmbeddedTerminal;
use terminal_session::{Attached, Endpoint, Record, SavedSession, SessionSpec, SessionStore};
use tokio::{runtime::Handle, sync::mpsc};

gpui_kit::assets::icon_assets!(
    Icons,
    [WifiOff, Terminal, PanelLeft, Sun, Moon, Server, Plus]
);
pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<std::borrow::Cow<'static, [u8]>>> {
        Icons.load(path)
    }
    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        Icons.list(path)
    }
}
// The test host uses gpui-kit's bundled assets (including its control icons).
#[derive(Clone)]
struct DragTerminal {
    id: u64,
    title: String,
}
impl Render for DragTerminal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::new(cx.theme().is_dark());
        div()
            .p(px(8.))
            .rounded(px(6.))
            .bg(rgb(look.card))
            .text_color(rgb(look.text))
            .border_1()
            .border_color(rgb(look.line))
            .child(self.title.clone())
    }
}

enum Command {
    Create(SessionSpec, usize),
    Resume(u64),
    Move(u64, usize, usize),
    Select(usize, u64),
}
enum Event {
    Layout(Vec<SavedSession>, [Vec<u64>; 2], [Option<u64>; 2]),
    Opened(
        u64,
        SessionSpec,
        Option<Attached>,
        Vec<Record>,
        Option<String>,
    ),
    Error(String),
}
async fn attach(store: &SessionStore, id: u64, tx: &mpsc::Sender<Event>) {
    let Ok(session) = store.session(id) else {
        return;
    };
    let spec = session.spec.clone();
    let (attached, history, error) = match store.attach(id).await {
        Ok(a) => (Some(a), Vec::new(), None),
        Err(e) => match store.history(id) {
            Ok(history) => (None, history, Some(e.to_string())),
            Err(history_error) => (
                None,
                Vec::new(),
                Some(format!(
                    "{e} Saved output could not be restored: {history_error}"
                )),
            ),
        },
    };
    let _ = tx
        .send(Event::Opened(id, spec, attached, history, error))
        .await;
}
pub struct Lab {
    tx: mpsc::Sender<Command>,
    rx: mpsc::Receiver<Event>,
    runtime: Handle,
    sessions: Vec<SavedSession>,
    groups: [Vec<u64>; 2],
    selected: [Option<u64>; 2],
    terminals: BTreeMap<u64, Entity<EmbeddedTerminal>>,
    text: Entity<InputState>,
    error: Option<String>,
    window: AnyWindowHandle,
    focused: usize,
    dark: bool,
    sidebar: bool,
    split: bool,
    font_size: f32,
    ssh: Option<Endpoint>,
    cwd: std::path::PathBuf,
    titles: BTreeMap<u64, String>,
    _worker: tokio::task::JoinHandle<()>,
}
impl Drop for Lab {
    fn drop(&mut self) {
        self._worker.abort();
    }
}
impl Lab {
    pub fn new(
        mut store: SessionStore,
        runtime: Handle,
        ssh: Option<Endpoint>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        gpui_kit::component::theme::Theme::change(
            gpui_kit::component::theme::ThemeMode::Dark,
            None,
            cx,
        );
        let (tx, mut commands) = mpsc::channel(128);
        let (events, rx) = mpsc::channel(128);
        let worker = runtime.spawn(async move {
            let _ = events
                .send(Event::Layout(
                    store.sessions().to_vec(),
                    store.groups().clone(),
                    store.selected(),
                ))
                .await;
            for session in store.sessions() {
                attach(&store, session.id, &events).await;
            }
            while let Some(command) = commands.recv().await {
                match command {
                    Command::Create(spec, group) => match store.create(spec) {
                        Ok(id) => {
                            if let Err(e) = store.move_to(id, group, usize::MAX) {
                                let _ = events.send(Event::Error(e.to_string())).await;
                            }
                            let _ = events
                                .send(Event::Layout(
                                    store.sessions().to_vec(),
                                    store.groups().clone(),
                                    store.selected(),
                                ))
                                .await;
                            attach(&store, id, &events).await;
                        }
                        Err(e) => {
                            let _ = events
                                .send(Event::Layout(
                                    store.sessions().to_vec(),
                                    store.groups().clone(),
                                    store.selected(),
                                ))
                                .await;
                            let _ = events.send(Event::Error(e.to_string())).await;
                        }
                    },
                    Command::Resume(id) => attach(&store, id, &events).await,
                    Command::Select(group, id) => {
                        if let Err(e) = store.select(group, id) {
                            let _ = events.send(Event::Error(e.to_string())).await;
                        }
                    }
                    Command::Move(id, g, i) => match store.move_to(id, g, i) {
                        Ok(()) => {
                            let _ = events
                                .send(Event::Layout(
                                    store.sessions().to_vec(),
                                    store.groups().clone(),
                                    store.selected(),
                                ))
                                .await;
                        }
                        Err(e) => {
                            let _ = events.send(Event::Error(e.to_string())).await;
                        }
                    },
                }
            }
        });
        let text = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Text to paste into the selected terminal")
        });
        // Polling is deliberately local to this experiment; host apps can use their own wake mechanism.
        cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if weak.update(cx, |lab, cx| lab.poll(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        Self {
            tx,
            rx,
            runtime,
            sessions: Vec::new(),
            groups: [Vec::new(), Vec::new()],
            selected: [None, None],
            terminals: BTreeMap::new(),
            titles: BTreeMap::new(),
            text,
            error: None,
            window: window.window_handle(),
            focused: 0,
            dark: true,
            sidebar: true,
            split: false,
            font_size: 13.,
            ssh,
            cwd: std::env::current_dir().unwrap_or_else(|_| "/tmp".into()),
            _worker: worker,
        }
    }
    fn send(&mut self, c: Command, cx: &mut Context<Self>) {
        if self.tx.try_send(c).is_err() {
            self.error=Some("The terminal operation was not queued. Wait for the current operation, then retry.".into());
            cx.notify();
        }
    }
    pub fn create_local(&mut self, cx: &mut Context<Self>) {
        self.create(Endpoint::Local, cx);
    }
    fn create(&mut self, endpoint: Endpoint, cx: &mut Context<Self>) {
        let remote = !matches!(endpoint, Endpoint::Local);
        let cwd = if remote {
            "/tmp".into()
        } else {
            self.cwd.clone()
        };
        let program = if remote {
            "/bin/sh".into()
        } else {
            std::env::var("SHELL").unwrap_or("/bin/zsh".into()).into()
        };
        self.send(
            Command::Create(
                SessionSpec {
                    title: if remote {
                        "SSH shell".into()
                    } else {
                        "Local shell".into()
                    },
                    program,
                    args: vec!["-i".into()],
                    cwd,
                    cols: 100,
                    rows: 30,
                    endpoint,
                },
                self.focused,
            ),
            cx,
        );
    }
    pub fn poll(&mut self, cx: &mut Context<Self>) {
        for terminal in self.terminals.values() {
            terminal.update(cx, |t, cx| t.poll(cx));
        }
        for _ in 0..128 {
            match self.rx.try_recv() {
                Ok(Event::Layout(s, g, selected)) => {
                    self.selected = selected;
                    self.sessions = s;
                    self.groups = g;
                    self.split |= !self.groups[1].is_empty();
                    for n in 0..2 {
                        if !self.selected[n].is_some_and(|id| self.groups[n].contains(&id)) {
                            self.selected[n] = self.groups[n].first().copied();
                        }
                    }
                    cx.notify();
                }
                Ok(Event::Opened(id, spec, a, history, error)) => {
                    self.error = error;
                    let runtime = self.runtime.clone();
                    match cx.update_window(self.window, |_, window, cx| {
                        EmbeddedTerminal::new(&spec, a, history, &runtime, window, cx)
                    }) {
                        Ok(Ok(terminal)) => {
                            let look = Look::new(self.dark);
                            if let Err(error) = terminal.update(cx, |terminal, cx| {
                                terminal.set_appearance(
                                    {
                                        let mut appearance = look.terminal();
                                        appearance.font_size = self.font_size;
                                        appearance.line_height = (self.font_size * 1.46).round();
                                        appearance
                                    },
                                    cx,
                                )
                            }) {
                                self.error = Some(error.to_string());
                            }
                            let pane = terminal.read(cx).pane().clone();
                            cx.subscribe(
                                &pane,
                                move |this, _, event: &terminal_pane::PaneEvent, cx| {
                                    if let terminal_pane::PaneEvent::TitleChanged(title) = event {
                                        this.titles.insert(
                                            id,
                                            title
                                                .chars()
                                                .filter(|c| !c.is_control())
                                                .take(120)
                                                .collect(),
                                        );
                                        cx.notify();
                                    }
                                    this.pane_event(event, cx)
                                },
                            )
                            .detach();
                            cx.subscribe(
                                &terminal,
                                move |this, _, _: &terminal_embed::EmbeddedEvent, cx| {
                                    if let Some(group) =
                                        this.groups.iter().position(|g| g.contains(&id))
                                    {
                                        this.focused = group;
                                    }
                                    cx.notify();
                                },
                            )
                            .detach();
                            if self.selected[self.focused] == Some(id) {
                                let to_focus = terminal.clone();
                                let _ = cx.update_window(self.window, |_, window, cx| {
                                    to_focus.update(cx, |terminal, cx| {
                                        terminal.focus_input(window, cx)
                                    });
                                });
                            }
                            self.terminals.insert(id, terminal);
                            cx.notify();
                        }
                        Ok(Err(e)) => {
                            self.error = Some(e.to_string());
                            cx.notify();
                        }
                        Err(e) => {
                            self.error = Some(e.to_string());
                            cx.notify();
                        }
                    }
                }
                Ok(Event::Error(e)) => {
                    self.error = Some(e);
                    cx.notify();
                }
                Err(_) => break,
            }
        }
    }
    fn pane_event(
        &mut self,
        event: &terminal_pane::PaneEvent,
        cx: &mut Context<Self>,
    ) {
        use terminal_pane::PaneEvent;
        match event {
            PaneEvent::SplitRequested => {
                self.split = true;
                self.focused = 1 - self.focused;
                self.create_local(cx);
            }
            PaneEvent::OpenLink(uri) => {
                let scheme = uri.split(':').next().unwrap_or_default();
                if ["https", "http", "mailto"]
                    .iter()
                    .any(|s| scheme.eq_ignore_ascii_case(s))
                    && !uri.chars().any(char::is_control)
                {
                    cx.open_url(uri);
                } else {
                    self.error=Some("This link cannot be opened here. Copy its address and open it in an appropriate application.".into());
                    cx.notify();
                }
            }
            PaneEvent::ClipboardWrite { text, .. } => {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()))
            }
            PaneEvent::PasteFiles(_) | PaneEvent::PasteImage { .. } => {
                self.error=Some("File/image upload is not implemented in this experiment. Use a path already present on the terminal's machine.".into());
                cx.notify();
            }
            PaneEvent::Failed(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
            _ => {}
        }
    }
    pub fn terminal(&self, id: u64) -> Option<Entity<EmbeddedTerminal>> {
        self.terminals.get(&id).cloned()
    }
    pub fn group(&self, index: usize) -> Option<&[u64]> {
        self.groups.get(index).map(Vec::as_slice)
    }
    pub fn move_terminal(&mut self, id: u64, g: usize, i: usize, cx: &mut Context<Self>) {
        self.send(Command::Move(id, g, i), cx);
    }
    fn active(&self) -> Option<u64> {
        self.selected[self.focused]
            .or(self.selected[0])
            .or(self.selected[1])
    }
}
#[derive(Clone)]
enum Action {
    Appearance,
    Split,
    Sidebar,
    NewLocal,
    NewSsh,
    Select(u64, usize),
    Detach(u64),
    Resume(u64),
    Stop(u64),
    Move(u64, usize, usize),
    Send,
}
impl Lab {
    fn action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            Action::Split => {
                self.split = true;
                self.focused = 1 - self.focused;
                self.create_local(cx);
            }
            Action::Sidebar => {
                self.sidebar = !self.sidebar;
                cx.notify();
            }
            Action::Appearance => {
                self.dark = !self.dark;
                gpui_kit::component::theme::Theme::change(
                    if self.dark {
                        gpui_kit::component::theme::ThemeMode::Dark
                    } else {
                        gpui_kit::component::theme::ThemeMode::Light
                    },
                    None,
                    cx,
                );
                let look = Look::new(self.dark);
                for terminal in self.terminals.values() {
                    if let Err(error) = terminal.update(cx, |terminal, cx| {
                        terminal.set_appearance(
                            {
                                let mut appearance = look.terminal();
                                appearance.font_size = self.font_size;
                                appearance.line_height = (self.font_size * 1.46).round();
                                appearance
                            },
                            cx,
                        )
                    }) {
                        self.error = Some(error.to_string());
                    }
                }
                cx.notify();
            }
            Action::NewLocal => self.create_local(cx),
            Action::NewSsh => {
                if let Some(e) = &self.ssh {
                    self.create(e.clone(), cx);
                }
            }
            Action::Select(id, g) => {
                self.selected[g] = Some(id);
                self.send(Command::Select(g, id), cx);
                self.focused = g;
                if let Some(t) = self.terminals.get(&id) {
                    t.update(cx, |terminal, cx| terminal.focus_input(window, cx));
                }
                cx.notify();
            }
            Action::Detach(id) => {
                if let Some(t) = self.terminals.get(&id) {
                    t.update(cx, |t, cx| t.detach(cx));
                }
            }
            Action::Resume(id) => {
                if let Some(t) = self.terminals.get(&id) {
                    t.update(cx, |t, cx| t.detach(cx));
                }
                self.send(Command::Resume(id), cx);
            }
            Action::Stop(id) => {
                if let Some(t) = self.terminals.get(&id) {
                    t.update(cx, |t, _| t.stop());
                }
            }
            Action::Move(id, g, i) => self.move_terminal(id, g, i, cx),
            Action::Send => {
                if let Some(id) = self.active() {
                    if let Some(t) = self.terminals.get(&id) {
                        let text = self.text.read(cx).value().to_string();
                        if t.read(cx).is_writable() && !text.is_empty() {
                            t.update(cx, |t, cx| t.send_text(text, cx));
                            self.text
                                .update(cx, |input, cx| input.set_value("", window, cx));
                            t.update(cx, |terminal, cx| terminal.focus_input(window, cx));
                        } else if !t.read(cx).is_writable() {
                            self.error=Some("This terminal is detached or has exited. Resume it before sending text.".into());
                            cx.notify();
                        }
                    }
                }
            }
        }
    }
    fn shortcut(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = event.keystroke.modifiers;
        if !m.platform || m.alt || m.control {
            return;
        }
        match event.keystroke.key.as_str() {
            "t" if !m.shift => self.create_local(cx),
            "+" | "=" | "-" | "0" => {
                self.font_size = match event.keystroke.key.as_str() {
                    "-" => (self.font_size - 1.).max(9.),
                    "0" => 13.,
                    _ => (self.font_size + 1.).min(28.),
                };
                let look = Look::new(self.dark);
                for t in self.terminals.values() {
                    let mut appearance = look.terminal();
                    appearance.font_size = self.font_size;
                    appearance.line_height = (self.font_size * 1.46).round();
                    if let Err(e) = t.update(cx, |t, cx| t.set_appearance(appearance, cx)) {
                        self.error = Some(e.to_string());
                    }
                }
                cx.notify();
            }
            _ => return,
        }
        let _ = window;
        cx.stop_propagation();
    }
    fn button(
        &self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        action: Action,
        cx: &Context<Self>,
    ) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
        let look = Look::new(self.dark);
        div()
            .id(id.into())
            .test_support()
            .px(px(9.))
            .py(px(3.))
            .rounded(px(6.))
            .cursor_pointer()
            .border_1()
            .border_color(rgb(look.line))
            .bg(rgb(look.fill))
            .text_color(rgb(look.text))
            .text_size(px(12.))
            .child(label.into())
            .on_click(
                cx.listener(move |this, _, window, cx| this.action(action.clone(), window, cx)),
            )
    }
}
impl Render for Lab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::new(self.dark);
        let show_sidebar = self.sidebar && f32::from(window.viewport_size().width) >= 900.;
        let lights = div().flex().gap(px(8.)).children(
            [(0, 0xff5f57), (1, 0xfebc2e), (2, 0x28c840)]
                .into_iter()
                .map(|(index, color)| {
                    div()
                        .id(SharedString::from(format!("window-{index}")))
                        .size(px(12.))
                        .rounded_full()
                        .bg(rgb(color))
                        .cursor_pointer()
                        .on_click(move |_, window, _| match index {
                            0 => window.remove_window(),
                            1 => window.minimize_window(),
                            _ => window.zoom_window(),
                        })
                }),
        );
        let titlebar = div()
            .h(px(52.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap(px(18.))
            .px(px(20.))
            .border_b_1()
            .border_color(rgb(look.line))
            .bg(rgb(look.sidebar))
            .child(lights)
            .child(self.button("toggle-sidebar", "☷", Action::Sidebar, cx))
            .child(
                div()
                    .flex_1()
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child("Humata  /  Terminal Lab"),
            )
            .child(self.button(
                "appearance",
                if self.dark { "Light" } else { "Dark" },
                Action::Appearance,
                cx,
            ));
        let mut sidebar = div()
            .w(px(236.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .bg(rgb(look.sidebar))
            .border_r_1()
            .border_color(rgb(look.line))
            .p(px(10.))
            .gap(px(4.));
        sidebar = sidebar.child(
            div()
                .px(px(8.))
                .pt(px(12.))
                .pb(px(8.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child("Terminals"),
        );
        for session in &self.sessions {
            let id = session.id;
            let group = self
                .groups
                .iter()
                .position(|g| g.contains(&id))
                .unwrap_or(0);
            let selected = self.selected[self.focused] == Some(id);
            let running = self
                .terminals
                .get(&id)
                .is_some_and(|t| t.read(cx).is_writable());
            sidebar = sidebar.child(
                div()
                    .id(SharedString::from(format!("session-{id}")))
                    .test_support()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(rgb(0x0a6cff)).text_color(rgb(0xffffff)))
                    .child(div().size(px(7.)).rounded_full().bg(rgb(if running {
                        0x30d158
                    } else {
                        0x8e8e93
                    })))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(12.))
                            .child(format!("{} {}", session.spec.title, id)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.action(Action::Select(id, group), window, cx)
                    })),
            );
        }
        sidebar = sidebar.child(div().flex_1()).child(
            div()
                .px(px(8.))
                .py(px(10.))
                .text_size(px(11.))
                .text_color(rgb(look.secondary))
                .child("Local and SSH sessions\nSaved on this machine"),
        );
        let mut heading = div()
            .min_h(px(64.))
            .flex_shrink_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .px(px(18.))
            .py(px(10.))
            .border_b_1()
            .border_color(rgb(look.line))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Terminal workspace"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(look.secondary))
                            .child("Real shells · saved output · resume where you left off"),
                    ),
            )
            .child(
                self.button("new-local", "+ Local", Action::NewLocal, cx)
                    .bg(rgb(look.accent))
                    .border_color(rgb(look.accent))
                    .text_color(rgb(0xffffff)),
            );
        heading = heading.child(self.button("split", "Split", Action::Split, cx));
        if self.ssh.is_some() {
            heading = heading.child(self.button("new-ssh", "+ SSH", Action::NewSsh, cx));
        }
        let mut groups = div()
            .flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .bg(rgb(look.line))
            .gap_px();
        for group in 0..2 {
            if group == 1 && !self.split {
                continue;
            }
            let mut tabs = div()
                .min_h(px(34.))
                .flex_shrink_0()
                .flex()
                .flex_wrap()
                .bg(rgb(look.panel))
                .border_b_1()
                .border_color(rgb(look.line));
            for (index, id) in self.groups[group].iter().copied().enumerate() {
                let name = self
                    .titles
                    .get(&id)
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("{s} · {id}"))
                    .unwrap_or_else(|| {
                        self.sessions
                            .iter()
                            .find(|s| s.id == id)
                            .map(|s| format!("{} · {}", s.spec.title, id))
                            .unwrap_or_else(|| id.to_string())
                    });
                tabs = tabs.child(
                    div()
                        .id(SharedString::from(format!("tab-{id}")))
                        .test_support()
                        .px(px(12.))
                        .py(px(7.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .border_r_1()
                        .border_color(rgb(look.line))
                        .text_size(px(12.))
                        .cursor_pointer()
                        .when(self.selected[group] == Some(id), |d| {
                            d.bg(rgb(look.terminal))
                        })
                        .child(
                            gpui_kit::component::Icon::new(gpui_kit::assets::IconName::Terminal)
                                .size(px(13.))
                                .text_color(rgb(look.secondary)),
                        )
                        .max_w_full()
                        .min_w_0()
                        .overflow_hidden()
                        .child(
                            div()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(name.clone()),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.action(Action::Select(id, group), window, cx)
                        }))
                        .on_drag(DragTerminal { id, title: name }, |d, _, _, cx| {
                            cx.new(|_| d.clone())
                        })
                        .on_drop(cx.listener(move |this, d: &DragTerminal, _, cx| {
                            cx.stop_propagation();
                            this.move_terminal(d.id, group, index, cx)
                        })),
                );
            }
            let mut column = div()
                .id(SharedString::from(format!("group-{group}")))
                .test_support()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .bg(rgb(look.terminal))
                .child(tabs)
                .on_drop(cx.listener(move |this, d: &DragTerminal, _, cx| {
                    this.move_terminal(d.id, group, usize::MAX, cx)
                }));
            if let Some(id) = self.selected[group] {
                column = column.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(5.))
                        .px(px(8.))
                        .py(px(5.))
                        .bg(rgb(look.panel))
                        .border_b_1()
                        .border_color(rgb(look.line))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.))
                                .text_color(rgb(look.secondary))
                                .child(format!("Group {}", group + 1)),
                        )
                        .child(self.button(
                            format!("move-{id}"),
                            if group == 0 { "Move →" } else { "← Move" },
                            Action::Move(id, 1 - group, usize::MAX),
                            cx,
                        ))
                        .child(self.button(
                            format!("detach-{id}"),
                            "Detach",
                            Action::Detach(id),
                            cx,
                        ))
                        .child(self.button(
                            format!("resume-{id}"),
                            "Resume",
                            Action::Resume(id),
                            cx,
                        ))
                        .child(self.button(format!("stop-{id}"), "Stop", Action::Stop(id), cx)),
                );
                if let Some(terminal) = self.terminals.get(&id) {
                    column =
                        column.child(div().flex_1().min_h_0().min_w_0().child(terminal.clone()));
                } else {
                    column = column.child(
                        div()
                            .p_4()
                            .text_color(rgb(look.secondary))
                            .child("Connecting…"),
                    );
                }
            } else {
                column = column.child(
                    div()
                        .p_4()
                        .text_size(px(12.))
                        .text_color(rgb(look.secondary))
                        .child("Drag a terminal tab here to split the workspace."),
                );
            }
            groups = groups.child(column);
        }
        let workspace=div().flex().flex_col().flex_1().min_w_0().min_h_0().bg(rgb(look.panel)).child(heading)
   .when_some(self.error.clone(),|d,error|d.child(div().px_3().py_2().text_size(px(12.)).bg(rgb(if self.dark{0x3a2c12}else{0xfff4e0})).child(error)))
   .child(groups)
   .child(div().flex_shrink_0().flex().items_center().gap(px(8.)).px(px(12.)).py(px(10.)).border_t_1().border_color(rgb(look.line))
    .child(div().id("send-text-input").test_support().flex_1().child(Input::new(&self.text)))
    .child(self.button("send-text","Send text",Action::Send,cx).bg(rgb(look.accent)).border_color(rgb(look.accent)).text_color(rgb(0xffffff))))
   .child(div().flex_shrink_0().px(px(14.)).pb(px(8.)).text_size(px(11.)).text_color(rgb(look.secondary)).child("Keyboard follows terminal focus. Send text pastes; Enter runs it. Closing a view keeps its process running."));
        let body = div()
            .flex()
            .flex_1()
            .min_h_0()
            .when(show_sidebar, |d| d.child(sidebar))
            .child(workspace);
        div()
            .size_full()
            .capture_key_down(cx.listener(Self::shortcut))
            .flex()
            .flex_col()
            .bg(rgb(look.panel))
            .text_color(rgb(look.text))
            .child(titlebar)
            .child(body)
    }
}
