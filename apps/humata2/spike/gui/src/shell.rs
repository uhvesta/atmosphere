//! The window: a tab bar over whichever screen is open, and the benchmarks
//! that drive it.
//!
//! Benchmarks tick once per frame through `on_next_frame`, which runs at
//! the start of each frame before it is drawn. The time between ticks is
//! the frame interval; the time from a tick to the [`FrameProbe`]'s paint
//! in that frame is the frame's own work.

use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::args::{Args, Bench, Screen};
use crate::bench::{self, FrameProbe, Percentiles, Usage};
use crate::diff::DiffScreen;
use crate::terminal::TerminalScreen;
use crate::tree::{TreeScreen, TreeStore};

/// Ticks to let the screen settle before the scroll benchmark records.
const WARMUP_TICKS: u32 = 10;
/// The most ticks a scroll benchmark records, in case a screen never ends.
const MAX_TICKS: usize = 100_000;

/// The window's root view.
pub struct Shell {
    args: Args,
    active: Screen,
    tree_store: Option<TreeStore>,
    tree: Option<Entity<TreeScreen>>,
    diff: Option<Entity<DiffScreen>>,
    terminal: Option<Entity<TerminalScreen>>,
    probe: FrameProbe,
    scroll: ScrollRun,
    refresh: Option<Duration>,
}

#[derive(Default)]
struct ScrollRun {
    ticks: u32,
    started: Option<Instant>,
    previous: Option<Instant>,
    intervals: Vec<Duration>,
    work: Vec<Duration>,
}

