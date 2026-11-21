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
    /// The OSC 8 URI at a viewport cell, supplied by the terminal engine.
    pub(super) fn hyperlink_uri(&self, at: ViewportPoint) -> Option<String> {
        let reference = self
            .grid_ref_at(ffi::POINT_TAG_VIEWPORT, at.x, u32::from(at.y))
            .ok()?;
        let mut len = 0;
        // SAFETY: a fresh reference, with a valid size output; NULL requests size only.
        let result = unsafe {
            ffi::ghostty_grid_ref_hyperlink_uri(&reference, std::ptr::null_mut(), 0, &mut len)
        };
        if result != ffi::OUT_OF_SPACE || len == 0 || len > 1024 * 1024 {
            return None;
        }
        let mut bytes = vec![0; len];
        // SAFETY: the reference remains live and bytes has the requested capacity.
        check(unsafe {
            ffi::ghostty_grid_ref_hyperlink_uri(
                &reference,
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut len,
            )
        })
        .ok()?;
        bytes.truncate(len);
        String::from_utf8(bytes).ok()
    }

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
            ffi::ghostty_terminal_get(
                self.raw.ptr,
                ffi::TERMINAL_DATA_MODE,
                (&raw mut config).cast(),
            )
        };
        result == ffi::SUCCESS && config.value
    }

    /// Selects the block of cells with `start` and `end` as opposite
    /// corners, as Option-drag does. [`terminal_grid::Grid::selection`]
    /// reports the same columns on each row and
    /// [`Terminal::selected_text`] copies them a row a line.
    pub fn select_rectangle(
        &mut self,
        start: ViewportPoint,
        end: ViewportPoint,
    ) -> Result<(), Error> {
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

/// Recognize a plain-text file path or `file://` URI in visible cells,
/// joining soft-wrapped rows.
pub(super) fn plain_file_link(grid: &Terminal, at: ViewportPoint) -> Option<String> {
    use terminal_grid::Grid as _;

    let row = grid.row(at.y)?;
    if usize::from(at.x) >= row.cells.len() {
        return None;
    }
    let mut first = at.y;
    while first > 0 && grid.row(first - 1).is_some_and(|r| r.wrapped) {
        first -= 1;
    }
    let mut text = String::new();
    let mut hit = 0;
    for y in first..grid.size().rows {
        let row = grid.row(y)?;
        for (x, cell) in row.cells.iter().enumerate() {
            if y == at.y && x == usize::from(at.x) {
                hit = text.len();
            }
            if cell.columns() > 0 {
                text.push_str(if cell.text.is_empty() {
                    " "
                } else {
                    &cell.text
                });
            }
            if text.len() > 16 * 1024 {
                return None;
            }
        }
        if !row.wrapped {
            break;
        }
    }

    let delimit = |c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`');
    let begin = text[..hit].rfind(delimit).map_or(0, |i| {
        i + text[i..].chars().next().map_or(0, char::len_utf8)
    });
    let end = text[hit..].find(delimit).map_or(text.len(), |i| hit + i);
    let mut token = &text[begin..end];
    token = token.trim_start_matches(['(', '[', '{']);
    token = token.trim_end_matches(['.', ',', ';', '!', ':']);
    for (open, close) in [('(', ')'), ('[', ']'), ('{', '}')] {
        while token.ends_with(close)
            && token.chars().filter(|c| *c == close).count()
                > token.chars().filter(|c| *c == open).count()
        {
            token = &token[..token.len() - 1];
        }
    }
    token = token.trim_end_matches(['.', ',', ';', '!', ':']);
    let start = token.as_ptr() as usize - text.as_ptr() as usize;
    if token.is_empty() || hit < start || hit >= start + token.len() {
        return None;
    }
    is_plain_file_link(token).then(|| token.to_string())
}

fn is_plain_file_link(token: &str) -> bool {
    if token.chars().any(char::is_control) {
        return false;
    }
    if token.len() > 7 && token[..7].eq_ignore_ascii_case("file://") {
        return true;
    }
    if token.contains("://") {
        return false;
    }
    let path = strip_line_suffix(token);
    if path.is_empty() || path.contains(':') {
        return false;
    }
    if path.starts_with('/') {
        return path.split('/').any(|seg| !seg.is_empty());
    }
    if path.contains('/') {
        return path
            .split('/')
            .any(|seg| !seg.is_empty() && seg != "." && seg != "..");
    }
    let Some((stem, ext)) = path.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && stem.bytes().any(|b| b.is_ascii_alphanumeric())
        && (1..=10).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        && ext.bytes().any(|b| b.is_ascii_alphabetic())
}

fn strip_line_suffix(token: &str) -> &str {
    let Some((head, tail)) = token.rsplit_once(':') else {
        return token;
    };
    if head.is_empty() || tail.is_empty() {
        return token;
    }
    if is_digits(tail) {
        if let Some((head2, line)) = head.rsplit_once(':')
            && !head2.is_empty()
            && is_digits(line)
        {
            return head2;
        }
        return head;
    }
    if let Some((start, end)) = tail.split_once('-')
        && is_digits(start)
        && is_digits(end)
    {
        return head;
    }
    token
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}
