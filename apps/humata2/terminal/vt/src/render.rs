//! The viewport snapshot a renderer reads, kept by libghostty-vt's render
//! state (`render.h`) and copied into Rust values once a frame.
//!
//! Ghostty's own renderer works the same way (`renderer/generic.zig`,
//! `rebuildCells`): update the render state from the terminal, then walk
//! its rows and rebuild only those marked dirty, unless the whole state is.

use std::mem::size_of;
use std::ptr;

use terminal_grid::{
    Cell, Colors, Cursor, CursorShape, CursorStyle, Dirty, Rgb, Row, Size, Span,
};

use crate::cell::cell_from;
use crate::error::{Status, check};
use crate::ffi;
use crate::terminal::Owned;

/// The render state handles and the snapshot copied out of them.
pub(crate) struct RenderState {
    // The row and cell handles are filled from the state; free them first.
    cells: Owned<ffi::RowCellsImpl>,
    row_iterator: Owned<ffi::RowIteratorImpl>,
    state: Owned<ffi::RenderStateImpl>,
    size: Size,
    rows: Vec<Row>,
    selection: Vec<Option<Span>>,
    dirty_rows: Vec<bool>,
    full: bool,
    cursor: Cursor,
    cursor_style: CursorStyle,
    colors: Colors,
    /// Reused for each cell's UTF-8 text.
    text: Vec<u8>,
}

impl RenderState {
    pub(crate) fn new() -> Result<Self, Status> {
        let state = Owned::new(
            // SAFETY: NULL selects the default allocator.
            |out| unsafe { ffi::ghostty_render_state_new(ptr::null(), out) },
            ffi::ghostty_render_state_free,
        )?;
        let row_iterator = Owned::new(
            // SAFETY: as above.
            |out| unsafe { ffi::ghostty_render_state_row_iterator_new(ptr::null(), out) },
            ffi::ghostty_render_state_row_iterator_free,
        )?;
        let cells = Owned::new(
            // SAFETY: as above.
            |out| unsafe { ffi::ghostty_render_state_row_cells_new(ptr::null(), out) },
            ffi::ghostty_render_state_row_cells_free,
        )?;
        Ok(RenderState {
            cells,
            row_iterator,
            state,
            size: Size::default(),
            rows: Vec::new(),
            selection: Vec::new(),
            dirty_rows: Vec::new(),
            full: true,
            cursor: Cursor::default(),
            cursor_style: CursorStyle::default(),
            colors: Colors::default(),
            text: vec![0; 32],
        })
    }

    /// Captures `terminal`'s viewport, copying the rows that changed since
    /// the last update. The snapshot's dirty marks add up until
    /// [`RenderState::clear_dirty`]; libghostty-vt's own are cleared here,
    /// once copied. On an error the rows not yet copied stay dirty in
    /// libghostty-vt, so the next update copies them.
    pub(crate) fn update(&mut self, terminal: ffi::GhosttyTerminal) -> Result<(), Status> {
        let state = self.state.ptr;
        // SAFETY: both handles are live; the state copies what it needs.
        check(unsafe { ffi::ghostty_render_state_update(state, terminal) })?;

        let dirty: ffi::GhosttyRenderStateDirty = self.get(ffi::RENDER_STATE_DATA_DIRTY)?;
        let size = Size {
            cols: self.get(ffi::RENDER_STATE_DATA_COLS)?,
            rows: self.get(ffi::RENDER_STATE_DATA_ROWS)?,
        };
        let mut full = dirty == ffi::RENDER_STATE_DIRTY_FULL;
        if size != self.size {
            self.resize(size);
            full = true;
        }
        let colors = self.read_colors()?;
        if colors != self.colors {
            self.colors = colors;
            full = true;
        }
        self.read_cursor()?;
        if full {
            self.full = true;
        }

        let mut iterator = self.row_iterator.ptr;
        // SAFETY: ROW_ITERATOR fills the iterator whose handle it is given.
        check(unsafe {
            ffi::ghostty_render_state_get(
                state,
                ffi::RENDER_STATE_DATA_ROW_ITERATOR,
                (&raw mut iterator).cast(),
            )
        })?;
        let mut y = 0;
        // SAFETY: the iterator was just filled from the live state.
        while y < self.rows.len() && unsafe { ffi::ghostty_render_state_row_iterator_next(iterator) }
        {
            let selection = read_selection(iterator)?;
            if selection != self.selection[y] {
                self.selection[y] = selection;
                self.dirty_rows[y] = true;
            }
            if full || row_get::<bool>(iterator, ffi::RENDER_STATE_ROW_DATA_DIRTY)? {
                self.read_row(iterator, y)?;
                self.dirty_rows[y] = true;
                let clean = false;
                // SAFETY: the row option takes a pointer to a bool.
                check(unsafe {
                    ffi::ghostty_render_state_row_set(
                        iterator,
                        ffi::RENDER_STATE_ROW_OPT_DIRTY,
                        (&raw const clean).cast(),
                    )
                })?;
            }
            y += 1;
        }

        let clean = ffi::RENDER_STATE_DIRTY_FALSE;
        // SAFETY: the option takes a pointer to a GhosttyRenderStateDirty.
        check(unsafe {
            ffi::ghostty_render_state_set(state, ffi::RENDER_STATE_OPT_DIRTY, (&raw const clean).cast())
        })
    }

