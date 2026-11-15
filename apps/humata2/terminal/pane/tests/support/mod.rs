//! A pane in a headless test window over a real `ghostty_vt` terminal,
//! with a pty that writes down what the program would have been sent.

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use ghostty_vt::{CellWidth, Size, Terminal};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, AppContext, Bounds, Entity, Focusable, Pixels, Point, TestAppContext,
    Window, WindowBounds, WindowOptions, point, px, size,
};
use terminal_pane::{PaneEvent, Pty, TerminalPane};
use terminal_view::{CellSize, Theme};

/// What the pane sent the program.
#[derive(Default)]
pub struct PtyLog {
    pub bytes: Vec<u8>,
    pub sizes: Vec<Size>,
}

impl PtyLog {
    /// Everything sent since the last call.
    pub fn take(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
}

pub struct RecordingPty(pub Rc<RefCell<PtyLog>>);

impl Pty for RecordingPty {
    fn write(&mut self, bytes: &[u8]) {
        self.0.borrow_mut().bytes.extend_from_slice(bytes);
    }

    fn resize(&mut self, size: Size, _: CellSize) {
        self.0.borrow_mut().sizes.push(size);
    }
}

/// What the pane told the app, in a form a test can compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Told {
    Title(String),
    Bell,
    ClipboardWrite(String),
    ClipboardRead,
    PasteImage,
    PasteFiles,
    Failed,
    Pwd(String),
    Split,
}

pub struct Opened {
    pub window: AnyWindowHandle,
    pub pane: Entity<TerminalPane>,
    pub pty: Rc<RefCell<PtyLog>>,
    pub told: Rc<RefCell<Vec<Told>>>,
}

/// Opens a pane of about 80 × 24 cells with `scrollback` lines of history,
/// activates and focuses it, and draws it until it has its size.
pub fn open(cx: &mut TestAppContext, scrollback: usize) -> Opened {
    let pty = Rc::new(RefCell::new(PtyLog::default()));
    let told = Rc::new(RefCell::new(Vec::new()));
    let (window, pane) = cx.update(|cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(720.), px(420.)),
            })),
            ..Default::default()
        };
        let pty = RecordingPty(pty.clone());
        gpui_kit::open_window(options, cx, move |window, cx| {
            let terminal = Terminal::new(80, 24, scrollback).expect("the terminal starts");
            cx.new(|cx| TerminalPane::new(terminal, pty, Theme::default(), window, cx))
        })
        .expect("the test window opens")
    });
    let log = told.clone();
    cx.update(|cx| {
        cx.subscribe(&pane, move |_, event: &PaneEvent, _| {
            log.borrow_mut().push(match event {
                PaneEvent::TitleChanged(title) => Told::Title(title.clone()),
                PaneEvent::PwdChanged(pwd) => Told::Pwd(pwd.clone()),
                PaneEvent::Bell => Told::Bell,
                PaneEvent::ClipboardWrite { text, .. } => Told::ClipboardWrite(text.clone()),
                PaneEvent::ClipboardReadRequested { .. } => Told::ClipboardRead,
                PaneEvent::PasteImage { .. } => Told::PasteImage,
                PaneEvent::PasteFiles(_) => Told::PasteFiles,
                PaneEvent::Failed(_) => Told::Failed,
                PaneEvent::SplitRequested => Told::Split,
            });
        })
        .detach();
    });
    // The test platform opens windows inactive; focus is only reported in
    // an active window.
    cx.update_window(window, |_, window, _| window.activate_window())
        .expect("the test window is open");
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        let focus = pane.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .expect("the test window is open");
    cx.run_until_parked();
    pty.borrow_mut().take();
    Opened {
        window,
        pane,
        pty,
        told,
    }
}

impl Opened {
    /// Feeds program output and draws.
    pub fn feed(&self, cx: &mut TestAppContext, bytes: &[u8]) {
        self.pane.update(cx, |pane, cx| pane.feed(bytes, cx));
        cx.run_until_parked();
        self.frame(cx, |_, _| {});
    }

    /// Runs `step` in the window and draws after it.
    pub fn frame<R>(&self, cx: &mut TestAppContext, step: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let result = cx
            .update_window(self.window, |_, window, cx| {
                let result = step(window, cx);
                window.render_frame(cx);
                result
            })
            .expect("the test window is open");
        cx.run_until_parked();
        result
    }

    /// What the program was sent since the last call.
    pub fn sent(&self) -> Vec<u8> {
        self.pty.borrow_mut().take()
    }

    pub fn told(&self) -> Vec<Told> {
        std::mem::take(&mut self.told.borrow_mut())
    }

    /// The terminal's visible text, row by row, trailing blanks trimmed.
    pub fn screen(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.pane.update(cx, |pane, cx| {
            let terminal = pane.view().read(cx).grid();
            let rows = terminal.size().rows;
            (0..rows)
                .map(|y| {
                    use ghostty_vt::Grid as _;
                    terminal
                        .row(y)
                        .map(|row| {
                            row.cells
                                .iter()
                                .filter(|cell| {
                                    !matches!(cell.width, CellWidth::SpacerTail | CellWidth::SpacerHead)
                                })
                                .map(|cell| if cell.text.is_empty() { " " } else { cell.text.as_str() })
                                .collect::<String>()
                        })
                        .unwrap_or_default()
                        .trim_end()
                        .to_owned()
                })
                .collect()
        })
    }

    /// The window position of the middle of visible cell (`x`, `y`).
    pub fn cell(&self, window: &Window, x: u16, y: u16, cx: &App) -> Point<Pixels> {
        let pane = self.pane.read(cx);
        let cell = pane.view().read(cx).cell_size().expect("a frame was drawn");
        let origin = window.find("terminal").bounds().origin;
        point(
            origin.x + px((f32::from(x) + 0.5) * cell.width),
            origin.y + px((f32::from(y) + 0.5) * cell.height),
        )
    }

    /// Whether the terminal has the keyboard.
    pub fn terminal_focused(&self, window: &Window, cx: &App) -> bool {
        self.pane.read(cx).focus_handle(cx).is_focused(window)
    }
}

/// The platform's spelling of a shortcut: `cmd` on a Mac, `ctrl-shift`
/// elsewhere (`ctrl` for keys that already have shift).
pub fn shortcut(mac: &str, other: &str) -> String {
    if cfg!(target_os = "macos") {
        mac.to_owned()
    } else {
        other.to_owned()
    }
}
