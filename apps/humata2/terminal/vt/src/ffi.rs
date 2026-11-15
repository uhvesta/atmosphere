//! Hand-written declarations for the parts of libghostty-vt's C API this
//! crate uses, from the headers under `include/ghostty/vt/` of the pinned
//! release. Every enum in the headers is a C `int`; every handle is an opaque
//! pointer. Struct layouts here must match the headers exactly.

#![allow(dead_code)]

use std::ffi::c_void;

pub type GhosttyResult = i32;
pub const SUCCESS: GhosttyResult = 0;
pub const OUT_OF_MEMORY: GhosttyResult = -1;
pub const INVALID_VALUE: GhosttyResult = -2;
pub const OUT_OF_SPACE: GhosttyResult = -3;
pub const NO_VALUE: GhosttyResult = -4;
pub const IO_ERROR: GhosttyResult = -5;
pub const LIMIT_EXCEEDED: GhosttyResult = -6;
pub const REJECTED: GhosttyResult = -7;
pub const UNSUPPORTED: GhosttyResult = -8;

#[repr(C)]
pub struct TerminalImpl {
    _private: [u8; 0],
}
#[repr(C)]
pub struct FormatterImpl {
    _private: [u8; 0],
}
#[repr(C)]
pub struct SearchImpl {
    _private: [u8; 0],
}
#[repr(C)]
pub struct KeyEncoderImpl {
    _private: [u8; 0],
}
#[repr(C)]
pub struct KeyEventImpl {
    _private: [u8; 0],
}
#[repr(C)]
pub struct MouseEncoderImpl {
    _private: [u8; 0],
}
#[repr(C)]
pub struct MouseEventImpl {
    _private: [u8; 0],
}

pub type GhosttyTerminal = *mut TerminalImpl;
pub type GhosttyFormatter = *mut FormatterImpl;
pub type GhosttySearch = *mut SearchImpl;
pub type GhosttyKeyEncoder = *mut KeyEncoderImpl;
pub type GhosttyKeyEvent = *mut KeyEventImpl;
pub type GhosttyMouseEncoder = *mut MouseEncoderImpl;
pub type GhosttyMouseEvent = *mut MouseEventImpl;

