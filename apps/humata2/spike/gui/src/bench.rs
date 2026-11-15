//! What the benchmarks measure and how they report it: frame times from a
//! probe painted last in every frame, process CPU time and peak memory from
//! `getrusage`, and one JSON line on stdout.

use std::cell::Cell;
use std::io::Write;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui_kit::{AnyElement, Bounds, IntoElement, ParentElement, Pixels, Styled, canvas, div, px};

static STARTED: OnceLock<Instant> = OnceLock::new();

/// Marks when the process started. Call first thing in `main`.
pub fn mark_start() {
    let _ = STARTED.set(Instant::now());
}

/// When [`mark_start`] was called, or now if it wasn't.
pub fn started() -> Instant {
    *STARTED.get_or_init(Instant::now)
}

/// Records when frames are painted. Shared by the shell, which reads it, and
/// a 1×1 canvas drawn after everything else, which writes it.
#[derive(Clone, Default)]
pub struct FrameProbe(Rc<ProbeState>);

#[derive(Default)]
struct ProbeState {
    first_paint: Cell<Option<Instant>>,
    last_paint: Cell<Option<Instant>>,
    paints: Cell<u64>,
}

impl FrameProbe {
    /// When the first frame was painted.
    pub fn first_paint(&self) -> Option<Instant> {
        self.0.first_paint.get()
    }

    /// When the latest frame was painted.
    pub fn last_paint(&self) -> Option<Instant> {
        self.0.last_paint.get()
    }

    /// How many frames have been painted.
    pub fn paints(&self) -> u64 {
        self.0.paints.get()
    }

    /// The element that records a paint. Put it last in the root, so it
    /// paints after the rest of the frame.
    pub fn element(&self) -> AnyElement {
        let state = self.0.clone();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size(px(1.))
            .child(
                canvas(
                    |_: Bounds<Pixels>, _, _| (),
                    move |_, (), _, _| {
                        let now = Instant::now();
                        if state.first_paint.get().is_none() {
                            state.first_paint.set(Some(now));
                        }
                        state.last_paint.set(Some(now));
                        state.paints.set(state.paints.get() + 1);
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

/// p50, p95, p99 and the largest of some durations, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
}

impl Percentiles {
    /// Nearest-rank percentiles, or None for no samples.
    pub fn of(samples: &[Duration]) -> Option<Self> {
        let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1e3).collect();
        ms.sort_by(f64::total_cmp);
        let rank = |p: f64| -> Option<f64> {
            let index = ((p / 100. * ms.len() as f64).ceil() as usize).max(1) - 1;
            ms.get(index.min(ms.len().checked_sub(1)?)).copied()
        };
        Some(Self {
            p50: rank(50.)?,
            p95: rank(95.)?,
            p99: rank(99.)?,
            max: *ms.last()?,
        })
    }

    /// `{"p50_ms":…,"p95_ms":…,"p99_ms":…,"max_ms":…}` without the braces.
    pub fn json_fields(&self) -> String {
        format!(
            "\"p50_ms\":{:.3},\"p95_ms\":{:.3},\"p99_ms\":{:.3},\"max_ms\":{:.3}",
            self.p50, self.p95, self.p99, self.max
        )
    }
}

/// The process's CPU time and peak memory so far.
#[derive(Clone, Copy, Debug)]
pub struct Usage {
    pub cpu: Duration,
    pub max_rss_bytes: u64,
}

impl Usage {
    /// Reads `getrusage(RUSAGE_SELF)`, or None if it fails.
    pub fn now() -> Option<Self> {
        // SAFETY: `rusage` is plain integers, for which all zeroes is valid,
        // and getrusage only writes into the struct it is given.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        // SAFETY: `usage` is a valid, writable rusage for the call's duration.
        let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
        if status != 0 {
            return None;
        }
        let time = |t: libc::timeval| {
            Duration::from_secs(u64::try_from(t.tv_sec).unwrap_or(0))
                + Duration::from_micros(u64::try_from(t.tv_usec).unwrap_or(0))
        };
        let max_rss = u64::try_from(usage.ru_maxrss).unwrap_or(0);
        // macOS reports bytes, Linux kibibytes.
        #[cfg(target_os = "macos")]
        let max_rss_bytes = max_rss;
        #[cfg(not(target_os = "macos"))]
        let max_rss_bytes = max_rss * 1024;
        Some(Self {
            cpu: time(usage.ru_utime) + time(usage.ru_stime),
            max_rss_bytes,
        })
    }
}

/// Milliseconds, for JSON.
pub fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1e3
}

/// Prints one JSON object as a line on stdout.
pub fn report(json: &str) {
    let mut stdout = std::io::stdout().lock();
    if let Err(error) = writeln!(stdout, "{json}").and_then(|()| stdout.flush()) {
        eprintln!(
            "could not write the benchmark result to stdout: {error}. \
             Check that stdout is not closed or a full pipe, then run the benchmark again"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let samples: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        let p = Percentiles::of(&samples);
        assert_eq!(
            p,
            Some(Percentiles {
                p50: 50.,
                p95: 95.,
                p99: 99.,
                max: 100.
            })
        );
    }

    #[test]
    fn no_samples_have_no_percentiles() {
        assert_eq!(Percentiles::of(&[]), None);
    }

    #[test]
    fn usage_reports_some_memory() {
        let usage = Usage::now();
        assert!(usage.is_some_and(|usage| usage.max_rss_bytes > 0));
    }
}
