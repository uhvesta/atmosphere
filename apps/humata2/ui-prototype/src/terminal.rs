//! Real Humata terminal renderer over an in-memory demonstration grid.
use gpui_kit::{AppContext, Entity, Window};
use terminal_grid::{
    Cell, Colors, Cursor, CursorStyle, Dirty, Grid, Key, KeyAction, KeyEvent, Mods, Row, Size,
    Span, ViewportPoint,
};
use terminal_view::{CellSize, Paste, TerminalHost, TerminalView, Theme};

pub(super) type View = Entity<TerminalView<DemoGrid>>;
pub(super) fn open(
    title: &str,
    agent: &str,
    machine: &str,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> View {
    let mut grid = DemoGrid::new();
    if agent == "zsh" {
        grid.output(&format!("you@{machine} · {title}\n~/Humata/payments-rewrite\n\n$ git status --short\n M migrations/004_ledger.sql\n M src/billing/service.rs\n\n[Dummy shell: no commands execute.]\n\n$ "));
    } else {
        grid.output(&format!("{agent} · {title}\n{machine} · ~/Humata/payments-rewrite\n\n> Implement the ledger migration and a reversible rollout.\n\n● Read src/billing/service.rs\n● Edit migrations/004_ledger.sql +42\n● Run ledger balance checks\n\nThe migration is ready for review. Add feedback in the Diff panel.\n\n[Interactive prototype: input stays in memory; no commands execute.]\n\n> "));
    }
    cx.new(|cx| {
        TerminalView::new(
            grid,
            DemoHost,
            Theme {
                font_family: "JetBrains Mono".into(),
                font_size: 12.,
                line_height: 21.,
                ..Theme::default()
            },
            window,
            cx,
        )
    })
}

pub(super) struct DemoGrid {
    size: Size,
    history: Vec<String>,
    rows: Vec<Row>,
    offset: usize,
    dirty: bool,
    colors: Colors,
    selection: Option<(ViewportPoint, ViewportPoint)>,
    enabled: bool,
    pub(super) messages_seen: usize,
}
impl DemoGrid {
    fn new() -> Self {
        Self {
            size: Size { cols: 90, rows: 30 },
            history: vec![String::new()],
            rows: vec![],
            offset: 0,
            dirty: true,
            colors: Colors {
                background: terminal_grid::Rgb::hex(0x151619),
                foreground: terminal_grid::Rgb::hex(0xdedfe3),
                ..Colors::default()
            },
            selection: None,
            enabled: true,
            messages_seen: 0,
        }
    }
    pub(super) fn set_enabled(&mut self, enabled: bool) -> bool {
        let changed = self.enabled != enabled;
        self.enabled = enabled;
        self.dirty |= changed;
        changed
    }
    pub(super) fn output(&mut self, text: &str) {
        for part in text.split_inclusive('\n') {
            self.history
                .last_mut()
                .unwrap()
                .push_str(part.trim_end_matches('\n'));
            if part.ends_with('\n') {
                self.history.push(String::new());
            }
        }
        self.offset = 0;
        self.dirty = true;
    }
    fn wrapped(&self) -> Vec<String> {
        let cols = usize::from(self.size.cols.max(1));
        self.history
            .iter()
            .flat_map(|line| {
                let chars: Vec<_> = line.chars().collect();
                if chars.is_empty() {
                    vec![String::new()]
                } else {
                    chars.chunks(cols).map(|s| s.iter().collect()).collect()
                }
            })
            .collect()
    }
}
impl Grid for DemoGrid {
    fn sync(&mut self) {
        if !self.dirty {
            return;
        }
        let lines = self.wrapped();
        let end = lines.len().saturating_sub(self.offset);
        let start = end.saturating_sub(usize::from(self.size.rows));
        self.rows = lines[start..end]
            .iter()
            .map(|line| {
                let mut cells: Vec<_> = line
                    .chars()
                    .map(|c| Cell {
                        text: c.to_string(),
                        ..Cell::default()
                    })
                    .collect();
                cells.resize(usize::from(self.size.cols), Cell::default());
                Row {
                    cells,
                    wrapped: false,
                }
            })
            .collect();
        self.rows.resize_with(usize::from(self.size.rows), || Row {
            cells: vec![Cell::default(); usize::from(self.size.cols)],
            wrapped: false,
        });
    }
    fn size(&self) -> Size {
        self.size
    }
    fn row(&self, y: u16) -> Option<&Row> {
        self.rows.get(usize::from(y))
    }
    fn cursor(&self) -> Cursor {
        let lines = self.wrapped();
        Cursor {
            x: lines
                .last()
                .map(|s| s.chars().count() as u16)
                .unwrap_or(0)
                .min(self.size.cols.saturating_sub(1)),
            y: (lines.len().saturating_sub(1) as u16).min(self.size.rows.saturating_sub(1)),
            visible: self.enabled && self.offset == 0,
        }
    }
    fn cursor_style(&self) -> CursorStyle {
        CursorStyle::default()
    }
    fn colors(&self) -> &Colors {
        &self.colors
    }
    fn selection(&self, y: u16) -> Option<Span> {
        let (a, b) = self.selection?;
        let (start, end) = if (a.y, a.x) <= (b.y, b.x) {
            (a, b)
        } else {
            (b, a)
        };
        if y < start.y || y > end.y {
            None
        } else {
            Some(Span {
                start: if y == start.y { start.x } else { 0 },
                end: if y == end.y {
                    end.x
                } else {
                    self.size.cols.saturating_sub(1)
                },
            })
        }
    }
    fn dirty(&self) -> Dirty {
        if self.dirty {
            Dirty::Full
        } else {
            Dirty::Clean
        }
    }
    fn is_row_dirty(&self, _: u16) -> bool {
        self.dirty
    }
    fn clear_dirty(&mut self) {
        self.dirty = false;
    }
}
struct DemoHost;
impl TerminalHost<DemoGrid> for DemoHost {
    fn key(&mut self, grid: &mut DemoGrid, event: KeyEvent<'_>) {
        if !grid.enabled || event.action == KeyAction::Release {
            return;
        }
        match event.key {
            Key::Enter => grid.output("\n[Dummy agent received your input]\n> "),
            Key::Backspace => {
                grid.history.last_mut().unwrap().pop();
                grid.dirty = true;
            }
            _ => {
                if !event.text.is_empty() {
                    grid.output(event.text);
                }
            }
        }
    }
    fn paste(&mut self, grid: &mut DemoGrid, paste: Paste) {
        if !grid.enabled {
            return;
        }
        match paste {
            Paste::Text(text) => grid.output(&text),
            Paste::Files(paths) => grid.output(
                &paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            Paste::Image { .. } => grid.output("[Pasted image · dummy attachment]"),
        }
    }
    fn resize(&mut self, grid: &mut DemoGrid, size: Size, _: CellSize) {
        grid.size = size;
        grid.dirty = true;
    }
    fn scroll(&mut self, grid: &mut DemoGrid, lines: i32, _: ViewportPoint, _: Mods) {
        grid.offset = grid.offset.saturating_add_signed(lines as isize).min(
            grid.wrapped()
                .len()
                .saturating_sub(usize::from(grid.size.rows)),
        );
        grid.dirty = true;
    }
    fn select(&mut self, grid: &mut DemoGrid, selection: Option<(ViewportPoint, ViewportPoint)>) {
        grid.selection = selection;
        grid.dirty = true;
    }
    fn copy(&mut self, grid: &mut DemoGrid) -> Option<String> {
        let text = (0..grid.size.rows)
            .filter_map(|y| {
                grid.selection(y).and_then(|span| {
                    grid.row(y).map(|row| {
                        row.cells[usize::from(span.start)..=usize::from(span.end)]
                            .iter()
                            .map(|cell| cell.text.as_str())
                            .collect::<String>()
                    })
                })
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() { None } else { Some(text) }
    }
}
