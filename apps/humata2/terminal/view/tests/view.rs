//! The terminal view in a headless window, driven through real input:
//! frames redraw only the rows that changed, keys and pastes reach the
//! host, a drag selects cells and the copy shortcut puts them on the
//! clipboard.

use std::cell::RefCell;
use std::rc::Rc;

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::base::input as edit;
use gpui_kit::gpui;
use gpui_kit::test::{ClickOptions, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, ClipboardEntry, ClipboardItem, Entity, Focusable, Image,
    ImageFormat, Modifiers, Pixels, Point, ScrollDelta, TestAppContext, Window, WindowBounds,
    WindowOptions, point, px, size,
};
use terminal_grid::{
    Cell, Cursor, CursorStyle, FakeGrid, Grid, Key, KeyAction, KeyEvent, Mods, Row, Scrollbar,
    Size, ViewportPoint,
};
use terminal_view::{
    CellSize, HistoryPoint, Mark, Paste, ScrollTo, TerminalHost, TerminalView, Theme,
};

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
    scrolled_to: Vec<ScrollTo>,
    texts: Vec<String>,
    mouse_moves: usize,
    links: Vec<String>,
}

/// A host over a [`FakeGrid`]: selecting selects in the grid, copying
/// reads the selected cells, and the rest is written down.
struct TestHost(Rc<RefCell<Log>>);

impl TerminalHost<FakeGrid> for TestHost {
    fn key(&mut self, _: &mut FakeGrid, event: KeyEvent<'_>) {
        self.0
            .borrow_mut()
            .keys
            .push((event.key, event.action, event.mods, event.text.to_owned()));
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

    fn select_rectangle(&mut self, grid: &mut FakeGrid, start: ViewportPoint, end: ViewportPoint) {
        grid.select_rectangle(start, end);
    }

    fn scroll_to(&mut self, _: &mut FakeGrid, to: ScrollTo) {
        self.0.borrow_mut().scrolled_to.push(to);
    }

    fn open_link(&mut self, uri: String) {
        self.0.borrow_mut().links.push(uri);
    }
    fn mouse(&mut self, _: &mut FakeGrid, event: terminal_grid::MouseEvent) -> bool {
        if event.action == terminal_grid::MouseAction::Motion {
            self.0.borrow_mut().mouse_moves += 1;
        }
        false
    }
    fn text(&mut self, _: &mut FakeGrid, text: &str) {
        self.0.borrow_mut().texts.push(text.to_owned());
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
    // The test platform opens windows inactive and activates them on a
    // later turn of the executor; focus changes in an inactive window are
    // not reported.
    cx.update_window(handle, |_, window, _| window.activate_window())
        .expect("the test window is open");
    cx.run_until_parked();
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
        assert_ne!(
            wanted,
            Size {
                cols: COLS,
                rows: ROWS
            }
        );
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
        assert_eq!(
            grid.selection(0).map(|span| (span.start, span.end)),
            Some((6, COLS - 1))
        );
        assert_eq!(
            grid.selection(1).map(|span| (span.start, span.end)),
            Some((0, 5))
        );

        window.press(COPY, cx);
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some("worldsecond"));
    })
    .expect("the test window is open");
}

