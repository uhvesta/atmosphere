use std::ffi::c_void;
use std::ptr;

use crate::Grid;
use crate::cell::{Cell, CellWidth, Color, Row, Underline};
use crate::error::{Error, Status, check};
use crate::ffi;
use crate::input::{KeyEvent, MouseEvent, OptionAsAlt};

/// A grid size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

/// The cursor's position in the active area (the bottom `rows` rows of the
/// screen, which the viewport shows unless it is scrolled back) and whether
/// the program shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
}

/// A cell on the screen: column `x` of row `y`, where row 0 is the oldest
/// row of scrollback and the last row is the bottom of the active area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: u16,
    pub y: u32,
}

/// A search match, from its first cell to its last, both included.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: Point,
    pub end: Point,
}

/// Where to move the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    /// To the oldest row of scrollback.
    Top,
    /// To the active area, following new output.
    Bottom,
    /// By this many rows; negative is up, toward older rows.
    Delta(isize),
    /// So that this screen row is at the top.
    Row(usize),
}

/// Something the program asked for while [`Terminal::feed`] parsed its
/// output. Collect them with [`Terminal::drain_events`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The title changed (OSC 0 or OSC 2).
    TitleChanged(String),
    /// The working directory changed (OSC 7).
    PwdChanged(String),
    /// The program rang the bell (BEL).
    Bell,
    /// Bytes to write back to the program, such as the answer to a cursor
    /// position query. Send them to the pty in order.
    Reply(Vec<u8>),
}

/// Where callbacks put events while `ghostty_terminal_vt_write` runs.
#[derive(Default)]
struct Sink {
    events: Vec<Event>,
}

/// Owns the sink's allocation, whose address libghostty-vt holds as the
/// terminal's userdata. It is reached only through that raw pointer, never
/// a Rust reference, while the terminal runs callbacks.
struct SinkBox(*mut Sink);

impl SinkBox {
    fn new() -> Self {
        SinkBox(Box::into_raw(Box::new(Sink::default())))
    }

    fn take(&mut self) -> Vec<Event> {
        // SAFETY: the pointer came from Box::into_raw and is freed only in
        // drop. `&mut self` means no feed (and so no callback) is running.
        unsafe { std::mem::take(&mut (*self.0).events) }
    }
}

impl Drop for SinkBox {
    fn drop(&mut self) {
        // SAFETY: the pointer came from Box::into_raw and is freed once,
        // here, after the terminal that called back into it is freed.
        drop(unsafe { Box::from_raw(self.0) });
    }
}

/// A handle from libghostty-vt, freed exactly once when dropped.
struct Owned<T> {
    ptr: *mut T,
    free: unsafe extern "C" fn(*mut T),
}

impl<T> Owned<T> {
    fn new(
        create: impl FnOnce(*mut *mut T) -> ffi::GhosttyResult,
        free: unsafe extern "C" fn(*mut T),
    ) -> Result<Self, Status> {
        let mut ptr = ptr::null_mut();
        check(create(&mut ptr))?;
        Ok(Owned { ptr, free })
    }
}

impl<T> Drop for Owned<T> {
    fn drop(&mut self) {
        // SAFETY: `ptr` came from the matching `_new` call and nothing else
        // frees it. The free functions accept NULL.
        unsafe { (self.free)(self.ptr) }
    }
}

/// A terminal: the screen, its scrollback and its modes, fed by the bytes a
/// program writes.
pub struct Terminal {
    // Fields drop in order: the encoders, then the terminal, then the sink
    // the terminal's callbacks write into.
    key_encoder: Owned<ffi::KeyEncoderImpl>,
    mouse_encoder: Owned<ffi::MouseEncoderImpl>,
    raw: Owned<ffi::TerminalImpl>,
    sink: SinkBox,
    cols: u16,
    rows: u16,
    cell_width: u32,
    cell_height: u32,
    option_as_alt: OptionAsAlt,
}

// SAFETY: libghostty-vt keeps no thread-local state and starts no threads;
// a handle may move between threads as long as one thread uses it at a
// time, which `&mut self` and the missing `Sync` guarantee. The sink is
// owned by the terminal and touched only from its calls.
unsafe impl Send for Terminal {}

