use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{
    App, BorderStyle, Bounds, ClipboardItem, ContentMask, Context, DispatchPhase, FocusHandle,
    Focusable, Font, FontStyle, FontWeight, Hsla, InteractiveElement, IntoElement, KeyDownEvent,
    KeyUpEvent, MouseButton as GpuiButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    NavigationDirection, ParentElement, Pixels, Point, Render, Rgba, ScrollEvent, ShapedLine,
    SharedString, Styled, Subscription, Task,
    TextAlign, TextRun, UnderlineStyle, Window, WindowTextSystem, canvas, div, fill, font,
    outline, point, px, size,
};
use gpui_kit::base::TestSupportExt as _;
use terminal_grid::{
    CursorShape, Dirty, Grid, KeyAction, KeyEvent, MouseAction, MouseButton, MouseEvent,
    Rgb, Size, Underline, ViewportPoint,
};

use crate::cursor::{CursorPlan, plan_cursor};
use crate::frame::{Decoration, Fill, LineKind, Piece, plan_row};
use crate::host::{CellSize, TerminalHost};
use crate::keys::{KeyIntent, Platform, intent, mods};
use crate::paste::Paste;
use crate::theme::Theme;

/// How long a blinking cursor stays on, and then off, as in `Ghostty`.
const BLINK: Duration = Duration::from_millis(600);

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
    /// Text pieces that failed to paint, over all frames. A piece that
    /// fails is left blank, usually because its font is missing.
    pub paint_failures: u64,
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
    /// The cell a left-button drag started on, while the button is down.
    anchor: Option<ViewportPoint>,
    /// Whether the drag has left its first cell, so a click alone selects
    /// nothing.
    selecting: bool,
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
    background: Rgb,
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
        let focus_handle = cx.focus_handle();
        let focus = [
            cx.on_focus(&focus_handle, window, |this: &mut Self, _, cx| {
                this.host.focus(&mut this.grid, true);
                this.restart_blink();
                cx.notify();
            }),
            cx.on_blur(&focus_handle, window, |this: &mut Self, _, cx| {
                this.host.focus(&mut this.grid, false);
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
            anchor: None,
            selecting: false,
            scroll_remainder: 0.0,
            blink_on: true,
            blink_task: None,
            platform: Platform::of(cx.platform_info()),
            requested: None,
            _focus: focus,
        }
    }

    pub fn grid(&self) -> &G {
        &self.grid
    }

    /// The grid, to feed it output. Call `cx.notify()` on the view after,
    /// so it redraws.
    pub fn grid_mut(&mut self) -> &mut G {
        &mut self.grid
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Changes the font or colors; every row is drawn again.
    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        if theme != self.theme {
            self.theme = theme;
            self.theme_changed = true;
            cx.notify();
        }
    }

    /// The size of a cell, once a frame has been drawn.
    pub fn cell_size(&self) -> Option<CellSize> {
        self.frame.borrow().metrics.as_ref().map(|metrics| metrics.cell)
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

        let mut full = self.theme_changed || self.grid.dirty() == Dirty::Full;
        if frame.metrics.as_ref().is_none_or(|metrics| !metrics.fits(&self.theme)) {
            frame.metrics = Some(Metrics::measure(&self.theme, &text_system));
            full = true;
        }
        let Some(metrics) = frame.metrics.clone() else {
            return;
        };
        let size = self.grid.size();
        if frame.size != size {
            frame.size = size;
            frame.rows.resize_with(usize::from(size.rows), ShapedRow::default);
            full = true;
        }
        let colors = self.grid.colors();
        if frame.background != colors.background {
            // Rows skip fills in the default background, so they change too.
            frame.background = colors.background;
            full = true;
        }

        let style = self.theme.style();
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
                };
                rows_rebuilt += 1;
            }
        }

        frame.cursor = plan_cursor(&self.grid, focused, self.blink_on, self.theme.cursor_text, &style)
            .map(|plan| {
                let text = plan.text.as_ref().map(|(text, color)| {
                    shape(
                        Piece {
                            col: plan.col,
                            text: text.clone(),
                            style: crate::frame::TextStyle {
                                color: *color,
                                alpha: 1.0,
                                bold: false,
                                italic: false,
                            },
                        },
                        &metrics,
                        &text_system,
                    )
                });
                ShapedCursor { plan, text }
            });

        self.grid.clear_dirty();
        self.theme_changed = false;
        self.stats.frames += 1;
        self.stats.rows_rebuilt = rows_rebuilt;
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
        if !focused || !style.blinking || !self.grid.cursor().visible {
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
        let rows = (f32::from(bounds.size.height) / cell.height).floor().max(1.0);
        let wanted = Size {
            cols: cols.min(f32::from(u16::MAX)) as u16,
            rows: rows.min(f32::from(u16::MAX)) as u16,
        };
        if wanted == self.grid.size() || self.requested == Some(wanted) {
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
        let col = (x / cell.width).floor().clamp(0.0, f32::from(size.cols.saturating_sub(1)));
        let row = (y / cell.height).floor().clamp(0.0, f32::from(size.rows.saturating_sub(1)));
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

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match intent(&event.keystroke, self.platform) {
            KeyIntent::Ignore => return,
            KeyIntent::Copy => {
                if let Some(text) = self.host.copy(&mut self.grid) {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            }
            KeyIntent::Paste => {
                if let Some(paste) = cx
                    .read_from_clipboard()
                    .and_then(|item| Paste::from_clipboard(&item))
                {
                    self.host.paste(&mut self.grid, paste);
                }
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
                self.host.key(&mut self.grid, KeyEvent {
                    key,
                    action,
                    mods,
                    consumed_mods,
                    text: &text,
                    unshifted,
                    composing: false,
                });
            }
        }
        self.restart_blink();
        cx.stop_propagation();
        cx.notify();
    }

    fn key_up(&mut self, event: &KeyUpEvent, cx: &mut Context<Self>) {
        if let KeyIntent::Send {
            key,
            mods,
            consumed_mods,
            unshifted,
            ..
        } = intent(&event.keystroke, self.platform)
        {
            self.host.key(&mut self.grid, KeyEvent {
                key,
                action: KeyAction::Release,
                mods,
                consumed_mods,
                text: "",
                unshifted,
                composing: false,
            });
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        let report = self.mouse_event(
            MouseAction::Press,
            Some(event.button),
            event.position,
            event.modifiers,
        );
        let taken = !event.modifiers.shift && self.host.mouse(&mut self.grid, report);
        if !taken && event.button == GpuiButton::Left {
            self.anchor = Some(self.cell_at(event.position));
            self.selecting = false;
            self.host.select(&mut self.grid, None);
        }
        cx.notify();
    }

    /// The pointer moved anywhere in the window; `inside` when over the
    /// grid. A drag that leaves the grid keeps selecting, clamped to its
    /// edge.
    fn mouse_move(&mut self, event: &MouseMoveEvent, inside: bool, cx: &mut Context<Self>) {
        if !inside && self.anchor.is_none() {
            return;
        }
        let report = self.mouse_event(
            MouseAction::Motion,
            event.pressed_button,
            event.position,
            event.modifiers,
        );
        let taken = inside && !event.modifiers.shift && self.host.mouse(&mut self.grid, report);
        if let Some(anchor) = self.anchor
            && event.dragging()
            && !taken
        {
            let here = self.cell_at(event.position);
            if here == anchor && !self.selecting {
                return;
            }
            self.selecting = true;
            self.host.select(&mut self.grid, Some((anchor, here)));
            cx.notify();
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        let report = self.mouse_event(
            MouseAction::Release,
            Some(event.button),
            event.position,
            event.modifiers,
        );
        if !event.modifiers.shift {
            self.host.mouse(&mut self.grid, report);
        }
        if event.button == GpuiButton::Left {
            self.anchor = None;
        }
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) {
        let Some(cell) = self.cell_size() else {
            return;
        };
        let delta = f32::from(event.delta.pixel_delta(px(cell.height)).y);
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
        self.rebuild(focused, window);
        self.schedule_blink(focused, cx);

        let painted = self.frame.clone();
        let measured = self.frame.clone();
        let view = cx.entity().downgrade();
        let moved = view.clone();
        div()
            .id("terminal")
            .test_support()
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.key_down(event, cx)))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| this.key_up(event, cx)))
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
            .on_scroll_wheel(cx.listener(|this, event: &ScrollEvent, _, cx| this.scroll(event, cx)))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        measured.borrow_mut().origin = bounds.origin;
                        let _ = view.update(cx, |this, cx| this.fit(bounds, cx));
                    },
                    move |bounds, (), window, cx| {
                        let mut frame = painted.borrow_mut();
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            paint(&mut frame, bounds, window, cx);
                        });
                        // Moves are taken from the window rather than the
                        // element, so a drag past the edge keeps selecting.
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                let inside = bounds.contains(&event.position);
                                let _ = moved.update(cx, |this, cx| this.mouse_move(event, inside, cx));
                            }
                        });
                    },
                )
                .size_full(),
            )
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
            window.paint_quad(fill(Bounds::new(at(fill_run.col, y), extent), hsla(fill_run.color, 1.0)));
        }
        for (col, line) in &row.lines {
            if line.paint(at(*col, y), px(cell.height), TextAlign::Left, None, window, cx).is_err() {
                failures += 1;
            }
        }
        for decoration in &row.decorations {
            paint_decoration(decoration, at(decoration.col, y), &metrics, window);
        }
    }

    if let Some(cursor) = &frame.cursor {
        let plan = &cursor.plan;
        let top_left = at(plan.col, plan.row);
        let width = f32::from(plan.cols) * cell.width;
        let color = hsla(plan.color, 1.0);
        match plan.shape {
            CursorShape::Block => {
                window.paint_quad(fill(Bounds::new(top_left, size(px(width), px(cell.height))), color));
                if let Some(text) = &cursor.text
                    && text.paint(top_left, px(cell.height), TextAlign::Left, None, window, cx).is_err()
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
                window.paint_quad(fill(Bounds::new(top_left, size(px(thickness), px(cell.height))), color));
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
    frame.paint_failures += failures;
}

fn paint_decoration(decoration: &Decoration, top_left: Point<Pixels>, metrics: &Metrics, window: &mut Window) {
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
            line(window, (cell.height / 2.0 - thickness / 2.0).round(), 0.0, width);
        }
        LineKind::Overline => line(window, 0.0, 0.0, width),
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
