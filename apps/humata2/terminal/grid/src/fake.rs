use crate::{
    Cell, CellWidth, Colors, Cursor, CursorStyle, Dirty, Grid, Row, Scrollbar, Size, Span,
    ViewportPoint,
};

/// A grid filled by hand, for tests and benchmarks of a renderer. Every
/// change marks what it touched dirty, as an engine would: writing to a
/// row marks the row, and resizing, scrolling or changing colors marks
/// everything.
#[derive(Debug, Clone)]
pub struct FakeGrid {
    size: Size,
    rows: Vec<Row>,
    cursor: Cursor,
    cursor_style: CursorStyle,
    colors: Colors,
    selection: Vec<Option<Span>>,
    dirty_rows: Vec<bool>,
    full: bool,
    syncs: u64,
    scrollbar: Option<Scrollbar>,
}

impl FakeGrid {
    /// A blank grid `cols` wide and `rows` tall, with the cursor shown in
    /// the top-left cell and every row dirty.
    pub fn new(cols: u16, rows: u16) -> Self {
        FakeGrid {
            size: Size { cols, rows },
            rows: (0..rows).map(|_| blank_row(cols)).collect(),
            cursor: Cursor {
                x: 0,
                y: 0,
                visible: true,
            },
            cursor_style: CursorStyle::default(),
            colors: Colors::default(),
            selection: vec![None; usize::from(rows)],
            dirty_rows: vec![true; usize::from(rows)],
            full: true,
            syncs: 0,
            scrollbar: None,
        }
    }

    /// How many times [`Grid::sync`] ran.
    pub fn syncs(&self) -> u64 {
        self.syncs
    }

    /// Writes `text` into row `y` from column `x`, one grapheme cluster a
    /// cell, styled like `style` (its own text is ignored). A wide
    /// character takes two cells; one that doesn't fit before the right
    /// edge leaves a [`CellWidth::SpacerHead`]. Text past the edge is
    /// dropped. Returns the column after the last cell written.
    pub fn write(&mut self, x: u16, y: u16, text: &str, style: &Cell) -> u16 {
        let cols = self.size.cols;
        let Some(row) = self.rows.get_mut(usize::from(y)) else {
            return x;
        };
        let mut col = x;
        for cluster in clusters(text) {
            if col >= cols {
                break;
            }
            let wide = cluster_is_wide(&cluster);
            if wide && col + 1 >= cols {
                row.cells[usize::from(col)] = Cell {
                    width: CellWidth::SpacerHead,
                    ..style_only(style)
                };
                col += 1;
                break;
            }
            row.cells[usize::from(col)] = Cell {
                text: cluster,
                width: if wide {
                    CellWidth::Wide
                } else {
                    CellWidth::Narrow
                },
                ..style_only(style)
            };
            col += 1;
            if wide {
                row.cells[usize::from(col)] = Cell {
                    width: CellWidth::SpacerTail,
                    ..style_only(style)
                };
                col += 1;
            }
        }
        self.mark_row(y);
        col
    }

    /// Puts `cell` at column `x` of row `y` as it is, spacers included.
    pub fn set_cell(&mut self, x: u16, y: u16, cell: Cell) {
        if let Some(slot) = self
            .rows
            .get_mut(usize::from(y))
            .and_then(|row| row.cells.get_mut(usize::from(x)))
        {
            *slot = cell;
            self.mark_row(y);
        }
    }

    /// Replaces row `y`, padding or cutting it to the grid's width.
    pub fn set_row(&mut self, y: u16, mut row: Row) {
        let cols = usize::from(self.size.cols);
        row.cells.resize_with(cols, Cell::default);
        if let Some(slot) = self.rows.get_mut(usize::from(y)) {
            *slot = row;
            self.mark_row(y);
        }
    }

    /// Sets whether row `y` wraps into the next one.
    pub fn set_wrapped(&mut self, y: u16, wrapped: bool) {
        if let Some(row) = self.rows.get_mut(usize::from(y)) {
            row.wrapped = wrapped;
            self.mark_row(y);
        }
    }

    /// Scrolls every row up by one, dropping the top row and putting `row`
    /// at the bottom, as output past the last line does. Every row
    /// changes, so the whole grid is dirty.
    pub fn scroll_up(&mut self, mut row: Row) {
        if self.rows.is_empty() {
            return;
        }
        row.cells
            .resize_with(usize::from(self.size.cols), Cell::default);
        self.rows.remove(0);
        self.rows.push(row);
        self.selection.remove(0);
        self.selection.push(None);
        self.full = true;
    }