impl Terminal {
    /// A terminal `cols` wide and `rows` tall that keeps about
    /// `scrollback_lines` rows of history (libghostty-vt drops history a
    /// page at a time, so it keeps somewhat more).
    ///
    /// Grapheme clustering (mode 2027) starts on, so emoji sequences and
    /// combining marks stay in one cell as Ghostty draws them.
    pub fn new(cols: u16, rows: u16, scrollback_lines: usize) -> Result<Self, Error> {
        let create = |source| Error::Create { cols, rows, source };
        let raw = Owned::new(
            // SAFETY: NULL selects the default allocator.
            |out| unsafe { ffi::ghostty_terminal_new(ptr::null(), out, cols, rows) },
            ffi::ghostty_terminal_free,
        )
        .map_err(create)?;
        let key_encoder = Owned::new(
            // SAFETY: as above.
            |out| unsafe { ffi::ghostty_key_encoder_new(ptr::null(), out) },
            ffi::ghostty_key_encoder_free,
        )
        .map_err(create)?;
        let mouse_encoder = Owned::new(
            // SAFETY: as above.
            |out| unsafe { ffi::ghostty_mouse_encoder_new(ptr::null(), out) },
            ffi::ghostty_mouse_encoder_free,
        )
        .map_err(create)?;
        let mut terminal = Terminal {
            key_encoder,
            mouse_encoder,
            raw,
            sink: SinkBox::new(),
            cols,
            rows,
            cell_width: 1,
            cell_height: 1,
            option_as_alt: OptionAsAlt::Off,
        };
        terminal.configure(scrollback_lines)?;
        terminal.feed(b"\x1b[?2027h");
        Ok(terminal)
    }

    fn configure(&mut self, scrollback_lines: usize) -> Result<(), Error> {
        let raw = self.raw.ptr;
        let set = |what: &'static str, option, value: *const c_void| {
            // SAFETY: each value below is the type the option documents and
            // lives until the call returns; libghostty-vt copies it.
            check(unsafe { ffi::ghostty_terminal_set(raw, option, value) })
                .map_err(|source| Error::Configure { what, source })
        };

        // History is limited by lines only, not the default 10,000 bytes.
        let unlimited = usize::MAX;
        set(
            "scrollback size",
            ffi::TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
            (&raw const unlimited).cast(),
        )?;
        set(
            "scrollback length",
            ffi::TERMINAL_OPT_SCROLLBACK_MAX_LINES,
            (&raw const scrollback_lines).cast(),
        )?;

