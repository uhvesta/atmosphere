//! Work from other threads while the headless app blocks, as a live client's
//! threads wake the app's tasks (D759); nothing is shown.

#![cfg(target_os = "macos")]

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

// `gpui_kit::*` exports a `test` attribute; keep the standard one.
use ::std::prelude::v1::test;
use gpui_kit::{AssetSource, Result, SharedString};
use humata2_engine::Resources;

/// How long the app keeps blocking while other threads wake it.
const BLOCKING: Duration = Duration::from_secs(5);
/// Past this the app is taken to have hung.
const HUNG: Duration = Duration::from_secs(60);
/// Tasks a thread may have queued on the app and not yet seen run.
const QUEUED: usize = 64;
/// Threads that queue work on the app at once.
const SENDERS: usize = 4;

struct NoAssets;

impl AssetSource for NoAssets {
    fn load(&self, _path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(None)
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(Vec::new())
    }
}

/// Never finishes; leaves its waker for another thread each time it runs.
struct Waits(Arc<Mutex<Option<Waker>>>);

impl Future for Waits {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(cx.waker().clone());
        Poll::Pending
    }
}

/// Blocks with a timeout over and over, as the app's shutdown does, while
/// other threads wake one of its tasks and queue work on it as fast as they
/// can.
fn block_while_woken() {
    let cx = humata2_engine::headless(&Resources::new(NoAssets)).expect("the headless context starts");
    let waker = Arc::new(Mutex::new(None::<Waker>));
    cx.foreground_executor().spawn(Waits(waker.clone())).detach();
    let stop = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::new();
    {
        let stop = stop.clone();
        threads.push(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let taken = waker.lock().unwrap_or_else(PoisonError::into_inner).take();
                match taken {
                    Some(waker) => waker.wake(),
                    None => std::thread::yield_now(),
                }
            }
        }));
    }
    for _ in 0..SENDERS {
        let stop = stop.clone();
        let executor = cx.background_executor().clone();
        let queued = Arc::new(AtomicUsize::new(0));
        threads.push(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if queued.load(Ordering::Relaxed) >= QUEUED {
                    std::thread::yield_now();
                    continue;
                }
                queued.fetch_add(1, Ordering::Relaxed);
                let ran = queued.clone();
                executor
                    .spawn(async move {
                        ran.fetch_sub(1, Ordering::Relaxed);
                    })
                    .detach();
            }
        }));
    }
    let until = Instant::now() + BLOCKING;
    while Instant::now() < until {
        let _ = cx
            .foreground_executor()
            .block_with_timeout(Duration::ZERO, std::future::pending::<()>());
    }
    stop.store(true, Ordering::Relaxed);
    for thread in threads {
        thread.join().expect("a thread that wakes the app ends");
    }
}

#[test]
fn work_from_other_threads_while_the_app_blocks_does_not_hang_it() {
    let (done, finished) = mpsc::channel();
    std::thread::spawn(move || {
        block_while_woken();
        let _ = done.send(());
    });
    match finished.recv_timeout(HUNG) {
        Ok(()) => {}
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("the app still blocked {} s on, so the work from other threads hung it", HUNG.as_secs())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => panic!("blocking while woken panicked"),
    }
}
