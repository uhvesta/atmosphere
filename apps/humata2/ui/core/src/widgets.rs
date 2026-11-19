//! Small building blocks every screen uses: rows and columns, text styles,
//! buttons, pills, status dots and empty states. Each control that does
//! something carries a role and a label, so a person with a screen reader,
//! a flow test and a recording all find it by what it says.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::progress::OpProgress;

use crate::bridge::Ui;
use crate::clock::{Now, ticked_now, until_next};
use crate::theme::{self, Look, color};

/// A horizontal flex row, items centered.
#[must_use]
pub fn row() -> Div {
    div().flex().flex_row().items_center()
}

/// A vertical flex column.
#[must_use]
pub fn col() -> Div {
    div().flex().flex_col()
}

/// A button that sends `action`.
#[must_use]
pub fn send(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .label(label)
        .small()
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

/// A push button as the frames draw it: 24 pt high, radius 6, with a 12 pt
/// medium label. The kit draws a `.label()` at its own smaller size, so the
/// label is a child and the button carries it as its accessibility label.
#[must_use]
pub fn push(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    let label = label.into();
    Button::new(id)
        .accessibility_label(label.clone())
        .small()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .child(div().text_size(px(12.)).child(label))
}

/// The frames' grey push button with a hairline. It has no click handler yet.
#[must_use]
pub fn push_quiet_shape(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    look: theme::Look,
) -> Button {
    push(id, label)
        .bg(rgb(look.fill))
        .border_color(rgb(look.line))
        .text_color(rgb(look.text))
}

/// The frames' filled push button. It has no click handler yet.
#[must_use]
pub fn push_primary_shape(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    look: theme::Look,
) -> Button {
    push(id, label)
        .primary()
        .bg(rgb(look.accent))
        .text_color(rgb(0xffffff))
}

/// The frames' grey push button, sending `action`. The click stops here, so
/// it can sit in a row that is itself clickable.
#[must_use]
pub fn push_quiet(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: theme::Look,
) -> Button {
    let ui = ui.clone();
    push_quiet_shape(id, label, look).on_click(move |_, _, cx| {
        cx.stop_propagation();
        ui.send(action.clone(), cx);
    })
}

/// The frames' filled push button, sending `action`. The click stops here.
#[must_use]
pub fn push_primary(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: theme::Look,
) -> Button {
    let ui = ui.clone();
    push_primary_shape(id, label, look).on_click(move |_, _, cx| {
        cx.stop_propagation();
        ui.send(action.clone(), cx);
    })
}

/// `primary`, drawn as the frames draw a filled button: accent fill, white label.
#[must_use]
pub fn primary_in(
    look: theme::Look,
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
) -> Button {
    push_primary(ui, id, label, action, look)
}

/// `ghost`, drawn as the frames draw a grey button with a hairline.
#[must_use]
pub fn ghost_in(
    look: theme::Look,
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
) -> Button {
    push_quiet(ui, id, label, action, look)
}

/// `title`, with its text in `look`.
#[must_use]
pub fn title_in(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color(look.text))
        .child(text.into())
}

/// `dim`, with its text in `look`.
#[must_use]
pub fn dim_in(look: Look, text: impl Into<SharedString>) -> Div {
    div().text_color(color(look.secondary)).child(text.into())
}

/// `faint`, with its text in `look`.
#[must_use]
pub fn faint_in(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .text_color(color(look.tertiary))
        .child(text.into())
}

/// `empty`, with its text in `look`.
#[must_use]
pub fn empty_in(
    look: Look,
    what: impl Into<SharedString>,
    next: impl Into<SharedString>,
) -> Div {
    col()
        .items_center()
        .gap_1()
        .py_6()
        .child(dim_in(look, what))
        .child(faint_in(look, next))
}

