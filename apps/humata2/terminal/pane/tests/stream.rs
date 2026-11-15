//! A pane of 200 × 60 cells streaming an agent's output beside other views,
//! in a headless window drawn with the platform renderer (Metal on macOS),
//! on whichever engine this test is built with. Nothing is shown.
//!
//! Each chunk of output is followed by a natural frame, as a running app
//! draws one when something changed. The views beside the terminal must
//! not be built again: on ordinary GPUI they are cached views, on GPUI Fast
//! any unchanged view is drawn again from the last frame. The terminal must
//! shape only the rows the output changed.
//!
//! The rendering snapshot is per engine and per platform: the platform's
//! fonts and rasterizer draw the pixels. Record it with
//! `HUMATA2_SNAPSHOT=record bazel run //apps/humata2/terminal/pane:stream_test`
//! (or `stream_fast_test`), then look at every picture written before
//! keeping it.

#![cfg(target_os = "macos")]

use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

// `gpui_kit::*` exports a `test` attribute; keep `#[test]` the standard one.
use ::std::prelude::v1::test;
use ghostty_vt::{CellWidth, Grid as _, Size, Terminal};
use gpui_kit::prelude::*;
use gpui_kit::*;
use humata2_engine::frame::{self, Picture};
use humata2_engine::window::{frame_counts, reset_frame_counts};
use humata2_engine::{Backend, Engine, Resources};
use terminal_pane::{Pty, Script, TerminalPane};
use terminal_view::{CellSize, Theme};

const COLS: u16 = 200;
const ROWS: u16 = 60;

/// No assets: the terminal draws text and quads only.
struct NoAssets;

impl AssetSource for NoAssets {
    fn load(&self, _path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct PtyLog {
    sizes: Vec<Size>,
}

struct RecordingPty(Rc<RefCell<PtyLog>>);

impl Pty for RecordingPty {
    fn write(&mut self, _bytes: &[u8]) {}

    fn resize(&mut self, size: Size, _: CellSize) {
        self.0.borrow_mut().sizes.push(size);
    }
}

/// A view beside the terminal, such as the session list: it counts how
/// often it is built.
struct Sibling {
    name: &'static str,
    builds: Rc<Cell<usize>>,
}

impl Render for Sibling {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.builds.set(self.builds.get() + 1);
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .bg(rgb(0x1c2029))
            .text_color(rgb(0xd0d4dc))
            .children((0..30).map(|n| div().child(format!("{} {n}", self.name))))
    }
}

struct Workspace {
    sidebar: Entity<Sibling>,
    footer: Entity<Sibling>,
    pane: Entity<TerminalPane>,
    /// The terminal's box, once the cell size is known.
    terminal: Option<gpui_kit::Size<Pixels>>,
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let terminal = div().flex_none().child(self.pane.clone());
        let terminal = match self.terminal {
            Some(size) => terminal.w(size.width).h(size.height),
            None => terminal.size_full(),
        };
        div()
            .size_full()
            .flex()
            .bg(rgb(0x111318))
            .child(
                self.sidebar
                    .clone()
                    .cached(StyleRefinement::default().w(px(240.)).h_full()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(terminal)
                    .child(self.footer.clone().cached(StyleRefinement::default().w_full().h(px(80.)))),
            )
    }
}

struct Opened {
    cx: HeadlessAppContext,
    window: AnyWindowHandle,
    workspace: Entity<Workspace>,
    builds: [Rc<Cell<usize>>; 2],
    pty: Rc<RefCell<PtyLog>>,
}

/// Opens a workspace whose terminal is `cols` × `rows` cells with
/// `scrollback` lines of history, and draws it until it settles.
fn open(cols: u16, rows: u16, scrollback: usize) -> Opened {
    let mut cx = humata2_engine::headless(&Resources::new(NoAssets)).expect("the headless context starts");
    let builds = [Rc::new(Cell::new(0)), Rc::new(Cell::new(0))];
    let pty = Rc::new(RefCell::new(PtyLog::default()));
    let (window, workspace) = {
        let builds = builds.clone();
        let pty = pty.clone();
        cx.update(move |cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1900.), px(1180.)),
                })),
                show: false,
                focus: false,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                let terminal = Terminal::new(80, 24, scrollback).expect("the terminal starts");
                let pane = cx.new(|cx| TerminalPane::new(terminal, RecordingPty(pty), Theme::default(), window, cx));
                let [sidebar, footer] = builds;
                cx.new(|cx| Workspace {
                    sidebar: cx.new(|_| Sibling {
                        name: "session",
                        builds: sidebar,
                    }),
                    footer: cx.new(|_| Sibling {
                        name: "status",
                        builds: footer,
                    }),
                    pane,
                    terminal: None,
                })
            })
            .expect("the headless window opens")
        })
    };
    let mut opened = Opened {
        cx,
        window,
        workspace,
        builds,
        pty,
    };
    opened.settle();
    // Now the font is measured: size the terminal's box to the cells asked
    // for, with half a cell to spare so rounding can't lose a column.
    let cell = opened.read(|workspace, cx| workspace.pane.read(cx).view().read(cx).cell_size());
    let cell = cell.expect("a frame was drawn, so the cell size is known");
    let workspace = opened.workspace.clone();
    opened.cx.update(|cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.terminal = Some(size(
                px((f32::from(cols) + 0.5) * cell.width),
                px((f32::from(rows) + 0.5) * cell.height),
            ));
            cx.notify();
        });
    });
    opened.settle();
    opened
}

