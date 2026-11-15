//! Plays flows in a GPUI window that is never shown: `TestAppContext` for
//! view snapshots everywhere, `HeadlessAppContext` when pixels are wanted.

use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, ClipboardItem, HeadlessAppContext, InvalidKeystrokeError,
    Keystroke, MouseButton, Pixels, Point, TestAppContext, Window,
};
use humata2_snapshot::{RgbaImage, view};
use thiserror::Error;

use crate::driver::Driver;
use crate::flow::{Action, Target};

/// Why a GPUI flow step failed.
#[derive(Debug, Error)]
pub enum GpuiDriverError {
    #[error(
        "the flow's window is no longer open, so the step could not run: {source}. Open the \
         window before the flow and keep it open until the flow ends; no step should close it"
    )]
    WindowGone {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error(
        "no visible element is {target}. Visible elements with that role: {seen}. Check the \
         role and label in the flow against the view, and that the element is drawn and marked \
         with .test_support() (gpui-kit components are marked already)"
    )]
    NotFound { target: String, seen: String },

    #[error(
        "{count} visible elements are {target}, so clicking one would be a guess. Give them \
         different accessible labels, so a person using a screen reader can tell them apart too"
    )]
    Ambiguous { target: String, count: usize },

    #[error(
        "{key:?} is not a key GPUI can press: {source}. Write keys as GPUI keystrokes, such as \
         \"enter\", \"backspace\", \"shift-tab\" or \"secondary-v\""
    )]
    InvalidKey {
        key: String,
        #[source]
        source: InvalidKeystrokeError,
    },

    #[error(transparent)]
    View(#[from] view::ViewError),

    #[error(
        "could not draw the window to pixels: {source}. Image snapshots need a context with a \
         renderer, such as humata2_snapshot::headless_context() on macOS"
    )]
    Render {
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// A GPUI app context a flow can play in.
pub trait Host {
    /// Runs `update` with the window open as `window`.
    ///
    /// # Errors
    ///
    /// [`GpuiDriverError::WindowGone`] when the window is closed.
    fn with_window<R>(
        &mut self,
        window: AnyWindowHandle,
        update: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Result<R, GpuiDriverError>;

    /// Runs the tasks a step started until none is left to run.
    fn settle(&mut self);

    /// Whether its windows can be drawn to pixels.
    fn draws_pixels(&self) -> bool;
}

fn with_window<C: AppContext, R>(
    cx: &mut C,
    window: AnyWindowHandle,
    update: impl FnOnce(&mut Window, &mut App) -> R,
) -> Result<R, GpuiDriverError> {
    cx.update_window(window, |_, window, cx| update(window, cx))
        .map_err(|error| GpuiDriverError::WindowGone {
            source: error.into(),
        })
}

impl Host for TestAppContext {
    fn with_window<R>(
        &mut self,
        window: AnyWindowHandle,
        update: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Result<R, GpuiDriverError> {
        with_window(self, window, update)
    }

    fn settle(&mut self) {
        self.run_until_parked();
    }

    fn draws_pixels(&self) -> bool {
        false
    }
}

impl Host for HeadlessAppContext {
    fn with_window<R>(
        &mut self,
        window: AnyWindowHandle,
        update: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Result<R, GpuiDriverError> {
        with_window(self, window, update)
    }

    fn settle(&mut self) {
        self.run_until_parked();
    }

    fn draws_pixels(&self) -> bool {
        true
    }
}

/// Plays flows in one window of a GPUI context that shows nothing on
/// screen. Clicks and drags go to the middle of the control a [`Target`]
/// names, as a pointer would.
pub struct GpuiDriver<'a, H: Host> {
    host: &'a mut H,
    window: AnyWindowHandle,
}

impl<'a, H: Host> GpuiDriver<'a, H> {
    /// Plays flows in `window`, which `host` opened.
    pub fn new(host: &'a mut H, window: AnyWindowHandle) -> Self {
        Self { host, window }
    }

    fn window<R>(
        &mut self,
        update: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Result<R, GpuiDriverError> {
        self.host.with_window(self.window, update)
    }
}

impl<H: Host> Driver for GpuiDriver<'_, H> {
    type Error = GpuiDriverError;

    fn name(&self) -> &str {
        if self.host.draws_pixels() {
            "gpui headless, drawn with the platform's GPU off screen"
        } else {
            "gpui test platform, nothing drawn"
        }
    }

    fn perform(&mut self, action: &Action) -> Result<(), GpuiDriverError> {
        match action {
            Action::Click(target) => self.window(|window, cx| {
                let at = locate(window, target, cx)?;
                window.pointer_move(at, None, cx);
                window.pointer_down(at, MouseButton::Left, cx);
                window.pointer_up(at, MouseButton::Left, cx);
                Ok::<(), GpuiDriverError>(())
            })??,
            Action::Type(text) => self.window(|window, cx| window.input(text, cx))?,
            Action::Key(key) => self.window(|window, cx| {
                Keystroke::parse_with_os(key, cx.platform_info().os).map_err(|source| {
                    GpuiDriverError::InvalidKey {
                        key: key.clone(),
                        source,
                    }
                })?;
                window.press(key, cx);
                Ok::<(), GpuiDriverError>(())
            })??,
            Action::Drag { from, to } => self.window(|window, cx| {
                let start = locate(window, from, cx)?;
                let end = locate(window, to, cx)?;
                window.drag(start, end, cx);
                Ok::<(), GpuiDriverError>(())
            })??,
            Action::Paste(text) => self.window(|window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                window.press("secondary-v", cx);
            })?,
        }
        self.host.settle();
        Ok(())
    }

    fn view(&mut self) -> Result<String, GpuiDriverError> {
        let tree = self.window(|window, cx| {
            window.render_frame(cx);
            view::capture(window)
        })??;
        Ok(tree)
    }

    fn image(&mut self) -> Result<Option<RgbaImage>, GpuiDriverError> {
        if !self.host.draws_pixels() {
            return Ok(None);
        }
        let image = self.window(|window, cx| {
            window.render_frame(cx);
            window.render_to_image()
        })?;
        image
            .map(Some)
            .map_err(|error| GpuiDriverError::Render {
                source: error.into(),
            })
    }
}

/// The middle of the one visible element `target` names.
fn locate(window: &mut Window, target: &Target, cx: &mut App) -> Result<Point<Pixels>, GpuiDriverError> {
    window.render_frame(cx);
    let candidates: Vec<_> = window
        .find_all_by_role(target.role())
        .into_iter()
        .filter(|element| element.visible())
        .collect();
    let matching: Vec<_> = candidates
        .iter()
        .filter(|element| element.label() == Some(target.label()))
        .collect();
    match matching.as_slice() {
        [element] => Ok(element.bounds().center()),
        [] => {
            let seen: Vec<String> = candidates
                .iter()
                .map(|element| format!("{:?}", element.label().unwrap_or("(no label)")))
                .collect();
            Err(GpuiDriverError::NotFound {
                target: target.to_string(),
                seen: if seen.is_empty() {
                    "none".to_owned()
                } else {
                    seen.join(", ")
                },
            })
        }
        many => Err(GpuiDriverError::Ambiguous {
            target: target.to_string(),
            count: many.len(),
        }),
    }
}