/// `error_banner`, filled with `look`'s error tint and outlined in red.
#[must_use]
pub fn error_banner_in(
    look: Look,
    ui: &Rc<Ui>,
    id: &'static str,
    text: impl Into<SharedString>,
    dismiss: AppAction,
) -> impl IntoElement {
    let text = text.into();
    row()
        .id(id)
        .role(Role::Alert)
        .aria_label(text.clone())
        .flex_shrink_0()
        .items_start()
        .gap_3()
        .px_3()
        .py_2()
        .rounded(px(6.))
        .bg(color(look.del))
        .border_1()
        .border_color(color(theme::RED))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(color(look.text))
                .child(text),
        )
        .child(
            div().flex_shrink_0().child(
                ghost_in(look, ui, (id, 1usize), "Dismiss", dismiss).text_color(color(look.text)),
            ),
        )
        .test_support()
}

/// Nothing draws for an operation's first this-many ms, so a quick one never
/// flickers.
pub const SHOW_AFTER_MS: i64 = 300;
/// The elapsed time shows once an operation has run this long.
pub const ELAPSED_AFTER_MS: i64 = 10_000;
/// One lap of the unmeasured sweep.
const SWEEP_MS: u64 = 1_600;
/// The sweep's segment is a quarter of the track wide.
const SWEEP_SHARE: f32 = 0.25;
/// Where the sweep sits in a still frame (goldens and the zero-delta frame).
const SWEEP_REST: f32 = 0.37;

/// What the progress line draws for one operation at one moment.
#[derive(Debug, PartialEq)]
pub struct ProgressDraw {
    /// 0..=1 when measured; `None` draws the sweep.
    pub fraction: Option<f32>,
    /// The label as the operation reports it: "42%", or empty.
    pub label: String,
    /// "14s", "2m 10s" or "1h 5m"; `None` before the elapsed time applies.
    pub elapsed: Option<String>,
}

/// What the progress line shows for `progress` at `now_ms` (unix ms). `None`
/// while the operation has run under 300 ms, and also when a start time in
/// the future makes its run time zero. With `now_ms` `None` (the start time
/// is not known) it always draws and never shows the elapsed time.
pub fn progress_draw(progress: &OpProgress, now_ms: Option<i64>) -> Option<ProgressDraw> {
    let run_ms = match now_ms {
        Some(now) => {
            let run = now.saturating_sub(progress.started_ms).max(0);
            if run < SHOW_AFTER_MS {
                return None;
            }
            Some(run)
        }
        None => None,
    };
    Some(ProgressDraw {
        fraction: progress.fraction.map(clamp_fraction),
        label: progress.label.clone(),
        elapsed: run_ms
            .filter(|run| *run >= ELAPSED_AFTER_MS)
            .map(elapsed_text),
    })
}

/// A fraction within 0..=1; anything not finite reads as 0.
fn clamp_fraction(fraction: f32) -> f32 {
    if fraction.is_finite() {
        fraction.clamp(0., 1.)
    } else {
        0.
    }
}

/// A run time as the progress line reads it: "14s", "2m 10s", "1h 5m".
pub fn elapsed_text(ms: i64) -> String {
    let secs = ms.max(0) / 1_000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3_600, (secs % 3_600) / 60)
    }
}

/// The size and colours of a thin bar. `width: None` takes the full width
/// the bar is given.
#[derive(Clone, Copy)]
pub struct BarMetrics {
    pub width: Option<Pixels>,
    pub height: Pixels,
    pub radius: Pixels,
    pub track: u32,
    pub fill: u32,
}

impl BarMetrics {
    /// The progress line's bar: full width, 2 px, the border colour under an
    /// accent fill.
    pub fn line(look: Look) -> Self {
        Self {
            width: None,
            height: px(2.),
            radius: px(1.),
            track: look.line,
            fill: look.accent,
        }
    }
}

