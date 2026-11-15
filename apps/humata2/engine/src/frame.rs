//! Frames as an app draws them, input as a person gives it, and what a drawn
//! frame shows, for headless tests and benchmarks on either engine.
//!
//! # Natural frames and full refreshes
//!
//! A running app draws a frame when something it shows changed: GPUI runs
//! the next-frame callbacks, then draws without refreshing, so a view that
//! wasn't notified keeps what it drew (a cached view on ordinary GPUI, any
//! unchanged view on GPUI Fast). [`natural_frame`] does the same.
//!
//! [`full_refresh`] refreshes the whole window first, so every view is
//! built again. That is a diagnostic: it hides a view that forgot to notify,
//! which is exactly what retention gets wrong when it goes wrong. These
//! helpers force a refresh and so are diagnostic too:
//!
//! - gpui-kit's `TestWindowExt` (`render_frame`, `click`, `click_at`,
//!   `click_with_options`, `right_click`, `double_click`, `hover`, `scroll`,
//!   `drag`, `drag_to`, `press`, `input`) and `TestEventExt` (`pointer_move`,
//!   `pointer_down`, `pointer_up`, `change_modifiers`, `key_down`, `key_up`)
//!   draw a refreshed frame before and after each event;
//! - `TestAppContextExt::wait_for` refreshes every 10 ms of test time;
//! - [`window::set_view_retention`](crate::window::set_view_retention) and
//!   [`window::set_scroll_layers`](crate::window::set_scroll_layers) refresh
//!   when GPUI Fast changes the setting.
//!
//! The input functions here ([`move_pointer`], [`click`], [`press`],
//! [`type_text`], [`scroll`], [`drag`]) dispatch real platform events with
//! `Window::dispatch_event` and draw a natural frame after each one, as a
//! person's events arrive between frames. Nothing here refreshes.
//!
//! GPUI itself refreshes on some input, and a refreshed frame builds every
//! view on both engines. The one that matters here: pressing a mouse button
//! on an element with click handlers refreshes, so a click handler that
//! forgets to notify is hidden by the click. Keyboard actions and changes
//! from outside input (a daemon event, a timer) don't refresh, and that is
//! where a forgotten notify shows on GPUI Fast.
//!
//! GPUI doesn't say from outside whether a window is dirty, so a natural
//! frame is drawn even when nothing changed. On GPUI Fast that frame reuses
//! every view, the same pixels a skipped frame would leave. On ordinary
//! GPUI it rebuilds every uncached view, so a view that forgot to notify is
//! only caught on GPUI Fast.
//!
//! # Observing a frame
//!
//! [`observe`] reads the last drawn frame without drawing: the elements
//! marked with `.test_support()`, the text drawn, the painted quads, and on
//! GPUI Fast the glyphs, icons and images painted and the frame counters.
//! [`pixels`] draws that same frame to pixels. Retained content is in all of
//! them except the text: gpui-base records text only as it paints it, so the
//! text of a view GPUI Fast drew again from the last frame (or a cached
//! view on either engine) is missing from [`Frame::texts`]. Its glyphs are
//! in [`Frame::sprites`] and its pixels in [`pixels`], which is how
//! [`crate::parity`] checks it.
//!
//! gpui-base forgets the last frame's text when its `Root` view renders. On
//! GPUI Fast the `Root` is drawn again from the last frame like any other
//! unchanged view, so its text would pile up across frames;
//! [`natural_frame`] and [`full_refresh`] forget it themselves before each
//! draw.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};

use gpui_kit::base::test_support::{TextSnapshot, begin_frame, texts};
use gpui_kit::test::{ElementSnapshot, TestQueryExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext, InputEvent as _, InvalidKeystrokeError, KeyDownEvent,
    KeyUpEvent, Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, ScrollDelta, ScrollEvent, Window, point,
};

use crate::window::{FrameCounts, frame_counts, painted_sprites};