impl Opened {
    fn read<R>(&mut self, read: impl FnOnce(&Workspace, &App) -> R) -> R {
        let workspace = self.workspace.clone();
        self.cx.update(|cx| read(workspace.read(cx), cx))
    }

    fn settle(&mut self) {
        frame::settle(&mut self.cx, self.window, |cx| cx.run_until_parked(), 12).expect("the window settles");
    }

    /// Feeds program output, then draws a natural frame.
    fn feed(&mut self, bytes: &[u8]) {
        let workspace = self.workspace.clone();
        self.cx.update(|cx| {
            let pane = workspace.read(cx).pane.clone();
            pane.update(cx, |pane, cx| pane.feed(bytes, cx));
        });
        self.cx.run_until_parked();
        frame::in_window(&mut self.cx, self.window, frame::natural_frame).expect("the window is open");
    }

    fn terminal_size(&mut self) -> Size {
        self.read(|workspace, cx| workspace.pane.read(cx).view().read(cx).grid().size())
    }

    fn stats(&mut self) -> terminal_view::FrameStats {
        self.read(|workspace, cx| workspace.pane.read(cx).view().read(cx).stats())
    }

    fn row(&mut self, y: u16) -> String {
        self.read(|workspace, cx| {
            let terminal = workspace.pane.read(cx).view().read(cx).grid();
            terminal
                .row(y)
                .map(|row| {
                    row.cells
                        .iter()
                        .filter(|cell| !matches!(cell.width, CellWidth::SpacerTail | CellWidth::SpacerHead))
                        .map(|cell| if cell.text.is_empty() { " " } else { cell.text.as_str() })
                        .collect::<String>()
                })
                .unwrap_or_default()
                .trim_end()
                .to_owned()
        })
    }

    fn sibling_builds(&self) -> [usize; 2] {
        [self.builds[0].get(), self.builds[1].get()]
    }

    fn picture(&mut self) -> Picture {
        frame::in_window(&mut self.cx, self.window, |window, _| frame::pixels(window))
            .expect("the window is open")
            .expect("Metal draws off screen")
    }
}

#[test]
fn streaming_at_200_by_60_draws_only_the_terminal_and_only_its_changed_rows() {
    let mut opened = open(COLS, ROWS, 10_000);
    assert_eq!(opened.terminal_size(), Size { cols: COLS, rows: ROWS });
    assert_eq!(
        opened.pty.borrow().sizes.last(),
        Some(&Size { cols: COLS, rows: ROWS }),
        "the program was told the pane's size"
    );

    // Fill the screen first, so every later line scrolls it.
    let lines = 600;
    let output = Script::agent_stream(usize::from(ROWS) + lines).into_bytes();
    let screenful = Script::agent_stream(usize::from(ROWS)).len();
    opened.feed(&output[..screenful]);
    let before = opened.sibling_builds();
    let frames_before = opened.stats().frames;
    frame::in_window(&mut opened.cx, opened.window, |window, _| reset_frame_counts(window))
        .expect("the window is open");

    // The agent writes 600 more lines, a few at a time.
    let script = Script::new().bytes(&output[screenful..]).split_every(300);
    let chunks: Vec<Vec<u8>> = script.chunks().map(<[u8]>::to_vec).collect();
    let mut most_rows = 0;
    for chunk in &chunks {
        opened.feed(chunk);
        most_rows = most_rows.max(opened.stats().rows_rebuilt);
    }

    // Every chunk drew the terminal (the headless platform may draw a
    // frame of its own as it parks), and nothing beside it was built.
    let frames = opened.stats().frames - frames_before;
    assert!(frames >= chunks.len() as u64, "{frames} frames for {} chunks", chunks.len());
    assert_eq!(opened.sibling_builds(), before, "a view beside the terminal was built again");
    let last = usize::from(ROWS) + lines - 1;
    assert_eq!(
        opened.row(ROWS - 2),
        format!("{last:>5} streaming answer text for line {last}")
    );
    // A scroll moves every row, so a chunk may reshape the whole screen,
    // but never more than it has.
    assert!(most_rows <= u32::from(ROWS), "{most_rows} rows rebuilt in one frame");

    if Engine::CURRENT.retains_views() {
        let counts = frame::in_window(&mut opened.cx, opened.window, |window, _| frame_counts(window))
            .expect("the window is open")
            .expect("GPUI Fast counts frames");
        // One view built per chunk: the terminal.
        assert_eq!(counts.views_built, chunks.len() as u64, "{counts:?}");
        // The sidebar, the footer and their parents are drawn again from
        // the last frame each time.
        assert!(counts.views_reused >= 2 * counts.frames, "{counts:?}");
    }
}

