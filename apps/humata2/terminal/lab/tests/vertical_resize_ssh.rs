//! Real SSH row grow/shrink/restore, independent of historical app screenshot goldens.
use ghostty_vt::Grid;
use gpui_kit::{
    AnyWindowHandle, Bounds, Entity, Focusable, HeadlessAppContext, WindowBounds, WindowOptions,
    point, prelude::*, px, size, test::TestWindowExt,
};
use humata2_engine::Resources;
use std::{
    os::unix::fs::PermissionsExt,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    time::Duration,
};
use terminal_lab::{Assets, Lab};
use terminal_session::{Endpoint, SessionSpec, SessionStore};
fn wait(
    runtime: &tokio::runtime::Runtime,
    cx: &mut HeadlessAppContext,
    lab: &Entity<Lab>,
    window: AnyWindowHandle,
) {
    for _ in 0..30 {
        runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
        });
        cx.update(|cx| lab.update(cx, |l, cx| l.poll(cx)));
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            w.render_frame(cx);
        })
        .unwrap();
    }
}
fn screen(cx: &mut HeadlessAppContext, lab: &Entity<Lab>, id: u64) -> String {
    cx.update(|cx| {
        let t = lab.read(cx).terminal(id).unwrap();
        let pane = t.read(cx).pane();
        let grid = pane.read(cx).view().read(cx).grid();
        (0..grid.size().rows)
            .filter_map(|y| grid.row(y))
            .map(|row| {
                row.cells
                    .iter()
                    .map(|c| c.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
}
#[test]
fn ssh_vertical_resize_changes_real_pty_rows_without_restarting_shell() {
    let config = std::env::var_os("TERMINAL_LAB_SSH_CONFIG")
        .expect("Set TERMINAL_LAB_SSH_CONFIG to the dedicated test VM SSH config");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::Builder::new()
        .prefix("h2-vertical-")
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
    let a = store
        .create(SessionSpec {
            title: "Vertical SSH fixture".into(),
            program: "/bin/sh".into(),
            args: vec!["-i".into()],
            cwd: "/tmp".into(),
            cols: 80,
            rows: 24,
            endpoint: endpoint.clone(),
        })
        .unwrap();
    runtime.block_on(async {
        let mut attached=store.attach(a).await.unwrap();
        attached.writer.input(b"PS1='$ '; export LAB_STILL=original; printf '\\033[?1049h\\033[2J\\033[H\\033]2;VERTICAL_READY\\007'\r").await.unwrap();
        tokio::time::timeout(Duration::from_secs(8),async {
            let mut output=Vec::new();
            loop {
                if let Some(terminal_session::Update::Record(terminal_session::Record::Output{bytes,..}))=attached.reader.next().await.unwrap(){output.extend(bytes);}
                if output.windows(b"\x1b]2;VERTICAL_READY\x07".len()).any(|bytes|bytes==b"\x1b]2;VERTICAL_READY\x07"){break;}
            }
        }).await.unwrap();
    });
    let mut cx = humata2_engine::headless(&Resources::new(Assets)).unwrap();
    let (window, lab) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1200.), px(760.)),
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
        wait(&runtime, &mut cx, &lab, window);
        // Keep width fixed: each row change must reach the real local/SSH PTY,
        // rather than only stretching or clipping a renderer's cached screen.
        let mut shell_pid = None;
        let mut sizes = Vec::new();
        for (phase, height) in [760., 420., 960., 760.].into_iter().enumerate() {
            cx.update_window(window, |_, w, cx| {
                w.resize(size(px(1200.), px(height)));
                w.bounds_changed(cx);
            })
            .unwrap();
            wait(&runtime, &mut cx, &lab, window);
            let expected = cx.update(|cx| {
                let terminal = lab.read(cx).terminal(a).unwrap();
                terminal
                    .read(cx)
                    .pane()
                    .read(cx)
                    .view()
                    .read(cx)
                    .grid()
                    .size()
            });
            let bottom = format!("V_BOTTOM_{phase}");
            let command = format!(
                r#"set -- $(stty size); printf '\033[2J\033[HSIZE_{phase}=%s,%s,%s,%s\n' "$1" "$2" "$$" "$LAB_STILL"; printf '\033[%s;1H{bottom}' "$1""#
            );
            cx.update(|cx| {
                lab.read(cx)
                    .terminal(a)
                    .unwrap()
                    .update(cx, |terminal, cx| {
                        terminal.send_text(command, cx);
                    });
            });
            cx.update_window(window, |_, w, cx| {
                let terminal = lab.read(cx).terminal(a).unwrap();
                w.focus(&terminal.read(cx).focus_handle(cx), cx);
                w.press("enter", cx);
            })
            .unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            let output = loop {
                wait(&runtime, &mut cx, &lab, window);
                let output = screen(&mut cx, &lab, a);
                if output
                    .lines()
                    .last()
                    .is_some_and(|line| line.starts_with(&bottom))
                {
                    break output;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "vertical resize did not paint the PTY's last row: {output}"
                );
            };
            let status = output
                .lines()
                .find_map(|line| line.strip_prefix(&format!("SIZE_{phase}=")))
                .unwrap();
            let fields: Vec<_> = status.trim().split(',').collect();
            assert_eq!(fields.len(), 4);
            assert_eq!(fields[0].parse::<u16>().unwrap(), expected.rows);
            assert_eq!(fields[1].parse::<u16>().unwrap(), expected.cols);
            assert_eq!(
                fields[3], "original",
                "resizing must retain the shell environment"
            );
            if let Some(pid) = &shell_pid {
                assert_eq!(fields[2], pid, "resizing must retain the shell process");
            } else {
                shell_pid = Some(fields[2].to_owned());
            }
            cx.update(|cx| {
                let terminal = lab.read(cx).terminal(a).unwrap();
                let grid = terminal.read(cx).pane().read(cx).view().read(cx).grid();
                assert_eq!(grid.size(), expected);
                assert_eq!(grid.cursor().y, expected.rows - 1);
                assert!(
                    grid.is_alternate_screen(),
                    "vertical resize must retain the active TUI screen"
                );
            });
            sizes.push(expected);
        }
        assert!(sizes[1].rows < sizes[0].rows);
        assert!(sizes[2].rows > sizes[0].rows);
        assert_eq!(
            sizes[3], sizes[0],
            "restoring the window restores both PTY dimensions"
        );
        assert!(sizes.iter().all(|size| size.cols == sizes[0].cols));
    }));
    // Stop only this fixture, including after a failed assertion.
    cx.update(|cx| {
        if let Some(terminal) = lab.read(cx).terminal(a) {
            terminal.update(cx, |terminal, _| terminal.stop());
        }
    });
    wait(&runtime, &mut cx, &lab, window);
    if let Err(error) = result {
        resume_unwind(error);
    }
}
