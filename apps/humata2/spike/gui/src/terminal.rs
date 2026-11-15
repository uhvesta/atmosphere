//! Screen 3: a terminal-sized grid of cells fed fake build output, drawn by
//! Humata 2's own terminal view (`terminal_view`, decision D22). The output
//! goes into a `FakeGrid`; libghostty-vt's terminal implements the same
//! `Grid` trait, so it replaces the fake output without touching the view.

use std::time::Instant;

use gpui_kit::*;
use terminal_grid::{Cell, Color, Colors, Cursor, FakeGrid, Grid, KeyEvent, Rgb, Row};
use terminal_view::{TerminalHost, TerminalView, Theme};

/// The monospace font the spike draws code and cells in.
pub const MONOSPACE: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "monospace"
};

/// The grid's size.
pub const COLS: u16 = 200;
pub const ROWS: u16 = 60;

/// How many lines of fake output appear each second while animating.
pub const LINES_PER_SECOND: f64 = 60.;

const FONT_SIZE: f32 = 12.;
const LINE_HEIGHT: f32 = 16.;
const DEFAULT_FG: u32 = 0xc8ced8;
const DEFAULT_BG: u32 = 0x111318;

/// Made-up build output, a line at a time, the same every run.
pub struct FakeOutput {
    cols: u16,
    written: u64,
}

impl FakeOutput {
    pub fn new(cols: u16) -> Self {
        Self { cols, written: 0 }
    }

    /// How many lines have been written since the start.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// The next line, `cols` cells wide.
    pub fn next_line(&mut self) -> Row {
        let n = self.written;
        self.written += 1;
        let fg = Color::Default;
        let mut line = LineBuilder::new(self.cols);
        match n % 40 {
            0 => {
                line.push("~/atmosphere ", rgb_color(0x6fb3ff), None, true);
                line.push("$ ", rgb_color(0x8fd18f), None, true);
                line.push("cargo build --release --workspace", fg, None, false);
            }
            7 | 23 => {
                line.push("warning", rgb_color(0xf2c65b), None, true);
                line.push(
                    &format!(": unused variable `row_{n}` in src/view/session_{}.rs:{}:{}", n % 17, n % 400, n % 80),
                    fg,
                    None,
                    false,
                );
            }
            13 | 31 => {
                let done = (n * 7 % 101) as usize;
                let width = 50;
                let filled = done * width / 100;
                line.push("    Building ", rgb_color(0x8fd18f), None, true);
                line.push("[", fg, None, false);
                line.push(&" ".repeat(filled), fg, Some(rgb_color(0x3f7f4f)), false);
                line.push(&" ".repeat(width - filled), fg, Some(rgb_color(0x2a2f38)), false);
                line.push(&format!("] {done:3}% {}/{} crates", n % 300, 300), fg, None, false);
            }
            19 | 37 => {
                line.push(" INFO ", rgb_color(0x0d1117), Some(rgb_color(0x6fb3ff)), true);
                line.push(
                    &format!(" daemon: session {} attached to channel #{} after {} ms; ", n % 977, n % 200, n % 53),
                    fg,
                    None,
                    false,
                );
                line.push(&"·".repeat((n % 90) as usize), rgb_color(0x4d5563), None, false);
            }
            _ => {
                line.push("   Compiling ", rgb_color(0x8fd18f), None, true);
                line.push(
                    &format!("crate-{:04} v0.{}.{} (/src/crates/crate-{:04})", n % 1000, n % 13, n % 7, n % 1000),
                    fg,
                    None,
                    false,
                );
            }
        }
        line.finish()
    }
}

fn rgb_color(value: u32) -> Color {
    let Rgb { r, g, b } = Rgb::hex(value);
    Color::Rgb { r, g, b }
}

struct LineBuilder {
    cols: usize,
    cells: Vec<Cell>,
}

impl LineBuilder {
    fn new(cols: u16) -> Self {
        Self {
            cols: usize::from(cols),
            cells: Vec::with_capacity(usize::from(cols)),
        }
    }

    fn push(&mut self, text: &str, fg: Color, bg: Option<Color>, bold: bool) {
        for ch in text.chars() {
            if self.cells.len() >= self.cols {
                return;
            }
            self.cells.push(Cell {
                text: if ch == ' ' { String::new() } else { ch.to_string() },
                fg,
                bg: bg.unwrap_or_default(),
                bold,
                ..Cell::default()
            });
        }
    }

    fn finish(mut self) -> Row {
        self.cells.resize(self.cols, Cell::default());
        Row {
            cells: self.cells,
            wrapped: false,
        }
    }
}

/// The spike's output has no program behind it, so input goes nowhere.
struct NoProgram;