        // Callbacks and userdata are passed as the pointers themselves.
        set(
            "event handler",
            ffi::TERMINAL_OPT_USERDATA,
            self.sink.0.cast_const().cast(),
        )?;
        set(
            "title handler",
            ffi::TERMINAL_OPT_TITLE_CHANGED,
            on_title_changed as ffi::TitleChangedFn as *const c_void,
        )?;
        set(
            "directory handler",
            ffi::TERMINAL_OPT_PWD_CHANGED,
            on_pwd_changed as ffi::PwdChangedFn as *const c_void,
        )?;
        set(
            "bell handler",
            ffi::TERMINAL_OPT_BELL,
            on_bell as ffi::BellFn as *const c_void,
        )?;
        set(
            "reply handler",
            ffi::TERMINAL_OPT_WRITE_PTY,
            on_write_pty as ffi::WritePtyFn as *const c_void,
        )?;
        Ok(())
    }

    /// Parses bytes the program wrote: text, control sequences, anything.
    /// Malformed input is handled the way other terminals handle it, so this
    /// never fails. Events it raises wait in [`Terminal::drain_events`].
    pub fn feed(&mut self, bytes: &[u8]) {
        // SAFETY: the handle is live; callbacks reach only the sink, which
        // no Rust reference covers while this runs.
        unsafe { ffi::ghostty_terminal_vt_write(self.raw.ptr, bytes.as_ptr(), bytes.len()) }
    }

    /// The events raised since the last call, oldest first.
    pub fn drain_events(&mut self) -> Vec<Event> {
        self.sink.take()
    }

    /// Resizes the grid, keeping the cell size in pixels. The primary screen
    /// reflows wrapped lines to the new width; the alternate screen doesn't.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        self.resize_with_cell_size(cols, rows, self.cell_width, self.cell_height)
    }

    /// Resizes the grid and sets each cell's size in pixels, which programs
    /// can ask for and mouse positions are measured in. Until it is set a
    /// cell is 1x1 pixel.
    pub fn resize_with_cell_size(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<(), Error> {
        // SAFETY: the handle is live.
        check(unsafe {
            ffi::ghostty_terminal_resize(self.raw.ptr, cols, rows, cell_width, cell_height)
        })
        .map_err(|source| Error::Resize {
            cols,
            rows,
            cell_width,
            cell_height,
            source,
        })?;
        self.cols = cols;
        self.rows = rows;
        self.cell_width = cell_width;
        self.cell_height = cell_height;
        Ok(())
    }

    /// The grid's size in cells.
    pub fn size(&self) -> Size {
        Size {
            cols: self.get(ffi::TERMINAL_DATA_COLS).unwrap_or(self.cols),
            rows: self.get(ffi::TERMINAL_DATA_ROWS).unwrap_or(self.rows),
        }
    }

    /// Where the cursor is and whether it is shown.
    pub fn cursor(&self) -> Cursor {
        // Reads of a live handle fail only for an unknown data kind, so the
        // fallbacks never show.
        Cursor {
            x: self.get(ffi::TERMINAL_DATA_CURSOR_X).unwrap_or(0),
            y: self.get(ffi::TERMINAL_DATA_CURSOR_Y).unwrap_or(0),
            visible: self.get(ffi::TERMINAL_DATA_CURSOR_VISIBLE).unwrap_or(false),
        }
    }

    /// The title the program set, if it set one.
    pub fn title(&self) -> Option<String> {
        self.string(ffi::TERMINAL_DATA_TITLE)
    }

    /// The working directory the program reported, if it reported one.
    pub fn pwd(&self) -> Option<String> {
        self.string(ffi::TERMINAL_DATA_PWD)
    }

    /// Whether the alternate screen (full-screen programs such as editors)
    /// is showing.
    pub fn is_alternate_screen(&self) -> bool {
        self.get::<i32>(ffi::TERMINAL_DATA_ACTIVE_SCREEN)
            .is_ok_and(|screen| screen == ffi::SCREEN_ALTERNATE)
    }

    /// Whether the program asked for mouse events.
    pub fn is_mouse_tracking(&self) -> bool {
        self.get(ffi::TERMINAL_DATA_MOUSE_TRACKING).unwrap_or(false)
    }

    /// How many rows of history sit above the active area.
    pub fn scrollback_rows(&self) -> usize {
        self.get(ffi::TERMINAL_DATA_SCROLLBACK_ROWS).unwrap_or(0)
    }

    /// How many rows the screen has: history and the active area.
    pub fn total_rows(&self) -> usize {
        self.get(ffi::TERMINAL_DATA_TOTAL_ROWS)
            .unwrap_or(usize::from(self.rows))
    }

    /// Row `y` of the screen, 0 being the oldest row of history.
    pub fn screen_row(&self, y: u32) -> Result<Row, Error> {
        self.row_at(ffi::POINT_TAG_SCREEN, y)
    }

    /// Row `y` of the viewport, 0 being the top row in view.
    pub fn viewport_row(&self, y: u16) -> Result<Row, Error> {
        self.row_at(ffi::POINT_TAG_VIEWPORT, u32::from(y))
    }

    /// Moves the viewport through history.
    pub fn scroll_viewport(&mut self, scroll: Scroll) -> Result<(), Error> {
        let (tag, value) = match scroll {
            Scroll::Top => (
                ffi::SCROLL_VIEWPORT_TOP,
                ffi::GhosttyTerminalScrollViewportValue { _padding: [0; 2] },
            ),
            Scroll::Bottom => (
                ffi::SCROLL_VIEWPORT_BOTTOM,
                ffi::GhosttyTerminalScrollViewportValue { _padding: [0; 2] },
            ),
            Scroll::Delta(delta) => (
                ffi::SCROLL_VIEWPORT_DELTA,
                ffi::GhosttyTerminalScrollViewportValue { delta },
            ),
            Scroll::Row(row) => (
                ffi::SCROLL_VIEWPORT_ROW,
                ffi::GhosttyTerminalScrollViewportValue { row },
            ),
        };
        // SAFETY: the handle is live and the tag names the member set.
        check(unsafe {
            ffi::ghostty_terminal_scroll_viewport(
                self.raw.ptr,
                ffi::GhosttyTerminalScrollViewport { tag, value },
            )
        })
        .map_err(|source| Error::Scroll { source })
    }

    /// The text from `start` to `end`, both included, as it would be copied:
    /// a line that wrapped at the right edge joins the next one, and
    /// trailing blanks are trimmed.
    pub fn selection_text(&self, start: Point, end: Point) -> Result<String, Error> {
        let selection = ffi::GhosttySelection {
            start: self.grid_ref(start)?,
            end: self.grid_ref(end)?,
            rectangle: false,
        };
        let raw = self.raw.ptr;
        let format = |source| Error::Format { source };
        let formatter = Owned::new(
            // SAFETY: the formatter borrows the terminal, which outlives it
            // (it is freed at the end of this call).
            |out| unsafe { ffi::ghostty_formatter_terminal_new(ptr::null(), out, raw) },
            ffi::ghostty_formatter_free,
        )
        .map_err(format)?;
        let plain = ffi::FORMATTER_FORMAT_PLAIN;
        let yes = true;
        for (option, value) in [
            (ffi::FORMATTER_OPT_EMIT, (&raw const plain).cast::<c_void>()),
            (ffi::FORMATTER_OPT_UNWRAP, (&raw const yes).cast()),
            (ffi::FORMATTER_OPT_TRIM, (&raw const yes).cast()),
            (ffi::FORMATTER_OPT_SELECTION, (&raw const selection).cast()),
        ] {
            // SAFETY: each value is the type its option documents.
            check(unsafe { ffi::ghostty_formatter_set(formatter.ptr, option, value) })
                .map_err(format)?;
        }

        let mut out: *mut u8 = ptr::null_mut();
        let mut len = 0usize;
        // SAFETY: on success `out` holds `len` bytes we must free with
        // ghostty_free, or NULL when there is no text.
        check(unsafe {
            ffi::ghostty_formatter_format_alloc(formatter.ptr, ptr::null(), &mut out, &mut len)
        })
        .map_err(format)?;
        if out.is_null() {
            return Ok(String::new());
        }
        // SAFETY: `out` points at `len` initialized bytes until freed below.
        let text =
            String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(out, len) }).into_owned();
        // SAFETY: allocated by format_alloc with the default allocator.
        unsafe { ffi::ghostty_free(ptr::null(), out, len) };
        Ok(text)
    }

    /// Every match for `needle` on the active screen, history included,
    /// oldest first. ASCII letters match either case; everything else must
    /// match byte for byte.
    pub fn search(&mut self, needle: &str) -> Result<Vec<Match>, Error> {
        let fail = |source| Error::Search {
            needle: needle.to_owned(),
            source,
        };
        let raw = self.raw.ptr;
        let search = Owned::new(
            // SAFETY: the search is freed at the end of this call, before
            // the terminal it binds to.
            |out| unsafe { ffi::ghostty_search_new(ptr::null(), out, raw) },
            ffi::ghostty_search_free,
        )
        .map_err(fail)?;
        let needle_string = ffi::GhosttyString {
            ptr: needle.as_ptr(),
            len: needle.len(),
        };
        // SAFETY: the needle outlives the search, which is freed below.
        check(unsafe {
            ffi::ghostty_search_set(
                search.ptr,
                ffi::SEARCH_OPT_NEEDLE,
                (&raw const needle_string).cast(),
            )
        })
        .map_err(fail)?;
        // SAFETY: the search and its terminal are live.
        check(unsafe { ffi::ghostty_search_run(search.ptr) }).map_err(fail)?;

        let mut buffer = ffi::GhosttySelectionBuffer {
            ptr: ptr::null_mut(),
            cap: 0,
            len: 0,
        };
        // SAFETY: an empty buffer asks only for the count.
        let sized = unsafe {
            ffi::ghostty_search_get(
                search.ptr,
                ffi::SEARCH_DATA_MATCHES,
                (&raw mut buffer).cast(),
            )
        };
        if sized != ffi::OUT_OF_SPACE {
            check(sized).map_err(fail)?;
        }
        let mut selections = vec![ffi::GhosttySelection::EMPTY; buffer.len];
        if !selections.is_empty() {
            buffer = ffi::GhosttySelectionBuffer {
                ptr: selections.as_mut_ptr(),
                cap: selections.len(),
                len: 0,
            };
            // SAFETY: the buffer holds `cap` selections.
            check(unsafe {
                ffi::ghostty_search_get(
                    search.ptr,
                    ffi::SEARCH_DATA_MATCHES,
                    (&raw mut buffer).cast(),
                )
            })
            .map_err(fail)?;
            selections.truncate(buffer.len);
        }

        // libghostty-vt lists the newest match first.
        Ok(selections
            .iter()
            .rev()
            .filter_map(|selection| {
                Some(Match {
                    start: self.point(&selection.start)?,
                    end: self.point(&selection.end)?,
                })
            })
            .collect())
    }

    /// Option on a Mac: Alt, or the character macOS types. Off by default,
    /// as in Ghostty.
    pub fn set_option_as_alt(&mut self, option_as_alt: OptionAsAlt) {
        self.option_as_alt = option_as_alt;
    }

    /// The bytes to send the program for a key event, in whatever encoding
    /// the program asked for (cursor key mode, the Kitty keyboard protocol
    /// and so on). Empty when the key sends nothing, such as Shift alone.
    pub fn encode_key(&mut self, event: &KeyEvent<'_>) -> Result<Vec<u8>, Error> {
        let fail = |source| Error::EncodeKey { source };
        let encoder = self.key_encoder.ptr;
        let option_as_alt = self.option_as_alt as i32;
        // SAFETY: the encoder and terminal are live; the option takes a
        // pointer to the enum.
        unsafe {
            check(ffi::ghostty_key_encoder_set_from_terminal(
                encoder,
                self.raw.ptr,
            ))
            .map_err(fail)?;
            check(ffi::ghostty_key_encoder_set(
                encoder,
                ffi::KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT,
                (&raw const option_as_alt).cast(),
            ))
            .map_err(fail)?;
        }

        let key_event = Owned::new(
            // SAFETY: NULL selects the default allocator.
            |out| unsafe { ffi::ghostty_key_event_new(ptr::null(), out) },
            ffi::ghostty_key_event_free,
        )
        .map_err(fail)?;
        // The event keeps a pointer to the text, not a copy; `event.text`
        // outlives `key_event`, which is freed when this call returns.
        let text = if event.text.chars().any(|c| c.is_ascii_control()) {
            ""
        } else {
            event.text
        };
        // SAFETY: the event is live and the text outlives it.
        unsafe {
            let raw = key_event.ptr;
            check(ffi::ghostty_key_event_set_action(raw, event.action as i32)).map_err(fail)?;
            check(ffi::ghostty_key_event_set_key(raw, event.key as i32)).map_err(fail)?;
            check(ffi::ghostty_key_event_set_mods(raw, event.mods.bits())).map_err(fail)?;
            check(ffi::ghostty_key_event_set_consumed_mods(
                raw,
                event.consumed_mods.bits(),
            ))
            .map_err(fail)?;
            check(ffi::ghostty_key_event_set_composing(raw, event.composing)).map_err(fail)?;
            check(ffi::ghostty_key_event_set_utf8(
                raw,
                ffi::GhosttyString {
                    ptr: text.as_ptr(),
                    len: text.len(),
                },
            ))
            .map_err(fail)?;
            check(ffi::ghostty_key_event_set_unshifted_codepoint(
                raw,
                event.unshifted.map_or(0, u32::from),
            ))
            .map_err(fail)?;
        }

        encode(|buf, len, written| {
            // SAFETY: the encoder and event are live; `buf` holds `len` bytes.
            unsafe { ffi::ghostty_key_encoder_encode(encoder, key_event.ptr, buf, len, written) }
        })
        .map_err(fail)
    }

    /// The bytes to send the program for a mouse event, in the tracking mode
    /// and format the program asked for. Empty when the program didn't ask
    /// for mouse events, or not for this one.
    pub fn encode_mouse(&mut self, event: &MouseEvent) -> Result<Vec<u8>, Error> {
        let fail = |source| Error::EncodeMouse { source };
        let encoder = self.mouse_encoder.ptr;
        let size = ffi::GhosttyMouseEncoderSize {
            size: size_of::<ffi::GhosttyMouseEncoderSize>(),
            screen_width: u32::from(self.cols).saturating_mul(self.cell_width),
            screen_height: u32::from(self.rows).saturating_mul(self.cell_height),
            cell_width: self.cell_width,
            cell_height: self.cell_height,
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        };
        // SAFETY: the encoder and terminal are live; the size is a sized
        // struct whose `size` is its own.
        unsafe {
            check(ffi::ghostty_mouse_encoder_set_from_terminal(
                encoder,
                self.raw.ptr,
            ))
            .map_err(fail)?;
            check(ffi::ghostty_mouse_encoder_set(
                encoder,
                ffi::MOUSE_ENCODER_OPT_SIZE,
                (&raw const size).cast(),
            ))
            .map_err(fail)?;
        }

        let mouse_event = Owned::new(
            // SAFETY: NULL selects the default allocator.
            |out| unsafe { ffi::ghostty_mouse_event_new(ptr::null(), out) },
            ffi::ghostty_mouse_event_free,
        )
        .map_err(fail)?;
        // SAFETY: the event is live.
        unsafe {
            let raw = mouse_event.ptr;
            check(ffi::ghostty_mouse_event_set_action(
                raw,
                event.action as i32,
            ))
            .map_err(fail)?;
            match event.button {
                Some(button) => check(ffi::ghostty_mouse_event_set_button(raw, button as i32)),
                None => check(ffi::ghostty_mouse_event_clear_button(raw)),
            }
            .map_err(fail)?;
            check(ffi::ghostty_mouse_event_set_mods(raw, event.mods.bits())).map_err(fail)?;
            check(ffi::ghostty_mouse_event_set_position(
                raw,
                ffi::GhosttyMousePosition {
                    x: event.x,
                    y: event.y,
                },
            ))
            .map_err(fail)?;
        }

        encode(|buf, len, written| {
            // SAFETY: the encoder and event are live; `buf` holds `len` bytes.
            unsafe {
                ffi::ghostty_mouse_encoder_encode(encoder, mouse_event.ptr, buf, len, written)
            }
        })
        .map_err(fail)
    }

    fn get<T: Default>(&self, data: ffi::GhosttyTerminalData) -> Result<T, Status> {
        let mut out = T::default();
        // SAFETY: callers pick `T` as the output type `data` documents.
        check(unsafe { ffi::ghostty_terminal_get(self.raw.ptr, data, (&raw mut out).cast()) })?;
        Ok(out)
    }

    fn string(&self, data: ffi::GhosttyTerminalData) -> Option<String> {
        let mut out = ffi::GhosttyString {
            ptr: ptr::null(),
            len: 0,
        };
        // SAFETY: `data` is a string kind; the bytes stay valid until the
        // terminal next changes, and are copied before this returns.
        check(unsafe { ffi::ghostty_terminal_get(self.raw.ptr, data, (&raw mut out).cast()) })
            .ok()?;
        string_from(out)
    }

    fn grid_ref(&self, point: Point) -> Result<ffi::GhosttyGridRef, Error> {
        self.grid_ref_at(ffi::POINT_TAG_SCREEN, point.x, point.y)
            .map_err(|source| Error::Point {
                x: point.x,
                y: point.y,
                source,
            })
    }

    fn grid_ref_at(
        &self,
        tag: ffi::GhosttyPointTag,
        x: u16,
        y: u32,
    ) -> Result<ffi::GhosttyGridRef, Status> {
        let point = ffi::GhosttyPoint {
            tag,
            value: ffi::GhosttyPointValue {
                coordinate: ffi::GhosttyPointCoordinate { x, y },
            },
        };
        let mut out = ffi::GhosttyGridRef::EMPTY;
        // SAFETY: the handle is live; the ref is used only before the
        // terminal next changes, which `&self` rules out.
        check(unsafe { ffi::ghostty_terminal_grid_ref(self.raw.ptr, point, &mut out) })?;
        Ok(out)
    }

    fn point(&self, grid_ref: &ffi::GhosttyGridRef) -> Option<Point> {
        let mut out = ffi::GhosttyPointCoordinate::default();
        // SAFETY: the ref came from this terminal, which hasn't changed since.
        check(unsafe {
            ffi::ghostty_terminal_point_from_grid_ref(
                self.raw.ptr,
                grid_ref,
                ffi::POINT_TAG_SCREEN,
                &mut out,
            )
        })
        .ok()?;
        Some(Point { x: out.x, y: out.y })
    }

    fn row_at(&self, tag: ffi::GhosttyPointTag, y: u32) -> Result<Row, Error> {
        let row_error = |source| Error::Row { y, source };
        let first = self.grid_ref_at(tag, 0, y).map_err(row_error)?;
        let mut row = 0;
        // SAFETY: the ref is fresh.
        check(unsafe { ffi::ghostty_grid_ref_row(&first, &mut row) }).map_err(row_error)?;
        let mut wrapped = false;
        // SAFETY: WRAP's output is a bool.
        check(unsafe { ffi::ghostty_row_get(row, ffi::ROW_DATA_WRAP, (&raw mut wrapped).cast()) })
            .map_err(row_error)?;

        let cols = self.size().cols;
        let mut cells = Vec::with_capacity(usize::from(cols));
        let mut codepoints = Vec::new();
        for x in 0..cols {
            let grid_ref = self.grid_ref_at(tag, x, y).map_err(row_error)?;
            cells.push(read_cell(&grid_ref, &mut codepoints).map_err(row_error)?);
        }
        Ok(Row { cells, wrapped })
    }
}