impl Shell {
    /// The shell on `args.screen`, with its benchmark, if any, started.
    pub fn new(args: Args, tree_store: TreeStore, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut shell = Self {
            active: args.screen,
            args,
            tree_store: Some(tree_store),
            tree: None,
            diff: None,
            terminal: None,
            probe: FrameProbe::default(),
            scroll: ScrollRun::default(),
            refresh: None,
        };
        shell.show(shell.active, window, cx);
        if shell.args.bench.is_some() {
            cx.on_next_frame(window, |this, window, cx| this.on_frame(window, cx));
        }
        if let Some(after) = shell.args.exit_after_ms {
            cx.spawn(async move |_, cx| {
                cx.background_executor().timer(Duration::from_millis(after)).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
        shell
    }

    /// Switches to `screen`, making it the first time it is shown.
    fn show(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        self.active = screen;
        match screen {
            Screen::Tree => {
                let tree = match (&self.tree, self.tree_store.take()) {
                    (Some(tree), _) => tree.clone(),
                    (None, Some(store)) => {
                        let tree = cx.new(|cx| TreeScreen::new(store, window, cx));
                        self.tree = Some(tree.clone());
                        tree
                    }
                    (None, None) => return,
                };
                let focus = tree.read(cx).focus_handle().clone();
                window.focus(&focus, cx);
            }
            Screen::Diff => {
                if self.diff.is_none() {
                    let wrap = self.args.wrap;
                    self.diff = Some(cx.new(|_| DiffScreen::new(wrap)));
                }
            }
            Screen::Terminal => {
                if self.terminal.is_none() {
                    let animate = self.args.animate;
                    self.terminal = Some(cx.new(|cx| TerminalScreen::new(animate, window, cx)));
                }
            }
        }
        cx.notify();
    }

    fn on_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.refresh.is_none() {
            self.refresh = window.refresh_interval(cx);
        }
        let again = match self.args.bench {
            None => false,
            Some(Bench::Start) => self.start_tick(now, cx),
            Some(Bench::Idle) => self.idle_tick(cx),
            Some(Bench::Scroll) => self.scroll_tick(now, cx),
        };
        if again {
            cx.on_next_frame(window, |this, window, cx| this.on_frame(window, cx));
        }
    }

    /// Reports the time to the first paint. False once reported.
    fn start_tick(&mut self, now: Instant, cx: &mut Context<Self>) -> bool {
        let Some(first_paint) = self.probe.first_paint() else {
            return true;
        };
        let started = bench::started();
        bench::report(&format!(
            "{{\"bench\":\"start\",\"screen\":\"{}\",\"first_paint_ms\":{:.3},\"next_frame_ms\":{:.3}}}",
            self.active.name(),
            bench::ms(first_paint.duration_since(started)),
            bench::ms(now.duration_since(started)),
        ));
        cx.quit();
        false
    }

    /// Once the first frame is painted, waits idle and reports what the wait
    /// cost. False once waiting.
    fn idle_tick(&mut self, cx: &mut Context<Self>) -> bool {
        if self.probe.first_paint().is_none() {
            return true;
        }
        let Some(before) = Usage::now() else {
            fail_usage();
            return false;
        };
        let paints = self.probe.paints();
        let began = Instant::now();
        let wait = Duration::from_millis(self.args.idle_ms);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| this.finish_idle(before, paints, began, cx));
        })
        .detach();
        false
    }

    fn finish_idle(&mut self, before: Usage, paints: u64, began: Instant, cx: &mut Context<Self>) {
        let Some(after) = Usage::now() else {
            fail_usage();
            return;
        };
        let wall = began.elapsed();
        let cpu = after.cpu.saturating_sub(before.cpu);
        let percent = if wall.is_zero() {
            0.
        } else {
            cpu.as_secs_f64() / wall.as_secs_f64() * 100.
        };
        bench::report(&format!(
            "{{\"bench\":\"idle\",\"screen\":\"{}\",\"idle_ms\":{:.3},\"cpu_ms\":{:.3},\"cpu_percent\":{:.3},\"frames_painted\":{},\"max_rss_mb\":{:.1}}}",
            self.active.name(),
            bench::ms(wall),
            bench::ms(cpu),
            percent,
            self.probe.paints().saturating_sub(paints),
            after.max_rss_bytes as f64 / (1024. * 1024.),
        ));
        cx.quit();
    }

    /// Scrolls one step and records the frame. False once reported.
    fn scroll_tick(&mut self, now: Instant, cx: &mut Context<Self>) -> bool {
        self.scroll.ticks += 1;
        let ticks = self.scroll.ticks;
        if ticks == 1 {
            self.prepare_scroll(cx);
            return true;
        }
        if ticks <= WARMUP_TICKS {
            self.step_active(cx);
            return true;
        }
        let painted = self.probe.last_paint();
        let run = &mut self.scroll;
        if let Some(previous) = run.previous {
            run.intervals.push(now.duration_since(previous));
            if let Some(painted) = painted
                && painted > previous
            {
                run.work.push(painted.duration_since(previous));
            }
        } else {
            run.started = Some(now);
        }
        run.previous = Some(now);
        let done = self.step_active(cx) || self.scroll.intervals.len() >= MAX_TICKS;
        if done {
            self.report_scroll(now, cx);
        }
        !done
    }

    /// Puts the screen in its benchmark state: the tree fully open and the
    /// terminal's output running.
    fn prepare_scroll(&mut self, cx: &mut Context<Self>) {
        match self.active {
            Screen::Tree => {
                if let Some(tree) = &self.tree {
                    tree.update(cx, |tree, cx| tree.expand_all(cx));
                }
            }
            Screen::Diff => {}
            Screen::Terminal => {
                if let Some(terminal) = &self.terminal {
                    terminal.update(cx, |terminal, cx| terminal.set_animating(true, cx));
                }
            }
        }
    }

    /// Moves the active screen one benchmark step. True when it is done.
    fn step_active(&mut self, cx: &mut Context<Self>) -> bool {
        let step = px(self.args.step_px as f32);
        match self.active {
            Screen::Tree => self.tree.as_ref().is_none_or(|tree| {
                tree.update(cx, |tree, cx| {
                    let done = tree.bench_step(step);
                    cx.notify();
                    done
                })
            }),
            Screen::Diff => self
                .diff
                .as_ref()
                .is_none_or(|diff| diff.update(cx, |diff, cx| diff.bench_step(step, cx))),
            Screen::Terminal => {
                let limit = Duration::from_secs(self.args.bench_secs);
                self.scroll
                    .started
                    .is_some_and(|started| started.elapsed() >= limit)
            }
        }
    }

    fn report_scroll(&self, now: Instant, cx: &mut Context<Self>) {
        let run = &self.scroll;
        let Some(intervals) = Percentiles::of(&run.intervals) else {
            eprintln!(
                "the scroll benchmark ended before recording a frame: the {} screen had nothing to \
                 scroll. Make the window shorter or the step smaller (--step-px) and run it again",
                self.active.name()
            );
            cx.quit();
            return;
        };
        let work = Percentiles::of(&run.work)
            .map_or_else(|| "null".to_owned(), |work| format!("{{{}}}", work.json_fields()));
        let wrap = match self.active {
            Screen::Diff => format!(",\"wrap\":{}", self.args.wrap),
            _ => String::new(),
        };
        let refresh = self
            .refresh
            .map_or_else(|| "null".to_owned(), |r| format!("{:.3}", bench::ms(r)));
        let elapsed = run.started.map_or(Duration::ZERO, |started| now.duration_since(started));
        bench::report(&format!(
            "{{\"bench\":\"scroll\",\"screen\":\"{}\"{wrap},\"step_px\":{},\"frames\":{},\"elapsed_ms\":{:.3},\"refresh_ms\":{refresh},{},\"work\":{work}}}",
            self.active.name(),
            self.args.step_px,
            run.intervals.len(),
            bench::ms(elapsed),
            intervals.json_fields(),
        ));
        cx.quit();
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let active = self.active;
        div()
            .flex()
            .flex_row()
            .gap_1()
            .px_2()
            .pt_2()
            .bg(rgb(0x14171d))
            .border_b_1()
            .border_color(rgb(0x2d3340))
            .children(Screen::ALL.into_iter().enumerate().map(|(index, screen)| {
                let selected = screen == active;
                div()
                    .id(("tab", index))
                    .test_support()
                    .role(Role::Tab)
                    .aria_label(screen.name())
                    .aria_selected(selected)
                    .px_3()
                    .py_1()
                    .rounded_t_md()
                    .cursor_pointer()
                    .text_color(if selected { rgb(0xf0f3f8) } else { rgb(0x8a93a3) })
                    .when(selected, |tab| tab.bg(rgb(0x1b1f27)))
                    .child(screen.name())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.show(screen, window, cx);
                    }))
            }))
    }
}

fn fail_usage() {
    eprintln!(
        "could not read the process's CPU time and memory (getrusage failed: {}). \
         The idle benchmark needs getrusage; run it on macOS or Linux",
        std::io::Error::last_os_error()
    );
    std::process::exit(1);
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen: AnyElement = match self.active {
            Screen::Tree => self.tree.clone().map(IntoElement::into_any_element),
            Screen::Diff => self.diff.clone().map(IntoElement::into_any_element),
            Screen::Terminal => self.terminal.clone().map(IntoElement::into_any_element),
        }
        .unwrap_or_else(|| div().into_any_element());
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1b1f27))
            .text_color(rgb(0xe6e9ef))
            .text_size(px(13.))
            .child(self.render_tabs(cx))
            .child(div().flex_1().min_h(px(0.)).child(screen))
            // Last, so it paints after the rest of the frame.
            .child(self.probe.element())
    }
}

/// Opens the spike's window.
///
/// # Errors
///
/// When the platform can't open a window.
pub fn open(args: Args, tree_store: TreeStore, cx: &mut App) -> Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1500.), px(980.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Humata 2 GUI spike".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| Shell::new(args, tree_store, window, cx))
    })?;
    cx.activate(true);
    Ok(())
}