impl TerminalHost<FakeGrid> for NoProgram {
    fn key(&mut self, _: &mut FakeGrid, _: KeyEvent<'_>) {}
}

/// A full screen of fake output, cursor hidden.
fn filled_grid(output: &mut FakeOutput) -> FakeGrid {
    let mut grid = FakeGrid::new(COLS, ROWS);
    grid.set_colors(Colors {
        foreground: Rgb::hex(DEFAULT_FG),
        background: Rgb::hex(DEFAULT_BG),
        ..Colors::default()
    });
    grid.set_cursor(Cursor {
        x: 0,
        y: 0,
        visible: false,
    });
    for y in 0..ROWS {
        grid.set_row(y, output.next_line());
    }
    grid
}

/// The terminal screen.
pub struct TerminalScreen {
    view: Entity<TerminalView<FakeGrid>>,
    output: FakeOutput,
    animating: bool,
    last_tick: Option<Instant>,
    owed: f64,
}

impl TerminalScreen {
    pub fn new(animate: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut output = FakeOutput::new(COLS);
        let grid = filled_grid(&mut output);
        let theme = Theme {
            font_family: MONOSPACE.into(),
            font_size: FONT_SIZE,
            line_height: LINE_HEIGHT,
            ..Theme::default()
        };
        let view = cx.new(|cx| TerminalView::new(grid, NoProgram, theme, window, cx));
        Self {
            view,
            output,
            animating: animate,
            last_tick: None,
            owed: 0.,
        }
    }

    pub fn animating(&self) -> bool {
        self.animating
    }

    pub fn set_animating(&mut self, animating: bool, cx: &mut Context<Self>) {
        if self.animating != animating {
            self.animating = animating;
            self.last_tick = None;
            self.owed = 0.;
            cx.notify();
        }
    }

    /// Writes the lines owed since the last frame, at [`LINES_PER_SECOND`].
    fn pump(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        if let Some(last) = self.last_tick {
            self.owed += now.duration_since(last).as_secs_f64() * LINES_PER_SECOND;
            // After a long stall, catch up by one screen at most.
            self.owed = self.owed.min(f64::from(ROWS));
            let whole = self.owed.floor();
            self.owed -= whole;
            if whole >= 1. {
                let output = &mut self.output;
                self.view.update(cx, |view, cx| {
                    for _ in 0..whole as usize {
                        view.grid_mut().scroll_up(output.next_line());
                    }
                    cx.notify();
                });
            }
        }
        self.last_tick = Some(now);
    }
}

impl Render for TerminalScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.animating {
            self.pump(cx);
            window.request_animation_frame();
        }
        let animating = self.animating;
        let size = self.view.read(cx).grid().size();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x1b1f27))
            .text_color(rgb(0xe6e9ef))
            .text_size(px(13.))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .border_b_1()
                    .border_color(rgb(0x2d3340))
                    .child(
                        div()
                            .id("animate-toggle")
                            .test_support()
                            .role(Role::Button)
                            .aria_label("Animate output")
                            .aria_selected(animating)
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(if animating { rgb(0x2f4a72) } else { rgb(0x2d3340) })
                            .cursor_pointer()
                            .child(if animating { "Output: running" } else { "Output: paused" })
                            .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                                let animating = !this.animating;
                                this.set_animating(animating, cx);
                            })),
                    )
                    .child(div().text_color(rgb(0x8a93a3)).child(format!(
                        "{}×{} cells · {} lines written",
                        size.cols,
                        size.rows,
                        self.output.written()
                    ))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(self.view.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(grid: &FakeGrid, y: u16) -> String {
        grid.row(y)
            .map(|row| row.cells.iter().map(|cell| cell.text.as_str()).collect())
            .unwrap_or_default()
    }

    #[::std::prelude::v1::test]
    fn every_line_fills_the_grid_width() {
        let mut output = FakeOutput::new(COLS);
        for _ in 0..80 {
            assert_eq!(output.next_line().cells.len(), usize::from(COLS));
        }
        let grid = filled_grid(&mut FakeOutput::new(COLS));
        assert!((0..ROWS).all(|y| !text(&grid, y).is_empty()));
    }

    #[::std::prelude::v1::test]
    fn new_output_scrolls_lines_up() {
        let mut output = FakeOutput::new(80);
        let mut grid = FakeGrid::new(80, 10);
        for y in 0..10 {
            grid.set_row(y, output.next_line());
        }
        let second = text(&grid, 1);
        grid.scroll_up(output.next_line());
        assert_eq!(text(&grid, 0), second);
        assert_eq!(output.written(), 11);
    }
}
