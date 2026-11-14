//! Time for effects that wait: the system's in the app, one moved by hand
//! in tests so they never wait.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::effect::BoxFuture;

/// Tells the time and waits. Effects take one as a dependency instead of
/// asking the system, so tests can move time on themselves.
pub trait Clock: Send + Sync + 'static {
    /// How long since the clock started.
    fn now(&self) -> Duration;

    /// Ends once `duration` has passed on this clock.
    fn sleep(&self, duration: Duration) -> BoxFuture<()>;
}

/// The system's time.
#[derive(Clone, Copy, Debug)]
pub struct SystemClock {
    start: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn sleep(&self, duration: Duration) -> BoxFuture<()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

/// A clock that stands still until [`TestClock::advance`] moves it; what
/// sleeps on it wakes once it has been moved far enough. Clones share the
/// same time.
#[derive(Clone, Default)]
pub struct TestClock {
    time: Arc<Mutex<TestTime>>,
}

#[derive(Default)]
struct TestTime {
    now: Duration,
    sleepers: Vec<(Duration, oneshot::Sender<()>)>,
}

impl TestClock {
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves time on by `by`, waking what was sleeping until then.
    pub fn advance(&self, by: Duration) {
        let due = {
            let mut time = self.time.lock().unwrap_or_else(PoisonError::into_inner);
            time.now += by;
            let now = time.now;
            let (due, waiting) = std::mem::take(&mut time.sleepers)
                .into_iter()
                .partition::<Vec<_>, _>(|(until, _)| *until <= now);
            time.sleepers = waiting;
            due
        };
        for (_, wake) in due {
            let _ = wake.send(());
        }
    }
}

impl Clock for TestClock {
    fn now(&self) -> Duration {
        self.time.lock().unwrap_or_else(PoisonError::into_inner).now
    }

    fn sleep(&self, duration: Duration) -> BoxFuture<()> {
        if duration.is_zero() {
            return Box::pin(async {});
        }
        let (wake, woken) = oneshot::channel();
        {
            let mut time = self.time.lock().unwrap_or_else(PoisonError::into_inner);
            let until = time.now + duration;
            time.sleepers.push((until, wake));
        }
        Box::pin(async move {
            if woken.await.is_err() {
                // The clock is gone, so time never comes.
                std::future::pending::<()>().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sleeper_wakes_only_once_time_has_moved_far_enough() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let clock = TestClock::new();
        let mut sleep = runtime.spawn(clock.sleep(Duration::from_secs(10)));
        clock.advance(Duration::from_secs(9));
        runtime.block_on(async {
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut sleep)
                    .await
                    .is_err()
            );
        });
        clock.advance(Duration::from_secs(1));
        runtime.block_on(async {
            assert!(
                tokio::time::timeout(Duration::from_secs(1), &mut sleep)
                    .await
                    .is_ok()
            );
        });
        assert_eq!(clock.now(), Duration::from_secs(10));
    }
}
