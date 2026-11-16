use terminal_grid::{Grid, KeyEvent, Mods, MouseEvent, Size, ViewportPoint};

use crate::paste::Paste;

/// Why an asynchronous paste's following input was canceled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputBufferError {
    Capacity {
        events: usize,
        bytes: usize,
        max_events: usize,
        max_bytes: usize,
    },
    ReadSequenceExhausted {
        last_id: u64,
    },
}
impl std::fmt::Display for InputBufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Capacity {
                events,
                bytes,
                max_events,
                max_bytes,
            } => write!(
                f,
                "The pending clipboard paste and following input were canceled because {events} events/{bytes} bytes exceed the {max_events}-event/{max_bytes}-byte buffer limit. Paste a smaller value or wait for clipboard access before adding more input. Retype only the unsent input."
            ),
            Self::ReadSequenceExhausted { last_id } => write!(
                f,
                "The clipboard read identifier space is exhausted after {last_id} reads. Reload or reopen the viewer before pasting again; waiting input was canceled."
            ),
        }
    }
}
impl std::error::Error for InputBufferError {}

/// Where to move the viewport in the history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollTo {
    /// To the oldest row.
    Top,
    /// To the newest output, which then keeps the view at the bottom.
    Bottom,
    /// Up by a screen, toward older output.
    PageUp,
    /// Down by a screen.
    PageDown,
    /// So that history row `n` (0 being the oldest, as in
    /// [`terminal_grid::Scrollbar::offset`]) is the top row in view.
    Row(u64),
}

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

    /// Modifier-clicked URL. Hosts decide which URL schemes may be opened.
    fn open_link(&mut self, uri: String) {
        let _ = uri;
    }

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

    /// The user selected the block of cells with `start` and `end` as
    /// opposite corners (Option-drag on a Mac, Alt-drag elsewhere). A host
    /// that can't select a block selects in reading order.
    fn select_rectangle(&mut self, grid: &mut G, start: ViewportPoint, end: ViewportPoint) {
        self.select(grid, Some((start, end)));
    }

    /// The user moved through the history with a key or the scrollbar.
    fn scroll_to(&mut self, grid: &mut G, to: ScrollTo) {
        let _ = (grid, to);
    }

    /// Text the input method committed, such as a composed Chinese or
    /// Japanese word, or an accented letter typed with a dead key. Send it
    /// as typed; it never carries control characters.
    fn text(&mut self, grid: &mut G, text: &str) {
        let _ = (grid, text);
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

    /// An explicitly requested asynchronous clipboard read failed. The embedder
    /// surfaces permission, secure-context or unsupported-content guidance.
    fn clipboard_read_failed(&mut self, error: gpui_kit::ClipboardReadError) {
        let _ = error;
    }

    /// A paste awaits explicit user confirmation; buffered keys must not approve it.
    fn paste_is_pending(&self) -> bool {
        false
    }

    /// Whether the most recent paste was accepted for local delivery. This is
    /// not a PTY-written receipt. False cancels queued keys after that paste.
    fn paste_input_accepted(&self) -> bool {
        true
    }

    /// Buffered typeahead was canceled rather than exceeding a pending-paste budget.
    fn input_buffer_failed(&mut self, error: InputBufferError) {
        let _ = error;
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
