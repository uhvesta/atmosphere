//! Safe Rust over libghostty-vt, Ghostty's terminal core (decision D10 in
//! `apps/humata2/DECISIONS.md`).
//!
//! A [`Terminal`] parses the bytes a program writes and keeps the screen:
//! the visible grid, its scrollback, the cursor, modes and title. Humata 2's
//! own view draws it through the [`Grid`] trait, which is all a renderer
//! needs, so another engine (`alacritty_terminal`, the fallback D10 names)
//! can stand behind the same view.
//!
//! The crate draws nothing and starts no threads. A `Terminal` is `Send` but
//! not `Sync`: libghostty-vt has no thread affinity, but a handle must only
//! be used by one thread at a time.

mod cell;
mod error;
mod ffi;
mod input;
mod terminal;

pub use cell::{Cell, CellWidth, Color, Row, Underline};
pub use error::{Error, Status};
pub use input::{
    Key, KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, OptionAsAlt,
};
pub use terminal::{Cursor, Event, Match, Point, Scroll, Size, Terminal};

/// What a renderer reads to draw a terminal: its size, the rows in view and
/// the cursor.
pub trait Grid {
    /// The grid's size in cells.
    fn size(&self) -> Size;

    /// The cells of row `y` of the viewport, 0 being the top row in view,
    /// left to right. Each cell is one column: a wide character is a
    /// [`CellWidth::Wide`] cell followed by a [`CellWidth::SpacerTail`]. A
    /// row outside the viewport has no cells.
    fn row(&self, y: u16) -> impl Iterator<Item = Cell>;

    /// Where the cursor is, in viewport cells, and whether it is shown.
    fn cursor(&self) -> Cursor;
}
