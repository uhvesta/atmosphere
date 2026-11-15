//! Safe Rust over libghostty-vt, Ghostty's terminal core (decision D10 in
//! `apps/humata2/DECISIONS.md`).
//!
//! A [`Terminal`] parses the bytes a program writes and keeps the screen:
//! the visible grid, its scrollback, the cursor, modes and title. Humata 2's
//! own view draws it through the [`Grid`] trait from `terminal_grid`, which
//! is all a renderer needs, so another engine (`alacritty_terminal`, the
//! fallback D10 names) can stand behind the same view. The grid's types
//! are re-exported here, so callers of this crate need only this crate.
//!
//! The crate draws nothing, touches no clipboard and starts no threads.
//! What a program asks of the host (a title, the clipboard, an answer to a
//! query) arrives as an [`Event`]. A `Terminal` is `Send` but not `Sync`:
//! libghostty-vt has no thread affinity, but a handle must only be used by
//! one thread at a time.

mod cell;
mod clipboard;
mod error;
mod ffi;
mod render;
mod terminal;

pub use clipboard::{ClipboardTarget, clipboard_read_reply};
pub use error::{Error, Status};
pub use terminal::{DecMode, Event, Match, Point, Scroll, Terminal};
pub use terminal_grid::{
    Cell, CellWidth, Color, Colors, Cursor, CursorShape, CursorStyle, Dirty, Grid, Key,
    KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, OptionAsAlt, Rgb, Row,
    Scrollbar, Size, Span, Underline, ViewportPoint,
};
