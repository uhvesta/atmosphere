//! The terminal view in a headless window, driven through real input:
//! frames redraw only the rows that changed, keys and pastes reach the
//! host, a drag selects cells and the copy shortcut puts them on the
//! clipboard.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ClipboardEntry, ClipboardItem, Entity, Focusable, Image,
    ImageFormat, Pixels, Point, ScrollDelta, TestAppContext, Window, WindowBounds, WindowOptions,
    point, px, size,
};
use terminal_grid::{
    Cell, FakeGrid, Grid, Key, KeyAction, KeyEvent, Mods, Row, Size, ViewportPoint,
};
use terminal_view::{CellSize, Paste, TerminalHost, TerminalView, Theme};

const COLS: u16 = 40;
const ROWS: u16 = 10;

/// What the view asked of the host.
#[derive(Default)]
struct Log {
    keys: Vec<(Key, KeyAction, Mods, String)>,
    scrolls: Vec<i32>,
    pastes: Vec<Paste>,
    resizes: Vec<Size>,
    focus: Vec<bool>,
}

/// A host over a [`FakeGrid`]: selecting selects in the grid, copying
/// reads the selected cells, and the rest is written down.
struct TestHost(Rc<RefCell<Log>>);

impl TerminalHost<FakeGrid> for TestHost {
    fn key(&mut self, _: &mut FakeGrid, event: KeyEvent<'_>) {
        self.0.borrow_mut().keys.push((
            event.key,
            event.action,
            event.mods,
            event.text.to_owned(),
        ));
    }

    fn scroll(&mut self, _: &mut FakeGrid, lines: i32, _: ViewportPoint, _: Mods) {
        self.0.borrow_mut().scrolls.push(lines);
    }

    fn select(&mut self, grid: &mut FakeGrid, selection: Option<(ViewportPoint, ViewportPoint)>) {
        match selection {
            Some((start, end)) => grid.select(start, end),
            None => grid.clear_selection(),
        }
    }

    fn copy(&mut self, grid: &mut FakeGrid) -> Option<String> {
        let mut text = String::new();
        for y in 0..grid.size().rows {
            let (Some(span), Some(row)) = (grid.selection(y), grid.row(y)) else {
                continue;
            };
            for cell in &row.cells[usize::from(span.start)..=usize::from(span.end)] {
                text.push_str(&cell.text);
            }
        }
        (!text.is_empty()).then_some(text)
    }

    fn paste(&mut self, _: &mut FakeGrid, paste: Paste) {
        self.0.borrow_mut().pastes.push(paste);
    }

    fn resize(&mut self, _: &mut FakeGrid, size: Size, _: CellSize) {
        self.0.borrow_mut().resizes.push(size);
    }

    fn focus(&mut self, _: &mut FakeGrid, focused: bool) {
        self.0.borrow_mut().focus.push(focused);
    }
}

type View = Entity<TerminalView<FakeGrid>>;

fn open(cx: &mut TestAppContext, grid: FakeGrid) -> (AnyWindowHandle, View, Rc<RefCell<Log>>) {
    let log = Rc::new(RefCell::new(Log::default()));
    let host = TestHost(log.clone());
    let (handle, view) = cx.update(|cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(480.), px(240.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| TerminalView::new(grid, host, Theme::default(), window, cx))
        })
        .expect("the test window opens")
    });
    (handle, view, log)
}

fn grid_with(lines: &[&str]) -> FakeGrid {
    let mut grid = FakeGrid::new(COLS, ROWS);
    for (y, line) in lines.iter().enumerate() {
        let y = u16::try_from(y).expect("few lines");
        grid.write(0, y, line, &Cell::default());
    }
    grid
}

fn focus(view: &View, window: &mut Window, cx: &mut gpui_kit::App) {
    let handle = view.read(cx).focus_handle(cx);
    window.focus(&handle, cx);
    window.render_frame(cx);
}

