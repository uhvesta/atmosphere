use gpui_kit::prelude::FluentBuilder as _;
use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::base::input as edit;
use gpui_kit::{
    App, AppContext as _, BorderStyle, Bounds, ClipboardItem, ContentMask, Context, DispatchPhase,
    ElementInputHandler, EntityInputHandler, FocusHandle, Focusable, Font, FontStyle, FontWeight,
    Global, Hsla, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, KeyUpEvent,
    MouseButton as GpuiButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, NavigationDirection,
    NoAction, ParentElement, Pixels, Point, Render, Rgba, ScrollEvent, ShapedLine, SharedString,
    Styled, Subscription, Task, TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window,
    WindowTextSystem, canvas, div, fill, font, outline, point, px, size,
};
use terminal_grid::{
    CursorShape, Dirty, Grid, KeyAction, KeyEvent, MouseAction, MouseButton, MouseEvent, Rgb,
    Scrollbar, Size, Underline, ViewportPoint,
};

use crate::boxes::{self, Area};
use crate::cursor::{CursorPlan, plan_cursor};
use crate::frame::{Decoration, Drawn, Fill, LineKind, Piece, plan_row};
use crate::host::{CellSize, ScrollTo, TerminalHost};
use crate::keys::{KeyIntent, Platform, intent, mods, program_key};
use crate::paste::Paste;
use crate::pending_input::{Input as PendingInputEvent, PendingInput};
use crate::selection::{Granularity, extend};
use crate::theme::Theme;

/// How long a blinking cursor stays on, and then off, as in `Ghostty`.
const BLINK: Duration = Duration::from_millis(600);

const TERMINAL_CONTEXT: &str = "HumataTerminal";
struct TerminalKeyBindings;
impl Global for TerminalKeyBindings {}

/// How wide the scrollbar is drawn, and how far from the right edge a
/// press grabs it, in pixels.
const SCROLLBAR_WIDTH: f32 = 6.0;
const SCROLLBAR_HIT: f32 = 12.0;
/// The shortest thumb, so a long history keeps one that can be grabbed.
const SCROLLBAR_MIN_THUMB: f32 = 16.0;

/// Find highlights, drawn under the text: every match, and the current one
/// brighter. Translucent, so the program's colors still show through.
const MARK_COLOR: Rgb = Rgb {
    r: 0xe5,
    g: 0xc0,
    b: 0x7b,
};
const MARK_ALPHA: f32 = 0.35;
const CURRENT_MARK_COLOR: Rgb = Rgb {
    r: 0xff,
    g: 0x9e,
    b: 0x3b,
};
const CURRENT_MARK_ALPHA: f32 = 0.75;
/// Higher-contrast find highlights for light terminal backgrounds (`#ffffff`).
const LIGHT_MARK_COLOR: Rgb = Rgb {
    r: 0xf5,
    g: 0xc5,
    b: 0x18,
};
const LIGHT_MARK_ALPHA: f32 = 0.45;
const LIGHT_CURRENT_MARK_COLOR: Rgb = Rgb {
    r: 0xff,
    g: 0x7a,
    b: 0x00,
};
const LIGHT_CURRENT_MARK_ALPHA: f32 = 0.65;

/// A cell of the whole history: column `col` of row `row`, 0 being the
/// oldest row, in the coordinates of [`Scrollbar::offset`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryPoint {
    pub row: u64,
    pub col: u16,
}

/// Cells to highlight, such as a find match, from `start` to `end`, both
/// included, in reading order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mark {
    pub start: HistoryPoint,
    pub end: HistoryPoint,
    /// The match the find bar is on, drawn brighter.
    pub current: bool,
}

/// What the last frame cost.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Frames drawn so far.
    pub frames: u64,
    /// Rows planned and shaped again in the last frame; the others were
    /// drawn from the cache.
    pub rows_rebuilt: u32,
    /// Text pieces shaped in the last frame.
    pub pieces_shaped: u32,
    /// Rows planned and shaped again over all frames.
    pub rows_rebuilt_total: u64,
    /// Text pieces that failed to paint, over all frames. A piece that
    /// fails is left blank, usually because its font is missing.
    pub paint_failures: u64,
}

/// Input routing counters without key values or committed text.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputStats {
    pub focus_gains: u64,
    pub focus_losses: u64,
    pub mouse_downs: u64,
    pub key_downs: u64,
    pub unfocused_key_downs: u64,
    pub key_ups: u64,
    pub unfocused_key_ups: u64,
    pub text_commits: u64,
}

/// A terminal, drawn from any [`Grid`]: one quad per run of background,
/// one shaped line per piece of text, and the cursor on top.
///
/// Each frame it syncs the grid, then plans and shapes again only the rows
/// the grid marks dirty, as `Ghostty`'s renderer rebuilds only dirty rows
/// (`renderer/generic.zig`). Input goes to a [`TerminalHost`].
pub struct TerminalView<G: Grid + 'static> {
    grid: G,
    host: Box<dyn TerminalHost<G>>,
    theme: Theme,
    theme_changed: bool,
    focus_handle: FocusHandle,
    frame: Rc<RefCell<Frame>>,
    stats: FrameStats,
    input_stats: InputStats,
    /// The cell a left-button selection started on, in history coordinates so
    /// scrolling and Shift+Click extend keep the original anchor.
    anchor: Option<HistoryPoint>,
    /// True from a local-selection `mouse_down` through `mouse_up`, so
    /// releasing `Shift` mid-drag keeps selecting and never leaks `Motion`
    /// or `Release` events to a mouse-reporting TUI.
    local_drag: bool,
    /// Whether the drag has left its first cell, so a click alone selects
    /// nothing.
    selecting: bool,
    /// What the press that started the drag selects: cells, words or
    /// lines, from its click count.
    granularity: Granularity,
    /// Whether the drag selects a block of cells (Option or Alt held).
    block: bool,
    /// While the scrollbar's thumb is dragged, where in it the pointer
    /// grabbed it, in pixels from its top.
    scrollbar_grab: Option<f32>,
    link_pressed: bool,
    hovered_link: Option<(ViewportPoint, String)>,
    /// Find highlights, sorted.
    marks: Vec<Mark>,
    marks_changed: bool,
    /// Whether a cursor the program asked to blink does; off saves the
    /// timer's wake-ups while the terminal sits idle.
    cursor_blink: bool,
    /// Text the input method is composing, drawn at the cursor until it
    /// commits.
    preedit: Option<String>,
    pending_input: PendingInput,
    canceled_key_releases: Vec<terminal_grid::Key>,
    /// Scrolled distance short of a whole row, in pixels.
    scroll_remainder: f32,
    /// Whether a blinking cursor is in its visible half. A timer flips it,
    /// so the view reads no clock (a `std` clock panics in a browser);
    /// typing turns it back on and restarts the timer.
    blink_on: bool,
    blink_task: Option<Task<()>>,
    /// Which system's copy and paste shortcuts apply, read from GPUI at
    /// start so a browser on a Mac gets Cmd+C and Cmd+V.
    platform: Platform,
    /// The size last asked of the host, so a host that can't resize isn't
    /// asked every frame.
    requested: Option<Size>,
    _focus: [Subscription; 2],
}

