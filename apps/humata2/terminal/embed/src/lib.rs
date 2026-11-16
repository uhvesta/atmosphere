//! Reusable GPUI terminal content backed by a real local or SSH session.
//! Layout, session selection and persistence policy belong to its host app.
use ghostty_vt::{Colors, Rgb, Size, Terminal};
use gpui_kit::{prelude::*, *};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use terminal_pane::{Pty, TerminalPane};
use terminal_session::{Attached, Record, SessionSpec, Update};
use terminal_view::{CellSize, Theme};
use tokio::{runtime::Handle, sync::mpsc, task::JoinHandle};

enum Input {
    Bytes(Vec<u8>),
    Resize(u16, u16),
    Stop,
}
enum Incoming {
    Update(Update),
    Failed(String),
    Closed,
}
struct Transport {
    enabled: Arc<AtomicBool>,
    tx: mpsc::Sender<Input>,
    error: Arc<Mutex<Option<String>>>,
}
impl Transport {
    fn push(&self, input: Input) -> bool {
        if !self.enabled.load(Ordering::Acquire) {
            return false;
        }
        if let Err(e) = self.tx.try_send(input) {
            *self
                .error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(format!(
                "Terminal input was not sent ({e}). Reconnect if disconnected; otherwise wait for pending input to drain and retry."
            ));
            return false;
        }
        true
    }
}
impl Pty for Transport {
    fn write(&mut self, bytes: &[u8]) { let _=self.write_accepted(bytes); }
    fn write_accepted(&mut self, bytes: &[u8]) -> bool {
        self.push(Input::Bytes(bytes.into()))
    }
    fn resize(&mut self, size: Size, _: CellSize) {
        self.push(Input::Resize(size.cols, size.rows));
    }
}
struct Tasks(Vec<JoinHandle<()>>);
impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

/// Appearance supplied by the embedding application, independent of its layout/state.
pub struct Appearance {
    pub background: u32,
    pub foreground: u32,
    pub font_size: f32,
    pub line_height: f32,
    pub font_family: SharedString,
    pub ansi_colors: Option<[u32; 16]>,
}
/// Focus changes emitted by the reusable terminal view.
#[derive(Clone, Debug)]
pub enum EmbeddedEvent {
    Focused,
}
impl EventEmitter<EmbeddedEvent> for EmbeddedTerminal {}
/// Host this entity anywhere. Moving it preserves its grid, selection and process.
pub struct EmbeddedTerminal {
    enabled: Arc<AtomicBool>,
    exited: bool,
    stopping: bool,
    background: u32,
    pane: Entity<TerminalPane>,
    tx: Option<mpsc::Sender<Input>>,
    rx: mpsc::Receiver<Incoming>,
    error: Arc<Mutex<Option<String>>>,
    notice: Option<String>,
    _tasks: Tasks,
    _focus: Subscription,
}
impl EmbeddedTerminal {
    pub fn new(
        spec: &SessionSpec,
        attached: Option<Attached>,
        history: Vec<Record>,
        runtime: &Handle,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<Entity<Self>, ghostty_vt::Error> {
        let mut terminal = Terminal::new(spec.cols, spec.rows, 20_000)?;
        let mut notice = None;
        let replay = attached
            .as_ref()
            .map(|a| a.history.as_slice())
            .unwrap_or(&history);
        for record in replay {
            match record {
                Record::Output { bytes, .. } => terminal.feed(bytes),
                Record::Resize { cols, rows } => terminal.resize(*cols, *rows)?,
                Record::Gap { .. } => {
                    terminal.feed(b"\x1bc\x1b[3J");
                    notice=Some("Some output was no longer retained by the holder; the terminal display was reset.".into());
                }
            }
            let _ = terminal.drain_events();
        } // Replaying output must never replay replies/clipboard side effects.
        if let Some(a) = &attached {
            terminal.resize(a.cols, a.rows)?;
        }
        let error = Arc::new(Mutex::new(None));
        let (tx, mut inputs) = mpsc::channel(256);
        let (output, rx) = mpsc::channel(256);
        let mut tasks = Vec::new();
        let connected = attached.is_some();
        let exited = attached.as_ref().is_some_and(|a| a.exited.is_some());
        let writable = connected && !exited;
        let enabled = Arc::new(AtomicBool::new(writable));
        if let Some(code) = attached.as_ref().and_then(|a| a.exited) {
            notice = Some(format!(
                "Process exited ({code}). Saved output remains available."
            ));
        }
        if let Some(a) = attached {
            let mut reader = a.reader;
            let mut writer = a.writer;
            let events = output.clone();
            tasks.push(runtime.spawn(async move {
                loop {
                    let message = match reader.next().await {
                        Ok(Some(u)) => Incoming::Update(u),
                        Ok(None) => {
                            let _ = events.send(Incoming::Closed).await;
                            break;
                        }
                        Err(e) => {
                            let _ = events.send(Incoming::Failed(e.to_string())).await;
                            break;
                        }
                    };
                    if events.send(message).await.is_err() {
                        break;
                    }
                }
            }));
            tasks.push(runtime.spawn(async move {
                while let Some(input) = inputs.recv().await {
                    let result = match input {
                        Input::Bytes(bytes) => writer.input(&bytes).await.map(|_| ()),
                        Input::Resize(c, r) => writer.resize(c, r).await,
                        Input::Stop => writer.stop().await,
                    };
                    if let Err(e) = result {
                        let _ = output.send(Incoming::Failed(e.to_string())).await;
                        break;
                    }
                }
            }));
        } else {
            notice=Some("Detached: saved output is available. Resume reconnects to the existing process; it never starts a replacement shell.".into());
        }
        let transport = Transport {
            enabled: enabled.clone(),
            tx: tx.clone(),
            error: error.clone(),
        };
        let pane =
            cx.new(|cx| TerminalPane::new(terminal, transport, Theme::default(), window, cx));
        pane.update(cx, |p, cx| p.set_connected(connected, cx));
        let focus = pane.read(cx).focus_handle(cx);
        Ok(cx.new(|cx| {
            let subscription = cx.on_focus(&focus, window, |_: &mut Self, _, cx| {
                cx.emit(EmbeddedEvent::Focused)
            });
            Self {
                enabled,
                exited,
                stopping: false,
                background: 0x000000,
                pane,
                tx: writable.then_some(tx),
                rx,
                error,
                notice,
                _tasks: Tasks(tasks),
                _focus: subscription,
            }
        }))
    }
    pub fn set_appearance(
        &mut self,
        appearance: Appearance,
        cx: &mut Context<Self>,
    ) -> Result<(), ghostty_vt::Error> {
        let mut colors = Colors {
            background: Rgb::hex(appearance.background),
            foreground: Rgb::hex(appearance.foreground),
            ..Colors::default()
        };
        if let Some(ansi) = appearance.ansi_colors {
            for (entry, color) in colors.palette.iter_mut().zip(ansi) {
                *entry = Rgb::hex(color);
            }
        }
        self.pane.update(cx, |pane, cx| {
            let view = pane.view().clone();
            view.update(cx, |view, cx| {
                view.grid_mut().set_default_colors(&colors)?;
                let mut theme = view.theme().clone();
                theme.font_family = appearance.font_family;
                theme.font_size = appearance.font_size;
                theme.line_height = appearance.line_height;
                view.set_theme(theme, cx);
                cx.notify();
                Ok::<(), ghostty_vt::Error>(())
            })
        })?;
        self.background = appearance.background;
        cx.notify();
        Ok(())
    }
    /// Focus the terminal and bind committed text input before the next event.
    pub fn focus_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pane
            .update(cx, |pane, cx| pane.focus_input(window, cx));
    }