impl Grid for Terminal {
    fn size(&self) -> Size {
        Terminal::size(self)
    }

    fn row(&self, y: u16) -> impl Iterator<Item = Cell> {
        self.viewport_row(y)
            .map(|row| row.cells)
            .unwrap_or_default()
            .into_iter()
    }

    fn cursor(&self) -> Cursor {
        Terminal::cursor(self)
    }
}

fn read_cell(grid_ref: &ffi::GhosttyGridRef, codepoints: &mut Vec<u32>) -> Result<Cell, Status> {
    let mut raw_cell = 0;
    let mut style = ffi::GhosttyStyle::EMPTY;
    // SAFETY: the ref is fresh; the outputs are the documented types.
    unsafe {
        check(ffi::ghostty_grid_ref_cell(grid_ref, &mut raw_cell))?;
        check(ffi::ghostty_grid_ref_style(grid_ref, &mut style))?;
    }
    let wide: i32 = cell_get(raw_cell, ffi::CELL_DATA_WIDE)?;
    let content: i32 = cell_get(raw_cell, ffi::CELL_DATA_CONTENT_TAG)?;

    // A cell erased with a background color holds just that color.
    let bg = match content {
        ffi::CELL_CONTENT_BG_COLOR_PALETTE => {
            Color::Palette(cell_get(raw_cell, ffi::CELL_DATA_COLOR_PALETTE)?)
        }
        ffi::CELL_CONTENT_BG_COLOR_RGB => {
            let rgb: ffi::GhosttyColorRgb = cell_get(raw_cell, ffi::CELL_DATA_COLOR_RGB)?;
            Color::Rgb {
                r: rgb.r,
                g: rgb.g,
                b: rgb.b,
            }
        }
        _ => Color::from_style(style.bg_color),
    };

    let flags = style.flags;
    Ok(Cell {
        text: graphemes(grid_ref, codepoints)?,
        width: match wide {
            ffi::CELL_WIDE_WIDE => CellWidth::Wide,
            ffi::CELL_WIDE_SPACER_TAIL => CellWidth::SpacerTail,
            ffi::CELL_WIDE_SPACER_HEAD => CellWidth::SpacerHead,
            _ => CellWidth::Narrow,
        },
        fg: Color::from_style(style.fg_color),
        bg,
        underline_color: Color::from_style(style.underline_color),
        bold: flags & ffi::STYLE_FLAG_BOLD != 0,
        italic: flags & ffi::STYLE_FLAG_ITALIC != 0,
        faint: flags & ffi::STYLE_FLAG_FAINT != 0,
        blink: flags & ffi::STYLE_FLAG_BLINK != 0,
        inverse: flags & ffi::STYLE_FLAG_INVERSE != 0,
        invisible: flags & ffi::STYLE_FLAG_INVISIBLE != 0,
        strikethrough: flags & ffi::STYLE_FLAG_STRIKETHROUGH != 0,
        overline: flags & ffi::STYLE_FLAG_OVERLINE != 0,
        underline: Underline::from_flags(flags),
    })
}

