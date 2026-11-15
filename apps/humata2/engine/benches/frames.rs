//! Headless frame costs on the engine this is built with: what GPUI spends
//! dispatching the input, building, laying out, shaping and painting a
//! natural frame in a hidden window. The GPU and the display take no part,
//! so these are not presented-frame times; the on-screen runs are in
//! `apps/humata2/spike/gui/README.md`.
//!
//! ```sh
//! bazel run -c opt //apps/humata2/engine:frames_bench
//! bazel run -c opt //apps/humata2/engine:frames_bench_fast
//! ```
//!
//! Prints one JSON line per case, each with the backend that drew it.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;
use humata2_engine::{Backend, Resources, frame, window};

/// A sidebar row, its own view, so GPUI Fast can draw it again from the
/// last frame when nothing it shows changed.
struct Row {
    label: SharedString,
    count: usize,
}

impl Render for Row {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(28.))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .child(div().size(px(10.)).rounded_full().bg(rgb(0x5b8def)))
            .child(div().flex_1().child(self.label.clone()))
            .child(div().text_color(rgb(0x8a93a3)).child(self.count.to_string()))
    }
}

/// 200 rows, each a view: the steady screen of an app at rest.
struct Sidebar {
    rows: Vec<Entity<Row>>,
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1e2128))
            .text_color(rgb(0xd7dae0))
            .text_size(px(13.))
            .children(self.rows.iter().cloned())
    }
}

/// A 5,000-row tree: 200 channels of 24 sessions each, all open, in a
/// virtualised `uniform_list`.
struct Tree;

impl Render for Tree {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("tree")
            .size_full()
            .bg(rgb(0x1e2128))
            .text_color(rgb(0xd7dae0))
            .text_size(px(13.))
            .child(
                uniform_list(
                    "tree-rows",
                    5_000,
                    cx.processor(|_, range: std::ops::Range<usize>, _, _| {
                        range
                            .map(|index| {
                                let (channel, session) = (index / 25, index % 25);
                                let label = if session == 0 {
                                    format!("channel-{channel}")
                                } else {
                                    format!("session {session} · fix the ledger rounding")
                                };
                                div()
                                    .id(index)
                                    .h(px(28.))
                                    .pl(px(if session == 0 { 8. } else { 24. }))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().size(px(8.)).rounded_full().bg(rgb(0x5b8def)))
                                    .child(label)
                            })
                            .collect()
                    }),
                )
                .size_full(),
            )
    }
}

/// A 20,000-line diff in a virtualised `list` that measures every row,
/// with about 8% of lines 180 to 420 characters long and wrapping on.
struct Diff {
    lines: Vec<SharedString>,
    list: ListState,
}

impl Diff {
    fn new() -> Self {
        let lines: Vec<SharedString> = (0..20_000usize)
            .map(|index| {
                let sign = ["+", "-", " "][index % 3];
                if index % 12 == 0 {
                    let words = 30 + (index * 7) % 40;
                    let long = (0..words)
                        .map(|word| ["ledger", "posting", "balance", "currency"][word % 4])
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{sign} {long}").into()
                } else {
                    format!("{sign}     let total_{index} = debit_{index} - credit_{index};")
                        .into()
                }
            })
            .collect();
        let list = ListState::new(lines.len(), ListAlignment::Top, px(400.));
        Self { lines, list }
    }
}

impl Render for Diff {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("diff")
            .size_full()
            .bg(rgb(0x1e2128))
            .text_color(rgb(0xd7dae0))
            .text_size(px(12.))
            .font_family("Menlo")
            .child(
                list(
                    self.list.clone(),
                    cx.processor(|this, index: usize, _, _| {
                        let line = this.lines[index].clone();
                        let background = match line.as_bytes().first() {
                            Some(b'+') => rgb(0x1f3a2a),
                            Some(b'-') => rgb(0x3d1f24),
                            _ => rgb(0x1e2128),
                        };
                        div()
                            .w_full()
                            .px_2()
                            .bg(background)
                            .child(line)
                            .into_any_element()
                    }),
                )
                .size_full(),
            )
    }
}

fn open<V: Render + 'static>(
    cx: &mut HeadlessAppContext,
    width: f32,
    height: f32,
    build: impl FnOnce(&mut Context<V>) -> V + 'static,
) -> AnyWindowHandle {
    let opened = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(width), px(height)),
            })),
            show: false,
            focus: false,
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(build))
    });
    let (handle, _) = match opened {
        Ok(opened) => opened,
        Err(error) => panic!("the bench window didn't open: {error}"),
    };
    cx.run_until_parked();
    handle.into()
}