    pub(crate) fn size(&self) -> Size {
        self.size
    }

    pub(crate) fn row(&self, y: u16) -> Option<&Row> {
        self.rows.get(usize::from(y))
    }

    pub(crate) fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub(crate) fn cursor_style(&self) -> CursorStyle {
        self.cursor_style
    }

    pub(crate) fn colors(&self) -> &Colors {
        &self.colors
    }

    pub(crate) fn selection(&self, y: u16) -> Option<Span> {
        self.selection.get(usize::from(y)).copied().flatten()
    }

    pub(crate) fn dirty(&self) -> Dirty {
        if self.full {
            Dirty::Full
        } else if self.dirty_rows.contains(&true) {
            Dirty::Partial
        } else {
            Dirty::Clean
        }
    }

    pub(crate) fn is_row_dirty(&self, y: u16) -> bool {
        let y = usize::from(y);
        y < self.rows.len() && (self.full || self.dirty_rows[y])
    }

    pub(crate) fn clear_dirty(&mut self) {
        self.full = false;
        self.dirty_rows.fill(false);
    }

    fn resize(&mut self, size: Size) {
        let cols = usize::from(size.cols);
        let rows = usize::from(size.rows);
        self.size = size;
        self.rows.resize_with(rows, Row::default);
        for row in &mut self.rows {
            row.cells.resize_with(cols, Cell::default);
        }
        self.selection.resize(rows, None);
        self.dirty_rows.resize(rows, true);
    }

    fn read_row(&mut self, iterator: ffi::GhosttyRenderStateRowIterator, y: usize) -> Result<(), Status> {
        let raw_row: ffi::GhosttyRow = row_get(iterator, ffi::RENDER_STATE_ROW_DATA_RAW)?;
        let mut wrapped = false;
        // SAFETY: WRAP's output is a bool; the row value is plain data.
        check(unsafe {
            ffi::ghostty_row_get(raw_row, ffi::ROW_DATA_WRAP, (&raw mut wrapped).cast())
        })?;
        let mut cells = self.cells.ptr;
        // SAFETY: CELLS fills the cells whose handle it is given.
        check(unsafe {
            ffi::ghostty_render_state_row_get(
                iterator,
                ffi::RENDER_STATE_ROW_DATA_CELLS,
                (&raw mut cells).cast(),
            )
        })?;

        let text = &mut self.text;
        let Some(row) = self.rows.get_mut(y) else {
            return Ok(());
        };
        row.wrapped = wrapped;
        let mut x = 0;
        // SAFETY: the cells were just filled from the live state.
        while x < row.cells.len() && unsafe { ffi::ghostty_render_state_row_cells_next(cells) } {
            row.cells[x] = read_cell(cells, text)?;
            x += 1;
        }
        for cell in &mut row.cells[x..] {
            *cell = Cell::default();
        }
        Ok(())
    }

    fn read_colors(&self) -> Result<Colors, Status> {
        let mut out = ffi::GhosttyRenderStateColors {
            size: size_of::<ffi::GhosttyRenderStateColors>(),
            background: ffi::GhosttyColorRgb::default(),
            foreground: ffi::GhosttyColorRgb::default(),
            cursor: ffi::GhosttyColorRgb::default(),
            cursor_has_value: false,
            palette: [ffi::GhosttyColorRgb::default(); 256],
        };
        // SAFETY: COLORS fills a sized struct whose `size` is its own.
        check(unsafe {
            ffi::ghostty_render_state_get(
                self.state.ptr,
                ffi::RENDER_STATE_DATA_COLORS,
                (&raw mut out).cast(),
            )
        })?;
        let mut palette = [Rgb::default(); 256];
        for (entry, color) in palette.iter_mut().zip(out.palette) {
            *entry = rgb(color);
        }
        Ok(Colors {
            foreground: rgb(out.foreground),
            background: rgb(out.background),
            cursor: out.cursor_has_value.then(|| rgb(out.cursor)),
            palette,
        })
    }