/// What driving a headless window failed at.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The window was closed.
    #[error(
        "the window is no longer open, so no frame could be drawn: {source}. Keep the window open until the test has drawn and observed its last frame"
    )]
    WindowGone {
        /// What GPUI reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Frames kept changing.
    #[error(
        "the window still changed after {frames} natural frames, so the test can't tell when it has settled. Something keeps changing what is drawn, such as an animation, a timer or a view that notifies itself as it renders. Advance the clock past the animation, stop the timer in the fixture, or raise the limit if the view really needs more frames"
    )]
    Unsettled {
        /// The frames drawn.
        frames: usize,
    },

    /// A key name GPUI doesn't know.
    #[error(
        "{key:?} is not a key GPUI can press: {source}. Write keys as GPUI keystrokes, such as \"enter\", \"backspace\", \"shift-tab\" or \"secondary-v\""
    )]
    InvalidKey {
        /// The key asked for.
        key: String,
        /// Why GPUI refused it.
        #[source]
        source: InvalidKeystrokeError,
    },

    /// The window couldn't be drawn to pixels.
    #[error(
        "could not draw the window to pixels: {source}. Pixels need a headless context with a renderer (humata2_engine::headless on macOS), not TestAppContext"
    )]
    Render {
        /// What the renderer reported.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Draws the next frame as a running app would: next-frame callbacks first,
/// then a draw that refreshes nothing. Returns how many next-frame callbacks
/// ran, so animations can be told apart from a still window.
pub fn natural_frame(window: &mut Window, cx: &mut App) -> usize {
    let callbacks = window.simulate_next_frame(cx);
    begin_frame(window);
    window.draw(cx).clear(cx);
    callbacks
}

/// Refreshes the whole window and draws it, building every view again. A
/// diagnostic, and the reference [`crate::parity`] compares against: it
/// hides views that forgot to notify.
pub fn full_refresh(window: &mut Window, cx: &mut App) {
    window.simulate_next_frame(cx);
    window.refresh();
    begin_frame(window);
    window.draw(cx).clear(cx);
}

/// Runs `step` in `window`.
///
/// # Errors
///
/// [`FrameError::WindowGone`] when the window is closed.
pub fn in_window<C: AppContext, R>(
    cx: &mut C,
    window: AnyWindowHandle,
    step: impl FnOnce(&mut Window, &mut App) -> R,
) -> Result<R, FrameError> {
    cx.update_window(window, |_, window, cx| step(window, cx))
        .map_err(|error| FrameError::WindowGone {
            source: error.into(),
        })
}

/// Draws natural frames until two in a row show the same thing, running
/// `park` (the context's `run_until_parked`) before each. Returns the frames
/// drawn. The clock doesn't move, so an animation waiting on time looks
/// still; advance the clock first to see it move.
///
/// # Errors
///
/// [`FrameError::Unsettled`] when frames still differ after `max_frames`,
/// and [`FrameError::WindowGone`] when the window is closed.
pub fn settle<C: AppContext>(
    cx: &mut C,
    window: AnyWindowHandle,
    mut park: impl FnMut(&mut C),
    max_frames: usize,
) -> Result<usize, FrameError> {
    let mut last = None;
    for frames in 1..=max_frames {
        park(cx);
        let shown = in_window(cx, window, |window, cx| {
            natural_frame(window, cx);
            fingerprint(window)
        })?;
        if last == Some(shown) {
            return Ok(frames);
        }
        last = Some(shown);
    }
    Err(FrameError::Unsettled { frames: max_frames })
}

/// A hash of what the last frame shows: its elements, text, quads and
/// sprites.
fn fingerprint(window: &Window) -> u64 {
    let frame = observe(window);
    let mut hasher = DefaultHasher::new();
    for element in &frame.elements {
        format!("{element:?}").hash(&mut hasher);
    }
    for text in &frame.texts {
        format!("{text:?}").hash(&mut hasher);
    }
    frame.quads.hash(&mut hasher);
    frame.sprites.hash(&mut hasher);
    hasher.finish()
}

/// Moves the pointer to `at`, with `pressed` held, then draws a natural
/// frame.
pub fn move_pointer(
    window: &mut Window,
    at: Point<Pixels>,
    pressed: Option<MouseButton>,
    cx: &mut App,
) {
    let modifiers = window.modifiers();
    window.dispatch_event(
        MouseMoveEvent {
            position: at,
            pressed_button: pressed,
            modifiers,
        }
        .to_platform_input(),
        cx,
    );
    natural_frame(window, cx);
}

/// Moves the pointer to `at`, presses `button` and releases it, with a
/// natural frame after each event.
pub fn click(window: &mut Window, at: Point<Pixels>, button: MouseButton, cx: &mut App) {
    move_pointer(window, at, None, cx);
    button_down(window, at, button, cx);
    button_up(window, at, button, cx);
}

fn button_down(window: &mut Window, at: Point<Pixels>, button: MouseButton, cx: &mut App) {
    let modifiers = window.modifiers();
    window.dispatch_event(
        MouseDownEvent {
            button,
            position: at,
            modifiers,
            click_count: 1,
            first_mouse: false,
        }
        .to_platform_input(),
        cx,
    );
    natural_frame(window, cx);
}

fn button_up(window: &mut Window, at: Point<Pixels>, button: MouseButton, cx: &mut App) {
    let modifiers = window.modifiers();
    window.dispatch_event(
        MouseUpEvent {
            button,
            position: at,
            modifiers,
            click_count: 1,
        }
        .to_platform_input(),
        cx,
    );
    natural_frame(window, cx);
}

/// Drags with the left button from `from` to `to` in `steps` moves, with a
/// natural frame after each event.
pub fn drag(
    window: &mut Window,
    from: Point<Pixels>,
    to: Point<Pixels>,
    steps: usize,
    cx: &mut App,
) {
    move_pointer(window, from, None, cx);
    button_down(window, from, MouseButton::Left, cx);
    let steps = steps.max(1);
    for step in 1..=steps {
        let fraction = step as f32 / steps as f32;
        let at = point(
            from.x + (to.x - from.x) * fraction,
            from.y + (to.y - from.y) * fraction,
        );
        move_pointer(window, at, Some(MouseButton::Left), cx);
    }
    button_up(window, to, MouseButton::Left, cx);
}

/// Scrolls by `delta` pixels with the pointer at `at`, then draws a natural
/// frame.
pub fn scroll(window: &mut Window, at: Point<Pixels>, delta: Point<Pixels>, cx: &mut App) {
    move_pointer(window, at, None, cx);
    let modifiers = window.modifiers();
    window.dispatch_event(
        ScrollEvent {
            position: at,
            delta: ScrollDelta::Pixels(delta),
            modifiers,
            ..Default::default()
        }
        .to_platform_input(),
        cx,
    );
    natural_frame(window, cx);
}

/// Presses and releases `key`, such as `"enter"` or `"secondary-k"`, then
/// draws a natural frame. Enter and Tab only run their bindings; other keys
/// insert their text too, as GPUI's simulated IME does.
///
/// # Errors
///
/// [`FrameError::InvalidKey`] when GPUI can't parse `key`.
pub fn press(window: &mut Window, key: &str, cx: &mut App) -> Result<(), FrameError> {
    let keystroke = Keystroke::parse_with_os(key, cx.platform_info().os).map_err(|source| {
        FrameError::InvalidKey {
            key: key.to_owned(),
            source,
        }
    })?;
    let keystroke = if matches!(keystroke.key.as_str(), "enter" | "tab") {
        window.dispatch_event(
            KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }
            .to_platform_input(),
            cx,
        );
        keystroke
    } else {
        let keystroke = keystroke.with_simulated_ime();
        window.dispatch_keystroke(keystroke.clone(), cx);
        keystroke
    };
    // Buttons activate on key-up, so every press completes the pair.
    window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    natural_frame(window, cx);
    Ok(())
}