/// The window position of the middle of cell (`x`, `y`).
fn cell_center(view: &View, window: &Window, x: u16, y: u16, cx: &gpui_kit::App) -> Point<Pixels> {
    let cell = view.read(cx).cell_size().expect("a frame was drawn");
    let origin = window.find("terminal").bounds().origin;
    point(
        origin.x + px((f32::from(x) + 0.5) * cell.width),
        origin.y + px((f32::from(y) + 0.5) * cell.height),
    )
}

const COPY: &str = if cfg!(target_os = "macos") {
    "cmd-c"
} else {
    "ctrl-shift-c"
};
const PASTE: &str = if cfg!(target_os = "macos") {
    "cmd-v"
} else {
    "ctrl-shift-v"
};

#[gpui_kit::test]
fn only_the_rows_that_changed_are_drawn_again(cx: &mut TestAppContext) {
    let (handle, view, _) = open(cx, grid_with(&["one", "two", "three"]));
    cx.update_window(handle, |_, window, cx| {
        // The window drew once when it opened, every row then.
        window.render_frame(cx);
        let first = view.read(cx).stats();
        assert!(first.frames >= 1);

        window.render_frame(cx);
        let idle = view.read(cx).stats();
        assert!(idle.frames > first.frames);
        assert_eq!(idle.rows_rebuilt, 0);
        assert_eq!(idle.pieces_shaped, 0);

        view.update(cx, |view, cx| {
            view.grid_mut().write(0, 4, "four", &Cell::default());
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(view.read(cx).stats().rows_rebuilt, 1);

        view.update(cx, |view, cx| {
            view.grid_mut().scroll_up(Row::default());
            cx.notify();
        });
        window.render_frame(cx);
        let scrolled = view.read(cx).stats();
        assert_eq!(scrolled.rows_rebuilt, u32::from(ROWS));
        // "one", "two" and "four" are left after scrolling.
        assert!(scrolled.pieces_shaped >= 3);
        assert_eq!(scrolled.paint_failures, 0);
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn a_new_theme_draws_every_row_again(cx: &mut TestAppContext) {
    let (handle, view, _) = open(cx, grid_with(&["one"]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let before = view.read(cx).cell_size().expect("a frame was drawn");
        view.update(cx, |view, cx| {
            let theme = Theme {
                font_size: 26.0,
                line_height: 34.0,
                ..view.theme().clone()
            };
            view.set_theme(theme, cx);
        });
        window.render_frame(cx);
        assert_eq!(view.read(cx).stats().rows_rebuilt, u32::from(ROWS));
        let after = view.read(cx).cell_size().expect("a frame was drawn");
        assert!(after.width > before.width);
        assert!(after.height > before.height);
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn the_view_asks_the_host_for_the_size_that_fills_it(cx: &mut TestAppContext) {
    let (handle, view, log) = open(cx, grid_with(&[]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let cell = view.read(cx).cell_size().expect("a frame was drawn");
        let bounds = window.find("terminal").bounds();
        let wanted = Size {
            cols: (f32::from(bounds.size.width) / cell.width).floor() as u16,
            rows: (f32::from(bounds.size.height) / cell.height).floor() as u16,
        };
        assert_ne!(wanted, Size { cols: COLS, rows: ROWS });
        // Asked once, though the grid (whose host ignored it) never fits.
        assert_eq!(log.borrow().resizes, vec![wanted]);
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn typed_keys_reach_the_host_with_their_text(cx: &mut TestAppContext) {
    let (handle, view, log) = open(cx, grid_with(&[]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        focus(&view, window, cx);
        assert_eq!(log.borrow().focus, vec![true]);

        window.press("a", cx);
        window.press("ctrl-c", cx);
        window.press("enter", cx);
        let presses: Vec<_> = log
            .borrow()
            .keys
            .iter()
            .filter(|(_, action, _, _)| *action == KeyAction::Press)
            .cloned()
            .collect();
        assert_eq!(
            presses,
            vec![
                (Key::A, KeyAction::Press, Mods::NONE, "a".to_owned()),
                (Key::C, KeyAction::Press, Mods::CTRL, String::new()),
                (Key::Enter, KeyAction::Press, Mods::NONE, String::new()),
            ]
        );
        assert!(
            log.borrow()
                .keys
                .iter()
                .any(|(key, action, _, _)| *key == Key::A && *action == KeyAction::Release)
        );
    })
    .expect("the test window is open");
}

#[cfg(target_os = "macos")]
#[gpui_kit::test]
fn the_apps_own_shortcuts_are_not_sent_to_the_program(cx: &mut TestAppContext) {
    let (handle, view, log) = open(cx, grid_with(&[]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        focus(&view, window, cx);
        window.press("cmd-t", cx);
        assert!(log.borrow().keys.is_empty());
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn dragging_selects_cells_and_the_copy_shortcut_copies_them(cx: &mut TestAppContext) {
    let (handle, view, _) = open(cx, grid_with(&["hello world", "second line"]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let from = cell_center(&view, window, 6, 0, cx);
        let to = cell_center(&view, window, 5, 1, cx);
        window.drag(from, to, cx);
        window.render_frame(cx);

        let grid = view.read(cx).grid();
        assert_eq!(grid.selection(0).map(|span| (span.start, span.end)), Some((6, COLS - 1)));
        assert_eq!(grid.selection(1).map(|span| (span.start, span.end)), Some((0, 5)));

        window.press(COPY, cx);
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some("worldsecond"));
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn a_click_clears_the_selection(cx: &mut TestAppContext) {
    let (handle, view, _) = open(cx, grid_with(&["hello world"]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let from = cell_center(&view, window, 0, 0, cx);
        let to = cell_center(&view, window, 4, 0, cx);
        window.drag(from, to, cx);
        assert!(view.read(cx).grid().selection(0).is_some());

        window.drag(to, to, cx);
        window.render_frame(cx);
        assert_eq!(view.read(cx).grid().selection(0), None);
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn pasted_text_and_images_reach_the_host(cx: &mut TestAppContext) {
    let (handle, view, log) = open(cx, grid_with(&[]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        focus(&view, window, cx);

        cx.write_to_clipboard(ClipboardItem::new_string("echo hi".to_owned()));
        window.press(PASTE, cx);

        let png = vec![0x89, b'P', b'N', b'G'];
        cx.write_to_clipboard(ClipboardItem {
            entries: vec![ClipboardEntry::Image(Image::from_bytes(
                ImageFormat::Png,
                png.clone(),
            ))],
        });
        window.press(PASTE, cx);

        assert_eq!(
            log.borrow().pastes,
            vec![
                Paste::Text("echo hi".to_owned()),
                Paste::Image {
                    format: ImageFormat::Png,
                    bytes: png,
                },
            ]
        );
        // A paste shortcut is never typed as a key.
        assert!(log.borrow().keys.is_empty());
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn scrolling_sends_whole_rows_older_output_first(cx: &mut TestAppContext) {
    let (handle, _view, log) = open(cx, grid_with(&[]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll("terminal", ScrollDelta::Lines(point(0., 3.)), cx);
        window.scroll("terminal", ScrollDelta::Lines(point(0., -2.)), cx);
        assert_eq!(log.borrow().scrolls, vec![3, -2]);
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn small_pixel_scrolls_add_up_to_a_row(cx: &mut TestAppContext) {
    let (handle, view, log) = open(cx, grid_with(&[]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let height = view.read(cx).cell_size().expect("a frame was drawn").height;
        let step = ScrollDelta::Pixels(point(px(0.), px(height * 0.4)));
        window.scroll("terminal", step, cx);
        window.scroll("terminal", step, cx);
        assert!(log.borrow().scrolls.is_empty());
        window.scroll("terminal", step, cx);
        assert_eq!(log.borrow().scrolls, vec![1]);
    })
    .expect("the test window is open");
}