/// The resident set size of this process in KiB, from `ps`, or `None` when
/// it can't be read.
fn rss_kib() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.
}

fn report(
    cx: &mut HeadlessAppContext,
    handle: AnyWindowHandle,
    case: &str,
    mut times: Vec<Duration>,
) {
    times.sort();
    let at = |q: f64| {
        let last = times.len().saturating_sub(1);
        times.get((last as f64 * q).round() as usize).copied().unwrap_or_default()
    };
    let counts = frame::in_window(cx, handle, |window, _| window::frame_counts(window))
        .ok()
        .flatten();
    let counts = counts.map_or_else(
        || "null".to_owned(),
        |counts| {
            format!(
                "{{\"views_built\":{},\"views_reused\":{},\"lines_shaped\":{},\"nodes_reused\":{}}}",
                counts.views_built, counts.views_reused, counts.lines_shaped, counts.nodes_reused
            )
        },
    );
    println!(
        "{{\"bench\":\"frames\",\"case\":\"{case}\",\"backend\":\"{}\",\"headless\":true,\"frames\":{},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3},\"rss_kib\":{},\"counts\":{counts}}}",
        Backend::current(),
        times.len(),
        ms(at(0.5)),
        ms(at(0.95)),
        ms(at(0.99)),
        ms(at(1.)),
        rss_kib().map_or_else(|| "null".to_owned(), |rss| rss.to_string()),
    );
}

/// Times `frames` runs of `step` in `handle`, after a warm-up of 10 that
/// isn't counted, with the window's counters reset between the two.
fn time(
    cx: &mut HeadlessAppContext,
    handle: AnyWindowHandle,
    frames: usize,
    mut step: impl FnMut(&mut Window, &mut App, usize),
) -> Vec<Duration> {
    let mut run = |cx: &mut HeadlessAppContext, index: usize| {
        let started = Instant::now();
        if let Err(error) = frame::in_window(cx, handle, |window, cx| step(window, cx, index)) {
            panic!("the bench window closed: {error}");
        }
        started.elapsed()
    };
    for index in 0..10 {
        run(cx, index);
    }
    if let Err(error) = frame::in_window(cx, handle, |window, _| window::reset_frame_counts(window))
    {
        panic!("the bench window closed: {error}");
    }
    (10..10 + frames).map(|index| run(cx, index)).collect()
}

fn main() {
    let mut cx = match humata2_engine::headless(&Resources::new(())) {
        Ok(cx) => cx,
        Err(error) => panic!("{error}"),
    };
    let rows = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let sidebar = {
        let rows = rows.clone();
        open(&mut cx, 320., 900., move |cx| {
            let made: Vec<Entity<Row>> = (0..200)
                .map(|index| {
                    cx.new(|_| Row {
                        label: format!("session {index} · fix the ledger rounding").into(),
                        count: index,
                    })
                })
                .collect();
            rows.borrow_mut().clone_from(&made);
            Sidebar { rows: made }
        })
    };
    let times = time(&mut cx, sidebar, 300, |window, cx, _| {
        frame::natural_frame(window, cx);
    });
    report(&mut cx, sidebar, "idle, 200 row views, nothing changed", times);
    let rows = rows.borrow().clone();
    let times = time(&mut cx, sidebar, 300, |window, cx, index| {
        if let Some(row) = rows.get(index % rows.len()) {
            row.update(cx, |row, cx| {
                row.count += 1;
                cx.notify();
            });
        }
        frame::natural_frame(window, cx);
    });
    report(&mut cx, sidebar, "200 row views, one changed a frame", times);

    let tree = open(&mut cx, 320., 900., |_| Tree);
    let at = point(px(160.), px(450.));
    let times = time(&mut cx, tree, 690, |window, cx, _| {
        frame::scroll(window, at, point(px(0.), px(-200.)), cx);
    });
    report(&mut cx, tree, "5,000-row tree, scrolling 200 px a frame", times);

    let diff = open(&mut cx, 1000., 900., |_| Diff::new());
    let at = point(px(500.), px(450.));
    let times = time(&mut cx, diff, 1_500, |window, cx, _| {
        frame::scroll(window, at, point(px(0.), px(-200.)), cx);
    });
    report(&mut cx, diff, "20,000-line diff, wrapping, scrolling 200 px a frame", times);
}
