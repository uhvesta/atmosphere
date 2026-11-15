/// One column of a row: the text drawn there and how to draw it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cell {
    /// The grapheme cluster in the cell, such as `"a"`, `"日"`, `"e\u{301}"`
    /// or a whole emoji ZWJ sequence; empty when the cell holds no text,
    /// including the spacer after a wide character.
    pub text: String,
    pub width: CellWidth,
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Color,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

impl Cell {
    /// How many columns the cell's text covers: 2 for a wide character, 1
    /// for a narrow one and 0 for a spacer.
    pub fn columns(&self) -> u8 {
        match self.width {
            CellWidth::Narrow => 1,
            CellWidth::Wide => 2,
            CellWidth::SpacerTail | CellWidth::SpacerHead => 0,
        }
    }
}

/// How a cell relates to the characters around it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CellWidth {
    /// A character one column wide, or an empty cell.
    #[default]
    Narrow,
    /// The first column of a character two columns wide.
    Wide,
    /// The second column of a wide character; draw nothing here.
    SpacerTail,
    /// The last column of a row whose wide character didn't fit and moved to
    /// the next row; draw nothing here.
    SpacerHead,
}

/// A color as the program set it. The view resolves [`Color::Default`] and
/// palette entries against its theme.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Color {
    /// The theme's foreground or background.
    #[default]
    Default,
    /// One of the 256 palette entries, as set by SGR 30-37, 90-97 or 38;5.
    Palette(u8),
    /// A 24-bit color, as set by SGR 38;2.
    Rgb { r: u8, g: u8, b: u8 },
}

/// The underline style SGR 4 and its sub-parameters set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// A row of the screen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Row {
    /// One cell per column, left to right.
    pub cells: Vec<Cell>,
    /// Whether the row's text goes on in the next row, because it reached
    /// the right edge rather than ending with a new line.
    pub wrapped: bool,
}
