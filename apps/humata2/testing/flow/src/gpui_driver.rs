//! Plays flows in a GPUI window that is never shown: `TestAppContext` for
//! view and layout snapshots everywhere, `HeadlessAppContext` when pixels are
//! wanted.
//!
//! A driver draws its frames one of two ways ([`FrameMode`]). Full refresh,
//! the default, builds every view again before each event and capture, as
//! gpui-kit's test helpers do; it hides a view that forgot to notify. Natural
//! frames draw as the app does, so what a flow snapshots is what the
//! engine's retention left on screen (see [`humata2_engine::frame`]).

use gpui_kit::test::{TestEventExt as _, TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, ClipboardItem, HeadlessAppContext, InvalidKeystrokeError,
    Keystroke, MouseButton, Pixels, Point, TestAppContext, Window, point, px,
};
use humata2_engine::frame;
use humata2_engine::parity::{self, Comparison};
use humata2_snapshot::layout::{self, Layout};
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

    #[error(
        "could not play {key:?} with natural frames: {source}. Check the flow's keys and text \
         against what the focused control takes"
    )]
    Frame {
        key: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error(
        "the window drawn as the app draws it differs from the same window rebuilt from scratch, \
         so the app would show something stale here. Usually a view changed what it shows \
         without calling cx.notify(); find the view whose elements differ below and notify \
         where it changes. Don't snapshot with refreshed frames to make this pass: that hides \
         the bug. {report}"
    )]
    Stale { report: String },

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

/// How a [`GpuiDriver`] draws its frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrameMode {
    /// Refresh the whole window before each event and capture, so every
    /// view is built again: gpui-kit's test helpers. A diagnostic; it hides
    /// a view that forgot to notify.
    #[default]
    FullRefresh,
    /// Draw only as the app would: real events, each followed by a natural
    /// frame (see [`humata2_engine::frame`]). Before a step's snapshots,
    /// the last natural frame is checked against the window refreshed
    /// ([`humata2_engine::parity::against_refresh`]), and a difference fails
    /// the step; the snapshots then read the refreshed frame, which matched.
    /// Scroll layers are turned off, so painted quads and glyphs compare
    /// exactly; `humata2_engine::parity::Pair` covers scroll layers.
    Natural,
}

/// Plays flows in one window of a GPUI context that shows nothing on
/// screen. Clicks and drags go to the middle of the control a [`Target`]
/// names, as a pointer would. With natural frames a drag's drop target is
/// looked up after the drag has begun, so a zone shown only while dragging
/// can be named.
pub struct GpuiDriver<'a, H: Host> {
    host: &'a mut H,
    window: AnyWindowHandle,
    frames: FrameMode,
    name: String,
    /// Natural frames: whether the window is set up for them yet.
    prepared: bool,
    /// Natural frames: whether this step's frame was checked already.
    checked: bool,
}

impl<'a, H: Host> GpuiDriver<'a, H> {
    /// Plays flows in `window`, which `host` opened, with full refreshes.
    pub fn new(host: &'a mut H, window: AnyWindowHandle) -> Self {
        Self::with_frames(host, window, FrameMode::FullRefresh)
    }

    /// Plays flows in `window`, which `host` opened, drawing frames as
    /// `frames` says.
    pub fn with_frames(host: &'a mut H, window: AnyWindowHandle, frames: FrameMode) -> Self {
        let platform = if host.draws_pixels() {
            "gpui headless, drawn with the platform's GPU off screen"
        } else {
            "gpui test platform, nothing drawn"
        };
        let drawn = match frames {
            FrameMode::FullRefresh => "every frame refreshed",
            FrameMode::Natural => "natural frames",
        };
        Self {
            host,
            window,
            frames,
            name: format!("{platform}; {drawn}"),
            prepared: false,
            checked: false,
        }
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
        &self.name
    }

