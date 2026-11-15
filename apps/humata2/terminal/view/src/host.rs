use terminal_grid::{Grid, KeyEvent, Mods, MouseEvent, Size, ViewportPoint};

use crate::paste::Paste;

/// The size of a cell, in pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CellSize {
    pub width: f32,
    pub height: f32,
}

/// Where the view sends what the user does: the session that owns the
/// engine and the pty. The view turns window events into these calls and
/// never writes to a pty itself, so the same view serves a local shell, a
/// remote session over gRPC or a test.
///
/// Each call gets the grid, so a host whose engine is the grid (such as
/// `ghostty_vt`'s `Terminal`) can encode keys, select or scroll with it.
/// The view redraws after every call; when the engine changes later
/// (output arriving from the pty), the owner feeds it through
/// `TerminalView::grid_mut` and calls `cx.notify()` on the view.
pub trait TerminalHost<G: Grid> {
    /// A key was pressed, held or released. Copy and paste shortcuts don't
    /// arrive here; they become [`TerminalHost::copy`] and
    /// [`TerminalHost::paste`].
    fn key(&mut self, grid: &mut G, event: KeyEvent<'_>);

    /// The mouse moved, or a button was pressed or released, over the
    /// grid. Return `true` when the program takes the mouse (it turned on
    /// mouse reporting), so the view doesn't select. Shift held always
    /// selects, as in other terminals.
    fn mouse(&mut self, grid: &mut G, event: MouseEvent) -> bool {
        let _ = (grid, event);
        false
    }

    /// The user scrolled `lines` rows over `at`: positive towards older
    /// output, negative towards the bottom.
    fn scroll(&mut self, grid: &mut G, lines: i32, at: ViewportPoint, mods: Mods) {
        let _ = (grid, lines, at, mods);
    }

    /// The user selected from one cell to another, both included, in the
    /// order they dragged; `None` clears the selection.
    fn select(&mut self, grid: &mut G, selection: Option<(ViewportPoint, ViewportPoint)>) {
        let _ = (grid, selection);
    }

    /// The text to copy, from the selection; `None` copies nothing.
    fn copy(&mut self, grid: &mut G) -> Option<String> {
        let _ = grid;
        None
    }

    /// The user pasted. Text goes through the engine's paste encoding,
    /// which asks before sending text that could run commands.
    fn paste(&mut self, grid: &mut G, paste: Paste) {
        let _ = (grid, paste);
    }

    /// The view's size changed to fit `size` cells of `cell` pixels.
    fn resize(&mut self, grid: &mut G, size: Size, cell: CellSize) {
        let _ = (grid, size, cell);
    }

    /// The view gained or lost keyboard focus, for programs that asked to
    /// know (focus reporting, mode 1004).
    fn focus(&mut self, grid: &mut G, focused: bool) {
        let _ = (grid, focused);
    }
}
