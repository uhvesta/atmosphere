//! Humata 2's terminal view: a `GPUI` view that draws any
//! [`terminal_grid::Grid`] and sends what the user does to a
//! [`TerminalHost`].
//!
//! It is written for Humata rather than taken from a third-party widget,
//! and copies `Ghostty`'s renderer where it can:
//!
//! - `renderer/generic.zig` (`rebuildCells`, `rebuildRow`): only the rows
//!   the grid marks dirty are planned and shaped again; the rest are drawn
//!   from the last frame.
//! - `renderer/cell.zig` and `rebuildRow`: per cell, a background, a glyph
//!   and decorations, with inverse video and the selection swapping colors.
//! - `font/shaper/run.zig`: text is shaped in runs that break on a style
//!   change, at the selection's edges and around wide or complex cells.
//! - `renderer/cursor.zig` (`style`) and `addCursor`: block, bar,
//!   underline and an outlined block while unfocused; blinking.
//! - `font/sprite/draw/box.zig`: box-drawing and block characters are
//!   drawn as rectangles on whole device pixels, not taken from the font,
//!   so boxes join across cells.
//!
//! The planning (what to draw) lives in `frame` and `cursor` and is plain
//! data, tested without a window; `view` only shapes and paints it.

mod boxes;
mod cursor;
mod frame;
mod host;
mod keys;
mod paste;
mod selection;
mod theme;
mod view;

pub use host::{CellSize, InputBufferError, ScrollTo, TerminalHost};
pub use paste::Paste;
pub use theme::Theme;
pub use view::{FrameStats, HistoryPoint, InputStats, Mark, TerminalView};

mod pending_input;

/// Whether `uri` is an external `http`, `https`, or `mailto` URL with a
/// non-empty body and no control characters.
pub fn is_web_or_mail_uri(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    ["http", "https", "mailto"]
        .iter()
        .any(|s| scheme.eq_ignore_ascii_case(s))
        && !rest.is_empty()
        && !uri.chars().any(char::is_control)
}