    pub fn is_writable(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
    pub fn pane(&self) -> &Entity<TerminalPane> {
        &self.pane
    }
    /// Drain bounded background events from the UI's normal refresh/wake path.
    pub fn poll(&mut self, cx: &mut Context<Self>) {
        for _ in 0..256 {
            match self.rx.try_recv() {
                Ok(Incoming::Update(Update::Record(record))) => match record {
                    Record::Output { bytes, .. } => {
                        self.pane.update(cx, |p, cx| p.feed(&bytes, cx))
                    }
                    Record::Gap { .. } => {
                        self.pane.update(cx, |p, cx| p.feed(b"\x1bc\x1b[3J", cx));
                        self.notice = Some(
                            "Replay gap: older holder output was trimmed; display reset.".into(),
                        );
                        cx.notify();
                    }
                    Record::Resize { .. } => {}
                },
                Ok(Incoming::Update(Update::Exited(code))) => {
                    self.exited = true;
                    self.notice = Some(format!(
                        "Process exited ({code}). Saved output remains available."
                    ));
                    self.enabled.store(false, Ordering::Release);
                    self.tx = None;
                    cx.notify();
                }
                Ok(Incoming::Update(Update::Refused(message))) | Ok(Incoming::Failed(message)) => {
                    self.notice = Some(message);
                    self.disconnect(cx);
                }
                Ok(Incoming::Closed) => {
                    if !self.exited {
                        self.notice=Some(if self.stopping{"Stopped. Saved output remains available."}else{"Connection closed. Resume tries the existing holder; it does not start a new process."}.into());
                    }
                    self.enabled.store(false, Ordering::Release);
                    self.tx = None;
                    cx.notify();
                }
                Ok(Incoming::Update(Update::Written(_))) => {}
                Err(_) => break,
            }
        }
        if let Some(e) = self
            .error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            self.notice = Some(e);
            cx.notify();
        }
    }
    fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.enabled.store(false, Ordering::Release);
        self.tx = None;
        self.pane.update(cx, |p, cx| p.set_connected(false, cx));
        cx.notify();
    }
    /// Explicit programmatic text delivery uses the pane's bracketed-paste rules.
    pub fn send_text(&mut self, text: String, cx: &mut Context<Self>) {
        if self.tx.is_some() {
            self.pane.update(cx, |p, cx| p.paste_text(text, cx));
        }
    }
    pub fn detach(&mut self, cx: &mut Context<Self>) {
        self._tasks.0.drain(..).for_each(|task| task.abort());
        self.notice =
            Some("Detached. The process keeps running; Resume reconnects and catches up.".into());
        self.disconnect(cx);
    }
    pub fn stop(&mut self) {
        if let Some(tx) = &self.tx {
            if tx.try_send(Input::Stop).is_ok() {
                self.stopping = true;
            } else {
                self.notice = Some(
                    "Stop was not sent. Wait for pending input or reconnect, then try again."
                        .into(),
                );
            }
        }
    }
}
impl Render for EmbeddedTerminal {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(rgb(self.background))
            .text_color(rgb(0xdce2ee))
            .when_some(self.notice.clone(), |d, n| {
                d.child(
                    div()
                        .px_3()
                        .py_2()
                        .text_size(px(12.))
                        .bg(rgb(0x30281c))
                        .child(n),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .p(px(10.))
                    .child(self.pane.clone()),
            )
    }
}
impl Focusable for EmbeddedTerminal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.pane.read(cx).focus_handle(cx)
    }
}
