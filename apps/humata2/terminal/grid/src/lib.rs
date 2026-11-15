//! What a terminal renderer reads, and nothing else: the cells in view,
//! the cursor, the colors, the selection and which rows changed.
//!
//! The engine (`ghostty_vt`, or the `alacritty_terminal` fallback decision
//! D10 names) implements [`Grid`]; the `GPUI` view in `terminal/view` draws
//! any [`Grid`]. The crate has no dependencies, so the view builds and
//! tests without the engine. The `fake` feature adds [`FakeGrid`], a grid
//! tests and benchmarks fill by hand.
//!
//! The model follows libghostty-vt's render state (`render.h`): the engine
//! keeps a snapshot of the viewport, [`Grid::sync`] refreshes it once a
//! frame, the renderer redraws the rows [`Grid::is_row_dirty`] names and
//! then calls [`Grid::clear_dirty`].

mod cell;
mod colors;
#[cfg(feature = "fake")]
mod fake;
mod input;

pub use cell::{Cell, CellWidth, Color, Row, Underline};
pub use colors::{Colors, Rgb, default_palette};
#[cfg(feature = "fake")]
pub use fake::FakeGrid;
pub use input::{
    Key, KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, OptionAsAlt,
};

/// A grid size in cells.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

/// Where the cursor is and whether the program shows it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
}

/// A cell of the viewport: column `x` of row `y`, row 0 being the top row
/// in view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct ViewportPoint {
    pub x: u16,
    pub y: u16,
}

/// The cursor's shape, as DECSCUSR (`CSI q`) or the user's setting chose.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    #[default]
    Block,
    Bar,
    Underline,
    /// An outlined block, which `Ghostty` draws while the view isn't focused.
    BlockHollow,
}

/// How to draw the cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CursorStyle {
    pub shape: CursorShape,
    pub blinking: bool,
}

/// The columns from `start` to `end` of one row, both included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u16,
    pub end: u16,
}

impl Span {
    /// Whether column `x` is in the span.
    pub fn contains(self, x: u16) -> bool {
        self.start <= x && x <= self.end
    }
}

/// What changed since the renderer last called [`Grid::clear_dirty`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Dirty {
    /// Nothing: the last frame is still right.
    #[default]
    Clean,
    /// Some rows, which [`Grid::is_row_dirty`] names.
    Partial,
    /// Everything, such as after a resize, a scroll of the viewport or a
    /// color change: redraw every row.
    Full,
}

/// What a renderer reads to draw a terminal.
///
/// Reads are cheap: they return what the last [`Grid::sync`] captured, so
/// a renderer may call them as often as it likes within a frame.
pub trait Grid {
    /// Captures the engine's current screen. Call once at the start of a
    /// frame, before any other read.
    fn sync(&mut self);

    /// The grid's size in cells.
    fn size(&self) -> Size;

    /// Row `y` of the viewport, 0 being the top row in view, or `None`
    /// past the bottom. Each cell is one column: a wide character is a
    /// [`CellWidth::Wide`] cell followed by a [`CellWidth::SpacerTail`].
    fn row(&self, y: u16) -> Option<&Row>;

    /// Where the cursor is in the viewport. It isn't visible when the
    /// program hid it or the viewport is scrolled away from it.
    fn cursor(&self) -> Cursor;

    /// The cursor's shape and whether it blinks.
    fn cursor_style(&self) -> CursorStyle;

    /// The default colors and the palette, as the program last set them.
    fn colors(&self) -> &Colors;

    /// The selected columns of row `y`, or `None` when nothing in it is
    /// selected.
    fn selection(&self, y: u16) -> Option<Span>;

    /// How much changed since the last [`Grid::clear_dirty`].
    fn dirty(&self) -> Dirty;

    /// Whether row `y` changed since the last [`Grid::clear_dirty`]. Every
    /// row is dirty while [`Grid::dirty`] is [`Dirty::Full`].
    fn is_row_dirty(&self, y: u16) -> bool;

    /// Marks everything drawn. Call after a frame has been painted.
    fn clear_dirty(&mut self);
}
