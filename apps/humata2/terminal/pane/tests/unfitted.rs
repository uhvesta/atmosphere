//! Output that reaches a pane before its first draw is laid out at the
//! size the pane is drawn at, not the size the terminal was made with:
//! a pane attached to an offline or exited session is never redrawn by
//! its program, so what it was sent has to land at the real size.

mod support;

use std::cell::RefCell;
use std::rc::Rc;

// `#[gpui_kit::test]` expands to paths under `gpui`.
use ghostty_vt::{CellWidth, Grid as _, Terminal};
use gpui_kit::gpui;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size};
use support::{PtyLog, RecordingPty};
use terminal_pane::TerminalPane;
use terminal_view::Theme;

#[gpui_kit::test]
fn output_sent_before_the_first_draw_lands_at_the_drawn_size(cx: &mut TestAppContext) {
    let pty = Rc::new(RefCell::new(PtyLog::default()));
    let (window, pane) = cx.update(|cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(720.), px(420.)),
            })),
            ..Default::default()
        };
        let pty = RecordingPty(pty.clone());
        gpui_kit::open_window(options, cx, move |window, cx| {
            // Far narrower than the window: ten columns.
            let terminal = Terminal::new(10, 5, 100).expect("the terminal starts");
            cx.new(|cx| TerminalPane::new(terminal, pty, Theme::default(), window, cx))
        })
        .expect("the test window opens")
    });
    // A program drawing at column 30, before the pane has been drawn.
    pane.update(cx, |pane, cx| pane.feed(b"\x1b[1;30Hmark", cx));
    for _ in 0..3 {
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .expect("the test window is open");
        cx.run_until_parked();
    }

    let first_row = pane.update(cx, |pane, cx| {
        let terminal = pane.view().read(cx).grid();
        assert!(
            terminal.size().cols > 40,
            "the pane was fitted to the window"
        );
        terminal
            .row(0)
            .map(|row| {
                row.cells
                    .iter()
                    .filter(|cell| {
                        !matches!(cell.width, CellWidth::SpacerTail | CellWidth::SpacerHead)
                    })
                    .map(|cell| {
                        if cell.text.is_empty() {
                            " "
                        } else {
                            cell.text.as_str()
                        }
                    })
                    .collect::<String>()
            })
            .unwrap_or_default()
            .trim_end()
            .to_owned()
    });
    assert_eq!(first_row, format!("{}mark", " ".repeat(29)));
}