/// The Edit menu's Copy, Paste and Select All act on a focused terminal as
/// its shortcuts do.
#[gpui_kit::test]
fn the_edit_menu_copies_pastes_and_selects_all(cx: &mut TestAppContext) {
    let (handle, view, log) = open(cx, grid_with(&["hello world", "second line"]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        focus(&view, window, cx);
        // `Window::dispatch_action` runs the action only after this update;
        // the menu reaches the focused terminal the same way, so dispatch
        // on its focus handle, which runs the handler now.
        let menu = view.read(cx).focus_handle(cx);

        menu.dispatch_action(&edit::SelectAll, window, cx);
        window.render_frame(cx);
        let grid = view.read(cx).grid();
        assert!((0..ROWS).all(|y| grid.selection(y).is_some()));

        menu.dispatch_action(&edit::Copy, window, cx);
        let copied = cx.read_from_clipboard().and_then(|item| item.text());
        let copied = copied.unwrap_or_default();
        assert!(copied.contains("hello world"));
        assert!(copied.contains("second line"));

        cx.write_to_clipboard(ClipboardItem::new_string("echo hi".to_owned()));
        menu.dispatch_action(&edit::Paste, window, cx);
        assert_eq!(log.borrow().pastes, vec![Paste::Text("echo hi".to_owned())]);
        assert!(log.borrow().keys.is_empty());
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

fn selected(view: &View, cx: &gpui_kit::App) -> Vec<Option<(u16, u16)>> {
    let grid = view.read(cx).grid();
    (0..3)
        .map(|y| grid.selection(y).map(|span| (span.start, span.end)))
        .collect()
}

#[gpui_kit::test]
fn a_double_click_selects_a_word_and_a_triple_click_a_line(cx: &mut TestAppContext) {
    let (handle, view, _) = open(cx, grid_with(&["ls ~/src/humata-2 --all", "next"]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let terminal = window.find("terminal").bounds().origin;
        let at = cell_center(&view, window, 8, 0, cx) - terminal;
        window.click_with_options(
            "terminal",
            ClickOptions::new().with_offset(at).with_count(2),
            cx,
        );
        window.render_frame(cx);
        // A path is one word: `/`, `~`, `-` and `.` don't break it.
        assert_eq!(selected(&view, cx), vec![Some((3, 16)), None, None]);

        window.click_with_options(
            "terminal",
            ClickOptions::new().with_offset(at).with_count(3),
            cx,
        );
        window.render_frame(cx);
        assert_eq!(selected(&view, cx), vec![Some((0, COLS - 1)), None, None]);
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn an_alt_drag_selects_a_block(cx: &mut TestAppContext) {
    let (handle, view, _) = open(cx, grid_with(&["abcdef", "ghijkl", "mnopqr"]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let from = cell_center(&view, window, 1, 0, cx);
        let to = cell_center(&view, window, 3, 2, cx);
        window.dispatch_event(
            gpui_kit::PlatformInput::ModifiersChanged(gpui_kit::ModifiersChangedEvent {
                modifiers: Modifiers::alt(),
                capslock: Default::default(),
            }),
            cx,
        );
        window.drag(from, to, cx);
        window.render_frame(cx);
        assert_eq!(selected(&view, cx), vec![Some((1, 3)); 3]);
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn history_keys_and_the_scrollbar_move_through_the_history(cx: &mut TestAppContext) {
    // Back in the history, so the scrollbar is shown.
    let mut grid = grid_with(&["older output"]);
    grid.set_scrollbar(Some(Scrollbar {
        total: 1000,
        offset: 500,
        len: u64::from(ROWS),
    }));
    let (handle, view, log) = open(cx, grid);
    cx.update_window(handle, |_, window, cx| {
        focus(&view, window, cx);
        let (up, top) = if cfg!(target_os = "macos") {
            ("cmd-pageup", "cmd-home")
        } else {
            ("shift-pageup", "shift-home")
        };
        window.press(up, cx);
        window.press(top, cx);
        assert_eq!(
            log.borrow().scrolled_to,
            vec![ScrollTo::PageUp, ScrollTo::Top]
        );
        // The keys went to the history, not the program.
        assert!(log.borrow().keys.is_empty());

        // Dragging the thumb from the bottom to the top of the track asks
        // for the first rows.
        log.borrow_mut().scrolled_to.clear();
        let bounds = window.find("terminal").bounds();
        let x = bounds.origin.x + bounds.size.width - px(3.);
        let bottom = point(x, bounds.origin.y + bounds.size.height - px(2.));
        let top = point(x, bounds.origin.y);
        window.drag(bottom, top, cx);
        let rows = log.borrow().scrolled_to.clone();
        assert_eq!(rows.last(), Some(&ScrollTo::Row(0)), "{rows:?}");
        assert!(
            view.read(cx).grid().selection(0).is_none(),
            "the drag didn't select"
        );
    })
    .expect("the test window is open");
}

/// L16: on the newest output the scrollbar is hidden, so a drag along the
/// right edge selects text instead of moving through the history.
#[gpui_kit::test]
fn at_the_newest_output_the_right_edge_selects_instead_of_scrolling(cx: &mut TestAppContext) {
    let mut grid = grid_with(&["older output"; ROWS as usize]);
    grid.set_scrollbar(Some(Scrollbar {
        total: 1000,
        offset: 1000 - u64::from(ROWS),
        len: u64::from(ROWS),
    }));
    let (handle, view, log) = open(cx, grid);
    cx.update_window(handle, |_, window, cx| {
        focus(&view, window, cx);
        let bounds = window.find("terminal").bounds();
        let x = bounds.origin.x + bounds.size.width - px(3.);
        let start = point(x, bounds.origin.y + bounds.size.height - px(2.));
        let end = point(x, bounds.origin.y + px(2.));
        window.drag(start, end, cx);
        assert!(
            log.borrow().scrolled_to.is_empty(),
            "the history didn't move"
        );
        assert!(
            view.read(cx).grid().selection(0).is_some(),
            "the drag selected"
        );
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn with_blinking_off_no_blink_is_scheduled(cx: &mut TestAppContext) {
    let mut grid = grid_with(&[]);
    grid.set_cursor_style(CursorStyle {
        blinking: true,
        ..CursorStyle::default()
    });
    grid.set_cursor(Cursor {
        x: 0,
        y: 0,
        visible: true,
    });
    let (handle, view, _) = open(cx, grid);
    cx.update_window(handle, |_, window, cx| {
        focus(&view, window, cx);
        // Gaining focus restarts the blink, on the frame after.
        window.render_frame(cx);
        assert!(view.read(cx).is_blink_scheduled());
        view.update(cx, |view, cx| view.set_cursor_blink(false, cx));
        window.render_frame(cx);
        assert!(!view.read(cx).is_blink_scheduled());
    })
    .expect("the test window is open");
}

/// FAST (D788): a window in the background schedules no blink, so its
/// idle terminal draws nothing; active again, the cursor blinks again.
#[gpui_kit::test]
fn a_window_in_the_background_stops_the_blink(cx: &mut TestAppContext) {
    let mut grid = grid_with(&[]);
    grid.set_cursor_style(CursorStyle {
        blinking: true,
        ..CursorStyle::default()
    });
    grid.set_cursor(Cursor {
        x: 0,
        y: 0,
        visible: true,
    });
    let (handle, view, _) = open(cx, grid);
    cx.update_window(handle, |_, window, cx| {
        focus(&view, window, cx);
        window.render_frame(cx);
        assert!(view.read(cx).is_blink_scheduled());
    })
    .expect("the test window is open");

    gpui_kit::VisualTestContext::from_window(handle, cx).deactivate_window();
    cx.update_window(handle, |_, window, cx| {
        assert!(!window.is_window_active());
        window.render_frame(cx);
        assert!(!view.read(cx).is_blink_scheduled());
    })
    .expect("the test window is open");

    cx.update_window(handle, |_, window, _| window.activate_window())
        .expect("the test window is open");
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).is_blink_scheduled());
    })
    .expect("the test window is open");
}

#[gpui_kit::test]
fn marks_in_view_draw_their_rows_again(cx: &mut TestAppContext) {
    let (handle, view, _) = open(cx, grid_with(&["find me", "and me"]));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
        let mark = |row| Mark {
            start: HistoryPoint { row, col: 0 },
            end: HistoryPoint { row, col: 3 },
            current: row == 1,
        };
        view.update(cx, |view, cx| view.set_marks(vec![mark(1), mark(0)], cx));
        window.render_frame(cx);
        // Kept in history order whatever order they came in.
        let rows: Vec<_> = view
            .read(cx)
            .marks()
            .iter()
            .map(|mark| mark.start.row)
            .collect();
        assert_eq!(rows, vec![0, 1]);
        assert!(view.read(cx).stats().rows_rebuilt >= 2);
    })
    .expect("the test window is open");
}

struct ClippedPair {
    left: View,
    right: View,
}
impl gpui_kit::Render for ClippedPair {
    fn render(
        &mut self,
        _: &mut Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::prelude::*;
        gpui_kit::div()
            .size_full()
            .flex()
            .child(
                gpui_kit::div()
                    .w(px(240.))
                    .h_full()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(
                        gpui_kit::div()
                            .w(px(640.))
                            .h_full()
                            .child(self.left.clone()),
                    ),
            )
            .child(
                gpui_kit::div()
                    .w(px(240.))
                    .h_full()
                    .flex_shrink_0()
                    .child(self.right.clone()),
            )
    }
}
fn clipped_pair(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    View,
    View,
    Rc<RefCell<Log>>,
    Rc<RefCell<Log>>,
) {
    let left_log = Rc::new(RefCell::new(Log::default()));
    let right_log = Rc::new(RefCell::new(Log::default()));
    let (window, pair) = cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Point::default(),
                    size(px(480.), px(280.)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                let mut grid = FakeGrid::new(80, 10);
                grid.write(40, 0, "https://hidden.test", &Cell::default());
                let left = cx.new(|cx| {
                    TerminalView::new(
                        grid,
                        TestHost(left_log.clone()),
                        Theme::default(),
                        window,
                        cx,
                    )
                });
                let right = cx.new(|cx| {
                    TerminalView::new(
                        grid_with(&["right terminal"]),
                        TestHost(right_log.clone()),
                        Theme::default(),
                        window,
                        cx,
                    )
                });
                cx.new(|_| ClippedPair { left, right })
            },
        )
        .unwrap()
    });
    cx.update_window(window, |_, w, _| w.activate_window())
        .unwrap();
    cx.run_until_parked();
    let (left, right) = cx.update(|cx| {
        let pair = pair.read(cx);
        (pair.left.clone(), pair.right.clone())
    });
    cx.update_window(window, |_, w, cx| {
        w.render_frame(cx);
        focus(&left, w, cx);
    })
    .unwrap();
    (window, left, right, left_log, right_log)
}

#[gpui_kit::test]
fn clipped_grid_does_not_hover_report_or_open_a_link_under_the_other_pane(cx: &mut TestAppContext) {
    let (window, left, right, left_log, _) = clipped_pair(cx);
    cx.update_window(window, |_, w, cx| {
        let view = left.read(cx);
        let bounds = view.viewport_bounds();
        let cell = view.cell_size().unwrap();
        let at = point(
            bounds.origin.x + px(42.5 * cell.width),
            bounds.origin.y + px(0.5 * cell.height),
        );
        assert!(at.x > px(240.) && at.x < px(480.));
        left_log.borrow_mut().mouse_moves = 0;
        w.dispatch_event(
            gpui_kit::PlatformInput::MouseMove(gpui_kit::MouseMoveEvent {
                position: at,
                pressed_button: None,
                modifiers: Modifiers::default(),
            }),
            cx,
        );
        w.render_frame(cx);
        assert!(
            w.try_find("terminal-link-hint").is_none(),
            "a clipped link must not expose its hover hint in the adjacent pane"
        );
        assert_eq!(
            left_log.borrow().mouse_moves,
            0,
            "a clipped grid must not report the adjacent pane's mouse motion"
        );
        let modifiers = Modifiers {
            platform: true,
            control: true,
            ..Default::default()
        };
        w.dispatch_event(
            gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
                position: at,
                button: gpui_kit::MouseButton::Left,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        w.dispatch_event(
            gpui_kit::PlatformInput::MouseUp(gpui_kit::MouseUpEvent {
                position: at,
                button: gpui_kit::MouseButton::Left,
                modifiers,
                click_count: 1,
            }),
            cx,
        );
        assert!(right.read(cx).focus_handle(cx).is_focused(w));
        assert!(
            left_log.borrow().links.is_empty(),
            "the invisible left link must not open when clicking the right pane"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn focus_handoff_routes_immediate_text_and_rejects_stale_ime_callbacks(cx: &mut TestAppContext) {
    use gpui_kit::EntityInputHandler;
    let (window, left, right, left_log, right_log) = clipped_pair(cx);
    cx.update(|cx| {
        cx.update_window(window, |_, w, cx| {
            let bounds = right.read(cx).viewport_bounds();
            let at = point(bounds.origin.x + px(20.), bounds.origin.y + px(30.));
            w.dispatch_event(
                gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
                    position: at,
                    button: gpui_kit::MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            w.dispatch_event(
                gpui_kit::PlatformInput::MouseUp(gpui_kit::MouseUpEvent {
                    position: at,
                    button: gpui_kit::MouseButton::Left,
                    modifiers: Modifiers::default(),
                    click_count: 1,
                }),
                cx,
            );
            // No test render_frame, executor wait or animation frame after the click.
        })
        .unwrap()
    });
    cx.update(|cx| {
        cx.update_window(window, |_, w, cx| {
            // Force the test platform's IME path rather than TerminalView's raw
            // key encoder; the commit must use the currently registered handler.
            right.update(cx, |view, cx| {
                view.replace_and_mark_text_in_range(None, "composition", None, w, cx)
            });
            let mut key = gpui_kit::Keystroke::parse_with_os("a", cx.platform_info().os).unwrap();
            key.key_char = Some("a".into());
            w.dispatch_keystroke(key, cx);
            assert_eq!(right_log.borrow().texts, vec!["a"]);
            assert!(left_log.borrow().texts.is_empty());
            left.update(cx, |view, cx| {
                view.replace_text_in_range(None, "stale", w, cx);
                view.replace_and_mark_text_in_range(None, "stale composition", None, w, cx);
                assert!(view.preedit().is_none());
                assert!(view.selected_text_range(false, w, cx).is_none());
            });
            assert!(
                left_log.borrow().texts.is_empty(),
                "an old platform handler must not write to an unfocused terminal"
            );
        })
        .unwrap()
    });
}
