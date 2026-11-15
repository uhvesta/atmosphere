//! Headless costs of a terminal pane of 200 × 60 cells on the engine this
//! is built with: feeding a chunk of output to `ghostty_vt`, then drawing
//! a natural frame of the window in a hidden window. The GPU and the
//! display take no part, so these are not presented-frame times.
//!
//! ```sh
//! bazel run -c opt //apps/humata2/terminal/pane:stream_bench
//! bazel run -c opt //apps/humata2/terminal/pane:stream_fast_bench
//! ```
//!
//! Prints one JSON line per case, each with the backend that drew it.

use std::borrow::Cow;
use std::time::{Duration, Instant};

use ghostty_vt::{Size, Terminal};
use gpui_kit::prelude::*;
use gpui_kit::*;
use humata2_engine::{Backend, Resources, frame, window};
use terminal_pane::{Pty, Script, TerminalPane};
use terminal_view::{CellSize, Theme};

const COLS: f32 = 200.;
const ROWS: f32 = 60.;

struct NoAssets;

impl AssetSource for NoAssets {
    fn load(&self, _path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

/// A program that reads nothing.
struct Silent;

impl Pty for Silent {
    fn write(&mut self, _bytes: &[u8]) {}

    fn resize(&mut self, _size: Size, _cell: CellSize) {}
}

/// One pane in a box of `size`, once the cell size is known, or several
/// panes splitting the window in a grid of `columns`.
struct Holder {
    panes: Vec<Entity<TerminalPane>>,
    columns: usize,
    size: Option<gpui_kit::Size<Pixels>>,
}

impl Render for Holder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if let (Some(size), [pane]) = (self.size, self.panes.as_slice()) {
            return div().flex_none().w(size.width).h(size.height).child(pane.clone());
        }
        div().size_full().flex().flex_col().children(self.panes.chunks(self.columns).map(|row| {
            div().flex().flex_1().min_h(px(0.)).children(
                row.iter()
                    .map(|pane| div().flex_1().min_w(px(0.)).h_full().overflow_hidden().child(pane.clone())),
            )
        }))
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.
}

fn report(cx: &mut HeadlessAppContext, handle: AnyWindowHandle, case: &str, bytes: usize, mut times: Vec<Duration>) {
    let total: Duration = times.iter().sum();
    times.sort();
    let at = |q: f64| {
        let last = times.len().saturating_sub(1);
        times.get((last as f64 * q).round() as usize).copied().unwrap_or_default()
    };
    let counts = frame::in_window(cx, handle, |window, _| window::frame_counts(window))
        .ok()
        .flatten()
        .map_or_else(
            || "null".to_owned(),
            |counts| {
                format!(
                    "{{\"views_built\":{},\"views_reused\":{},\"lines_shaped\":{}}}",
                    counts.views_built, counts.views_reused, counts.lines_shaped
                )
            },
        );
    let mib_per_s = bytes as f64 / (1024. * 1024.) / total.as_secs_f64().max(f64::EPSILON);
    println!(
        "{{\"bench\":\"terminal_stream\",\"case\":\"{case}\",\"backend\":\"{}\",\"headless\":true,\"frames\":{},\"bytes\":{bytes},\"mib_per_s\":{mib_per_s:.2},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"counts\":{counts}}}",
        Backend::current(),
        times.len(),
        ms(at(0.5)),
        ms(at(0.95)),
        ms(at(0.99)),
        ms(at(1.)),
    );
}

/// Feeds each chunk to every one of `panes` and draws a natural frame
/// after it, timing both.
fn stream(
    cx: &mut HeadlessAppContext,
    handle: AnyWindowHandle,
    panes: &[Entity<TerminalPane>],
    chunks: &[Vec<u8>],
) -> Vec<Duration> {
    chunks
        .iter()
        .map(|chunk| {
            let started = Instant::now();
            cx.update(|cx| {
                for pane in panes {
                    pane.update(cx, |pane, cx| pane.feed(chunk, cx));
                }
            });
            cx.run_until_parked();
            if let Err(error) = frame::in_window(cx, handle, frame::natural_frame) {
                panic!("the bench window closed: {error}");
            }
            started.elapsed()
        })
        .collect()
}

fn chunks(script: Script) -> Vec<Vec<u8>> {
    script.chunks().map(<[u8]>::to_vec).collect()
}

/// Opens a hidden window of `panes` terminals in a grid of `columns`.
fn open(cx: &mut HeadlessAppContext, panes: usize, columns: usize) -> (AnyWindowHandle, Entity<Holder>) {
    let opened = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1700.), px(1100.)),
            })),
            show: false,
            focus: false,
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let panes = (0..panes)
                .map(|_| {
                    let terminal = match Terminal::new(80, 24, 10_000) {
                        Ok(terminal) => terminal,
                        Err(error) => panic!("{error}"),
                    };
                    cx.new(|cx| TerminalPane::new(terminal, Silent, Theme::default(), window, cx))
                })
                .collect();
            cx.new(|_| Holder {
                panes,
                columns,
                size: None,
            })
        })
    });
    let opened = match opened {
        Ok(opened) => opened,
        Err(error) => panic!("the bench window didn't open: {error}"),
    };
    settle(cx, opened.0);
    opened
}

