//! Ordinary remote output -> isolated frontend clipboard -> exact remote PTY bytes.
//! This uses GPUI's headless clipboard, never the owner's macOS clipboard.
use ghostty_vt::Grid;
use gpui_kit::{
    AnyWindowHandle, Bounds, ClipboardItem, Entity, HeadlessAppContext, WindowBounds,
    WindowOptions, point, prelude::*, px, size, test::TestWindowExt,
};
use humata2_engine::Resources;
use std::{
    os::unix::fs::PermissionsExt,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    time::{Duration, Instant},
};
use terminal_lab::{Assets, Lab};
use terminal_session::{Endpoint, SessionSpec, SessionStore};

const TEXT: &str = "REMOTE_α界é\nSECOND_e\u{301}_終_END";
const REMOTE: &str = r#"
import os, select, termios, tty
tty.setraw(0)
text = 'REMOTE_α界é\nSECOND_e\u0301_終_END'
os.write(1, ('\x1b[2J\x1b[H\x1b[?2004h\x1b[38;2;235;76;88m' + text.replace('\n', '\r\n') + '\x1b[0m\r\nREMOTE_READY\r\n').encode())
received = b''
while not received.endswith(b'\x1b[201~') and len(received) < 4096:
    if not select.select([0], [], [], 30)[0]:
        os.write(1, b'\r\nREMOTE_TIMEOUT\r\n')
        raise SystemExit(1)
    received += os.read(0, 4096)
expected = b'\x1b[200~' + text.encode() + b'\x1b[201~'
os.write(1, b'\r\nREMOTE_EXACT_UTF8_PASS\r\n' if received == expected else b'\r\nREMOTE_BYTES_FAIL\r\n')
while select.select([0], [], [], 30)[0]:
    if not os.read(0, 4096):
        break
"#;

fn pump(
    runtime: &tokio::runtime::Runtime,
    cx: &mut HeadlessAppContext,
    lab: &Entity<Lab>,
    window: AnyWindowHandle,
) {
    runtime.block_on(async {
        tokio::time::sleep(Duration::from_millis(10)).await;
    });
    cx.update(|cx| lab.update(cx, |lab, cx| lab.poll(cx)));
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .unwrap();
}

fn visible(cx: &mut HeadlessAppContext, lab: &Entity<Lab>, id: u64) -> String {
    cx.update(|cx| {
        let Some(terminal) = lab.read(cx).terminal(id) else {
            return String::new();
        };
        let grid = terminal.read(cx).pane().read(cx).view().read(cx).grid();
        (0..grid.size().rows)
            .filter_map(|y| grid.row(y))
            .map(|row| {
                row.cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}

fn until(
    runtime: &tokio::runtime::Runtime,
    cx: &mut HeadlessAppContext,
    lab: &Entity<Lab>,
    window: AnyWindowHandle,
    id: u64,
    marker: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        pump(runtime, cx, lab, window);
        if visible(cx, lab, id).contains(marker) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "remote clipboard fixture did not reach {marker}"
        );
    }
}

#[test]
fn ssh_unicode_multiline_color_copy_pasteback_and_focus_are_exact() {
    let config = std::env::var_os("TERMINAL_LAB_SSH_CONFIG")
        .expect("Set TERMINAL_LAB_SSH_CONFIG to the dedicated h2-terminal-c1 Lima SSH config");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::Builder::new()
        .prefix("h2-clipboard-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let mut store = SessionStore::open(dir.path(), holder).unwrap();
    let endpoint = Endpoint::Ssh {
        config: config.into(),
        destination: "lima-h2-terminal-c1".into(),
        holder: "/tmp/h2-terminal-holder".into(),
        root: format!(
            "/var/tmp/{}",
            dir.path().file_name().unwrap().to_str().unwrap()
        ),
    };
    let id = store
        .create(SessionSpec {
            title: "Clipboard fixture".into(),
            program: "/usr/bin/python3".into(),
            args: vec!["-u".into(), "-c".into(), REMOTE.into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            endpoint: endpoint.clone(),
        })
        .unwrap();
    let mut cx = humata2_engine::headless(&Resources::new(Assets)).unwrap();
    let (window, lab) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1100.), px(720.)),
                    ))),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Lab::new(store, runtime.handle().clone(), Some(endpoint), window, cx)
                    })
                },
            )
        })
        .unwrap();
    let result = catch_unwind(AssertUnwindSafe(|| {
        until(&runtime, &mut cx, &lab, window, id, "REMOTE_READY");
        // A clipboard shortcut in an ordinary application input must stay there.
        cx.update_window(window, |_, window, cx| {
            window.activate_window();
            window.click("send-text-input", cx);
            cx.write_to_clipboard(ClipboardItem::new_string(
                "UNFOCUSED_MUST_STAY_LOCAL".into(),
            ));
            window.press("cmd-v", cx);
        })
        .unwrap();
        pump(&runtime, &mut cx, &lab, window);
        let (start, end, cell) = cx.update(|cx| {
            let terminal = lab.read(cx).terminal(id).unwrap();
            let view = terminal.read(cx).pane().read(cx).view().read(cx);
            let grid = view.grid();
            let row = |expected: &str| {
                (0..grid.size().rows)
                    .find(|y| {
                        grid.row(*y).is_some_and(|row| {
                            row.cells
                                .iter()
                                .map(|cell| cell.text.as_str())
                                .collect::<String>()
                                .trim_end()
                                == expected
                        })
                    })
                    .unwrap()
            };
            let first = row("REMOTE_α界é");
            let last = row("SECOND_e\u{301}_終_END");
            let last_x = grid
                .row(last)
                .unwrap()
                .cells
                .iter()
                .rposition(|cell| !cell.text.trim().is_empty())
                .unwrap();
            ((0usize, first), (last_x, last), view.cell_size().unwrap())
        });
        cx.update_window(window, |_, window, cx| {
            let bounds = window.within("group-0").find("terminal").bounds();
            let at = |(x, y): (usize, u16)| {
                point(
                    bounds.origin.x + px((x as f32 + 0.5) * cell.width),
                    bounds.origin.y + px((f32::from(y) + 0.5) * cell.height),
                )
            };
            window.drag(at(start), at(end), cx);
            window.press("cmd-c", cx);
        })
        .unwrap();
        assert_eq!(
            cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
                .as_deref(),
            Some(TEXT),
            "copy must preserve hard newlines/Unicode and strip ANSI colors"
        );
        // Paste the copied value itself, without replacing it with the expectation.
        cx.update_window(window, |_, window, cx| window.press("cmd-v", cx))
            .unwrap();
        until(
            &runtime,
            &mut cx,
            &lab,
            window,
            id,
            "REMOTE_EXACT_UTF8_PASS",
        );
        assert!(!visible(&mut cx, &lab, id).contains("REMOTE_BYTES_FAIL"));
    }));
    // Stop only this fixture even when an assertion fails; no persistent user's shell is touched.
    cx.update(|cx| {
        if let Some(terminal) = lab.read(cx).terminal(id) {
            terminal.update(cx, |terminal, _| terminal.stop());
        }
    });
    for _ in 0..30 {
        pump(&runtime, &mut cx, &lab, window);
    }
    if let Err(error) = result {
        resume_unwind(error);
    }
}