    fn read_cursor(&mut self) -> Result<(), Status> {
        let mut out = ffi::GhosttyRenderStateCursor {
            size: size_of::<ffi::GhosttyRenderStateCursor>(),
            ..ffi::GhosttyRenderStateCursor::default()
        };
        // SAFETY: CURSOR fills a sized struct whose `size` is its own.
        check(unsafe {
            ffi::ghostty_render_state_get(
                self.state.ptr,
                ffi::RENDER_STATE_DATA_CURSOR,
                (&raw mut out).cast(),
            )
        })?;
        // The position is undefined when the cursor is outside the viewport.
        self.cursor = if out.viewport_has_value {
            Cursor {
                x: out.viewport_x,
                y: out.viewport_y,
                visible: out.visible,
            }
        } else {
            Cursor::default()
        };
        self.cursor_style = CursorStyle {
            shape: match out.visual_style {
                ffi::CURSOR_STYLE_BAR => CursorShape::Bar,
                ffi::CURSOR_STYLE_UNDERLINE => CursorShape::Underline,
                ffi::CURSOR_STYLE_BLOCK_HOLLOW => CursorShape::BlockHollow,
                _ => CursorShape::Block,
            },
            blinking: out.blinking,
        };
        Ok(())
    }

    fn get<T: Default>(&self, data: ffi::GhosttyRenderStateData) -> Result<T, Status> {
        let mut out = T::default();
        // SAFETY: callers pick `T` as the output type `data` documents.
        check(unsafe { ffi::ghostty_render_state_get(self.state.ptr, data, (&raw mut out).cast()) })?;
        Ok(out)
    }
}

fn row_get<T: Default>(
    iterator: ffi::GhosttyRenderStateRowIterator,
    data: ffi::GhosttyRenderStateRowData,
) -> Result<T, Status> {
    let mut out = T::default();
    // SAFETY: the iterator is on a row; callers pick `T` as the output type
    // `data` documents.
    check(unsafe { ffi::ghostty_render_state_row_get(iterator, data, (&raw mut out).cast()) })?;
    Ok(out)
}

fn read_selection(iterator: ffi::GhosttyRenderStateRowIterator) -> Result<Option<Span>, Status> {
    let mut out = ffi::GhosttyRenderStateRowSelection {
        size: size_of::<ffi::GhosttyRenderStateRowSelection>(),
        ..ffi::GhosttyRenderStateRowSelection::default()
    };
    // SAFETY: SELECTION fills a sized struct whose `size` is its own.
    let result = unsafe {
        ffi::ghostty_render_state_row_get(
            iterator,
            ffi::RENDER_STATE_ROW_DATA_SELECTION,
            (&raw mut out).cast(),
        )
    };
    if result == ffi::NO_VALUE {
        return Ok(None);
    }
    check(result)?;
    Ok(Some(Span {
        start: out.start_x,
        end: out.end_x,
    }))
}

fn read_cell(cells: ffi::GhosttyRenderStateRowCells, text: &mut Vec<u8>) -> Result<Cell, Status> {
    let mut value: ffi::GhosttyCell = 0;
    let mut style = ffi::GhosttyStyle::EMPTY;
    // SAFETY: the cells are on a cell; the outputs are the documented types.
    unsafe {
        check(ffi::ghostty_render_state_row_cells_get(
            cells,
            ffi::RENDER_STATE_ROW_CELLS_DATA_RAW,
            (&raw mut value).cast(),
        ))?;
        check(ffi::ghostty_render_state_row_cells_get(
            cells,
            ffi::RENDER_STATE_ROW_CELLS_DATA_STYLE,
            (&raw mut style).cast(),
        ))?;
    }
    cell_from(value, &style, graphemes(cells, text)?)
}

/// The cell's grapheme cluster as UTF-8, growing `buf` when it doesn't fit.
fn graphemes(cells: ffi::GhosttyRenderStateRowCells, buf: &mut Vec<u8>) -> Result<String, Status> {
    loop {
        let mut out = ffi::GhosttyBuffer {
            ptr: buf.as_mut_ptr(),
            cap: buf.len(),
            len: 0,
        };
        // SAFETY: the buffer holds `cap` bytes.
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(
                cells,
                ffi::RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8,
                (&raw mut out).cast(),
            )
        };
        if result == ffi::OUT_OF_SPACE && out.len > buf.len() {
            buf.resize(out.len, 0);
            continue;
        }
        check(result)?;
        let len = out.len.min(buf.len());
        return Ok(String::from_utf8_lossy(&buf[..len]).into_owned());
    }
}

fn rgb(color: ffi::GhosttyColorRgb) -> Rgb {
    Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}