/// A thin bar: a track in `metrics.track` with a fill in `metrics.fill`.
/// A measured `fraction` fills that share of the track. Unmeasured (`None`)
/// a quarter-track segment sweeps left to right every 1.6 s; `id` keeps that
/// animation's state with its own bar, so it must be stable per call site.
pub fn progress_bar(
    _look: Look,
    fraction: Option<f32>,
    metrics: BarMetrics,
    id: impl Into<ElementId>,
) -> AnyElement {
    let track = div()
        .flex_shrink_0()
        .h(metrics.height)
        .rounded(metrics.radius)
        .bg(rgb(metrics.track));
    let track = match metrics.width {
        Some(width) => track.w(width),
        None => track.w_full(),
    };
    let fill = metrics.fill;
    let radius = metrics.radius;
    match fraction {
        Some(fraction) => track
            .child(
                div()
                    .h_full()
                    .rounded(radius)
                    .bg(rgb(fill))
                    .w(relative(clamp_fraction(fraction))),
            )
            .into_any_element(),
        None => track
            .relative()
            .overflow_hidden()
            .with_animation(
                id,
                Animation::new(Duration::from_millis(SWEEP_MS))
                    .repeat()
                    .with_max_fps(30.),
                move |track, t| {
                    track.child(
                        div()
                            .absolute()
                            .top(px(0.))
                            .h_full()
                            .w(relative(SWEEP_SHARE))
                            .left(relative((SWEEP_REST + t) % 1.))
                            .rounded(radius)
                            .bg(rgb(fill)),
                    )
                },
            )
            .into_any_element(),
    }
}

/// The progress line: the step's title, its label and the elapsed time on
/// one row, the bar under it. Nothing draws for the first 300 ms. `now` is
/// the caller's clock (`Ui::now`); the line reads it when it ticks, at 300 ms,
/// at 10 s and then each second of the run (see [`crate::clock`]). `None`
/// draws at once, without the elapsed time.
pub fn progress_line(look: Look, progress: &OpProgress, now: Option<Now>) -> AnyElement {
    ProgressLine {
        look,
        progress: progress.clone(),
        now,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct ProgressLine {
    look: Look,
    progress: OpProgress,
    now: Option<Now>,
}

impl RenderOnce for ProgressLine {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            look,
            progress,
            now,
        } = self;
        // Keyed by the operation, so each line on screen keeps its own clock
        // and sweep.
        let key = format!("{}@{}", progress.title, progress.started_ms);
        let started_ms = progress.started_ms;
        let now_ms = now.map(|now| {
            ticked_now(
                window,
                cx,
                SharedString::from(format!("progress-clock-{key}")),
                &now,
                |at| Some(progress_next_change(started_ms, at)),
            )
        });
        let Some(draw) = progress_draw(&progress, now_ms) else {
            return div().into_any_element();
        };
        let bar = progress_bar(
            look,
            draw.fraction,
            BarMetrics::line(look),
            SharedString::from(format!("progress-sweep-{key}")),
        );
        let heading = row()
            .w_full()
            .gap(px(6.))
            .text_size(px(11.))
            .whitespace_nowrap()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(rgb(look.secondary))
                    .child(progress.title.clone()),
            )
            .children((!draw.label.is_empty()).then(|| {
                div()
                    .flex_shrink_0()
                    .text_color(rgb(look.secondary))
                    .child(draw.label)
            }))
            .children(draw.elapsed.map(|elapsed| {
                div()
                    .flex_shrink_0()
                    .text_color(rgb(look.tertiary))
                    .child(format!(" · {elapsed}"))
            }));
        col()
            .w_full()
            .gap(px(3.))
            .child(heading)
            .child(bar)
            .into_any_element()
    }
}

/// How many ms after `at` the progress line of an operation started at
/// `started_ms` next draws differently: when it appears, when its elapsed
/// time appears, then each second.
fn progress_next_change(started_ms: i64, at: i64) -> i64 {
    let run = at.saturating_sub(started_ms);
    if run < SHOW_AFTER_MS {
        SHOW_AFTER_MS - run
    } else if run < ELAPSED_AFTER_MS {
        ELAPSED_AFTER_MS - run
    } else {
        until_next(run, 1_000)
    }
}

