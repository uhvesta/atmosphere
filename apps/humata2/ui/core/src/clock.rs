//! Clocks for words that count time ("Working · 12s", "3 min").
//!
//! Such words are drawn from a clock that ticks when they next change, never
//! read at draw time: a redraw between ticks then shows the same words a
//! fresh window would, and the words turn over on time even when nothing
//! else on screen changes.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{App, ElementId, Task, Window};

/// Reads the time now, in unix ms: the app's clock (`Ui::now`), which a
/// test can fix.
pub type Now = Rc<dyn Fn() -> i64>;

/// A [`ticked_now`] clock: the time it last ticked, when it next ticks, and
/// the tick.
struct TickedClock {
    now_ms: i64,
    due_ms: Option<i64>,
    _tick: Option<Task<()>>,
}

/// The time words keyed `key` are drawn at: `now` as read when the clock
/// last ticked (or when the words were first drawn). `next_change` says how
/// many ms after a given time the words drawn at that time change, or
/// `None` while they never will; the clock ticks then and redraws the view
/// that drew it. Keep `key` stable while the same words are on screen.
pub fn ticked_now(
    window: &mut Window,
    cx: &mut App,
    key: impl Into<ElementId>,
    now: &Now,
    next_change: impl FnOnce(i64) -> Option<i64>,
) -> i64 {
    let first = now.clone();
    let clock = window.use_keyed_state(key, cx, move |_, _| TickedClock {
        now_ms: first(),
        due_ms: None,
        _tick: None,
    });
    let (now_ms, due_ms) = {
        let clock = clock.read(cx);
        (clock.now_ms, clock.due_ms)
    };
    let due = next_change(now_ms).map(|ms| now_ms.saturating_add(ms.max(1)));
    if due != due_ms {
        let now = now.clone();
        clock.update(cx, |clock, cx| {
            clock.due_ms = due;
            clock._tick = due.map(|due| {
                cx.spawn(async move |clock, cx| {
                    // Waits again if the timer ran ahead of the clock.
                    loop {
                        let wait = due.saturating_sub(now());
                        if wait <= 0 {
                            break;
                        }
                        cx.background_executor()
                            .timer(Duration::from_millis(wait.unsigned_abs()))
                            .await;
                    }
                    let _ = clock.update(cx, |clock, cx| {
                        clock.now_ms = now();
                        clock.due_ms = None;
                        cx.notify();
                    });
                })
            });
        });
    }
    now_ms
}

/// How long from a run of `run_ms` until it next turns over a whole
/// `unit_ms` (a second, a minute): from 1 ms to `unit_ms`. A run that hasn't
/// started yet (negative) counts the same way.
#[must_use]
pub const fn until_next(run_ms: i64, unit_ms: i64) -> i64 {
    unit_ms - run_ms.rem_euclid(unit_ms)
}

#[cfg(test)]
mod tests {
    use super::until_next;

    #[test]
    fn a_run_ticks_on_its_next_whole_unit() {
        assert_eq!(until_next(10_200, 1_000), 800);
        assert_eq!(until_next(10_000, 1_000), 1_000);
        assert_eq!(until_next(9_999, 1_000), 1);
        assert_eq!(until_next(90_000, 60_000), 30_000);
    }

    #[test]
    fn a_run_not_started_yet_still_ticks_within_a_unit() {
        assert_eq!(until_next(-300, 1_000), 300);
    }
}