fn cell_get<T: Default>(cell: ffi::GhosttyCell, data: ffi::GhosttyCellData) -> Result<T, Status> {
    let mut out = T::default();
    // SAFETY: callers pick `T` as the output type `data` documents.
    check(unsafe { ffi::ghostty_cell_get(cell, data, (&raw mut out).cast()) })?;
    Ok(out)
}

fn graphemes(grid_ref: &ffi::GhosttyGridRef, codepoints: &mut Vec<u32>) -> Result<String, Status> {
    if codepoints.len() < 8 {
        codepoints.resize(8, 0);
    }
    let mut written = 0;
    loop {
        // SAFETY: the buffer holds `len` codepoints.
        let result = unsafe {
            ffi::ghostty_grid_ref_graphemes(
                grid_ref,
                codepoints.as_mut_ptr(),
                codepoints.len(),
                &mut written,
            )
        };
        if result == ffi::OUT_OF_SPACE && written > codepoints.len() {
            codepoints.resize(written, 0);
            continue;
        }
        check(result)?;
        break;
    }
    Ok(codepoints[..written.min(codepoints.len())]
        .iter()
        .filter_map(|&c| char::from_u32(c))
        .collect())
}

/// Runs an encoder into a buffer, growing it when the output doesn't fit.
fn encode(
    mut run: impl FnMut(*mut u8, usize, *mut usize) -> ffi::GhosttyResult,
) -> Result<Vec<u8>, Status> {
    let mut buf = vec![0u8; 64];
    loop {
        let mut written = 0;
        let result = run(buf.as_mut_ptr(), buf.len(), &mut written);
        if result == ffi::OUT_OF_SPACE && written > buf.len() {
            buf.resize(written, 0);
            continue;
        }
        check(result)?;
        buf.truncate(written);
        return Ok(buf);
    }
}