fn settle(cx: &mut HeadlessAppContext, handle: AnyWindowHandle) {
    if let Err(error) = frame::settle(cx, handle, |cx| cx.run_until_parked(), 12) {
        panic!("{error}");
    }
}

fn reset(cx: &mut HeadlessAppContext, handle: AnyWindowHandle) {
    let _ = frame::in_window(cx, handle, |window, _| window::reset_frame_counts(window));
}

fn main() {
    let mut cx = match humata2_engine::headless(&Resources::new(NoAssets)) {
        Ok(cx) => cx,
        Err(error) => panic!("{error}"),
    };
    let (handle, holder) = open(&mut cx, 1, 1);
    let panes = cx.update(|cx| holder.read(cx).panes.clone());
    let cell = cx.update(|cx| panes.first().and_then(|pane| pane.read(cx).view().read(cx).cell_size()));
    let Some(cell) = cell else {
        panic!("the first frame didn't measure the font");
    };
    cx.update(|cx| {
        holder.update(cx, |holder, cx| {
            holder.size = Some(size(px((COLS + 0.5) * cell.width), px((ROWS + 0.5) * cell.height)));
            cx.notify();
        });
    });
    settle(&mut cx, handle);
    let grid = cx.update(|cx| panes.first().map(|pane| pane.read(cx).view().read(cx).grid().size()));
    assert_eq!(grid, Some(Size { cols: 200, rows: 60 }), "the pane isn't 200 × 60 cells");

    // Warm up: fill the screen and the glyph caches.
    stream(&mut cx, handle, &panes, &chunks(Script::agent_stream(200).split_every(4096)));
    reset(&mut cx, handle);

    let lines = Script::agent_stream(20_000);
    let bytes = lines.len();
    let times = stream(&mut cx, handle, &panes, &chunks(lines.split_every(300)));
    report(&mut cx, handle, "200x60, agent stream, 300-byte chunks (about 5 lines) a frame", bytes, times);

    reset(&mut cx, handle);
    let lines = Script::agent_stream(50_000);
    let bytes = lines.len();
    let times = stream(&mut cx, handle, &panes, &chunks(lines.split_every(64 * 1024)));
    report(&mut cx, handle, "200x60, agent stream, 64 KiB chunks a frame (a flood)", bytes, times);

    reset(&mut cx, handle);
    let tricky = Script::tricky_rendering().into_bytes();
    let bytes = tricky.len() * 500;
    let times = stream(&mut cx, handle, &panes, &vec![tricky; 500]);
    report(&mut cx, handle, "200x60, wide, emoji, combining, box drawing and true color, one script a frame", bytes, times);

    reset(&mut cx, handle);
    let times = stream(&mut cx, handle, &panes, &vec![Vec::new(); 300]);
    report(&mut cx, handle, "200x60, idle, nothing new", 0, times);

    // Split workspaces (NV-28): every visible terminal streaming at once,
    // then one streaming beside still ones.
    for (count, columns) in [(2, 2), (4, 2)] {
        let (handle, holder) = open(&mut cx, count, columns);
        let panes = cx.update(|cx| holder.read(cx).panes.clone());
        let size = cx.update(|cx| panes.first().map(|pane| pane.read(cx).view().read(cx).grid().size()));
        let size = size.map_or_else(String::new, |size| format!("{}x{}", size.cols, size.rows));
        stream(&mut cx, handle, &panes, &chunks(Script::agent_stream(200).split_every(4096)));
        reset(&mut cx, handle);
        let lines = Script::agent_stream(10_000);
        let bytes = lines.len() * count;
        let times = stream(&mut cx, handle, &panes, &chunks(lines.split_every(300)));
        report(
            &mut cx,
            handle,
            &format!("{count} panes of {size}, all streaming 300-byte chunks a frame"),
            bytes,
            times,
        );
        reset(&mut cx, handle);
        let lines = Script::agent_stream(10_000);
        let bytes = lines.len();
        let times = stream(&mut cx, handle, &panes[..1], &chunks(lines.split_every(300)));
        report(
            &mut cx,
            handle,
            &format!("{count} panes of {size}, one streaming 300-byte chunks a frame"),
            bytes,
            times,
        );
    }
}