/// `GhosttyAllocator`; this crate always passes NULL for the default one.
#[repr(C)]
pub struct GhosttyAllocator {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyString {
    pub ptr: *const u8,
    pub len: usize,
}

// types.h / terminal.h

pub type GhosttyTerminalOption = i32;
pub const TERMINAL_OPT_USERDATA: GhosttyTerminalOption = 1;
pub const TERMINAL_OPT_WRITE_PTY: GhosttyTerminalOption = 2;
pub const TERMINAL_OPT_BELL: GhosttyTerminalOption = 3;
pub const TERMINAL_OPT_TITLE_CHANGED: GhosttyTerminalOption = 6;
pub const TERMINAL_OPT_PWD_CHANGED: GhosttyTerminalOption = 26;
pub const TERMINAL_OPT_SCROLLBACK_MAX_BYTES: GhosttyTerminalOption = 28;
pub const TERMINAL_OPT_SCROLLBACK_MAX_LINES: GhosttyTerminalOption = 29;
pub const TERMINAL_OPT_COLOR_FOREGROUND: GhosttyTerminalOption = 12;
pub const TERMINAL_OPT_COLOR_BACKGROUND: GhosttyTerminalOption = 13;
pub const TERMINAL_OPT_COLOR_CURSOR: GhosttyTerminalOption = 14;
pub const TERMINAL_OPT_COLOR_PALETTE: GhosttyTerminalOption = 15;
pub const TERMINAL_OPT_SELECTION: GhosttyTerminalOption = 22;
pub const TERMINAL_OPT_CLIPBOARD_WRITE: GhosttyTerminalOption = 27;
pub const TERMINAL_OPT_CLIPBOARD_READ: GhosttyTerminalOption = 39;

pub type GhosttyTerminalData = i32;
pub const TERMINAL_DATA_COLS: GhosttyTerminalData = 1;
pub const TERMINAL_DATA_ROWS: GhosttyTerminalData = 2;
pub const TERMINAL_DATA_CURSOR_X: GhosttyTerminalData = 3;
pub const TERMINAL_DATA_CURSOR_Y: GhosttyTerminalData = 4;
pub const TERMINAL_DATA_CURSOR_PENDING_WRAP: GhosttyTerminalData = 5;
pub const TERMINAL_DATA_ACTIVE_SCREEN: GhosttyTerminalData = 6;
pub const TERMINAL_DATA_CURSOR_VISIBLE: GhosttyTerminalData = 7;
pub const TERMINAL_DATA_MOUSE_TRACKING: GhosttyTerminalData = 11;
pub const TERMINAL_DATA_TITLE: GhosttyTerminalData = 12;
pub const TERMINAL_DATA_PWD: GhosttyTerminalData = 13;
pub const TERMINAL_DATA_TOTAL_ROWS: GhosttyTerminalData = 14;
pub const TERMINAL_DATA_SCROLLBACK_ROWS: GhosttyTerminalData = 15;
pub const TERMINAL_DATA_SELECTION: GhosttyTerminalData = 31;
pub const TERMINAL_DATA_MODE: GhosttyTerminalData = 37;

/// `GhosttyMode`: the mode number in the low 15 bits, and bit 15 set for
/// an ANSI mode rather than a DEC private one.
pub type GhosttyMode = u16;
pub const MODE_BRACKETED_PASTE: GhosttyMode = 2004;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GhosttyTerminalModeConfig {
    pub mode: GhosttyMode,
    pub value: bool,
}

pub const SCREEN_PRIMARY: i32 = 0;
pub const SCREEN_ALTERNATE: i32 = 1;

pub type BellFn = unsafe extern "C" fn(GhosttyTerminal, *mut c_void);
pub type TitleChangedFn = unsafe extern "C" fn(GhosttyTerminal, *mut c_void);
pub type PwdChangedFn = unsafe extern "C" fn(GhosttyTerminal, *mut c_void);
pub type WritePtyFn = unsafe extern "C" fn(GhosttyTerminal, *mut c_void, *const u8, usize);

pub type GhosttyTerminalScrollViewportTag = i32;
pub const SCROLL_VIEWPORT_TOP: GhosttyTerminalScrollViewportTag = 0;
pub const SCROLL_VIEWPORT_BOTTOM: GhosttyTerminalScrollViewportTag = 1;
pub const SCROLL_VIEWPORT_DELTA: GhosttyTerminalScrollViewportTag = 2;
pub const SCROLL_VIEWPORT_ROW: GhosttyTerminalScrollViewportTag = 3;

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyTerminalScrollViewportValue {
    pub delta: isize,
    pub row: usize,
    pub _padding: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyTerminalScrollViewport {
    pub tag: GhosttyTerminalScrollViewportTag,
    pub value: GhosttyTerminalScrollViewportValue,
}

// point.h

pub type GhosttyPointTag = i32;
pub const POINT_TAG_ACTIVE: GhosttyPointTag = 0;
pub const POINT_TAG_VIEWPORT: GhosttyPointTag = 1;
pub const POINT_TAG_SCREEN: GhosttyPointTag = 2;
pub const POINT_TAG_HISTORY: GhosttyPointTag = 3;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GhosttyPointCoordinate {
    pub x: u16,
    pub y: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyPointValue {
    pub coordinate: GhosttyPointCoordinate,
    pub _padding: [u64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyPoint {
    pub tag: GhosttyPointTag,
    pub value: GhosttyPointValue,
}

// grid_ref.h, selection.h

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyGridRef {
    pub node: *mut c_void,
    pub x: u16,
    pub y: u16,
}

impl GhosttyGridRef {
    pub const EMPTY: Self = Self {
        node: std::ptr::null_mut(),
        x: 0,
        y: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttySelection {
    pub start: GhosttyGridRef,
    pub end: GhosttyGridRef,
    pub rectangle: bool,
}

impl GhosttySelection {
    pub const EMPTY: Self = Self {
        start: GhosttyGridRef::EMPTY,
        end: GhosttyGridRef::EMPTY,
        rectangle: false,
    };
}

#[repr(C)]
pub struct GhosttySelectionBuffer {
    pub ptr: *mut GhosttySelection,
    pub cap: usize,
    pub len: usize,
}

// screen.h

pub type GhosttyCell = u64;
pub type GhosttyRow = u64;

pub type GhosttyCellData = i32;
pub const CELL_DATA_CODEPOINT: GhosttyCellData = 1;
pub const CELL_DATA_CONTENT_TAG: GhosttyCellData = 2;
pub const CELL_DATA_WIDE: GhosttyCellData = 3;
pub const CELL_DATA_HAS_TEXT: GhosttyCellData = 4;
pub const CELL_DATA_COLOR_PALETTE: GhosttyCellData = 10;
pub const CELL_DATA_COLOR_RGB: GhosttyCellData = 11;

pub const CELL_CONTENT_CODEPOINT: i32 = 0;
pub const CELL_CONTENT_CODEPOINT_GRAPHEME: i32 = 1;
pub const CELL_CONTENT_BG_COLOR_PALETTE: i32 = 2;
pub const CELL_CONTENT_BG_COLOR_RGB: i32 = 3;

pub const CELL_WIDE_NARROW: i32 = 0;
pub const CELL_WIDE_WIDE: i32 = 1;
pub const CELL_WIDE_SPACER_TAIL: i32 = 2;
pub const CELL_WIDE_SPACER_HEAD: i32 = 3;

pub type GhosttyRowData = i32;
pub const ROW_DATA_WRAP: GhosttyRowData = 1;
pub const ROW_DATA_WRAP_CONTINUATION: GhosttyRowData = 2;

// color.h, style.h

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GhosttyColorRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const STYLE_COLOR_NONE: u8 = 0;
pub const STYLE_COLOR_PALETTE: u8 = 1;
pub const STYLE_COLOR_RGB: u8 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub union GhosttyStyleColorValue {
    pub palette: u8,
    pub rgb: GhosttyColorRgb,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyStyleColor {
    pub tag: u8,
    pub value: GhosttyStyleColorValue,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyStyle {
    pub fg_color: GhosttyStyleColor,
    pub bg_color: GhosttyStyleColor,
    pub underline_color: GhosttyStyleColor,
    pub flags: u16,
    pub reserved: u16,
}

impl GhosttyStyle {
    pub const EMPTY: Self = {
        let none = GhosttyStyleColor {
            tag: STYLE_COLOR_NONE,
            value: GhosttyStyleColorValue {
                rgb: GhosttyColorRgb { r: 0, g: 0, b: 0 },
            },
        };
        Self {
            fg_color: none,
            bg_color: none,
            underline_color: none,
            flags: 0,
            reserved: 0,
        }
    };
}

pub const STYLE_FLAG_BOLD: u16 = 1 << 0;
pub const STYLE_FLAG_ITALIC: u16 = 1 << 1;
pub const STYLE_FLAG_FAINT: u16 = 1 << 2;
pub const STYLE_FLAG_BLINK: u16 = 1 << 3;
pub const STYLE_FLAG_INVERSE: u16 = 1 << 4;
pub const STYLE_FLAG_INVISIBLE: u16 = 1 << 5;
pub const STYLE_FLAG_STRIKETHROUGH: u16 = 1 << 6;
pub const STYLE_FLAG_OVERLINE: u16 = 1 << 7;
pub const STYLE_UNDERLINE_SHIFT: u16 = 8;
pub const STYLE_UNDERLINE_MASK: u16 = 0x7 << STYLE_UNDERLINE_SHIFT;

// formatter.h

pub type GhosttyFormatterOption = i32;
pub const FORMATTER_OPT_EMIT: GhosttyFormatterOption = 1;
pub const FORMATTER_OPT_UNWRAP: GhosttyFormatterOption = 2;
pub const FORMATTER_OPT_TRIM: GhosttyFormatterOption = 3;
pub const FORMATTER_OPT_SELECTION: GhosttyFormatterOption = 4;

pub const FORMATTER_FORMAT_PLAIN: i32 = 0;

// search.h

pub type GhosttySearchOption = i32;
pub const SEARCH_OPT_NEEDLE: GhosttySearchOption = 1;

pub type GhosttySearchData = i32;
pub const SEARCH_DATA_TOTAL_MATCHES: GhosttySearchData = 3;
pub const SEARCH_DATA_MATCHES: GhosttySearchData = 6;

// key/event.h, key/encoder.h

pub type GhosttyKey = i32;
pub type GhosttyKeyAction = i32;
pub type GhosttyMods = u16;

pub type GhosttyKeyEncoderOption = i32;
pub const KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT: GhosttyKeyEncoderOption = 7;

// mouse/event.h, mouse/encoder.h

pub type GhosttyMouseAction = i32;
pub type GhosttyMouseButton = i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyMousePosition {
    pub x: f32,
    pub y: f32,
}

pub type GhosttyMouseEncoderOption = i32;
pub const MOUSE_ENCODER_OPT_SIZE: GhosttyMouseEncoderOption = 3;

/// `GhosttyMouseEncoderSize`, a sized struct: `size` must be its own size.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyMouseEncoderSize {
    pub size: usize,
    pub screen_width: u32,
    pub screen_height: u32,
    pub cell_width: u32,
    pub cell_height: u32,
    pub padding_top: u32,
    pub padding_bottom: u32,
    pub padding_right: u32,
    pub padding_left: u32,
}

// terminal.h: clipboard requests (OSC 52, OSC 5522)

pub type GhosttyClipboardLocation = i32;
pub const CLIPBOARD_LOCATION_STANDARD: GhosttyClipboardLocation = 0;
pub const CLIPBOARD_LOCATION_SELECTION: GhosttyClipboardLocation = 1;
pub const CLIPBOARD_LOCATION_PRIMARY: GhosttyClipboardLocation = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyClipboardContent {
    pub mime: GhosttyString,
    pub data: GhosttyString,
}

#[repr(C)]
pub struct GhosttyClipboardWrite {
    pub location: GhosttyClipboardLocation,
    pub contents: *const GhosttyClipboardContent,
    pub contents_len: usize,
    pub name: GhosttyString,
    pub granted: bool,
    pub can_remember: bool,
    pub ctx: *const c_void,
    pub reply: *const c_void,
}

#[repr(C)]
pub struct GhosttyClipboardRead {
    pub location: GhosttyClipboardLocation,
    pub mimes: *const GhosttyString,
    pub mimes_len: usize,
    pub list: bool,
    pub name: GhosttyString,
    pub granted: bool,
    pub can_remember: bool,
    pub ctx: *const c_void,
    pub reply: *const c_void,
}

pub type ClipboardWriteFn =
    unsafe extern "C" fn(GhosttyTerminal, *mut c_void, *const GhosttyClipboardWrite);
pub type ClipboardReadFn =
    unsafe extern "C" fn(GhosttyTerminal, *mut c_void, *const GhosttyClipboardRead);

// render.h

#[repr(C)]
pub struct RenderStateImpl {
    _private: [u8; 0],
}
#[repr(C)]
pub struct RowIteratorImpl {
    _private: [u8; 0],
}
#[repr(C)]
pub struct RowCellsImpl {
    _private: [u8; 0],
}

pub type GhosttyRenderState = *mut RenderStateImpl;
pub type GhosttyRenderStateRowIterator = *mut RowIteratorImpl;
pub type GhosttyRenderStateRowCells = *mut RowCellsImpl;

pub type GhosttyRenderStateDirty = i32;
pub const RENDER_STATE_DIRTY_FALSE: GhosttyRenderStateDirty = 0;
pub const RENDER_STATE_DIRTY_PARTIAL: GhosttyRenderStateDirty = 1;
pub const RENDER_STATE_DIRTY_FULL: GhosttyRenderStateDirty = 2;

pub type GhosttyRenderStateData = i32;
pub const RENDER_STATE_DATA_COLS: GhosttyRenderStateData = 1;
pub const RENDER_STATE_DATA_ROWS: GhosttyRenderStateData = 2;
pub const RENDER_STATE_DATA_DIRTY: GhosttyRenderStateData = 3;
pub const RENDER_STATE_DATA_ROW_ITERATOR: GhosttyRenderStateData = 4;
pub const RENDER_STATE_DATA_CURSOR: GhosttyRenderStateData = 18;
pub const RENDER_STATE_DATA_COLORS: GhosttyRenderStateData = 19;

pub type GhosttyRenderStateOption = i32;
pub const RENDER_STATE_OPT_DIRTY: GhosttyRenderStateOption = 1;

pub type GhosttyRenderStateRowData = i32;
pub const RENDER_STATE_ROW_DATA_DIRTY: GhosttyRenderStateRowData = 1;
pub const RENDER_STATE_ROW_DATA_RAW: GhosttyRenderStateRowData = 2;
pub const RENDER_STATE_ROW_DATA_CELLS: GhosttyRenderStateRowData = 3;
pub const RENDER_STATE_ROW_DATA_SELECTION: GhosttyRenderStateRowData = 4;

pub type GhosttyRenderStateRowOption = i32;
pub const RENDER_STATE_ROW_OPT_DIRTY: GhosttyRenderStateRowOption = 1;

pub type GhosttyRenderStateRowCellsData = i32;
pub const RENDER_STATE_ROW_CELLS_DATA_RAW: GhosttyRenderStateRowCellsData = 1;
pub const RENDER_STATE_ROW_CELLS_DATA_STYLE: GhosttyRenderStateRowCellsData = 2;
pub const RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_UTF8: GhosttyRenderStateRowCellsData = 9;

pub type GhosttyCursorStyle = i32;
pub const CURSOR_STYLE_BAR: GhosttyCursorStyle = 0;
pub const CURSOR_STYLE_BLOCK: GhosttyCursorStyle = 1;
pub const CURSOR_STYLE_UNDERLINE: GhosttyCursorStyle = 2;
pub const CURSOR_STYLE_BLOCK_HOLLOW: GhosttyCursorStyle = 3;

/// A sized struct: set `size` to its own size before passing it.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GhosttyRenderStateRowSelection {
    pub size: usize,
    pub start_x: u16,
    pub end_x: u16,
}

/// A sized struct: set `size` to its own size before passing it.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GhosttyRenderStateCursor {
    pub size: usize,
    pub viewport_has_value: bool,
    pub viewport_x: u16,
    pub viewport_y: u16,
    pub wide_tail: bool,
    pub visible: bool,
    pub blinking: bool,
    pub password_input: bool,
    pub visual_style: GhosttyCursorStyle,
}

/// A sized struct: set `size` to its own size before passing it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GhosttyRenderStateColors {
    pub size: usize,
    pub background: GhosttyColorRgb,
    pub foreground: GhosttyColorRgb,
    pub cursor: GhosttyColorRgb,
    pub cursor_has_value: bool,
    pub palette: [GhosttyColorRgb; 256],
}

/// `GhosttyBuffer`: bytes written, or on `OUT_OF_SPACE` the bytes needed.
#[repr(C)]
pub struct GhosttyBuffer {
    pub ptr: *mut u8,
    pub cap: usize,
    pub len: usize,
}

unsafe extern "C" {
    pub fn ghostty_free(allocator: *const GhosttyAllocator, ptr: *mut u8, len: usize);

    pub fn ghostty_terminal_new(
        allocator: *const GhosttyAllocator,
        terminal: *mut GhosttyTerminal,
        cols: u16,
        rows: u16,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_free(terminal: GhosttyTerminal);
    pub fn ghostty_terminal_resize(
        terminal: GhosttyTerminal,
        cols: u16,
        rows: u16,
        cell_width_px: u32,
        cell_height_px: u32,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_set(
        terminal: GhosttyTerminal,
        option: GhosttyTerminalOption,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_vt_write(terminal: GhosttyTerminal, data: *const u8, len: usize);
    pub fn ghostty_terminal_get(
        terminal: GhosttyTerminal,
        data: GhosttyTerminalData,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_grid_ref(
        terminal: GhosttyTerminal,
        point: GhosttyPoint,
        out_ref: *mut GhosttyGridRef,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_point_from_grid_ref(
        terminal: GhosttyTerminal,
        grid_ref: *const GhosttyGridRef,
        tag: GhosttyPointTag,
        out: *mut GhosttyPointCoordinate,
    ) -> GhosttyResult;
    pub fn ghostty_terminal_scroll_viewport(
        terminal: GhosttyTerminal,
        behavior: GhosttyTerminalScrollViewport,
    ) -> GhosttyResult;

    pub fn ghostty_grid_ref_cell(
        grid_ref: *const GhosttyGridRef,
        out_cell: *mut GhosttyCell,
    ) -> GhosttyResult;
    pub fn ghostty_grid_ref_row(
        grid_ref: *const GhosttyGridRef,
        out_row: *mut GhosttyRow,
    ) -> GhosttyResult;
    pub fn ghostty_grid_ref_graphemes(
        grid_ref: *const GhosttyGridRef,
        buf: *mut u32,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_grid_ref_style(
        grid_ref: *const GhosttyGridRef,
        out_style: *mut GhosttyStyle,
    ) -> GhosttyResult;

    pub fn ghostty_cell_get(
        cell: GhosttyCell,
        data: GhosttyCellData,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_row_get(
        row: GhosttyRow,
        data: GhosttyRowData,
        out: *mut c_void,
    ) -> GhosttyResult;

    pub fn ghostty_formatter_terminal_new(
        allocator: *const GhosttyAllocator,
        formatter: *mut GhosttyFormatter,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_formatter_set(
        formatter: GhosttyFormatter,
        option: GhosttyFormatterOption,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_formatter_format_alloc(
        formatter: GhosttyFormatter,
        allocator: *const GhosttyAllocator,
        out_ptr: *mut *mut u8,
        out_len: *mut usize,
    ) -> GhosttyResult;
    pub fn ghostty_formatter_free(formatter: GhosttyFormatter);

    pub fn ghostty_search_new(
        allocator: *const GhosttyAllocator,
        out_search: *mut GhosttySearch,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_search_set(
        search: GhosttySearch,
        option: GhosttySearchOption,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_search_run(search: GhosttySearch) -> GhosttyResult;
    pub fn ghostty_search_get(
        search: GhosttySearch,
        data: GhosttySearchData,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_search_free(search: GhosttySearch);

    pub fn ghostty_key_encoder_new(
        allocator: *const GhosttyAllocator,
        encoder: *mut GhosttyKeyEncoder,
    ) -> GhosttyResult;
    pub fn ghostty_key_encoder_free(encoder: GhosttyKeyEncoder);
    pub fn ghostty_key_encoder_set(
        encoder: GhosttyKeyEncoder,
        option: GhosttyKeyEncoderOption,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_key_encoder_set_from_terminal(
        encoder: GhosttyKeyEncoder,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_key_encoder_encode(
        encoder: GhosttyKeyEncoder,
        event: GhosttyKeyEvent,
        buf: *mut u8,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;

    pub fn ghostty_key_event_new(
        allocator: *const GhosttyAllocator,
        event: *mut GhosttyKeyEvent,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_free(event: GhosttyKeyEvent);
    pub fn ghostty_key_event_set_action(
        event: GhosttyKeyEvent,
        action: GhosttyKeyAction,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_set_key(event: GhosttyKeyEvent, key: GhosttyKey) -> GhosttyResult;
    pub fn ghostty_key_event_set_mods(event: GhosttyKeyEvent, mods: GhosttyMods) -> GhosttyResult;
    pub fn ghostty_key_event_set_consumed_mods(
        event: GhosttyKeyEvent,
        consumed_mods: GhosttyMods,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_set_composing(
        event: GhosttyKeyEvent,
        composing: bool,
    ) -> GhosttyResult;
    pub fn ghostty_key_event_set_utf8(event: GhosttyKeyEvent, utf8: GhosttyString)
    -> GhosttyResult;
    pub fn ghostty_key_event_set_unshifted_codepoint(
        event: GhosttyKeyEvent,
        codepoint: u32,
    ) -> GhosttyResult;

    pub fn ghostty_mouse_encoder_new(
        allocator: *const GhosttyAllocator,
        encoder: *mut GhosttyMouseEncoder,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_encoder_free(encoder: GhosttyMouseEncoder);
    pub fn ghostty_mouse_encoder_set(
        encoder: GhosttyMouseEncoder,
        option: GhosttyMouseEncoderOption,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_encoder_set_from_terminal(
        encoder: GhosttyMouseEncoder,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_encoder_encode(
        encoder: GhosttyMouseEncoder,
        event: GhosttyMouseEvent,
        buf: *mut u8,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;

    pub fn ghostty_mouse_event_new(
        allocator: *const GhosttyAllocator,
        event: *mut GhosttyMouseEvent,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_event_free(event: GhosttyMouseEvent);
    pub fn ghostty_mouse_event_set_action(
        event: GhosttyMouseEvent,
        action: GhosttyMouseAction,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_event_set_button(
        event: GhosttyMouseEvent,
        button: GhosttyMouseButton,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_event_clear_button(event: GhosttyMouseEvent) -> GhosttyResult;
    pub fn ghostty_mouse_event_set_mods(
        event: GhosttyMouseEvent,
        mods: GhosttyMods,
    ) -> GhosttyResult;
    pub fn ghostty_mouse_event_set_position(
        event: GhosttyMouseEvent,
        position: GhosttyMousePosition,
    ) -> GhosttyResult;

    pub fn ghostty_render_state_new(
        allocator: *const GhosttyAllocator,
        state: *mut GhosttyRenderState,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_free(state: GhosttyRenderState);
    pub fn ghostty_render_state_update(
        state: GhosttyRenderState,
        terminal: GhosttyTerminal,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_get(
        state: GhosttyRenderState,
        data: GhosttyRenderStateData,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_set(
        state: GhosttyRenderState,
        option: GhosttyRenderStateOption,
        value: *const c_void,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_clean(state: GhosttyRenderState) -> GhosttyResult;

    pub fn ghostty_render_state_row_iterator_new(
        allocator: *const GhosttyAllocator,
        iterator: *mut GhosttyRenderStateRowIterator,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_iterator_free(iterator: GhosttyRenderStateRowIterator);
    pub fn ghostty_render_state_row_iterator_next(iterator: GhosttyRenderStateRowIterator) -> bool;
    pub fn ghostty_render_state_row_get(
        iterator: GhosttyRenderStateRowIterator,
        data: GhosttyRenderStateRowData,
        out: *mut c_void,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_set(
        iterator: GhosttyRenderStateRowIterator,
        option: GhosttyRenderStateRowOption,
        value: *const c_void,
    ) -> GhosttyResult;

    pub fn ghostty_render_state_row_cells_new(
        allocator: *const GhosttyAllocator,
        cells: *mut GhosttyRenderStateRowCells,
    ) -> GhosttyResult;
    pub fn ghostty_render_state_row_cells_free(cells: GhosttyRenderStateRowCells);
    pub fn ghostty_render_state_row_cells_next(cells: GhosttyRenderStateRowCells) -> bool;
    pub fn ghostty_render_state_row_cells_get(
        cells: GhosttyRenderStateRowCells,
        data: GhosttyRenderStateRowCellsData,
        out: *mut c_void,
    ) -> GhosttyResult;

    pub fn ghostty_paste_is_safe(data: *const u8, len: usize) -> bool;
    pub fn ghostty_paste_encode(
        data: *mut u8,
        data_len: usize,
        bracketed: bool,
        buf: *mut u8,
        buf_len: usize,
        out_written: *mut usize,
    ) -> GhosttyResult;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    // The headers' layouts, so a struct that drifts from C fails here
    // rather than corrupting memory.
    #[test]
    fn structs_have_the_sizes_the_headers_give() {
        assert_eq!(size_of::<GhosttyString>(), 16);
        assert_eq!(size_of::<GhosttyPointCoordinate>(), 8);
        assert_eq!(size_of::<GhosttyPoint>(), 24);
        assert_eq!(size_of::<GhosttyGridRef>(), 16);
        assert_eq!(size_of::<GhosttySelection>(), 40);
        assert_eq!(size_of::<GhosttyStyleColor>(), 4);
        assert_eq!(size_of::<GhosttyStyle>(), 16);
        assert_eq!(size_of::<GhosttyTerminalScrollViewport>(), 24);
        assert_eq!(size_of::<GhosttyMouseEncoderSize>(), 40);
        assert_eq!(size_of::<GhosttyTerminalModeConfig>(), 4);
        assert_eq!(size_of::<GhosttyClipboardContent>(), 32);
        assert_eq!(size_of::<GhosttyClipboardWrite>(), 64);
        assert_eq!(size_of::<GhosttyClipboardRead>(), 72);
        assert_eq!(size_of::<GhosttyRenderStateRowSelection>(), 16);
        assert_eq!(size_of::<GhosttyRenderStateCursor>(), 24);
        assert_eq!(size_of::<GhosttyRenderStateColors>(), 792);
        assert_eq!(size_of::<GhosttyBuffer>(), 24);
    }
}
