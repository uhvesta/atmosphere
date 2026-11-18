//! How long a frame of a 200×60 terminal takes: idle, with one row
//! changed (a shell echoing a key), and with every row changed (output
//! scrolling). Shapes with the platform's real text system, in a headless
//! window, so it measures planning, shaping and building the scene, not
//! the GPU.
//!
//! FAST adds an idle minute of a focused terminal whose program asked for a
//! blinking cursor, with the view's blink on and off: how many frames it
//! draws and what they cost (fast mode turns the blink off, D787).
//!
//! Run with `bazel run -c opt //apps/humata2/terminal/view:frame_bench`
//! (`:frame_bench_fast` for GPUI Fast); prints one line per case.

use std::time::{Duration, Instant};

use std::cell::Cell as Counter;
use std::rc::Rc;

use gpui_kit::{
    AppContext, Bounds, Focusable, Point, TestAppContext, TestDispatcher, WindowBounds,
    WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt;
use terminal_grid::{Cell, Color, Cursor, CursorStyle, FakeGrid, Grid, KeyEvent, Row};
use terminal_view::{TerminalHost, TerminalView, Theme};

const COLS: u16 = 200;
const ROWS: u16 = 60;
const FRAMES: usize = 300;

struct NoHost;

impl TerminalHost<FakeGrid> for NoHost {
    fn key(&mut self, _: &mut FakeGrid, _: KeyEvent<'_>) {}
}

/// A row like a colored `ls -l` or compiler output: runs of styles,
/// spaces between words.
fn busy_row(seed: usize) -> Row {
    let words = ["drwxr-xr-x", "src", "Cargo.toml", "warning:", "unused", "1.2K", "main.rs", "->"];
    let mut grid = FakeGrid::new(COLS, 1);
    let mut x = 0;
    let mut n = seed;
    while x < COLS {
        let word = words[n % words.len()];
        let style = Cell {
            fg: Color::Palette(u8::try_from(n % 16).unwrap_or(7)),
            bold: n % 5 == 0,
            ..Cell::default()
        };
        x = grid.write(x, 0, word, &style).saturating_add(1);
        n += 3;
    }
    grid.row(0).cloned().unwrap_or_default()
}

fn report(name: &str, mut times: Vec<Duration>, rows: u32) {
    times.sort();
    let at = |q: f64| times[((times.len() as f64 - 1.0) * q) as usize];
    println!(
        "{name}: median {:?}, p95 {:?}, max {:?}, rows rebuilt per frame {rows}",
        at(0.5),
        at(0.95),
        at(1.0),
    );
}

/// A focused, idle 200×60 terminal whose program asked for a blinking
/// cursor, left alone for a minute of the test clock: the frames it asks
/// for, and the time spent drawing them.
fn idle_minute(blink: bool, active: bool) {
    const MINUTE: Duration = Duration::from_secs(60);
    const STEP: Duration = Duration::from_millis(50);
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    let mut cx = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text_system);
    let mut grid = FakeGrid::new(COLS, ROWS);
    for y in 0..ROWS {
        grid.set_row(y, busy_row(usize::from(y)));
    }
    grid.set_cursor_style(CursorStyle {
        blinking: true,
        ..CursorStyle::default()
    });
    grid.set_cursor(Cursor {
        x: 0,
        y: ROWS - 1,
        visible: true,
    });
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1700.), px(1100.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| TerminalView::new(grid, NoHost, Theme::default(), window, cx))
        })
        .unwrap_or_else(|error| panic!("the bench window didn't open: {error}"))
    });
    let asked = Rc::new(Counter::new(0_u32));
    let _watch = cx.update(|cx| {
        let asked = asked.clone();
        cx.observe(&view, move |_, _| asked.set(asked.get() + 1))
    });
    let opened = cx.update_window(handle, |_, window, cx| {
        window.activate_window();
        view.update(cx, |view, cx| view.set_cursor_blink(blink, cx));
    });
    if let Err(error) = opened {
        panic!("the bench window closed early: {error}");
    }
    cx.run_until_parked();
    let focused = cx.update_window(handle, |_, window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.render_frame(cx);
    });
    if let Err(error) = focused {
        panic!("the bench window closed early: {error}");
    }
    if !active {
        // Another app in front: the window keeps its focused view.
        gpui_kit::VisualTestContext::from_window(handle.into(), &cx).deactivate_window();
        let drawn = cx.update_window(handle, |_, window, cx| window.render_frame(cx));
        if let Err(error) = drawn {
            panic!("the bench window closed early: {error}");
        }
    }
    cx.run_until_parked();
    asked.set(0);
    let mut frames = 0_u32;
    let mut drawing = Duration::ZERO;
    let mut waited = Duration::ZERO;
    while waited < MINUTE {
        cx.executor().advance_clock(STEP);
        cx.run_until_parked();
        waited += STEP;
        if asked.get() > frames {
            frames = asked.get();
            let drawn = cx.update_window(handle, |_, window, cx| {
                let started = Instant::now();
                window.render_frame(cx);
                started.elapsed()
            });
            drawing += drawn.unwrap_or_else(|error| panic!("the bench window closed early: {error}"));
        }
    }
    println!(
        "idle minute, focused, blinking cursor asked, window {}, view blink {}: frames {frames}, drawing {drawing:?}",
        if active { "active" } else { "in the background" },
        if blink { "on" } else { "off (fast mode)" },
    );
}

fn main() {
    idle_minute(true, true);
    idle_minute(false, true);
    idle_minute(true, false);
    let text_system = gpui_kit::platform::current_platform(true).text_system();
    let mut cx = TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text_system);

    let mut grid = FakeGrid::new(COLS, ROWS);
    for y in 0..ROWS {
        grid.set_row(y, busy_row(usize::from(y)));
    }
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1700.), px(1100.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| TerminalView::new(grid, NoHost, Theme::default(), window, cx))
        })
        .unwrap_or_else(|error| panic!("the bench window didn't open: {error}"))
    });

    let result = cx.update_window(handle, |_, window, cx| {
        let started = Instant::now();
        window.render_frame(cx);
        println!("first frame (every row, fonts loaded): {:?}", started.elapsed());

        let mut idle = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let started = Instant::now();
            window.render_frame(cx);
            idle.push(started.elapsed());
        }
        report("idle", idle, view.read(cx).stats().rows_rebuilt);

        let mut one = Vec::with_capacity(FRAMES);
        for frame in 0..FRAMES {
            view.update(cx, |view, cx| {
                let row = busy_row(frame);
                view.grid_mut().set_row(u16::try_from(frame % 60).unwrap_or(0), row);
                cx.notify();
            });
            let started = Instant::now();
            window.render_frame(cx);
            one.push(started.elapsed());
        }
        report("one row changed", one, view.read(cx).stats().rows_rebuilt);

        let mut all = Vec::with_capacity(FRAMES);
        for frame in 0..FRAMES {
            view.update(cx, |view, cx| {
                view.grid_mut().scroll_up(busy_row(frame));
                cx.notify();
            });
            let started = Instant::now();
            window.render_frame(cx);
            all.push(started.elapsed());
        }
        report("every row changed", all, view.read(cx).stats().rows_rebuilt);
        let stats = view.read(cx).stats();
        println!(
            "pieces shaped in the last frame {}, paint failures {}",
            stats.pieces_shaped, stats.paint_failures
        );
    });
    if let Err(error) = result {
        panic!("the bench window closed early: {error}");
    }
}
