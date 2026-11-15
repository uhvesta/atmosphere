//! Screen 3: a terminal-sized grid of cells, painted by hand: one quad per
//! run of background colour and one shaped text line per row. The grid sits
//! behind [`GridSource`], so libghostty-vt's screen can replace the fake
//! output later without touching the painting.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use gpui_kit::*;

/// The monospace font the spike draws code and cells in.
pub const MONOSPACE: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "monospace"
};

/// The grid's size.
pub const COLS: usize = 200;
pub const ROWS: usize = 60;

/// How many lines of fake output appear each second while animating.
pub const LINES_PER_SECOND: f64 = 60.;

const FONT_SIZE: f32 = 12.;
const LINE_HEIGHT: f32 = 16.;
const DEFAULT_FG: CellColor = CellColor(0xc8ced8);
const DEFAULT_BG: u32 = 0x111318;

/// A colour as 0xRRGGBB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellColor(pub u32);

/// One cell: a character, its colours and its weight. A cell with no
/// background shows the terminal's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridCell {
    pub ch: char,
    pub fg: CellColor,
    pub bg: Option<CellColor>,
    pub bold: bool,
}

impl GridCell {
    const BLANK: GridCell = GridCell {
        ch: ' ',
        fg: DEFAULT_FG,
        bg: None,
        bold: false,
    };
}

/// A grid's width and height in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: usize,
    pub rows: usize,
}

/// Something with a grid of cells to show, such as a terminal's screen.
pub trait GridSource {
    fn size(&self) -> GridSize;
    /// Row `index`'s cells, left to right; empty for a row past the end.
    fn row(&self, index: usize) -> &[GridCell];
}

/// Made-up build output that scrolls up a line at a time, the same every run.
pub struct FakeOutput {
    size: GridSize,
    lines: VecDeque<Vec<GridCell>>,
    written: u64,
}

impl FakeOutput {
    /// A full screen of output, `cols` wide and `rows` high.
    pub fn new(size: GridSize) -> Self {
        let mut output = Self {
            size,
            lines: VecDeque::with_capacity(size.rows + 1),
            written: 0,
        };
        output.advance(size.rows);
        output
    }

    /// How many lines have been written since the start.
    pub fn written(&self) -> u64 {
        self.written
    }

    /// Writes `count` more lines, scrolling the oldest off the top.
    pub fn advance(&mut self, count: usize) {
        for _ in 0..count {
            let line = self.make_line(self.written);
            self.written += 1;
            self.lines.push_back(line);
            while self.lines.len() > self.size.rows {
                self.lines.pop_front();
            }
        }
    }

    fn make_line(&self, n: u64) -> Vec<GridCell> {
        let mut line = LineBuilder::new(self.size.cols);
        match n % 40 {
            0 => {
                line.push("~/atmosphere ", CellColor(0x6fb3ff), None, true);
                line.push("$ ", CellColor(0x8fd18f), None, true);
                line.push("cargo build --release --workspace", DEFAULT_FG, None, false);
            }
            7 | 23 => {
                line.push("warning", CellColor(0xf2c65b), None, true);
                line.push(
                    &format!(": unused variable `row_{n}` in src/view/session_{}.rs:{}:{}", n % 17, n % 400, n % 80),
                    DEFAULT_FG,
                    None,
                    false,
                );
            }
            13 | 31 => {
                let done = (n * 7 % 101) as usize;
                let width = 50;
                let filled = done * width / 100;
                line.push("    Building ", CellColor(0x8fd18f), None, true);
                line.push("[", DEFAULT_FG, None, false);
                line.push(&" ".repeat(filled), DEFAULT_FG, Some(CellColor(0x3f7f4f)), false);
                line.push(&" ".repeat(width - filled), DEFAULT_FG, Some(CellColor(0x2a2f38)), false);
                line.push(&format!("] {done:3}% {}/{} crates", n % 300, 300), DEFAULT_FG, None, false);
            }
            19 | 37 => {
                line.push(" INFO ", CellColor(0x0d1117), Some(CellColor(0x6fb3ff)), true);
                line.push(
                    &format!(" daemon: session {} attached to channel #{} after {} ms; ", n % 977, n % 200, n % 53),
                    DEFAULT_FG,
                    None,
                    false,
                );
                line.push(&"·".repeat((n % 90) as usize), CellColor(0x4d5563), None, false);
            }
            _ => {
                line.push("   Compiling ", CellColor(0x8fd18f), None, true);
                line.push(
                    &format!("crate-{:04} v0.{}.{} (/src/crates/crate-{:04})", n % 1000, n % 13, n % 7, n % 1000),
                    DEFAULT_FG,
                    None,
                    false,
                );
            }
        }
        line.finish()
    }
}