/// What the canvas paints, shared between the view (which rebuilds it in
/// `render`) and the canvas (which paints it and reports its bounds).
#[derive(Default)]
struct Frame {
    metrics: Option<Metrics>,
    size: Size,
    /// The canvas's size, for the scrollbar.
    extent: gpui_kit::Size<Pixels>,
    scrollbar: Option<Scrollbar>,
    scrollbar_color: Rgb,
    /// Whether the thumb is being dragged, so it stays drawn when the
    /// drag reaches the newest output.
    scrollbar_held: bool,
    link_underline: Option<(u16, u16, u16)>,
    preedit: Option<ShapedPreedit>,
    background: Rgb,
    light_background: bool,
    rows: Vec<ShapedRow>,
    cursor: Option<ShapedCursor>,
    origin: Point<Pixels>,
    paint_failures: u64,
}

#[derive(Default)]
struct ShapedRow {
    fills: Vec<Fill>,
    lines: Vec<(u16, ShapedLine)>,
    decorations: Vec<Decoration>,
    drawings: Vec<Drawn>,
    /// Find highlights: first column, columns, and whether it is the
    /// current match.
    marks: Vec<(u16, u16, bool)>,
}

struct ShapedPreedit {
    col: u16,
    row: u16,
    cols: u16,
    text: ShapedLine,
    color: Rgb,
}

struct ShapedCursor {
    plan: CursorPlan,
    text: Option<ShapedLine>,
}

/// The fonts and cell size for a theme.
#[derive(Clone)]
struct Metrics {
    family: SharedString,
    font_size: f32,
    line_height: f32,
    fonts: [Font; 4],
    cell: CellSize,
    /// From the top of a row to the baseline.
    baseline: f32,
    /// The thickness of underlines, strikethroughs and bars.
    thickness: f32,
}

impl Metrics {
    fn measure(theme: &Theme, text_system: &WindowTextSystem) -> Metrics {
        let regular = font(theme.font_family.clone());
        let fonts = [
            regular.clone(),
            Font {
                weight: FontWeight::BOLD,
                ..regular.clone()
            },
            Font {
                style: FontStyle::Italic,
                ..regular.clone()
            },
            Font {
                weight: FontWeight::BOLD,
                style: FontStyle::Italic,
                ..regular.clone()
            },
        ];
        let font_size = px(theme.font_size);
        let id = text_system.resolve_font(&regular);
        // Like `Ghostty`, a cell is as wide as the font's advance, rounded
        // up to a whole pixel so cell edges stay sharp.
        let width = text_system
            .advance(id, font_size, 'm')
            .map(|advance| f32::from(advance.width))
            .unwrap_or(theme.font_size * 0.6)
            .ceil()
            .max(1.0);
        let height = theme.line_height.max(1.0).round();
        let ascent = f32::from(text_system.ascent(id, font_size)).abs();
        let descent = f32::from(text_system.descent(id, font_size)).abs();
        let baseline = ((height - ascent - descent) / 2.0 + ascent).round();
        Metrics {
            family: theme.font_family.clone(),
            font_size: theme.font_size,
            line_height: theme.line_height,
            fonts,
            cell: CellSize { width, height },
            baseline,
            thickness: (theme.font_size / 13.0).round().max(1.0),
        }
    }

    fn fits(&self, theme: &Theme) -> bool {
        self.family == theme.font_family
            && self.font_size == theme.font_size
            && self.line_height == theme.line_height
    }

    fn font(&self, bold: bool, italic: bool) -> Font {
        let index = usize::from(bold) | (usize::from(italic) << 1);
        self.fonts[index].clone()
    }
}

