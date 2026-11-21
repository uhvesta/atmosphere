//! The window shown while the app installs its host tools and waits for the
//! daemon, so the main thread never blocks on either. If start-up fails, the
//! window says why and offers Quit; closing it quits. The view is
//! [`humata2_ui::Starting`]; this file runs the job and feeds it.

use std::fmt::Display;
use std::future::Future;

use gpui_kit::{prelude::*, *};
use humata2_ui::{STARTING_WINDOW, Starting};
use tca::Storage;
use tokio::sync::{oneshot, watch};

use crate::live::Phase;

/// The start-up job ended without an answer.
#[derive(Debug, thiserror::Error)]
enum Lost {
    /// The background task dropped its answer, which happens when it
    /// panicked or the runtime shut down.
    #[error(
        "Humata's start-up stopped without an answer. Quit and open Humata again; if it keeps happening, report a bug."
    )]
    NoAnswer {
        /// What the channel said.
        #[source]
        source: oneshot::error::RecvError,
    },
}

/// The line under the title for a phase; "Installing its tools" shows only
/// while installing.
fn status(phase: Phase) -> &'static str {
    match phase {
        Phase::Installing => "Installing its tools",
        Phase::Waiting => "Waiting for humatad2",
    }
}

/// Opens the starting window now, runs `start` on `runtime`, and when it
/// finishes calls `then` on the UI thread with what it made, after closing
/// the starting window, drawn in the theme `storage` saved. If `start` fails,
/// the window shows the error and stays; closing it quits the app. `start`
/// gets a function to report each phase it enters.
pub(crate) fn show_while<T, E, F>(
    cx: &mut App,
    runtime: &tokio::runtime::Handle,
    storage: &dyn Storage,
    first: Phase,
    start: impl FnOnce(Box<dyn Fn(Phase) + Send + Sync>) -> F,
    then: impl FnOnce(&mut App, T) + 'static,
) where
    T: Send + 'static,
    E: Display + Send + 'static,
    F: Future<Output = Result<T, E>> + Send + 'static,
{
    let opened = gpui_kit::open_window(window_options(cx), cx, |window, cx| {
        cx.new(|cx| Starting::new(storage, status(first), window, cx))
    });
    let (window, view) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            eprintln!("Humata couldn't open its window: {error}. Check the system's display settings and try again.");
            cx.quit();
            return;
        }
    };
    cx.activate(true);
    // While this is held, closing the starting window quits.
    let quit_on_close = cx.on_window_closed(|cx, _| cx.quit());

    let (phases, mut phase_changes) = watch::channel(first);
    let (answer, answered) = oneshot::channel();
    let job = start(Box::new(move |phase| {
        // Nobody listening means the window is gone, and the app is quitting.
        let _ = phases.send(phase);
    }));
    runtime.spawn(async move {
        // A closed receiver means the app quit while this ran.
        let _ = answer.send(job.await);
    });

    let shown = view.clone();
    cx.spawn(async move |cx| {
        while phase_changes.changed().await.is_ok() {
            let phase = *phase_changes.borrow_and_update();
            cx.update(|cx| {
                shown.update(cx, |starting, cx| starting.set_status(status(phase), cx));
            });
        }
    })
    .detach();

    cx.spawn(async move |cx| {
        let result = match answered.await {
            Ok(result) => result.map_err(|error| error.to_string()),
            Err(source) => Err(Lost::NoAnswer { source }.to_string()),
        };
        match result {
            Ok(made) => {
                // Close the starting window before `then` registers the main
                // window's own quit-on-close, and in its own update so the
                // close is seen before that.
                drop(quit_on_close);
                cx.update(|cx| {
                    // A window that closed already has nothing to remove.
                    let _ = window.update(cx, |_, window, _| window.remove_window());
                });
                cx.update(|cx| then(cx, made));
            }
            Err(text) => {
                eprintln!("{text}");
                quit_on_close.detach();
                cx.update(|cx| {
                    view.update(cx, |starting, cx| starting.fail(text, cx));
                });
            }
        }
    })
    .detach();
}

fn window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(STARTING_WINDOW.0), px(STARTING_WINDOW.1)), cx);
    starting_window_options(bounds)
}

fn starting_window_options(bounds: Bounds<Pixels>) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        is_resizable: false,
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: None,
        }),
        ..WindowOptions::default()
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Bounds, point, px, size};

    use super::{Phase, starting_window_options, status};

    #[test]
    fn each_phase_says_something_different() {
        assert_ne!(status(Phase::Installing), status(Phase::Waiting));
        assert!(!status(Phase::Waiting).is_empty());
    }

    #[test]
    fn splash_window_is_locked_with_transparent_titlebar() {
        let options = starting_window_options(Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(360.), px(220.)),
        });
        assert!(!options.is_resizable);
        let titlebar = options.titlebar.expect("transparent titlebar on splash window");
        assert!(titlebar.appears_transparent);
        assert!(titlebar.title.is_none());
    }
}