#[cfg(test)]
mod tests {
    // Named, not `super::*`: the module's gpui glob brings in gpui's own
    // `test` attribute, which would shadow the built-in one.
    use ::core::prelude::v1::test;

    use super::{ELAPSED_AFTER_MS, SHOW_AFTER_MS, elapsed_text, progress_draw, progress_next_change};
    use humata2_clients::progress::OpProgress;

    fn op(fraction: Option<f32>, label: &str, started_ms: i64) -> OpProgress {
        OpProgress {
            title: "Cloning acme/api".to_owned(),
            fraction,
            label: label.to_owned(),
            started_ms,
        }
    }

    #[test]
    fn a_measured_fraction_is_kept_within_the_bar() {
        let at = |fraction| {
            progress_draw(&op(Some(fraction), "", 0), Some(SHOW_AFTER_MS))
                .and_then(|draw| draw.fraction)
        };
        assert_eq!(at(0.42), Some(0.42));
        assert_eq!(at(1.7), Some(1.0));
        assert_eq!(at(-0.2), Some(0.0));
        assert_eq!(at(f32::NAN), Some(0.0), "a value that is not finite reads as 0");
    }

    #[test]
    fn the_label_is_passed_through_and_an_unmeasured_step_has_no_fraction() {
        let measured = progress_draw(&op(Some(0.42), "42%", 0), Some(5_000));
        assert_eq!(measured.as_ref().map(|d| d.label.as_str()), Some("42%"));
        let unmeasured = progress_draw(&op(None, "", 0), Some(5_000));
        assert_eq!(unmeasured.as_ref().map(|d| d.fraction), Some(None));
    }

    #[test]
    fn nothing_draws_before_the_first_300_ms() {
        assert!(progress_draw(&op(Some(0.5), "50%", 1_000), Some(1_299)).is_none());
        assert!(progress_draw(&op(Some(0.5), "50%", 1_000), Some(1_300)).is_some());
    }

    #[test]
    fn a_start_time_in_the_future_has_not_begun() {
        assert!(progress_draw(&op(None, "", 5_000), Some(1_000)).is_none());
    }

    #[test]
    fn elapsed_time_shows_once_the_run_reaches_ten_seconds() {
        let started = 1_000;
        let at_9s = progress_draw(&op(None, "", started), Some(started + 9_000));
        assert_eq!(at_9s.and_then(|d| d.elapsed), None);
        let at_10s = progress_draw(&op(None, "", started), Some(started + ELAPSED_AFTER_MS));
        assert_eq!(at_10s.and_then(|d| d.elapsed).as_deref(), Some("10s"));
    }

    #[test]
    fn a_long_run_reads_in_minutes() {
        let two_minutes = progress_draw(&op(None, "", 0), Some(130_000));
        assert_eq!(two_minutes.and_then(|d| d.elapsed).as_deref(), Some("2m 10s"));
        assert_eq!(elapsed_text(14_000), "14s");
        assert_eq!(elapsed_text(3_900_000), "1h 5m");
    }

    #[test]
    fn with_no_clock_it_always_draws_and_shows_no_elapsed_time() {
        let draw = progress_draw(&op(Some(0.3), "30%", 0), None);
        assert!(draw.is_some(), "no clock, no wait");
        assert_eq!(draw.and_then(|d| d.elapsed), None);
        assert!(progress_draw(&op(None, "", i64::MAX), None).is_some());
    }

    #[test]
    fn the_line_ticks_when_it_appears_when_its_time_shows_and_each_second() {
        assert_eq!(progress_next_change(1_000, 1_100), SHOW_AFTER_MS - 100);
        assert_eq!(progress_next_change(1_000, 1_000 + SHOW_AFTER_MS), ELAPSED_AFTER_MS - SHOW_AFTER_MS);
        assert_eq!(progress_next_change(1_000, 1_000 + ELAPSED_AFTER_MS + 250), 750);
    }
}