impl<G: Grid + 'static> TerminalView<G> {
    pub fn new(
        grid: G,
        host: impl TerminalHost<G> + 'static,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        if !cx.has_global::<TerminalKeyBindings>() {
            cx.bind_keys([
                KeyBinding::new("tab", NoAction, Some(TERMINAL_CONTEXT)),
                KeyBinding::new("shift-tab", NoAction, Some(TERMINAL_CONTEXT)),
                #[cfg(not(target_os = "macos"))]
                KeyBinding::new("ctrl-c", NoAction, Some(TERMINAL_CONTEXT)),
            ]);
            cx.set_global(TerminalKeyBindings);
        }
        let focus_handle = cx.focus_handle();
        let focus = [
            cx.on_focus(&focus_handle, window, |this: &mut Self, _, cx| {
                this.input_stats.focus_gains = this.input_stats.focus_gains.saturating_add(1);
                this.host.focus(&mut this.grid, true);
                this.restart_blink();
                cx.notify();
            }),
            cx.on_blur(&focus_handle, window, |this: &mut Self, _, cx| {
                this.input_stats.focus_losses = this.input_stats.focus_losses.saturating_add(1);
                this.host.focus(&mut this.grid, false);
                this.preedit = None;
                this.cancel_buffered_input();
                cx.notify();
            }),
        ];
        TerminalView {
            grid,
            host: Box::new(host),
            theme,
            theme_changed: true,
            focus_handle,
            frame: Rc::new(RefCell::new(Frame::default())),
            stats: FrameStats::default(),
            input_stats: InputStats::default(),
            anchor: None,
            local_drag: false,
            selecting: false,
            granularity: Granularity::Cell,
            block: false,
            scrollbar_grab: None,
            link_pressed: false,
            hovered_link: None,
            marks: Vec::new(),
            marks_changed: false,
            cursor_blink: true,
            preedit: None,
            pending_input: PendingInput::default(),
            canceled_key_releases: Vec::new(),
            scroll_remainder: 0.0,
            blink_on: true,
            blink_task: None,
            platform: Platform::of(cx.platform_info()),
            requested: None,
            _focus: focus,
        }
    }

    /// Bounded input metadata for host diagnostics; never contains typed text.
    pub fn input_stats(&self) -> InputStats {
        self.input_stats
    }

    pub fn grid(&self) -> &G {
        &self.grid
    }

    /// The grid, to feed it output. Call `cx.notify()` on the view after,
    /// so it redraws.
    pub fn grid_mut(&mut self) -> &mut G {
        &mut self.grid
    }

    /// Replace validated state without committing stale composition or anchors.
    pub fn replace_grid(&mut self, grid: G, cx: &mut Context<Self>) {
        self.cancel_buffered_input();
        self.grid = grid;
        self.anchor = None;
        self.local_drag = false;
        self.selecting = false;
        self.scrollbar_grab = None;
        self.hovered_link = None;
        self.preedit = None;
        self.marks.clear();
        self.marks_changed = true;
        self.theme_changed = true;
        self.requested = None;
        cx.notify();
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Changes the font or colors; every row is drawn again.
    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        if theme != self.theme {
            self.theme = theme;
            self.theme_changed = true;
            self.requested = None;
            cx.notify();
        }
    }

    /// Re-submit this viewport's desired geometry after its transport gains control.
    pub fn request_fit(&mut self, cx: &mut Context<Self>) {
        self.requested = None;
        cx.notify();
    }

    /// Focuses terminal input and installs its IME handler in this input effect,
    /// including programmatic handoffs from find bars and terminal tabs.
    pub fn focus_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        window.defer(cx, |window, cx| window.draw(cx).clear(cx));
    }

    /// The size of a cell, once a frame has been drawn.
    pub fn cell_size(&self) -> Option<CellSize> {
        self.frame
            .borrow()
            .metrics
            .as_ref()
            .map(|metrics| metrics.cell)
    }

    /// Highlights `marks`, such as find matches, wherever they scroll into
    /// view. Replaces the last set; an empty set clears them.
    pub fn set_marks(&mut self, mut marks: Vec<Mark>, cx: &mut Context<Self>) {
        marks.sort_by_key(|mark| (mark.start, mark.end));
        if marks != self.marks {
            self.marks = marks;
            self.marks_changed = true;
            cx.notify();
        }
    }

    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    /// Whether a cursor the program asked to blink blinks. Off, the view
    /// sets no timer, so an idle terminal draws nothing.
    pub fn set_cursor_blink(&mut self, blink: bool, cx: &mut Context<Self>) {
        if blink != self.cursor_blink {
            self.cursor_blink = blink;
            self.restart_blink();
            cx.notify();
        }
    }

    /// Whether a blink timer is pending, so an idle view will draw again.
    pub fn is_blink_scheduled(&self) -> bool {
        self.blink_task.is_some()
    }

    /// Text the input method is composing, before it commits.
    pub fn preedit(&self) -> Option<&str> {
        self.preedit.as_deref()
    }

    /// The rendered viewport in window coordinates, for host overlays and hit testing.
    pub fn viewport_bounds(&self) -> Bounds<Pixels> {
        let frame = self.frame.borrow();
        Bounds::new(frame.origin, frame.extent)
    }

    /// Releases cached shaped rows when the view is hidden or parked so
    /// off-screen terminals do not retain shaped line buffers. The next
    /// render rebuilds them from the grid.
    pub fn clear_frame(&mut self) {
        let mut frame = self.frame.borrow_mut();
        frame.rows.clear();
        frame.rows.shrink_to_fit();
        frame.cursor = None;
        frame.preedit = None;
        frame.size = Size { cols: 0, rows: 0 };
    }

    /// What the last frame cost.
    pub fn stats(&self) -> FrameStats {
        FrameStats {
            paint_failures: self.frame.borrow().paint_failures,
            ..self.stats
        }
    }

    /// Syncs the grid and rebuilds the rows that changed.
    fn rebuild(&mut self, focused: bool, window: &mut Window) {
        self.grid.sync();
        let frame = self.frame.clone();
        let mut frame = frame.borrow_mut();
        let text_system = window.text_system().clone();

        let mut full = self.theme_changed || self.marks_changed || self.grid.dirty() == Dirty::Full;
        if frame
            .metrics
            .as_ref()
            .is_none_or(|metrics| !metrics.fits(&self.theme))
        {
            frame.metrics = Some(Metrics::measure(&self.theme, &text_system));
            full = true;
        }
        let Some(metrics) = frame.metrics.clone() else {
            return;
        };
        let size = self.grid.size();
        if frame.size != size {
            frame.size = size;
            frame
                .rows
                .resize_with(usize::from(size.rows), ShapedRow::default);
            full = true;
        }
        let colors = self.grid.colors();
        let light_bg = colors.is_light();
        if frame.background != colors.background || frame.light_background != light_bg {
            // Rows skip fills in the default background, so they change too.
            frame.background = colors.background;
            frame.light_background = light_bg;
            full = true;
        }

        let scrollbar = self.grid.scrollbar();
        if frame.scrollbar.map(|bar| bar.offset) != scrollbar.map(|bar| bar.offset) {
            // Marks sit on history rows, so scrolling moves them.
            full = true;
        }
        frame.link_underline = self.hovered_link.as_ref().and_then(|(at, uri)| {
            if self.grid.hyperlink(*at).as_deref() != Some(uri.as_str()) {
                return None;
            }
            let mut start = at.x;
            let mut end = at.x;
            while start > 0
                && self
                    .grid
                    .hyperlink(ViewportPoint {
                        x: start - 1,
                        y: at.y,
                    })
                    .as_deref()
                    == Some(uri.as_str())
            {
                start -= 1;
            }
            while end + 1 < size.cols
                && self
                    .grid
                    .hyperlink(ViewportPoint {
                        x: end + 1,
                        y: at.y,
                    })
                    .as_deref()
                    == Some(uri.as_str())
            {
                end += 1;
            }
            Some((at.y, start, end))
        });
        frame.scrollbar = scrollbar;
        frame.scrollbar_color = colors.foreground;
        let top = scrollbar.map_or(0, |bar| bar.offset);

        let style = self.theme.style(light_bg);
        let mut rows_rebuilt = 0;
        let mut pieces_shaped = 0;
        if full || self.grid.dirty() != Dirty::Clean {
            for y in 0..size.rows {
                if !full && !self.grid.is_row_dirty(y) {
                    continue;
                }
                let Some(slot) = frame.rows.get_mut(usize::from(y)) else {
                    continue;
                };
                let Some(row) = self.grid.row(y) else {
                    *slot = ShapedRow::default();
                    continue;
                };
                let plan = plan_row(row, self.grid.selection(y), colors, &style);
                pieces_shaped += plan.pieces.len() as u32;
                *slot = ShapedRow {
                    fills: plan.fills,
                    lines: plan
                        .pieces
                        .into_iter()
                        .map(|piece| (piece.col, shape(piece, &metrics, &text_system)))
                        .collect(),
                    decorations: plan.decorations,
                    drawings: plan.drawings,
                    marks: row_marks(&self.marks, top + u64::from(y), size.cols),
                };
                rows_rebuilt += 1;
            }
        }

        frame.cursor = plan_cursor(
            &self.grid,
            focused,
            self.blink_on,
            self.theme.cursor_text,
            &style,
        )
        .map(|plan| {
            let text = plan.text.as_ref().map(|cursor_text| {
                shape(
                    Piece {
                        col: plan.col,
                        text: cursor_text.text.clone(),
                        style: crate::frame::TextStyle {
                            color: cursor_text.color,
                            alpha: 1.0,
                            bold: cursor_text.bold,
                            italic: cursor_text.italic,
                        },
                    },
                    &metrics,
                    &text_system,
                )
            });
            ShapedCursor { plan, text }
        });

        let cursor = self.grid.cursor();
        frame.preedit = self
            .preedit
            .as_ref()
            .filter(|text| !text.is_empty())
            .map(|text| {
                let piece = Piece {
                    col: cursor.x,
                    text: text.clone(),
                    style: crate::frame::TextStyle {
                        color: colors.foreground,
                        alpha: 1.0,
                        bold: false,
                        italic: false,
                    },
                };
                let text = shape(piece, &metrics, &text_system);
                let cols = (f32::from(text.width) / metrics.cell.width).ceil().max(1.0) as u16;
                ShapedPreedit {
                    col: cursor.x,
                    row: cursor.y,
                    cols,
                    text,
                    color: colors.foreground,
                }
            });

        self.grid.clear_dirty();
        self.theme_changed = false;
        self.marks_changed = false;
        self.stats.frames += 1;
        self.stats.rows_rebuilt = rows_rebuilt;
        self.stats.rows_rebuilt_total += u64::from(rows_rebuilt);
        self.stats.pieces_shaped = pieces_shaped;
    }

    /// Shows the cursor and starts its blink over. Dropping the pending
    /// timer cancels it; the next frame schedules a fresh one.
    fn restart_blink(&mut self) {
        self.blink_on = true;
        self.blink_task = None;
    }

    /// Flips a blinking cursor after `BLINK` and redraws.
    fn schedule_blink(&mut self, focused: bool, cx: &mut Context<Self>) {
        let style = self.grid.cursor_style();
        if !self.cursor_blink || !focused || !style.blinking || !self.grid.cursor().visible {
            self.restart_blink();
            return;
        }
        if self.blink_task.is_some() {
            return;
        }
        self.blink_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BLINK).await;
            let _ = this.update(cx, |this, cx| {
                this.blink_on = !this.blink_on;
                this.blink_task = None;
                cx.notify();
            });
        }));
    }

    /// Asks the host to fit the grid to `bounds`.
    fn fit(&mut self, bounds: Bounds<Pixels>, cx: &mut Context<Self>) {
        let Some(cell) = self.cell_size() else {
            return;
        };
        let cols = (f32::from(bounds.size.width) / cell.width).floor().max(1.0);
        let rows = (f32::from(bounds.size.height) / cell.height)
            .floor()
            .max(1.0);
        let wanted = Size {
            cols: cols.min(f32::from(u16::MAX)) as u16,
            rows: rows.min(f32::from(u16::MAX)) as u16,
        };
        if self.requested == Some(wanted) {
            return;
        }
        self.requested = Some(wanted);
        self.host.resize(&mut self.grid, wanted, cell);
        cx.notify();
    }

    /// The cell under `position`, a window position, clamped to the grid.
    fn cell_at(&self, position: Point<Pixels>) -> ViewportPoint {
        let (x, y) = self.local(position);
        let cell = self.cell_size().unwrap_or(CellSize {
            width: 1.0,
            height: 1.0,
        });
        let size = self.grid.size();
        let col = (x / cell.width)
            .floor()
            .clamp(0.0, f32::from(size.cols.saturating_sub(1)));
        let row = (y / cell.height)
            .floor()
            .clamp(0.0, f32::from(size.rows.saturating_sub(1)));
        ViewportPoint {
            x: col as u16,
            y: row as u16,
        }
    }

    /// `position` in pixels from the grid's top-left corner.
    fn local(&self, position: Point<Pixels>) -> (f32, f32) {
        let origin = self.frame.borrow().origin;
        (
            f32::from(position.x - origin.x),
            f32::from(position.y - origin.y),
        )
    }

    fn mouse_event(
        &self,
        action: MouseAction,
        button: Option<GpuiButton>,
        position: Point<Pixels>,
        modifiers: gpui_kit::Modifiers,
    ) -> MouseEvent {
        let (x, y) = self.local(position);
        MouseEvent {
            action,
            button: button.map(terminal_button),
            mods: mods(modifiers),
            x,
            y,
        }
    }

    fn cancel_buffered_input(&mut self) {
        for key in self.pending_input.queued_presses() {
            if !self.canceled_key_releases.contains(&key) {
                self.canceled_key_releases.push(key);
            }
        }
        self.pending_input.clear();
    }
    /// Ends a composition the input method has in progress, as when the
    /// program can no longer take its text: nothing half-typed stays drawn.
    pub fn end_composition(&mut self, cx: &mut Context<Self>) {
        if self.preedit.take().is_some() {
            cx.notify();
        }
    }

    /// Cancel a platform read and its following typeahead after authority changes.
    pub fn cancel_pending_clipboard(&mut self, cx: &mut Context<Self>) {
        if !self.pending_input.is_empty() {
            self.cancel_buffered_input();
            cx.notify();
        }
    }
    pub fn has_pending_clipboard(&self) -> bool {
        !self.pending_input.is_empty()
    }
    fn input_buffer_failed(&mut self, error: crate::InputBufferError, cx: &mut Context<Self>) {
        self.cancel_buffered_input();
        self.host.input_buffer_failed(error);
        cx.notify();
    }
    fn begin_clipboard_read(&mut self, cx: &mut Context<Self>) -> Option<u64> {
        match self.pending_input.begin_clipboard() {
            Ok(id) => {
                cx.notify();
                Some(id)
            }
            Err(error) => {
                self.input_buffer_failed(error, cx);
                None
            }
        }
    }
    fn send_input(&mut self, input: PendingInputEvent, cx: &mut Context<Self>) {
        if !self.pending_input.is_empty() {
            let key = match &input {
                PendingInputEvent::Key {
                    intent: KeyIntent::Send { key, .. },
                    action: KeyAction::Press | KeyAction::Repeat,
                } => Some(*key),
                _ => None,
            };
            if let Err(error) = self.pending_input.push(input) {
                self.input_buffer_failed(error, cx);
                if let Some(key) = key {
                    self.canceled_key_releases.push(key);
                }
            }
            return;
        }
        self.deliver_input(input);
    }
    fn deliver_input(&mut self, input: PendingInputEvent) {
        match input {
            PendingInputEvent::Key {
                intent:
                    KeyIntent::Send {
                        key,
                        mods,
                        consumed_mods,
                        text,
                        unshifted,
                    },
                action,
            } => self.host.key(
                &mut self.grid,
                KeyEvent {
                    key,
                    action,
                    mods,
                    consumed_mods,
                    text: &text,
                    unshifted,
                    composing: false,
                },
            ),
            PendingInputEvent::Text(text) => self.host.text(&mut self.grid, &text),
            PendingInputEvent::Clipboard {
                result: Some(Ok(Some(paste))),
                ..
            } => self.host.paste(&mut self.grid, paste),
            _ => {}
        }
    }
    fn clipboard_read_completed(
        &mut self,
        id: u64,
        result: Result<Option<ClipboardItem>, gpui_kit::ClipboardReadError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            self.cancel_buffered_input();
            return;
        }
        let result = result.map(|item| item.and_then(|item| Paste::from_clipboard(&item)));
        match self.pending_input.complete(id, result) {
            Ok(false) => return,
            Err(error) => {
                self.input_buffer_failed(error, cx);
                return;
            }
            Ok(true) => {}
        }
        while let Some(input) = self.pending_input.pop_ready() {
            if let PendingInputEvent::Clipboard {
                result: Some(Err(error)),
                ..
            } = input
            {
                self.cancel_buffered_input();
                self.host.clipboard_read_failed(error);
                break;
            }
            let paste = matches!(&input, PendingInputEvent::Clipboard { .. });
            self.deliver_input(input);
            if paste && (self.host.paste_is_pending() || !self.host.paste_input_accepted()) {
                self.cancel_buffered_input();
                break;
            }
        }
        self.restart_blink();
        cx.notify();
    }

    /// Puts the selection on the clipboard: the copy shortcut and
    /// Edit ▸ Copy.
    fn copy_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.host.copy(&mut self.grid) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Pastes the clipboard: the paste shortcut and Edit ▸ Paste.
    fn paste_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = cx.read_from_clipboard() {
            if let Some(paste) = Paste::from_clipboard(&item) {
                if self.pending_input.is_empty() {
                    self.host.paste(&mut self.grid, paste);
                } else if let Some(id) = self.begin_clipboard_read(cx) {
                    self.clipboard_read_completed(id, Ok(Some(item)), window, cx);
                }
            }
        } else if cx.platform_info().target == gpui_kit::PlatformTarget::Web {
            let Some(id) = self.begin_clipboard_read(cx) else {
                return;
            };
            // Web clipboard permission must be requested during this gesture.
            // Completion may arrive after focus moved to another terminal.
            let read = cx.read_from_clipboard_async();
            let target = window.window_handle();
            cx.spawn(async move |view, cx| {
                let result = read.await;
                let _ = cx.update_window(target, |_, window, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.clipboard_read_completed(id, result, window, cx);
                    });
                });
            })
            .detach();
        }
    }

    /// What `keystroke` does here. The history keys are a full-screen
    /// program's, since its screen keeps no history.
    fn key_intent(&self, keystroke: &gpui_kit::Keystroke) -> KeyIntent {
        match intent(keystroke, self.platform) {
            KeyIntent::Scroll(_) if self.grid.is_alternate_screen() => {
                program_key(keystroke, self.platform)
            }
            intent => intent,
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            self.input_stats.unfocused_key_downs =
                self.input_stats.unfocused_key_downs.saturating_add(1);
            return;
        }
        self.input_stats.key_downs = self.input_stats.key_downs.saturating_add(1);
        if self.preedit.is_some() {
            // The input method is composing: the key is its to take.
            return;
        }
        match self.key_intent(&event.keystroke) {
            KeyIntent::Ignore => return,
            KeyIntent::Scroll(to) => self.host.scroll_to(&mut self.grid, to),
            KeyIntent::Copy => self.copy_selection(cx),
            KeyIntent::Paste => self.paste_clipboard(window, cx),
            KeyIntent::SelectAll => {
                self.host.select_all(&mut self.grid);
                self.anchor = None;
            }
            KeyIntent::Text(text) => {
                self.send_input(PendingInputEvent::Text(text.to_owned()), cx)
            }
            KeyIntent::Send {
                key,
                mods,
                consumed_mods,
                text,
                unshifted,
            } => {
                let action = if event.is_held {
                    KeyAction::Repeat
                } else {
                    KeyAction::Press
                };
                self.canceled_key_releases
                    .retain(|canceled| *canceled != key);
                self.send_input(
                    PendingInputEvent::Key {
                        intent: KeyIntent::Send {
                            key,
                            mods,
                            consumed_mods,
                            text,
                            unshifted,
                        },
                        action,
                    },
                    cx,
                );
            }
        }
        self.restart_blink();
        cx.stop_propagation();
        cx.notify();
    }

    fn key_up(&mut self, event: &KeyUpEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus_handle.is_focused(window) {
            self.input_stats.unfocused_key_ups =
                self.input_stats.unfocused_key_ups.saturating_add(1);
            return;
        }
        self.input_stats.key_ups = self.input_stats.key_ups.saturating_add(1);
        if let KeyIntent::Send {
            key,
            mods,
            consumed_mods,
            unshifted,
            ..
        } = self.key_intent(&event.keystroke)
        {
            if self.canceled_key_releases.contains(&key) {
                self.canceled_key_releases
                    .retain(|canceled| *canceled != key);
                cx.stop_propagation();
                return;
            }
            self.send_input(
                PendingInputEvent::Key {
                    intent: KeyIntent::Send {
                        key,
                        mods,
                        consumed_mods,
                        text: String::new(),
                        unshifted,
                    },
                    action: KeyAction::Release,
                },
                cx,
            );
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn to_history(&self, point: ViewportPoint) -> HistoryPoint {
        let offset = self.grid.scrollbar().map_or(0, |bar| bar.offset);
        HistoryPoint {
            row: offset.saturating_add(u64::from(point.y)),
            col: point.x,
        }
    }

    fn to_viewport_clamped(&self, point: HistoryPoint) -> ViewportPoint {
        let size = self.grid.size();
        let max_x = size.cols.saturating_sub(1);
        let max_y = size.rows.saturating_sub(1);
        let offset = self.grid.scrollbar().map_or(0, |bar| bar.offset);
        if point.row < offset {
            ViewportPoint { x: 0, y: 0 }
        } else {
            let rel = point.row - offset;
            if rel > u64::from(max_y) {
                ViewportPoint { x: max_x, y: max_y }
            } else {
                ViewportPoint {
                    x: point.col.min(max_x),
                    y: rel as u16,
                }
            }
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.input_stats.mouse_downs = self.input_stats.mouse_downs.saturating_add(1);
        self.focus_input(window, cx);
        let link_modifier = if self.platform == Platform::Mac {
            event.modifiers.platform
        } else {
            event.modifiers.control
        };
        if event.button == GpuiButton::Left && link_modifier && !event.modifiers.shift {
            if let Some(uri) = self.grid.hyperlink(self.cell_at(event.position)) {
                self.host.open_link(uri);
                self.link_pressed = true;
                self.local_drag = false;
                self.anchor = None;
                cx.stop_propagation();
                cx.notify();
                return;
            }
        }
        if event.button == GpuiButton::Left && self.press_scrollbar(event.position) {
            self.local_drag = false;
            cx.notify();
            return;
        }
        let report = self.mouse_event(
            MouseAction::Press,
            Some(event.button),
            event.position,
            event.modifiers,
        );
        let taken = !event.modifiers.shift && self.host.mouse(&mut self.grid, report);
        if !taken && event.button == GpuiButton::Left {
            self.local_drag = true;
            let here = self.cell_at(event.position);
            if event.modifiers.shift
                && event.click_count == 1
                && let Some(anchor) = self.anchor
            {
                // Shift+Click extends the selection from the existing anchor.
                self.granularity = Granularity::Cell;
                self.block = event.modifiers.alt;
                self.selecting = true;
                let anchor_vp = self.to_viewport_clamped(anchor);
                if self.block {
                    self.host.select_rectangle(&mut self.grid, anchor_vp, here);
                } else {
                    let selection = extend(&self.grid, self.granularity, anchor_vp, here);
                    self.host.select(&mut self.grid, Some(selection));
                }
            } else {
                self.anchor = Some(self.to_history(here));
                self.granularity = Granularity::of_clicks(event.click_count);
                self.block = event.modifiers.alt && self.granularity == Granularity::Cell;
                if self.granularity == Granularity::Cell {
                    self.selecting = false;
                    self.host.select(&mut self.grid, None);
                } else {
                    // A double or triple click selects at once, before any drag.
                    self.selecting = true;
                    let selection = extend(&self.grid, self.granularity, here, here);
                    self.host.select(&mut self.grid, Some(selection));
                }
            }
        } else if event.button == GpuiButton::Left {
            self.local_drag = false;
        }
        cx.notify();
    }

    /// The scrollbar's thumb: its top and height in pixels, or `None` when
    /// there is no history to scroll.
    fn thumb(&self) -> Option<(f32, f32)> {
        let frame = self.frame.borrow();
        let bar = frame
            .scrollbar
            .filter(|bar| scrollbar_shown(*bar, frame.scrollbar_held))?;
        thumb(bar, f32::from(frame.extent.height))
    }

    /// Starts dragging the scrollbar if `position` is on it. A press on the
    /// track off the thumb first jumps there.
    fn press_scrollbar(&mut self, position: Point<Pixels>) -> bool {
        let (x, y) = self.local(position);
        let width = f32::from(self.frame.borrow().extent.width);
        let Some((top, height)) = self.thumb() else {
            return false;
        };
        if x < width - SCROLLBAR_HIT || x > width {
            return false;
        }
        let grab = if (top..top + height).contains(&y) {
            y - top
        } else {
            height / 2.0
        };
        self.scrollbar_grab = Some(grab);
        self.frame.borrow_mut().scrollbar_held = true;
        self.drag_scrollbar(y);
        true
    }

    /// Moves the viewport so the thumb's top follows the pointer at `y`.
    fn drag_scrollbar(&mut self, y: f32) {
        let Some(grab) = self.scrollbar_grab else {
            return;
        };
        let (bar, extent) = {
            let frame = self.frame.borrow();
            (frame.scrollbar, f32::from(frame.extent.height))
        };
        let Some(bar) = bar.filter(|bar| bar.scrollable()) else {
            return;
        };
        let Some((_, height)) = thumb(bar, extent) else {
            return;
        };
        let travel = (extent - height).max(1.0);
        let fraction = ((y - grab) / travel).clamp(0.0, 1.0);
        let last = bar.total.saturating_sub(bar.len);
        let row = (fraction as f64 * last as f64).round() as u64;
        if row != bar.offset {
            self.host
                .scroll_to(&mut self.grid, ScrollTo::Row(row.min(last)));
        }
    }

    /// The pointer moved anywhere in the window; `inside` when over the
    /// grid. A drag that leaves the grid keeps selecting, clamped to its
    /// edge.
    fn mouse_move(&mut self, event: &MouseMoveEvent, inside: bool, cx: &mut Context<Self>) {
        let at = self.cell_at(event.position);
        let link = inside
            .then(|| self.grid.hyperlink(at).map(|uri| (at, uri)))
            .flatten();
        if self.hovered_link != link {
            self.hovered_link = link;
            cx.notify();
        }
        if self.link_pressed {
            return;
        }
        if self.scrollbar_grab.is_some() {
            if event.dragging() {
                let (_, y) = self.local(event.position);
                self.drag_scrollbar(y);
                cx.notify();
            } else {
                self.scrollbar_grab = None;
                self.frame.borrow_mut().scrollbar_held = false;
                cx.notify();
            }
            return;
        }
        if self.local_drag {
            if !event.dragging() {
                self.local_drag = false;
                return;
            }
            if let Some(anchor) = self.anchor {
                let anchor_vp = self.to_viewport_clamped(anchor);
                let here = self.cell_at(event.position);
                if here == anchor_vp && !self.selecting {
                    return;
                }
                self.selecting = true;
                if self.block {
                    self.host.select_rectangle(&mut self.grid, anchor_vp, here);
                } else {
                    let selection = extend(&self.grid, self.granularity, anchor_vp, here);
                    self.host.select(&mut self.grid, Some(selection));
                }
                cx.notify();
            }
            return;
        }
        if !inside {
            return;
        }
        let report = self.mouse_event(
            MouseAction::Motion,
            event.pressed_button,
            event.position,
            event.modifiers,
        );
        if !event.modifiers.shift {
            self.host.mouse(&mut self.grid, report);
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if event.button == GpuiButton::Left && self.link_pressed {
            self.link_pressed = false;
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if event.button == GpuiButton::Left && self.scrollbar_grab.take().is_some() {
            self.frame.borrow_mut().scrollbar_held = false;
            cx.notify();
            return;
        }
        let was_local_drag = event.button == GpuiButton::Left && self.local_drag;
        if event.button == GpuiButton::Left {
            self.local_drag = false;
        }
        let report = self.mouse_event(
            MouseAction::Release,
            Some(event.button),
            event.position,
            event.modifiers,
        );
        if !was_local_drag && !event.modifiers.shift {
            self.host.mouse(&mut self.grid, report);
        }
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(cell) = self.cell_size() else {
            return;
        };
        let delta = event.delta.pixel_delta(px(cell.height));
        // A Mac turns a ⇧ mouse wheel into a sideways scroll; the terminal
        // has no sideways history, so it scrolls up and down.
        let delta = f32::from(if event.modifiers.shift && delta.y == px(0.) {
            delta.x
        } else {
            delta.y
        });
        self.scroll_remainder += delta;
        let lines = (self.scroll_remainder / cell.height).trunc();
        if lines == 0.0 {
            return;
        }
        self.scroll_remainder -= lines * cell.height;
        let at = self.cell_at(event.position);
        self.host
            .scroll(&mut self.grid, lines as i32, at, mods(event.modifiers));
        cx.notify();
    }
}

impl<G: Grid + 'static> Focusable for TerminalView<G> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<G: Grid + 'static> Render for TerminalView<G> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        // Blinks only while its window is active, as Ghostty does: a window
        // in the background draws nothing for its cursor (FAST, D788). The
        // window draws again when it becomes active, which starts the blink
        // over. Decided before the rebuild, so a cursor stopped in its
        // hidden half is drawn shown.
        self.schedule_blink(focused && window.is_window_active(), cx);
        self.rebuild(focused, window);

        let painted = self.frame.clone();
        let measured = self.frame.clone();
        let entity = cx.entity();
        let view = entity.downgrade();
        let moved = view.clone();
        let input_focus = self.focus_handle.clone();
        div()
            .id("terminal")
            .test_support()
            .track_focus(&self.focus_handle)
            .key_context(TERMINAL_CONTEXT)
            .when(self.hovered_link.is_some(), |d| d.cursor_pointer())
            .size_full()
            .relative()
            .overflow_hidden()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                this.key_down(event, window, cx)
            }))
            .on_key_up(
                cx.listener(|this, event: &KeyUpEvent, window, cx| this.key_up(event, window, cx)),
            )
            // The Edit menu sends these to whatever has focus.
            .on_action(cx.listener(|this, _: &edit::Copy, _, cx| {
                this.copy_selection(cx);
            }))
            .on_action(cx.listener(|this, _: &edit::Paste, window, cx| {
                this.paste_clipboard(window, cx);
                this.restart_blink();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &edit::SelectAll, _, cx| {
                this.host.select_all(&mut this.grid);
                this.anchor = None;
                cx.notify();
            }))
            .on_mouse_down(
                GpuiButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.mouse_down(event, window, cx);
                }),
            )
            .on_mouse_down(
                GpuiButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.mouse_down(event, window, cx);
                }),
            )
            .on_mouse_down(
                GpuiButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.mouse_down(event, window, cx);
                }),
            )
            .on_mouse_up(
                GpuiButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.mouse_up(event, cx)),
            )
            .on_mouse_up_out(
                GpuiButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.mouse_up(event, cx)),
            )
            .on_mouse_up(
                GpuiButton::Right,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.mouse_up(event, cx)),
            )
            .on_mouse_up(
                GpuiButton::Middle,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.mouse_up(event, cx)),
            )
            .on_scroll(cx.listener(|this, event: &ScrollEvent, _, cx| this.scroll(event, cx)))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        {
                            let mut frame = measured.borrow_mut();
                            frame.origin = bounds.origin;
                            frame.extent = bounds.size;
                        }
                        let _ = view.update(cx, |this, cx| this.fit(bounds, cx));
                    },
                    move |bounds, (), window, cx| {
                        let mouse_bounds = bounds.intersect(&window.content_mask().bounds);
                        let mut frame = painted.borrow_mut();
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            paint(&mut frame, bounds, window, cx);
                        });
                        drop(frame);
                        window.handle_input(
                            &input_focus,
                            ElementInputHandler::new(bounds, entity.clone()),
                            cx,
                        );
                        // Moves are taken from the window rather than the
                        // element, so a drag past the edge keeps selecting.
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                let inside = mouse_bounds.contains(&event.position);
                                let _ =
                                    moved.update(cx, |this, cx| this.mouse_move(event, inside, cx));
                            }
                        });
                    },
                )
                .size_full(),
            )
            .children(
                self.hovered_link
                    .as_ref()
                    .filter(|(_, uri)| !crate::is_web_or_mail_uri(uri))
                    .map(|(_, uri)| {
                div()
                    .id("terminal-link-hint")
                    .test_support()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .max_w_full()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .px(px(6.))
                    .py(px(3.))
                    .text_size(px(11.))
                    .bg(hsla(self.grid.colors().background, 1.0))
                    .text_color(hsla(self.grid.colors().foreground, 1.0))
                    .child(format!(
                        "{}-click to open · {}",
                        if self.platform == Platform::Mac {
                            "⌘"
                        } else {
                            "Ctrl"
                        },
                        uri
                    ))
            }))
    }
}

