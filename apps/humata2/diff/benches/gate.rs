//! The gate for building the diff view on gpui-component's diff (D67): what
//! a frame costs on the 20,000-line fixture, wrapped, in a hidden window,
//! when the user opens it, scrolls, collapses a file, opens every fold,
//! switches between inline and side by side, and adds a comment.
//!
//! ```sh
//! bazel run -c opt //apps/humata2/diff:gate
//! bazel run -c opt //apps/humata2/diff:gate_fast
//! ```
//!
//! Prints one JSON line per case with the frame times' p50, p95 and max
//! against the 8.33 ms a frame has at 120 Hz. Collapsing, folds and the mode
//! rebuild the diff's rows on the UI thread, so their frames are the ones
//! the gate is about. The times are headless: GPUI builds, lays out, shapes
//! and paints, but nothing is presented.

use std::time::{Duration, Instant};

use gpui_kit::component::diff::{
    Diff, DiffAnnotation, DiffFile, DiffLinePosition, DiffMode, DiffSide, DiffState,
};
use gpui_kit::prelude::*;
use gpui_kit::*;
use humata2_diff_view::{Side, fixture};
use humata2_engine::{Backend, Resources, frame};

/// A frame's time at 120 Hz.
const BUDGET_MS: f64 = 1_000. / 120.;

struct Host {
    state: Entity<DiffState>,
    annotations: Vec<DiffAnnotation>,
    wrap: bool,
    syntax: bool,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            Diff::new(&self.state)
                .soft_wrap(self.wrap)
                .syntax_highlight(self.syntax)
                .annotations(self.annotations.iter().cloned())
                .render_annotation(|annotation, _, _| {
                    div()
                        .p_2()
                        .child(format!("A comment, {:?}", annotation.id()))
                        .child("It wraps onto a second line when the pane is narrow enough.")
                })
                .size_full(),
        )
    }
}

fn parse(patch: &str) -> Vec<DiffFile> {
    match DiffFile::parse(patch) {
        Ok(files) => files,
        Err(error) => panic!("the fixture's patch doesn't parse: {error:?}"),
    }
}

fn open(cx: &mut HeadlessAppContext, files: Vec<DiffFile>) -> (AnyWindowHandle, Entity<Host>) {
    let opened = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1000.), px(900.)),
            })),
            show: false,
            focus: false,
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            let state = cx.new(|cx| DiffState::new(files, cx));
            cx.new(|_| Host {
                state,
                annotations: Vec::new(),
                wrap: true,
                syntax: true,
            })
        })
    });
    let (handle, host) = match opened {
        Ok(opened) => opened,
        Err(error) => panic!("the gate's window didn't open: {error}"),
    };
    (handle.into(), host)
}