fn string_from(string: ffi::GhosttyString) -> Option<String> {
    if string.ptr.is_null() || string.len == 0 {
        return None;
    }
    // SAFETY: libghostty-vt hands out `len` valid bytes, borrowed until the
    // terminal next changes; they are copied here.
    let bytes = unsafe { std::slice::from_raw_parts(string.ptr, string.len) };
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn push_event(userdata: *mut c_void, event: Event) {
    let sink = userdata.cast::<Sink>();
    if sink.is_null() {
        return;
    }
    // SAFETY: userdata is the SinkBox pointer, live for the terminal's
    // lifetime; callbacks run on the thread inside `feed`, while no Rust
    // reference to the sink exists.
    unsafe { (*sink).events.push(event) }
}

fn read_string(terminal: ffi::GhosttyTerminal, data: ffi::GhosttyTerminalData) -> String {
    let mut out = ffi::GhosttyString {
        ptr: ptr::null(),
        len: 0,
    };
    // SAFETY: the terminal is the live one calling back; `data` is a string.
    let result = unsafe { ffi::ghostty_terminal_get(terminal, data, (&raw mut out).cast()) };
    if result != ffi::SUCCESS {
        return String::new();
    }
    string_from(out).unwrap_or_default()
}

unsafe extern "C" fn on_title_changed(terminal: ffi::GhosttyTerminal, userdata: *mut c_void) {
    let title = read_string(terminal, ffi::TERMINAL_DATA_TITLE);
    push_event(userdata, Event::TitleChanged(title));
}

unsafe extern "C" fn on_pwd_changed(terminal: ffi::GhosttyTerminal, userdata: *mut c_void) {
    let pwd = read_string(terminal, ffi::TERMINAL_DATA_PWD);
    push_event(userdata, Event::PwdChanged(pwd));
}

unsafe extern "C" fn on_bell(_terminal: ffi::GhosttyTerminal, userdata: *mut c_void) {
    push_event(userdata, Event::Bell);
}

unsafe extern "C" fn on_write_pty(
    _terminal: ffi::GhosttyTerminal,
    userdata: *mut c_void,
    data: *const u8,
    len: usize,
) {
    if data.is_null() || len == 0 {
        return;
    }
    // SAFETY: libghostty-vt passes `len` valid bytes for this call.
    let bytes = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    push_event(userdata, Event::Reply(bytes));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>() {}

    #[test]
    fn a_terminal_can_move_to_another_thread() {
        assert_send::<Terminal>();
    }
}