fn paint(frame: &mut Frame, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let Some(metrics) = frame.metrics.clone() else {
        return;
    };
    let cell = metrics.cell;
    let origin = bounds.origin;
    window.paint_quad(fill(bounds, hsla(frame.background, 1.0)));

    let at = |col: u16, row: u16| {
        point(
            origin.x + px(f32::from(col) * cell.width),
            origin.y + px(f32::from(row) * cell.height),
        )
    };
    let mut failures = 0;
    for (y, row) in frame.rows.iter().enumerate() {
        let y = u16::try_from(y).unwrap_or(u16::MAX);
        for fill_run in &row.fills {
            let extent = size(px(f32::from(fill_run.cols) * cell.width), px(cell.height));
            window.paint_quad(fill(
                Bounds::new(at(fill_run.col, y), extent),
                hsla(fill_run.color, 1.0),
            ));
        }
        for &(col, cols, current) in &row.marks {
            let extent = size(px(f32::from(cols) * cell.width), px(cell.height));
            let (color, alpha) = match (frame.light_background, current) {
                (true, true) => (LIGHT_CURRENT_MARK_COLOR, LIGHT_CURRENT_MARK_ALPHA),
                (true, false) => (LIGHT_MARK_COLOR, LIGHT_MARK_ALPHA),
                (false, true) => (CURRENT_MARK_COLOR, CURRENT_MARK_ALPHA),
                (false, false) => (MARK_COLOR, MARK_ALPHA),
            };
            window.paint_quad(fill(Bounds::new(at(col, y), extent), hsla(color, alpha)));
        }
        for (col, line) in &row.lines {
            if line
                .paint(
                    at(*col, y),
                    px(cell.height),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .is_err()
            {
                failures += 1;
            }
        }
        for drawn in &row.drawings {
            paint_drawn(drawn, at(drawn.col, y), &metrics, window);
        }
        for decoration in &row.decorations {
            paint_decoration(decoration, at(decoration.col, y), &metrics, window);
        }
    }

    if let Some((row, start, end)) = frame.link_underline {
        let position = at(start, row);
        window.paint_quad(fill(
            Bounds::new(
                point(position.x, position.y + px(cell.height - 2.)),
                size(px(f32::from(end - start + 1) * cell.width), px(1.)),
            ),
            hsla(frame.scrollbar_color, 1.0),
        ));
    }
    if let Some(cursor) = &frame.cursor {
        let plan = &cursor.plan;
        let top_left = at(plan.col, plan.row);
        let width = f32::from(plan.cols) * cell.width;
        let color = hsla(plan.color, 1.0);
        match plan.shape {
            CursorShape::Block => {
                window.paint_quad(fill(
                    Bounds::new(top_left, size(px(width), px(cell.height))),
                    color,
                ));
                if let Some(text) = &cursor.text
                    && text
                        .paint(top_left, px(cell.height), TextAlign::Left, None, window, cx)
                        .is_err()
                {
                    failures += 1;
                }
            }
            CursorShape::BlockHollow => {
                window.paint_quad(outline(
                    Bounds::new(top_left, size(px(width), px(cell.height))),
                    color,
                    BorderStyle::Solid,
                ));
            }
            CursorShape::Bar => {
                let thickness = (metrics.thickness * 2.0).min(cell.width);
                window.paint_quad(fill(
                    Bounds::new(top_left, size(px(thickness), px(cell.height))),
                    color,
                ));
            }
            CursorShape::Underline => {
                let thickness = metrics.thickness * 2.0;
                let y = top_left.y + px(cell.height - thickness);
                window.paint_quad(fill(
                    Bounds::new(point(top_left.x, y), size(px(width), px(thickness))),
                    color,
                ));
            }
        }
    }
    if let Some(preedit) = &frame.preedit {
        let top_left = at(preedit.col, preedit.row);
        let width = f32::from(preedit.cols) * cell.width;
        window.paint_quad(fill(
            Bounds::new(top_left, size(px(width), px(cell.height))),
            hsla(frame.background, 1.0),
        ));
        if preedit
            .text
            .paint(top_left, px(cell.height), TextAlign::Left, None, window, cx)
            .is_err()
        {
            failures += 1;
        }
        let y = top_left.y + px(cell.height - metrics.thickness);
        window.paint_quad(fill(
            Bounds::new(point(top_left.x, y), size(px(width), px(metrics.thickness))),
            hsla(preedit.color, 1.0),
        ));
    }

    if let Some(bar) = frame
        .scrollbar
        .filter(|bar| scrollbar_shown(*bar, frame.scrollbar_held))
        && let Some((top, height)) = thumb(bar, f32::from(bounds.size.height))
    {
        let x = origin.x + bounds.size.width - px(SCROLLBAR_WIDTH + 2.0);
        window.paint_quad(
            fill(
                Bounds::new(
                    point(x, origin.y + px(top)),
                    size(px(SCROLLBAR_WIDTH), px(height)),
                ),
                hsla(frame.scrollbar_color, 0.35),
            )
            .corner_radii(px(SCROLLBAR_WIDTH / 2.0)),
        );
    }
    frame.paint_failures += failures;
}

/// Whether the scrollbar is drawn and can be grabbed: only while the view
/// is back in the history or the thumb is held (L16). At rest on the
/// newest output it is hidden, and a press at the right edge selects or
/// reaches the program as anywhere else.
fn scrollbar_shown(bar: Scrollbar, held: bool) -> bool {
    bar.scrollable() && (held || !bar.at_bottom())
}

/// The scrollbar's thumb in a track `extent` pixels tall: its top and
/// height.
fn thumb(bar: Scrollbar, extent: f32) -> Option<(f32, f32)> {
    if bar.total == 0 || extent <= 0.0 {
        return None;
    }
    let total = bar.total as f64;
    let height =
        ((bar.len as f64 / total) as f32 * extent).clamp(SCROLLBAR_MIN_THUMB.min(extent), extent);
    let last = bar.total.saturating_sub(bar.len).max(1) as f64;
    let top = (bar.offset as f64 / last) as f32 * (extent - height);
    Some((top.clamp(0.0, extent - height), height))
}

/// The marks on history row `row` as (first column, columns, current),
/// from marks sorted by where they start.
fn row_marks(marks: &[Mark], row: u64, cols: u16) -> Vec<(u16, u16, bool)> {
    if marks.is_empty() {
        return Vec::new();
    }
    let right = cols.saturating_sub(1);
    let first = marks.partition_point(|mark| mark.end.row < row);
    marks[first..]
        .iter()
        .take_while(|mark| mark.start.row <= row)
        .filter(|mark| mark.end.row >= row)
        .map(|mark| {
            let from = if mark.start.row == row {
                mark.start.col
            } else {
                0
            };
            let to = if mark.end.row == row {
                mark.end.col
            } else {
                right
            };
            let (from, to) = (from.min(right), to.min(right));
            (from, to.saturating_sub(from) + 1, mark.current)
        })
        .collect()
}

/// UTF-16 length, the unit input methods count in.
fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The input method's view of the terminal: there is no document to edit,
/// only the text being composed, which the view draws at the cursor and
/// sends to the host once committed.
impl<G: Grid + 'static> EntityInputHandler for TerminalView<G> {
    fn accepts_text_input(&self, window: &mut Window, _: &mut Context<Self>) -> bool {
        if !self.focus_handle.is_focused(window) {
            return false;
        }
        // Bypass macOS dead-key IME routing (⌥e, ⌥u, ⌥i, ⌥n, ⌥`) when
        // Option acts as Alt and no CJK preedit is active.
        if self.platform == Platform::Mac
            && self.preedit.is_none()
            && self.host.option_as_alt(&self.grid)
            && window.modifiers().alt
        {
            return false;
        }
        true
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        if !self.focus_handle.is_focused(window) {
            *adjusted = None;
            return None;
        }
        let preedit = self.preedit.as_deref().unwrap_or_default();
        let units: Vec<u16> = preedit.encode_utf16().collect();
        let end = range.end.min(units.len());
        let start = range.start.min(end);
        *adjusted = Some(start..end);
        String::from_utf16(&units[start..end]).ok()
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if !self.focus_handle.is_focused(window) {
            return None;
        }
        let end = self.preedit.as_deref().map_or(0, utf16_len);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        if !self.focus_handle.is_focused(window) {
            return None;
        }
        self.preedit.as_deref().map(|text| 0..utf16_len(text))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if self.preedit.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        self.input_stats.text_commits = self.input_stats.text_commits.saturating_add(1);
        self.preedit = None;
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        if !text.is_empty() {
            self.send_input(PendingInputEvent::Text(text), cx);
        }
        self.restart_blink();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if self.platform == Platform::Mac
            && self.preedit.is_none()
            && self.host.option_as_alt(&self.grid)
            && window.modifiers().alt
        {
            return;
        }
        let starting = self.preedit.is_none() && !text.is_empty();
        self.preedit = (!text.is_empty()).then(|| text.to_owned());
        // Composing is typing: a view scrolled back goes to the live
        // cursor, where the candidate window opens and the text lands.
        if starting && self.grid.scrollbar().is_some_and(|bar| !bar.at_bottom()) {
            self.host.scroll_to(&mut self.grid, ScrollTo::Bottom);
        }
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if !self.focus_handle.is_focused(window) {
            return None;
        }
        // The candidate window opens under the cursor.
        let cell = self.cell_size()?;
        let cursor = self.grid.cursor();
        Some(Bounds::new(
            point(
                element_bounds.origin.x + px(f32::from(cursor.x) * cell.width),
                element_bounds.origin.y + px(f32::from(cursor.y) * cell.height),
            ),
            size(px(cell.width), px(cell.height)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

fn paint_decoration(
    decoration: &Decoration,
    top_left: Point<Pixels>,
    metrics: &Metrics,
    window: &mut Window,
) {
    let cell = metrics.cell;
    let width = f32::from(decoration.cols) * cell.width;
    let thickness = metrics.thickness;
    let color = hsla(decoration.color, decoration.alpha);
    let underline_y = (metrics.baseline + thickness).min(cell.height - thickness);
    let line = |window: &mut Window, y: f32, from: f32, length: f32| {
        window.paint_quad(fill(
            Bounds::new(
                point(top_left.x + px(from), top_left.y + px(y)),
                size(px(length), px(thickness)),
            ),
            color,
        ));
    };
    match decoration.kind {
        LineKind::Underline(Underline::None) => {}
        LineKind::Underline(Underline::Single) => line(window, underline_y, 0.0, width),
        LineKind::Underline(Underline::Double) => {
            let upper = (underline_y - thickness).max(0.0);
            let lower = (underline_y + thickness).min(cell.height - thickness);
            line(window, upper, 0.0, width);
            line(window, lower, 0.0, width);
        }
        LineKind::Underline(Underline::Curly) => {
            window.paint_underline(
                point(top_left.x, top_left.y + px(underline_y)),
                px(width),
                &UnderlineStyle {
                    thickness: px(thickness),
                    color: Some(color),
                    wavy: true,
                },
            );
        }
        LineKind::Underline(Underline::Dotted) => {
            let step = thickness * 2.0;
            let mut x = 0.0;
            while x < width {
                line(window, underline_y, x, thickness.min(width - x));
                x += step;
            }
        }
        LineKind::Underline(Underline::Dashed) => {
            let dash = (cell.width / 2.0).max(thickness);
            let mut x = 0.0;
            while x < width {
                line(window, underline_y, x, dash.min(width - x));
                x += dash * 2.0;
            }
        }
        LineKind::Strikethrough => {
            line(
                window,
                (cell.height / 2.0 - thickness / 2.0).round(),
                0.0,
                width,
            );
        }
        LineKind::Overline => line(window, 0.0, 0.0, width),
    }
}

/// Paints a box-drawing or block character whose cell starts at
/// `top_left`, on whole device pixels.
fn paint_drawn(drawn: &Drawn, top_left: Point<Pixels>, metrics: &Metrics, window: &mut Window) {
    let scale = window.scale_factor();
    let cell = metrics.cell;
    let left = f32::from(top_left.x);
    let top = f32::from(top_left.y);
    let area = Area {
        left: left * scale,
        top: top * scale,
        right: (left + f32::from(drawn.cols) * cell.width) * scale,
        bottom: (top + cell.height) * scale,
    };
    for rect in boxes::rects(drawn.drawing, area, metrics.thickness * scale) {
        let origin = point(px(rect.left / scale), px(rect.top / scale));
        let extent = size(
            px((rect.right - rect.left) / scale),
            px((rect.bottom - rect.top) / scale),
        );
        window.paint_quad(fill(
            Bounds::new(origin, extent),
            hsla(drawn.color, drawn.alpha * rect.alpha),
        ));
    }
}

fn shape(piece: Piece, metrics: &Metrics, text_system: &WindowTextSystem) -> ShapedLine {
    let run = TextRun {
        len: piece.text.len(),
        font: metrics.font(piece.style.bold, piece.style.italic),
        color: hsla(piece.style.color, piece.style.alpha),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    text_system.shape_line(
        SharedString::from(piece.text),
        px(metrics.font_size),
        &[run],
        Some(px(metrics.cell.width)),
    )
}

fn hsla(color: Rgb, alpha: f32) -> Hsla {
    Rgba {
        r: f32::from(color.r) / 255.0,
        g: f32::from(color.g) / 255.0,
        b: f32::from(color.b) / 255.0,
        a: alpha,
    }
    .into()
}

fn terminal_button(button: GpuiButton) -> MouseButton {
    match button {
        GpuiButton::Left => MouseButton::Left,
        GpuiButton::Right => MouseButton::Right,
        GpuiButton::Middle => MouseButton::Middle,
        GpuiButton::Navigate(NavigationDirection::Back) => MouseButton::Eight,
        GpuiButton::Navigate(NavigationDirection::Forward) => MouseButton::Nine,
    }
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod clipboard_tests;