fn draw(cx: &mut HeadlessAppContext, handle: AnyWindowHandle) {
    if let Err(error) = frame::in_window(cx, handle, |window, cx| {
        frame::natural_frame(window, cx);
    }) {
        panic!("the gate's window closed: {error}");
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.
}

fn report(case: &str, mut times: Vec<Duration>) {
    times.sort();
    let at = |q: f64| {
        let last = times.len().saturating_sub(1);
        times.get((last as f64 * q).round() as usize).copied().unwrap_or_default()
    };
    let over = times.iter().filter(|time| ms(**time) > BUDGET_MS).count();
    println!(
        "{{\"bench\":\"diff_gate\",\"case\":\"{case}\",\"backend\":\"{}\",\"headless\":true,\"frames\":{},\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"max_ms\":{:.3},\"over_budget\":{over},\"budget_ms\":{BUDGET_MS:.2}}}",
        Backend::current(),
        times.len(),
        ms(at(0.5)),
        ms(at(0.95)),
        ms(at(1.)),
    );
}

/// Times `frames` runs of `step`, each with the frame it causes, after a
/// warm-up of 10 that isn't counted.
fn time(
    cx: &mut HeadlessAppContext,
    handle: AnyWindowHandle,
    frames: usize,
    mut step: impl FnMut(&mut HeadlessAppContext, usize),
) -> Vec<Duration> {
    let mut run = |cx: &mut HeadlessAppContext, index: usize| {
        let started = Instant::now();
        step(cx, index);
        draw(cx, handle);
        started.elapsed()
    };
    for index in 0..10 {
        run(cx, index);
    }
    (10..10 + frames).map(|index| run(cx, index)).collect()
}

/// Scrolls 300 frames of 200 px, from `path`, wrapped and highlighted, then
/// again without wrapping and without syntax colours, so a slow frame can be
/// put down to the lines' layout or to their colours.
fn scroll_cases(
    cx: &mut HeadlessAppContext,
    handle: AnyWindowHandle,
    host: &Entity<Host>,
    path: &str,
    case: &str,
) {
    let at = point(px(500.), px(450.));
    for (wrap, syntax, variant) in
        [(true, true, ""), (false, true, ", not wrapped"), (true, false, ", no syntax colours")]
    {
        cx.update(|cx| {
            host.update(cx, |host, cx| {
                host.wrap = wrap;
                host.syntax = syntax;
                host.state.update(cx, |state, cx| state.scroll_to_file(path, cx));
                cx.notify();
            });
        });
        draw(cx, handle);
        let times = time(cx, handle, 300, |cx, _| {
            let _ = frame::in_window(cx, handle, |window, cx| {
                frame::scroll(window, at, point(px(0.), px(-200.)), cx);
            });
        });
        report(&format!("{case}{variant}"), times);
    }
    cx.update(|cx| {
        host.update(cx, |host, cx| {
            host.wrap = true;
            host.syntax = true;
            cx.notify();
        });
    });
}

/// Splits frame times by step parity, for cases that alternate two steps.
fn split(times: Vec<Duration>) -> (Vec<Duration>, Vec<Duration>) {
    let (even, odd): (Vec<_>, Vec<_>) = times.into_iter().enumerate().partition(|(ix, _)| ix % 2 == 0);
    (even.into_iter().map(|(_, time)| time).collect(), odd.into_iter().map(|(_, time)| time).collect())
}

fn side(side: Side) -> DiffSide {
    match side {
        Side::Old => DiffSide::Original,
        Side::New => DiffSide::Modified,
    }
}

fn main() {
    let mut cx = match humata2_engine::headless(&Resources::new(())) {
        Ok(cx) => cx,
        Err(error) => panic!("{error}"),
    };
    let review = fixture::large();
    let hunks = fixture::patch_text(&review, false);
    let full = fixture::patch_text(&review, true);
    let paths: Vec<String> = review
        .repos
        .iter()
        .flat_map(|repo| repo.files.iter().map(move |file| format!("{}/{}", repo.name, file.path)))
        .collect();
    println!(
        "{{\"bench\":\"diff_gate\",\"fixture\":\"large\",\"files\":{},\"diff_lines\":{},\"patch_bytes\":{},\"full_patch_bytes\":{}}}",
        review.file_count(),
        review.line_count(),
        hunks.len(),
        full.len(),
    );

    // Opening: parsing the patch, then the frames until the first one that
    // shows the diff laid out (the body measures itself on its first paint
    // and lays out again).
    let mut parses = Vec::new();
    let mut opens = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let files = parse(&hunks);
        parses.push(started.elapsed());
        let started = Instant::now();
        let (handle, _) = open(&mut cx, files);
        for _ in 0..3 {
            draw(&mut cx, handle);
        }
        opens.push(started.elapsed());
        let _ = frame::in_window(&mut cx, handle, |window, _| window.remove_window());
    }
    report("parse the hunks' patch", parses);
    report("open: new state, window and 3 frames", opens);

    let (handle, host) = open(&mut cx, parse(&hunks));
    for _ in 0..3 {
        draw(&mut cx, handle);
    }
    let started = Instant::now();
    cx.run_until_parked();
    report("highlight every file (off the UI thread in the app)", vec![started.elapsed()]);
    let state = cx.update(|cx| host.read(cx).state.clone());

    let at = point(px(500.), px(450.));
    let times = time(&mut cx, handle, 1_500, |cx, _| {
        let _ = frame::in_window(cx, handle, |window, cx| {
            frame::scroll(window, at, point(px(0.), px(-200.)), cx);
        });
    });
    report("scroll 200 px a frame, wrapped", times);

    // Back near the top, so the file toggled is on screen.
    cx.update(|cx| state.update(cx, |state, cx| state.scroll_to_file(&paths[2], cx)));
    draw(&mut cx, handle);
    let times = time(&mut cx, handle, 200, |cx, index| {
        let path = &paths[3];
        cx.update(|cx| state.update(cx, |state, cx| state.set_file_collapsed(path, index % 2 == 0, cx)));
    });
    report("collapse and expand a file on screen", times);

    let times = time(&mut cx, handle, 200, |cx, index| {
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                let mode = if index % 2 == 0 { DiffMode::Split } else { DiffMode::Unified };
                state.set_mode(mode, cx);
            });
        });
    });
    report("switch inline and side by side", times);

    let notes = fixture::large_annotations(&review);
    let annotations: Vec<DiffAnnotation> = notes
        .iter()
        .map(|note| {
            let target = &note.target;
            DiffAnnotation::line(
                ElementId::Integer(note.id.0),
                DiffLinePosition::new(
                    format!("{}/{}", target.repo, target.path),
                    side(target.side),
                    target.lines.end as usize,
                ),
            )
        })
        .collect();
    cx.update(|cx| {
        host.update(cx, |host, cx| {
            host.annotations.clone_from(&annotations);
            cx.notify();
        });
    });
    draw(&mut cx, handle);
    let times = time(&mut cx, handle, 200, |cx, index| {
        cx.update(|cx| {
            host.update(cx, |host, cx| {
                let extra = DiffAnnotation::line(
                    ElementId::Integer(u64::MAX),
                    DiffLinePosition::new(paths[2].clone(), DiffSide::Modified, 10 + index),
                );
                host.annotations.truncate(annotations.len());
                host.annotations.push(extra);
                cx.notify();
            });
        });
    });
    report(
        &format!("move a comment, {} comments in all", annotations.len() + 1),
        times,
    );
    scroll_cases(&mut cx, handle, &host, &paths[2], "scroll 200 px a frame, with comments");

    // Every file whole, folded to three lines around each change.
    cx.update(|cx| {
        state.update(cx, |state, cx| state.set_files(parse(&full), cx));
        host.update(cx, |host, cx| {
            host.annotations.clear();
            cx.notify();
        });
    });
    for _ in 0..3 {
        draw(&mut cx, handle);
    }
    cx.run_until_parked();
    cx.update(|cx| state.update(cx, |state, cx| state.scroll_to_file(&paths[2], cx)));
    draw(&mut cx, handle);
    let times = time(&mut cx, handle, 100, |cx, index| {
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                if index % 2 == 0 {
                    state.expand_unchanged(cx);
                } else {
                    state.collapse_unchanged(cx);
                }
            });
        });
    });
    report("open and close every fold of every file", times);
    let times = time(&mut cx, handle, 100, |cx, index| {
        let path = &paths[3];
        cx.update(|cx| state.update(cx, |state, cx| state.set_file_collapsed(path, index % 2 == 0, cx)));
    });
    report("collapse and expand a file, whole files", times);
    scroll_cases(&mut cx, handle, &host, &paths[2], "scroll 200 px a frame, whole files folded");

    // What one file costs with the patch: opening one fold, and replacing
    // the file with a fresh copy of itself (a new document, so its folds
    // close again). Every other file keeps its rows.
    let fresh = [parse(&full), parse(&full)];
    let folded = review
        .repos
        .iter()
        .flat_map(|repo| &repo.files)
        .position(|file| file.hunks.first().is_some_and(|hunk| hunk.new_start > 8));
    if let Some(file) = folded {
        let path = paths[file].clone();
        cx.update(|cx| state.update(cx, |state, cx| state.scroll_to_file(&path, cx)));
        draw(&mut cx, handle);
        let times = time(&mut cx, handle, 200, |cx, index| {
            cx.update(|cx| {
                state.update(cx, |state, cx| {
                    if index % 2 == 0 {
                        state.expand_unchanged_at(&DiffLinePosition::new(path.clone(), DiffSide::Modified, 1), cx);
                    } else {
                        state.replace_file(fresh[(index / 2) % 2][file].clone(), cx);
                    }
                });
            });
        });
        let (opens, replaces) = split(times);
        report("open one fold, whole files", opens);
        report("replace one file (update_files), whole files", replaces);
    }
    let times = time(&mut cx, handle, 100, |cx, _| {
        cx.update(|cx| {
            state.update(cx, |state, cx| {
                let files = state.files().to_vec();
                state.update_files(files, cx);
            });
        });
    });
    report("update_files with nothing changed", times);
    let times = time(&mut cx, handle, 100, |cx, index| {
        cx.update(|cx| state.update(cx, |state, cx| state.set_file_collapsed(&paths[index % 40], index % 80 < 40, cx)));
    });
    report("collapse or expand a file anywhere, whole files", times);

    // What replacing the files costs without the patch: set_files drops
    // every measured height and starts again at the top.
    let hunk_files = parse(&hunks);
    let full_files = parse(&full);
    let times = time(&mut cx, handle, 40, |cx, index| {
        let files = if index % 2 == 0 { hunk_files.clone() } else { full_files.clone() };
        cx.update(|cx| state.update(cx, |state, cx| state.set_files(files, cx)));
    });
    report("replace every file with set_files (no patch)", times);
    let times = time(&mut cx, handle, 40, |cx, index| {
        let files = if index % 2 == 0 { hunk_files.clone() } else { full_files.clone() };
        cx.update(|cx| state.update(cx, |state, cx| state.update_files(files, cx)));
    });
    report("replace every file with update_files", times);
}
