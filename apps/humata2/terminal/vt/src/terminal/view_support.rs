//! What an embedded terminal view needs beyond drawing: where the viewport
//! sits in the history, the modes a host reads to route input, a
//! rectangular selection and focus reports.

use terminal_grid::{OptionAsAlt, Scrollbar, ViewportPoint};

use super::{Terminal, encode};
use crate::error::{Error, check};
use crate::ffi;

/// `GHOSTTY_TERMINAL_DATA_SCROLLBAR`: a `GhosttyTerminalScrollbar`.
const TERMINAL_DATA_SCROLLBAR: ffi::GhosttyTerminalData = 9;
/// `GhosttyFocusEvent`.
const FOCUS_GAINED: i32 = 0;
const FOCUS_LOST: i32 = 1;

/// `GhosttyTerminalScrollbar`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RawScrollbar {
    total: u64,
    offset: u64,
    len: u64,
}

unsafe extern "C" {
    fn ghostty_focus_encode(
        event: i32,
        buf: *mut u8,
        buf_len: usize,
        out_written: *mut usize,
    ) -> ffi::GhosttyResult;
}

/// A DEC private mode a host reads to decide where input goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecMode {
    /// DECCKM (1): arrow keys send application sequences. The key encoder
    /// already follows it; hosts read it only to show or log it.
    ApplicationCursorKeys,
    /// 1004: the program wants focus in and out reports.
    FocusReporting,
    /// 1007: on the alternate screen, the wheel sends arrow keys instead
    /// of scrolling history (`less`, `vim` without mouse reporting).
    AlternateScroll,
    /// 2004: pasted text arrives between bracketed paste markers.
    BracketedPaste,
}

impl DecMode {
    fn number(self) -> ffi::GhosttyMode {
        match self {
            DecMode::ApplicationCursorKeys => 1,
            DecMode::FocusReporting => 1004,
            DecMode::AlternateScroll => 1007,
            DecMode::BracketedPaste => ffi::MODE_BRACKETED_PASTE,
        }
    }
}

impl Terminal {
    /// What Option does on a Mac, as set with
    /// [`Terminal::set_option_as_alt`].
    pub fn option_as_alt(&self) -> OptionAsAlt {
        self.option_as_alt
    }

    /// Where the viewport sits in the history, for a scrollbar. `None`
    /// only if the engine couldn't say, which it always can for a live
    /// terminal.
    pub fn scrollbar(&self) -> Option<Scrollbar> {
        let raw: RawScrollbar = self.get(TERMINAL_DATA_SCROLLBAR).ok()?;
        Some(Scrollbar {
            total: raw.total,
            offset: raw.offset,
            len: raw.len,
        })
    }

    /// Whether the program turned `mode` on.
    pub fn is_mode_set(&self, mode: DecMode) -> bool {
        let mut config = ffi::GhosttyTerminalModeConfig {
            mode: mode.number(),
            value: false,
        };
        // SAFETY: MODE reads and writes a GhosttyTerminalModeConfig.
        let result = unsafe {
            ffi::ghostty_terminal_get(self.raw.ptr, ffi::TERMINAL_DATA_MODE, (&raw mut config).cast())
        };
        result == ffi::SUCCESS && config.value
    }

    /// Selects the block of cells with `start` and `end` as opposite
    /// corners, as Option-drag does. [`terminal_grid::Grid::selection`]
    /// reports the same columns on each row and
    /// [`Terminal::selected_text`] copies them a row a line.
    pub fn select_rectangle(&mut self, start: ViewportPoint, end: ViewportPoint) -> Result<(), Error> {
        let fail = |source| Error::Selection { source };
        let selection = ffi::GhosttySelection {
            start: self
                .grid_ref_at(ffi::POINT_TAG_VIEWPORT, start.x, u32::from(start.y))
                .map_err(fail)?,
            end: self
                .grid_ref_at(ffi::POINT_TAG_VIEWPORT, end.x, u32::from(end.y))
                .map_err(fail)?,
            rectangle: true,
        };
        // SAFETY: the refs are fresh; the terminal copies the selection.
        check(unsafe {
            ffi::ghostty_terminal_set(
                self.raw.ptr,
                ffi::TERMINAL_OPT_SELECTION,
                (&raw const selection).cast(),
            )
        })
        .map_err(fail)
    }

    /// The report to send the program when the view gains or loses focus,
    /// or nothing when it didn't ask for focus reports (mode 1004).
    pub fn focus_report(&self, focused: bool) -> Result<Option<Vec<u8>>, Error> {
        if !self.is_mode_set(DecMode::FocusReporting) {
            return Ok(None);
        }
        let event = if focused { FOCUS_GAINED } else { FOCUS_LOST };
        encode(|buf, len, written| {
            // SAFETY: `buf` holds `len` bytes; `written` is a valid out pointer.
            unsafe { ghostty_focus_encode(event, buf, len, written) }
        })
        .map(Some)
        .map_err(|source| Error::EncodeFocus { source })
    }
}