impl GridSource for FakeOutput {
    fn size(&self) -> GridSize {
        self.size
    }

    fn row(&self, index: usize) -> &[GridCell] {
        self.lines.get(index).map_or(&[][..], Vec::as_slice)
    }
}

struct LineBuilder {
    cols: usize,
    cells: Vec<GridCell>,
}

impl LineBuilder {
    fn new(cols: usize) -> Self {
        Self {
            cols,
            cells: Vec::with_capacity(cols),
        }
    }

    fn push(&mut self, text: &str, fg: CellColor, bg: Option<CellColor>, bold: bool) {
        for ch in text.chars() {
            if self.cells.len() >= self.cols {
                return;
            }
            self.cells.push(GridCell { ch, fg, bg, bold });
        }
    }

    fn finish(mut self) -> Vec<GridCell> {
        self.cells.resize(self.cols, GridCell::BLANK);
        self.cells
    }
}

/// One row ready to shape: its text, its colour runs and its backgrounds.
struct RowPaint {
    text: SharedString,
    /// Byte length, colour and weight of each run of text.
    runs: Vec<(usize, CellColor, bool)>,
    /// First column, column count and colour of each run of background.
    backgrounds: Vec<(usize, usize, CellColor)>,
}

fn row_paint(cells: &[GridCell]) -> RowPaint {
    let mut text = String::with_capacity(cells.len());
    let mut runs: Vec<(usize, CellColor, bool)> = Vec::new();
    let mut backgrounds: Vec<(usize, usize, CellColor)> = Vec::new();
    for (col, cell) in cells.iter().enumerate() {
        text.push(cell.ch);
        let len = cell.ch.len_utf8();
        match runs.last_mut() {
            Some((run_len, fg, bold)) if *fg == cell.fg && *bold == cell.bold => *run_len += len,
            _ => runs.push((len, cell.fg, cell.bold)),
        }
        if let Some(bg) = cell.bg {
            match backgrounds.last_mut() {
                Some((start, count, color)) if *color == bg && *start + *count == col => *count += 1,
                _ => backgrounds.push((col, 1, bg)),
            }
        }
    }
    RowPaint {
        text: text.into(),
        runs,
        backgrounds,
    }
}

fn hsla(color: CellColor) -> Hsla {
    rgb(color.0).into()
}

static PAINT_FAILED: AtomicBool = AtomicBool::new(false);

/// The terminal screen.
pub struct TerminalScreen {
    grid: FakeOutput,
    animating: bool,
    last_tick: Option<Instant>,
    owed: f64,
}

impl TerminalScreen {
    pub fn new(animate: bool) -> Self {
        Self {
            grid: FakeOutput::new(GridSize {
                cols: COLS,
                rows: ROWS,
            }),
            animating: animate,
            last_tick: None,
            owed: 0.,
        }
    }