#[test]
fn a_still_terminal_draws_no_rows() {
    let mut opened = open(COLS, ROWS, 1_000);
    opened.feed(&Script::shell_session().into_bytes());
    opened.feed(b"");
    assert_eq!(opened.stats().rows_rebuilt, 0);
}

/// Where this engine's and platform's snapshot of `name` lives, under the
/// runfiles (to compare) and under the source tree (to record).
fn snapshot_path(name: &str) -> (PathBuf, Option<PathBuf>) {
    let backend = Backend::current();
    let file = format!("{name}-{}-{}-{}.png", backend.engine.name(), backend.os, backend.arch);
    let relative = PathBuf::from("apps/humata2/terminal/pane/tests/__snapshots__").join(file);
    let source = std::env::var_os("BUILD_WORKSPACE_DIRECTORY").map(|root| PathBuf::from(root).join(&relative));
    (relative, source)
}

/// How many pixels differ in any channel by more than `tolerance`.
fn differing(a: &Picture, b: &image::RgbaImage, tolerance: u8) -> usize {
    a.rgba
        .chunks_exact(4)
        .zip(b.as_raw().chunks_exact(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(a, b)| a.abs_diff(*b) > tolerance))
        .count()
}

#[test]
fn tricky_output_draws_as_recorded() {
    let mut opened = open(60, 9, 100);
    opened.feed(&Script::tricky_rendering().into_bytes());
    opened.settle();
    let picture = opened.picture();
    // Only the terminal's corner of the window: the cells and the rows
    // around them.
    let cell = opened
        .read(|workspace, cx| workspace.pane.read(cx).view().read(cx).cell_size())
        .expect("a frame was drawn");
    let scale = picture.width as f32 / 1900.0;
    let left = (240.0 * scale) as u32;
    let width = ((60.5 * cell.width) * scale) as u32;
    let height = ((9.5 * cell.height) * scale) as u32;
    let full = image::RgbaImage::from_raw(picture.width, picture.height, picture.rgba)
        .expect("the picture is width × height RGBA pixels");
    let crop = image::imageops::crop_imm(&full, left, 0, width, height).to_image();

    let (relative, source) = snapshot_path("tricky");
    if std::env::var("HUMATA2_SNAPSHOT").as_deref() == Ok("record") {
        let source = source.expect("record with `bazel run`, which sets BUILD_WORKSPACE_DIRECTORY");
        if let Some(parent) = source.parent() {
            std::fs::create_dir_all(parent).expect("the snapshot folder can be made");
        }
        crop.save(&source).expect("the snapshot can be written");
        println!("recorded {}; look at it before keeping it", source.display());
        return;
    }
    let recorded = match image::open(&relative) {
        Ok(recorded) => recorded.to_rgba8(),
        Err(error) => panic!(
            "no snapshot at {} ({error}). Record it with HUMATA2_SNAPSHOT=record bazel run on this test, look at the picture, and keep it",
            relative.display()
        ),
    };
    assert_eq!(
        (crop.width(), crop.height()),
        recorded.dimensions(),
        "the terminal drew at another size than recorded"
    );
    let picture = Picture {
        width: crop.width(),
        height: crop.height(),
        rgba: crop.into_raw(),
    };
    let off = differing(&picture, &recorded, 2);
    assert_eq!(off, 0, "{off} pixels differ from {}", relative.display());
}
