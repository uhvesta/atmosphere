//! Several terminals visible at once, as a split workspace shows them
//! (NV-28): each is sized to its own box, a click gives it the keyboard,
//! and what is typed reaches only its own program.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

// `#[gpui_kit::test]` expands to paths under `gpui`.
use ghostty_vt::{Size, Terminal};
use gpui_kit::gpui;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, App, Bounds, Context, Entity, Point, TestAppContext, TestSupportExt as _, Window,
    WindowBounds, WindowOptions, div, px, size,
};
use support::{PtyLog, RecordingPty};
use terminal_pane::{Script, TerminalPane};
use terminal_view::Theme;

/// Terminals in a grid of `columns`, each in a box with an id the test
/// can click.
struct Split {
    panes: Vec<Entity<TerminalPane>>,
    columns: usize,
}

impl Render for Split {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().flex().flex_col().children(self.panes.chunks(self.columns).enumerate().map(
            |(row, panes)| {
                div().flex().flex_1().min_h(px(0.)).children(panes.iter().enumerate().map(|(column, pane)| {
                    div()
                        .id(format!("pane-{}", row * self.columns + column))
                        .test_support()
                        .flex_1()
                        .min_w(px(0.))
                        .h_full()
                        .overflow_hidden()
                        .child(pane.clone())
                }))
            },
        ))
    }
}

struct Opened {
    window: AnyWindowHandle,
    split: Entity<Split>,
    ptys: Vec<Rc<RefCell<PtyLog>>>,
}

fn open(cx: &mut TestAppContext, count: usize, columns: usize) -> Opened {
    let ptys: Vec<_> = (0..count).map(|_| Rc::new(RefCell::new(PtyLog::default()))).collect();
    let (window, split) = {
        let ptys = ptys.clone();
        cx.update(move |cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(1200.), px(800.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, move |window, cx| {
                let panes = ptys
                    .iter()
                    .map(|pty| {
                        let terminal = Terminal::new(80, 24, 100).expect("the terminal starts");
                        let pty = RecordingPty(pty.clone());
                        cx.new(|cx| TerminalPane::new(terminal, pty, Theme::default(), window, cx))
                    })
                    .collect();
                cx.new(|_| Split { panes, columns })
            })
            .expect("the test window opens")
        })
    };
    cx.update_window(window, |_, window, _| window.activate_window())
        .expect("the test window is open");
    cx.run_until_parked();
    let opened = Opened { window, split, ptys };
    opened.frame(cx, |_, _| {});
    opened.frame(cx, |_, _| {});
    opened.sent();
    opened
}

impl Opened {
    fn frame<R>(&self, cx: &mut TestAppContext, step: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let result = cx
            .update_window(self.window, |_, window, cx| {
                let result = step(window, cx);
                window.render_frame(cx);
                result
            })
            .expect("the test window is open");
        cx.run_until_parked();
        result
    }

    fn panes(&self, cx: &mut TestAppContext) -> Vec<Entity<TerminalPane>> {
        self.split.read_with(cx, |split, _| split.panes.clone())
    }

    /// What each program was sent since the last call.
    fn sent(&self) -> Vec<Vec<u8>> {
        self.ptys.iter().map(|pty| pty.borrow_mut().take()).collect()
    }

    /// The size each program was last told.
    fn sizes(&self) -> Vec<Option<Size>> {
        self.ptys.iter().map(|pty| pty.borrow().sizes.last().copied()).collect()
    }

    fn grid_sizes(&self, cx: &mut TestAppContext) -> Vec<Size> {
        self.panes(cx)
            .iter()
            .map(|pane| pane.read_with(cx, |pane, cx| pane.view().read(cx).grid().size()))
            .collect()
    }
}

/// Clicks each pane in turn and types its number into it.
fn type_into_each(opened: &Opened, cx: &mut TestAppContext) {
    for n in 0..opened.ptys.len() {
        opened.frame(cx, |window, cx| {
            window.click(format!("pane-{n}"), cx);
            window.press(&n.to_string(), cx);
        });
    }
}

#[gpui_kit::test]
fn two_terminals_side_by_side_each_get_their_own_size_and_input(cx: &mut TestAppContext) {
    let opened = open(cx, 2, 2);
    let sizes = opened.grid_sizes(cx);
    assert_eq!(sizes[0], sizes[1]);
    assert_eq!(opened.sizes(), vec![Some(sizes[0]), Some(sizes[1])]);
    // Each fills half of the 1200 × 800 window, to the whole cell.
    let panes = opened.panes(cx);
    let cell = panes[0]
        .read_with(cx, |pane, cx| pane.view().read(cx).cell_size())
        .expect("a frame was drawn");
    let width = f32::from(sizes[0].cols) * cell.width;
    let height = f32::from(sizes[0].rows) * cell.height;
    assert!(width <= 600. && width > 600. - cell.width, "{sizes:?} at {cell:?}");
    assert!(height <= 800. && height > 800. - cell.height, "{sizes:?} at {cell:?}");

    type_into_each(&opened, cx);
    assert_eq!(opened.sent(), vec![b"0".to_vec(), b"1".to_vec()]);
}

#[gpui_kit::test]
fn four_terminals_take_input_one_at_a_time_and_draw_only_what_changed(cx: &mut TestAppContext) {
    let opened = open(cx, 4, 2);
    let sizes = opened.grid_sizes(cx);
    assert!(sizes.iter().all(|size| *size == sizes[0]), "{sizes:?}");
    assert_eq!(opened.sizes(), sizes.iter().copied().map(Some).collect::<Vec<_>>());

    type_into_each(&opened, cx);
    assert_eq!(
        opened.sent(),
        vec![b"0".to_vec(), b"1".to_vec(), b"2".to_vec(), b"3".to_vec()]
    );

    // Output streams into one; the other three shape no rows.
    let panes = opened.panes(cx);
    let rows = |cx: &mut TestAppContext| -> Vec<u64> {
        panes
            .iter()
            .map(|pane| pane.read_with(cx, |pane, cx| pane.view().read(cx).stats().rows_rebuilt_total))
            .collect()
    };
    let before = rows(cx);
    for chunk in Script::agent_stream(40).split_every(200).chunks() {
        panes[2].update(cx, |pane, cx| pane.feed(chunk, cx));
        opened.frame(cx, |_, _| {});
    }
    let after = rows(cx);
    assert!(after[2] > before[2], "{before:?} then {after:?}");
    for n in [0, 1, 3] {
        assert_eq!(after[n], before[n], "pane {n} shaped rows: {before:?} then {after:?}");
    }

    // Hiding one keeps the others as they were.
    panes[1].update(cx, |pane, cx| pane.set_visible(false, cx));
    opened.frame(cx, |window, cx| {
        window.click("pane-3", cx);
        window.press("x", cx);
    });
    assert_eq!(opened.sent(), vec![vec![], vec![], vec![], b"x".to_vec()]);
}