    pub fn grid(&self) -> &impl GridSource {
        &self.grid
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
    fn pump(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_tick {
            self.owed += now.duration_since(last).as_secs_f64() * LINES_PER_SECOND;
            // After a long stall, catch up by one screen at most.
            self.owed = self.owed.min(ROWS as f64);
            let whole = self.owed.floor();
            self.owed -= whole;
            self.grid.advance(whole as usize);
        }
        self.last_tick = Some(now);
    }

    fn render_grid(&self) -> impl IntoElement + use<> {
        let size = self.grid.size();
        let rows: Vec<RowPaint> = (0..size.rows).map(|row| row_paint(self.grid.row(row))).collect();
        canvas(
            move |_bounds, window, _cx| {
                let font_size = px(FONT_SIZE);
                let regular = font(MONOSPACE);
                let bold = Font {
                    weight: FontWeight::BOLD,
                    ..regular.clone()
                };
                let text_system = window.text_system().clone();
                let cell_width = text_system
                    .advance(text_system.resolve_font(&regular), font_size, 'm')
                    .map(|advance| advance.width)
                    .unwrap_or(px(FONT_SIZE * 0.6));
                let shaped: Vec<(ShapedLine, Vec<(usize, usize, CellColor)>)> = rows
                    .into_iter()
                    .map(|row| {
                        let runs: Vec<TextRun> = row
                            .runs
                            .iter()
                            .map(|&(len, fg, is_bold)| TextRun {
                                len,
                                font: if is_bold { bold.clone() } else { regular.clone() },
                                color: hsla(fg),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            })
                            .collect();
                        let line = text_system.shape_line(row.text, font_size, &runs, Some(cell_width));
                        (line, row.backgrounds)
                    })
                    .collect();
                (cell_width, shaped)
            },
            move |bounds, (cell_width, shaped), window, cx| {
                let line_height = px(LINE_HEIGHT);
                window.paint_quad(fill(bounds, hsla(CellColor(DEFAULT_BG))));
                for (index, (line, backgrounds)) in shaped.iter().enumerate() {
                    let y = bounds.origin.y + line_height * index as f32;
                    for &(start, count, color) in backgrounds {
                        let origin = point(bounds.origin.x + cell_width * start as f32, y);
                        let extent = gpui_kit::size(cell_width * count as f32, line_height);
                        window.paint_quad(fill(Bounds::new(origin, extent), hsla(color)));
                    }
                    let origin = point(bounds.origin.x, y);
                    if let Err(error) = line.paint(origin, line_height, TextAlign::Left, None, window, cx)
                        && !PAINT_FAILED.swap(true, Ordering::Relaxed)
                    {
                        eprintln!(
                            "could not paint a row of the terminal grid: {error}. \
                             Rows that fail are left blank; check that the {MONOSPACE} font is installed"
                        );
                    }
                }
            },
        )
        .size_full()
    }
}

impl Render for TerminalScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.animating {
            self.pump();
            window.request_animation_frame();
        }
        let animating = self.animating;
        let size = self.grid.size();
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
                        self.grid.written()
                    ))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(self.render_grid()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(grid: &impl GridSource, row: usize) -> String {
        grid.row(row).iter().map(|cell| cell.ch).collect()
    }

    #[::std::prelude::v1::test]
    fn every_row_fills_the_grid_width() {
        let grid = FakeOutput::new(GridSize { cols: COLS, rows: ROWS });
        assert_eq!(grid.size(), GridSize { cols: COLS, rows: ROWS });
        assert!((0..ROWS).all(|row| grid.row(row).len() == COLS));
        assert!(grid.row(ROWS).is_empty());
    }

    #[::std::prelude::v1::test]
    fn advancing_scrolls_lines_up() {
        let mut grid = FakeOutput::new(GridSize { cols: 80, rows: 10 });
        let second = text(&grid, 1);
        grid.advance(1);
        assert_eq!(text(&grid, 0), second);
        assert_eq!(grid.written(), 11);
    }

    #[::std::prelude::v1::test]
    fn runs_cover_the_row_and_backgrounds_join_neighbours() {
        let red = CellColor(0xff0000);
        let cells = [
            GridCell { ch: 'a', fg: DEFAULT_FG, bg: Some(red), bold: false },
            GridCell { ch: 'é', fg: DEFAULT_FG, bg: Some(red), bold: true },
            GridCell { ch: 'c', fg: DEFAULT_FG, bg: None, bold: true },
            GridCell { ch: 'd', fg: DEFAULT_FG, bg: Some(red), bold: true },
        ];
        let row = row_paint(&cells);
        assert_eq!(row.runs.iter().map(|run| run.0).sum::<usize>(), row.text.len());
        assert_eq!(row.backgrounds, vec![(0, 2, red), (3, 1, red)]);
    }
}