/// Types `text` one character at a time into the focused control, with a
/// natural frame after each.
///
/// # Errors
///
/// [`FrameError::InvalidKey`] for a character GPUI can't express as a key.
pub fn type_text(window: &mut Window, text: &str, cx: &mut App) -> Result<(), FrameError> {
    for character in text.chars() {
        let text = character.to_string();
        let mut keystroke =
            Keystroke::parse_with_os(&text, cx.platform_info().os).map_err(|source| {
                FrameError::InvalidKey {
                    key: text.clone(),
                    source,
                }
            })?;
        keystroke.key_char = Some(text);
        window.dispatch_keystroke(keystroke, cx);
        natural_frame(window, cx);
    }
    Ok(())
}

/// What the last drawn frame shows, read without drawing.
#[derive(Clone, Debug)]
pub struct Frame {
    /// Every element marked with `.test_support()` that the frame holds,
    /// visible or not, in reading order.
    pub elements: Vec<ElementSnapshot>,
    /// The text drawn in views built this frame, in paint order. Text of a
    /// view drawn again from the last frame is missing (see the module
    /// docs).
    pub texts: Vec<TextSnapshot>,
    /// The painted quads, one line each: bounds, clip, background, border
    /// and corners, in scaled pixels. Draw order is left out.
    pub quads: Vec<String>,
    /// The painted glyphs, icons, images and underlines, one line each, or
    /// `None` on ordinary GPUI, which doesn't list them.
    pub sprites: Option<Vec<String>>,
    /// The frame counters, or `None` on ordinary GPUI.
    pub counts: Option<FrameCounts>,
}

/// Reads the last drawn frame without drawing.
#[must_use]
pub fn observe(window: &Window) -> Frame {
    let mut elements = window.elements();
    elements.sort_by(|a, b| {
        let (a, b) = (a.bounds().origin, b.bounds().origin);
        a.y.partial_cmp(&b.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
    });
    let quads = window
        .painted_quads()
        .iter()
        .map(|quad| {
            format!(
                "quad {:?} {:?} {:?} border {:?} {:?} {:?} corners {:?}",
                quad.bounds,
                quad.content_mask,
                quad.background,
                quad.border_style,
                quad.border_color,
                quad.border_widths,
                quad.corner_radii
            )
        })
        .collect();
    Frame {
        elements,
        texts: texts(window),
        quads,
        sprites: painted_sprites(window),
        counts: frame_counts(window),
    }
}

/// The visible element of the last drawn frame whose own id (the last part
/// of its path) is `id`, read without drawing. A removed or hidden control
/// is not found; two visible elements with the id are a test bug and give
/// the first in reading order.
#[must_use]
pub fn find(window: &Window, id: &str) -> Option<ElementSnapshot> {
    observe(window).elements.into_iter().find(|element| {
        element.visible() && element.path().last().is_some_and(|last| last.to_string() == id)
    })
}

/// A frame drawn to pixels: `width` × `height`, four bytes (RGBA) a pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
    /// The pixels, row by row.
    pub rgba: Vec<u8>,
}

/// Draws the last drawn frame to pixels, without drawing a new frame.
///
/// # Errors
///
/// [`FrameError::Render`] when the context has no renderer.
pub fn pixels(window: &Window) -> Result<Picture, FrameError> {
    let image = window
        .render_to_image()
        .map_err(|error| FrameError::Render {
            source: error.into(),
        })?;
    Ok(Picture {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    })
}