    /// Changes the grid's size, keeping what fits. Everything is dirty.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.size = Size { cols, rows };
        self.rows.resize_with(usize::from(rows), || blank_row(cols));
        for row in &mut self.rows {
            row.cells.resize_with(usize::from(cols), Cell::default);
        }
        self.selection.resize(usize::from(rows), None);
        self.dirty_rows.resize(usize::from(rows), true);
        self.cursor.x = self.cursor.x.min(cols.saturating_sub(1));
        self.cursor.y = self.cursor.y.min(rows.saturating_sub(1));
        self.full = true;
    }

    /// Moves or hides the cursor. The cursor is drawn over the rows, so
    /// moving it doesn't dirty them.
    pub fn set_cursor(&mut self, cursor: Cursor) {
        self.cursor = cursor;
    }

    pub fn set_cursor_style(&mut self, style: CursorStyle) {
        self.cursor_style = style;
    }

    /// Changes the default colors or palette. Everything is dirty.
    pub fn set_colors(&mut self, colors: Colors) {
        self.colors = colors;
        self.full = true;
    }

    /// Selects from `start` to `end`, both included, in reading order: the
    /// first row from `start` to its end, whole rows between, and the last
    /// row up to `end`. Rows whose selection changed are dirty.
    pub fn select(&mut self, start: ViewportPoint, end: ViewportPoint) {
        let (first, last) = if (start.y, start.x) <= (end.y, end.x) {
            (start, end)
        } else {
            (end, start)
        };
        let right = self.size.cols.saturating_sub(1);
        for y in 0..self.size.rows {
            let span = if y < first.y || y > last.y {
                None
            } else {
                let from = if y == first.y { first.x } else { 0 };
                let to = if y == last.y { last.x } else { right };
                Some(Span {
                    start: from,
                    end: to.min(right),
                })
            };
            self.set_selection(y, span);
        }
    }

    /// Selects the block of cells with `start` and `end` as opposite
    /// corners, as a rectangular selection does. Rows whose selection
    /// changed are dirty.
    pub fn select_rectangle(&mut self, start: ViewportPoint, end: ViewportPoint) {
        let right = self.size.cols.saturating_sub(1);
        let (top, bottom) = (start.y.min(end.y), start.y.max(end.y));
        let span = Span {
            start: start.x.min(end.x).min(right),
            end: start.x.max(end.x).min(right),
        };
        for y in 0..self.size.rows {
            let selected = (top..=bottom).contains(&y).then_some(span);
            self.set_selection(y, selected);
        }
    }

    /// Sets where the viewport sits in a history the fake doesn't hold, so
    /// a renderer draws a scrollbar. Changing it dirties everything, as
    /// scrolling the viewport does.
    pub fn set_scrollbar(&mut self, scrollbar: Option<Scrollbar>) {
        if self.scrollbar != scrollbar {
            self.scrollbar = scrollbar;
            self.full = true;
        }
    }

    /// Sets row `y`'s selected columns directly.
    pub fn set_selection(&mut self, y: u16, span: Option<Span>) {
        if let Some(slot) = self.selection.get_mut(usize::from(y))
            && *slot != span
        {
            *slot = span;
            self.mark_row(y);
        }
    }

    /// Clears the selection.
    pub fn clear_selection(&mut self) {
        for y in 0..self.size.rows {
            self.set_selection(y, None);
        }
    }

    fn mark_row(&mut self, y: u16) {
        if let Some(dirty) = self.dirty_rows.get_mut(usize::from(y)) {
            *dirty = true;
        }
    }
}

impl Grid for FakeGrid {
    fn sync(&mut self) {
        self.syncs += 1;
    }

    fn size(&self) -> Size {
        self.size
    }

    fn row(&self, y: u16) -> Option<&Row> {
        self.rows.get(usize::from(y))
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }

    fn colors(&self) -> &Colors {
        &self.colors
    }

    fn selection(&self, y: u16) -> Option<Span> {
        self.selection.get(usize::from(y)).copied().flatten()
    }

    fn dirty(&self) -> Dirty {
        if self.full {
            Dirty::Full
        } else if self.dirty_rows.iter().any(|&dirty| dirty) {
            Dirty::Partial
        } else {
            Dirty::Clean
        }
    }

    fn is_row_dirty(&self, y: u16) -> bool {
        y < self.size.rows && (self.full || self.dirty_rows[usize::from(y)])
    }

    fn clear_dirty(&mut self) {
        self.full = false;
        self.dirty_rows.fill(false);
    }

    fn scrollbar(&self) -> Option<Scrollbar> {
        self.scrollbar
    }
}

fn blank_row(cols: u16) -> Row {
    Row {
        cells: vec![Cell::default(); usize::from(cols)],
        wrapped: false,
    }
}

/// `style` with no text.
fn style_only(style: &Cell) -> Cell {
    Cell {
        text: String::new(),
        width: CellWidth::Narrow,
        ..style.clone()
    }
}

/// Splits `text` into grapheme clusters, roughly: combining marks,
/// variation selectors, skin tones and anything after a zero-width joiner
/// stay with the character before them. Enough for test text; the engine
/// does the real clustering.
fn clusters(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut after_joiner = false;
    for c in text.chars() {
        let joins = after_joiner || is_extender(c);
        match out.last_mut() {
            Some(last) if joins => last.push(c),
            _ => out.push(c.to_string()),
        }
        after_joiner = c == '\u{200d}';
    }
    out
}

fn is_extender(c: char) -> bool {
    matches!(
        u32::from(c),
        0x0300..=0x036f
            | 0x0483..=0x0489
            | 0x0591..=0x05bd
            | 0x0610..=0x061a
            | 0x064b..=0x065f
            | 0x200d
            | 0x20e3
            | 0xfe00..=0xfe0f
            | 0x1f3fb..=0x1f3ff
            | 0xe0020..=0xe007f
    )
}

/// Whether a cluster takes two columns: East Asian wide and full-width
/// characters, emoji, and anything with the emoji presentation selector.
fn cluster_is_wide(cluster: &str) -> bool {
    let mut chars = cluster.chars();
    let first = chars.next().map_or(0, u32::from);
    let wide_first = matches!(
        first,
        0x1100..=0x115f
            | 0x2e80..=0x303e
            | 0x3041..=0x33ff
            | 0x3400..=0x4dbf
            | 0x4e00..=0x9fff
            | 0xa000..=0xa4cf
            | 0xac00..=0xd7a3
            | 0xf900..=0xfaff
            | 0xfe30..=0xfe4f
            | 0xff00..=0xff60
            | 0xffe0..=0xffe6
            | 0x1f300..=0x1f64f
            | 0x1f680..=0x1f6ff
            | 0x1f900..=0x1f9ff
            | 0x20000..=0x3fffd
    );
    wide_first || cluster.contains('\u{fe0f}')
}