    fn perform(&mut self, action: &Action) -> Result<(), GpuiDriverError> {
        if self.frames == FrameMode::Natural {
            self.prepare()?;
            self.checked = false;
            return self.perform_naturally(action);
        }
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
            Action::Scroll { target, x, y } => self.window(|window, cx| {
                let at = locate(window, target, cx)?;
                frame::scroll(window, at, point(px(*x as f32), px(*y as f32)), cx);
                Ok::<(), GpuiDriverError>(())
            })??,
            Action::Paste(text) => self.window(|window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                window.press("secondary-v", cx);
            })?,
            Action::Hover(target) => self.window(|window, cx| {
                window.render_frame(cx);
                let at = pick_hoverable(window, target)?;
                window.pointer_move(at, None, cx);
                Ok::<(), GpuiDriverError>(())
            })??,
        }
        self.host.settle();
        Ok(())
    }

    fn view(&mut self) -> Result<String, GpuiDriverError> {
        self.frame_to_capture()?;
        Ok(self.window(|window, _| view::capture(window))??)
    }

    fn layout(&mut self) -> Result<Option<Layout>, GpuiDriverError> {
        self.frame_to_capture()?;
        Ok(Some(self.window(|window, _| layout::capture(window))?))
    }

    fn image(&mut self) -> Result<Option<RgbaImage>, GpuiDriverError> {
        if !self.host.draws_pixels() {
            return Ok(None);
        }
        self.frame_to_capture()?;
        let image = self.window(|window, _| window.render_to_image())?;
        image
            .map(Some)
            .map_err(|error| GpuiDriverError::Render {
                source: error.into(),
            })
    }
}

impl<H: Host> GpuiDriver<'_, H> {
    /// Natural frames: turns scroll layers off and draws a first natural
    /// frame, once.
    fn prepare(&mut self) -> Result<(), GpuiDriverError> {
        if self.prepared {
            return Ok(());
        }
        self.window(|window, cx| {
            // Turning scroll layers off works on both engines.
            let _ = humata2_engine::window::set_scroll_layers(window, false);
            frame::natural_frame(window, cx);
        })?;
        self.prepared = true;
        Ok(())
    }

    /// Draws the frame a capture reads: a refreshed one, or with natural
    /// frames the last natural frame, checked once a step against a refresh.
    fn frame_to_capture(&mut self) -> Result<(), GpuiDriverError> {
        match self.frames {
            FrameMode::FullRefresh => self.window(|window, cx| window.render_frame(cx)),
            FrameMode::Natural => {
                self.prepare()?;
                if self.checked {
                    return Ok(());
                }
                let pixels = self.host.draws_pixels();
                let comparison = Comparison {
                    channel_tolerance: 2,
                    painted: true,
                };
                let (report, pictures) = self
                    .window(|window, cx| {
                        parity::against_refresh_with_pictures(window, cx, pixels, comparison)
                    })?
                    .map_err(|source| GpuiDriverError::Render {
                        source: Box::new(source),
                    })?;
                if !report.matches() {
                    let saved = save_stale_pictures(&pictures);
                    return Err(GpuiDriverError::Stale {
                        report: format!("{report}{saved}"),
                    });
                }
                self.checked = true;
                Ok(())
            }
        }
    }

    /// [`Driver::perform`] with natural frames: real events, each followed
    /// by a natural frame, then the step's tasks and one more natural frame
    /// for what they changed.
    fn perform_naturally(&mut self, action: &Action) -> Result<(), GpuiDriverError> {
        match action {
            Action::Click(target) => self.window(|window, cx| {
                let at = find(window, target)?;
                frame::click(window, at, MouseButton::Left, cx);
                Ok::<(), GpuiDriverError>(())
            })??,
            Action::Type(text) => {
                for character in text.chars() {
                    self.window(|window, cx| {
                        frame::type_character(window, character, cx)
                            .map_err(|error| frame_error(text, error))
                    })??;
                    self.window(|window, cx| {
                        frame::natural_frame(window, cx);
                    })?;
                }
            }
            Action::Key(key) => {
                let keystroke = self.window(|window, cx| {
                    frame::press(window, key, cx).map_err(|error| frame_error(key, error))
                })??;
                // Separate updates, so the key's events have reached their
                // subscribers before anything draws (see `frame::press`).
                self.window(|window, cx| frame::release(window, keystroke, cx))?;
                self.window(|window, cx| frame::natural_frame(window, cx))?;
            }
            Action::Drag { from, to } => self.window(|window, cx| {
                let start = find(window, from)?;
                // Where to drop is looked up once the drag has started: a
                // drop zone that only exists during a drag isn't drawn yet.
                frame::drag_to_found(window, start, 8, |window| find(window, to), cx)
            })??,
            Action::Scroll { target, x, y } => self.window(|window, cx| {
                let at = find(window, target)?;
                frame::scroll(window, at, point(px(*x as f32), px(*y as f32)), cx);
                Ok::<(), GpuiDriverError>(())
            })??,
            Action::Paste(text) => {
                let keystroke = self.window(|window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    frame::press(window, "secondary-v", cx)
                        .map_err(|error| frame_error("secondary-v", error))
                })??;
                self.window(|window, cx| frame::release(window, keystroke, cx))?;
                self.window(|window, cx| frame::natural_frame(window, cx))?;
            }
            Action::Hover(target) => self.window(|window, cx| {
                let at = pick_hoverable(window, target)?;
                frame::move_pointer(window, at, None, cx);
                Ok::<(), GpuiDriverError>(())
            })??,
        }
        self.host.settle();
        self.window(|window, cx| {
            frame::natural_frame(window, cx);
        })?;
        Ok(())
    }
}

/// A key or text the natural-frame input couldn't press.
fn frame_error(key: &str, error: frame::FrameError) -> GpuiDriverError {
    match error {
        frame::FrameError::InvalidKey { key, source } => GpuiDriverError::InvalidKey { key, source },
        other => GpuiDriverError::Frame {
            key: key.to_owned(),
            source: Box::new(other),
        },
    }
}

/// The middle of the one visible element `target` names, in the last frame
/// drawn, without drawing.
fn find(window: &Window, target: &Target) -> Result<Point<Pixels>, GpuiDriverError> {
    pick(window, target)
}

/// The middle of the one visible element `target` names.
fn locate(window: &mut Window, target: &Target, cx: &mut App) -> Result<Point<Pixels>, GpuiDriverError> {
    window.render_frame(cx);
    pick(window, target)
}

fn pick(window: &Window, target: &Target) -> Result<Point<Pixels>, GpuiDriverError> {
    pick_among(window, target, |element| element.visible())
}

/// The middle of the one element `target` names whose middle is inside the
/// window, drawn or still transparent: a hover-only control shows once the
/// pointer is over it, so a person moves there before they can see it.
fn pick_hoverable(window: &Window, target: &Target) -> Result<Point<Pixels>, GpuiDriverError> {
    let size = window.viewport_size();
    pick_among(window, target, |element| {
        let middle = element.bounds().center();
        middle.x >= px(0.) && middle.y >= px(0.) && middle.x < size.width && middle.y < size.height
    })
}

fn pick_among(
    window: &Window,
    target: &Target,
    shown: impl Fn(&gpui_kit::test::ElementSnapshot) -> bool,
) -> Result<Point<Pixels>, GpuiDriverError> {
    let candidates: Vec<_> = window
        .find_all_by_role(target.role())
        .into_iter()
        .filter(|element| shown(element))
        .collect();
    let matching: Vec<_> = candidates
        .iter()
        .filter(|element| element.label() == Some(target.label()))
        .collect();
    match matching.as_slice() {
        [element] => {
            let bounds = element.bounds();
            Ok(if target.starts_at_leading_edge() {
                // Just inside the edge, so the press lands before the first
                // character rather than on the control's border.
                point(bounds.left() + px(2.), bounds.center().y)
            } else {
                bounds.center()
            })
        }
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

/// Writes a stale step's natural and refreshed pictures to the test's
/// undeclared outputs, so the difference can be looked at. Returns a note
/// for the error saying where they went, or why they weren't saved.
/// Files are named after the test's thread, which the test harness names
/// after the test.
fn save_stale_pictures(pictures: &[frame::Picture; 2]) -> String {
    let Some(directory) = std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR") else {
        return String::new();
    };
    let directory = std::path::PathBuf::from(directory);
    let thread = std::thread::current();
    let stem: String = thread
        .name()
        .unwrap_or("flow")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
        .collect();
    let mut note = String::new();
    for (picture, kind) in pictures.iter().zip(["natural", "refreshed"]) {
        if picture.rgba.is_empty() {
            continue;
        }
        let path = directory.join(format!("stale-{stem}-{kind}.png"));
        let saved = RgbaImage::from_raw(picture.width, picture.height, picture.rgba.clone())
            .map(|image| image.save(&path));
        match saved {
            Some(Ok(())) => note.push_str(&format!("\nThe {kind} frame is at {}", path.display())),
            Some(Err(error)) => {
                note.push_str(&format!("\nCouldn't save the {kind} frame to {}: {error}", path.display()));
            }
            None => note.push_str(&format!("\nThe {kind} frame's pixels don't fill its size")),
        }
    }
    note
}
