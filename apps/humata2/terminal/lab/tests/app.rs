use ghostty_vt::Grid;
use gpui_kit::{
    AnyWindowHandle, Bounds, Entity, Focusable, HeadlessAppContext, SharedString, WindowBounds,
    WindowOptions, point, prelude::*, px, size, test::TestWindowExt,
};
use humata2_engine::Resources;
use std::time::Duration;
use terminal_lab::{Assets, Lab};
use terminal_session::{Endpoint, Record, SessionSpec, SessionStore, Update};

/// Stops every process whose command line names this test's own folder (its
/// holders, under `/private/tmp`), even when the test fails partway.
struct OwnHolders(String);
impl OwnHolders {
    fn of(dir: &std::path::Path) -> Self {
        let dir = std::fs::canonicalize(dir).unwrap();
        assert!(dir.starts_with("/private/tmp/"), "{} is not under /private/tmp", dir.display());
        Self(format!("{}/", dir.display()))
    }
}
impl Drop for OwnHolders {
    fn drop(&mut self) {
        let _ = std::process::Command::new("pkill").arg("-f").arg(&self.0).status();
    }
}

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
/// Waits as [`wait`] does until `done` holds. A real shell starts and
/// answers slower while other tests run, so this fails only after 10 s.
fn wait_until(
    runtime: &tokio::runtime::Runtime,
    cx: &mut HeadlessAppContext,
    lab: &Entity<Lab>,
    window: AnyWindowHandle,
    what: &str,
    mut done: impl FnMut(&mut HeadlessAppContext) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    wait(runtime, cx, lab, window);
    while !done(cx) {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        wait(runtime, cx, lab, window);
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
async fn marker(r: &mut terminal_session::Reader, text: &str) {
    tokio::time::timeout(Duration::from_secs(6), async {
        let mut out = Vec::new();
        loop {
            if let Some(Update::Record(Record::Output { bytes, .. })) = r.next().await.unwrap() {
                out.extend(bytes);
            }
            if String::from_utf8_lossy(&out).contains(text) {
                break;
            }
        }
    })
    .await
    .unwrap();
}
#[test]
fn real_shells_render_move_paste_detach_resume_and_resize_headlessly() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::Builder::new()
        .prefix("tui-")
        .tempdir_in("/tmp")
        .unwrap();
    let _holders = OwnHolders::of(dir.path());
    let holder = std::fs::canonicalize(std::env::var("HUMATA2_HOLDER").unwrap()).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut store = SessionStore::open(dir.path(), holder).unwrap();
    let spec = SessionSpec {
        title: if std::env::var_os("TERMINAL_LAB_SSH_CONFIG").is_some() {
            "SSH shell".into()
        } else {
            "Local shell".into()
        },
        program: "/bin/sh".into(),
        args: vec!["-i".into()],
        cwd: "/tmp".into(),
        cols: 80,
        rows: 24,
        endpoint: std::env::var("TERMINAL_LAB_SSH_CONFIG")
            .map(|config| Endpoint::Ssh {
                config: config.into(),
                destination: "lima-h2-terminal-c1".into(),
                holder: "/tmp/h2-terminal-holder".into(),
                root: format!("/var/tmp/terminal-ui-{}", std::process::id()),
            })
            .unwrap_or(Endpoint::Local),
    };
    let remote = !matches!(spec.endpoint, Endpoint::Local);
    let a = store.create(spec.clone()).unwrap();
    let b = store.create(spec).unwrap();
    runtime.block_on(async {
        for id in [a,b] {
            let mut terminal=store.attach(id).await.unwrap();
            // Set up small shell helpers with echo off and wait until the shell
            // has run them, so the real commands below are never echoed early
            // however slowly the shell starts; the last one turns echo back on.
            let setup=format!(r#"stty -echo; PS1='$ '; export LAB_STILL=original; colors() {{ printf '\033[31mred  \033[32mgreen  \033[33myellow\033[0m\n\033[34mblue \033[35mmagenta \033[36mcyan\033[0m\n\033[38;2;111;194;255m24-bit color\033[0m\n'; }}; docs() {{ printf '\033]8;;https://ghostty.org\033\\Terminal documentation\033]8;;\033\\\nhttps://example.com\n\033]2;READY_{id}\007'; stty echo; }}; printf '\033c\033]2;SETUP_{id}\007'
"#);
            terminal.writer.input(setup.as_bytes()).await.unwrap();
            marker(&mut terminal.reader,&format!("\x1b]2;SETUP_{id}\x07")).await;
            terminal.writer.input(b"pwd\runame -sm\rcolors\rdocs\r").await.unwrap();
            marker(&mut terminal.reader,&format!("\x1b]2;READY_{id}\x07")).await;
        }
    });
    store.move_to(b, 1, 0).unwrap();
    let mut cx = humata2_engine::headless(&Resources::new(Assets)).unwrap();
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(1200.), px(760.)),
        ))),
        show: false,
        focus: false,
        ..Default::default()
    };
    let ssh = remote.then(|| store.sessions()[0].spec.endpoint.clone());
    let (window, lab) = cx
        .update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Lab::new(store, runtime.handle().clone(), ssh, window, cx))
            })
        })
        .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    for id in [a, b] {
        assert!(screen(&mut cx, &lab, id).contains("Terminal documentation"));
    }
    cx.update_window(window, |_, w, cx| {
        w.activate_window();
        for id in [a, b] {
            let t = lab.read(cx).terminal(id).unwrap();
            let pane = t.read(cx).pane().clone();
            pane.update(cx, |p, cx| {
                let view = p.view().clone();
                view.update(cx, |v, cx| v.set_cursor_blink(false, cx));
            });
        }
        w.render_frame(cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    for id in [a, b] {
        let text = screen(&mut cx, &lab, id);
        assert!(text.contains("24-bit color"));
        assert!(text.contains("https://example.com"));
    }
    let snapshots = humata2_snapshot::snapshots!().unwrap();
    let areas = cx
        .update_window(window, |_, window, _| {
            ["group-0", "group-1"].map(|group| window.within(group).find("terminal").bounds())
        })
        .unwrap();
    let row_height = cx.update(|cx| {
        let terminal = lab.read(cx).terminal(a).unwrap();
        terminal
            .read(cx)
            .pane()
            .read(cx)
            .view()
            .read(cx)
            .cell_size()
            .unwrap()
            .height
    });
    // Idle frames must keep painting cached terminal rows, including GPU captures.
    let mut image = cx.capture_screenshot(window).unwrap();
    for _ in 0..8 {
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        image = cx.capture_screenshot(window).unwrap();
        for id in [a, b] {
            let stats = cx.update(|cx| {
                let terminal = lab.read(cx).terminal(id).unwrap();
                terminal.read(cx).pane().read(cx).view().read(cx).stats()
            });
            assert_eq!(stats.paint_failures, 0);
        }
        let scale = image.width() as f32 / 1200.;
        for (half, bounds) in areas.iter().enumerate() {
            for row in 0..4 {
                let left = f32::from(bounds.origin.x) * scale;
                let top = (f32::from(bounds.origin.y) + row as f32 * row_height) * scale;
                let painted = image
                    .enumerate_pixels()
                    .filter(|(x, y, p)| {
                        *x as f32 >= left
                            && (*x as f32) < left + f32::from(bounds.size.width) * scale
                            && *y as f32 >= top
                            && (*y as f32) < top + row_height * scale
                            && u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]) > 400
                    })
                    .count();
                assert!(
                    painted > 50,
                    "terminal {half} row {row} must paint its output after idle frames ({painted} pixels)"
                );
            }
        }
    }
    let name = if remote {
        "two-ssh-terminals"
    } else {
        "two-real-terminals"
    };
    let backend = if gpui_kit::GPUI_FAST { "-fast" } else { "" };
    let tolerance = if std::env::var("HUMATA2_SNAPSHOT").as_deref() == Ok("record") {
        humata2_snapshot::Tolerance::EXACT
    } else {
        humata2_snapshot::Tolerance::default()
    };
    snapshots.assert_image(&format!("{name}{backend}"), &image, tolerance);
    cx.update_window(window, |_, window, cx| window.click("appearance", cx))
        .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    let foreground = cx.update(|cx| {
        let terminal = lab.read(cx).terminal(a).unwrap();
        terminal
            .read(cx)
            .pane()
            .read(cx)
            .view()
            .read(cx)
            .grid()
            .colors()
            .foreground
    });
    assert_eq!(foreground, ghostty_vt::Rgb::hex(0x1d1d1f));
    let light = cx.capture_screenshot(window).unwrap();
    snapshots.assert_image(&format!("{name}-light{backend}"), &light, tolerance);
    cx.update_window(window, |_, window, cx| window.click("appearance", cx))
        .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    // Drag across groups and reorder tabs without replacing the embedded entity.
    let moved_entity = cx.update(|cx| lab.read(cx).terminal(a).unwrap().entity_id());
    cx.update_window(window, |_, w, cx| {
        let from = w
            .find(SharedString::from(format!("tab-{a}")))
            .bounds()
            .center();
        let to = w.find("group-1").bounds().center();
        w.drag(from, to, cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    assert_eq!(
        cx.update(|cx| lab.read(cx).group(1).unwrap().to_vec()),
        vec![b, a]
    );
    cx.update_window(window, |_, w, cx| {
        let from = w
            .find(SharedString::from(format!("tab-{a}")))
            .bounds()
            .center();
        let to = w
            .find(SharedString::from(format!("tab-{b}")))
            .bounds()
            .center();
        w.drag(from, to, cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    assert_eq!(
        cx.update(|cx| lab.read(cx).group(1).unwrap().to_vec()),
        vec![a, b]
    );
    assert_eq!(
        cx.update(|cx| lab.read(cx).terminal(a).unwrap().entity_id()),
        moved_entity
    );
    cx.update(|cx| lab.update(cx, |l, cx| l.move_terminal(a, 0, 0, cx)));
    wait(&runtime, &mut cx, &lab, window);
    // Turn echo on so a focus leak would be observable, not hidden by stty -echo.
    cx.update(|cx| {
        lab.read(cx)
            .terminal(a)
            .unwrap()
            .update(cx, |t, cx| t.send_text("stty echo".into(), cx))
    });
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(a).unwrap();
        w.focus(&t.read(cx).focus_handle(cx), cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    // A focused nonterminal field must not type into either PTY.
    let before = screen(&mut cx, &lab, a);
    cx.update_window(window, |_, w, cx| {
        w.click("send-text-input", cx);
        w.input("printf 'PASTED_%s\\n' OK", cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    assert_eq!(screen(&mut cx, &lab, a), before);
    cx.update_window(window, |_, w, cx| {
        w.click("send-text", cx);
        let t = lab.read(cx).terminal(a).unwrap();
        assert!(t.read(cx).focus_handle(cx).is_focused(w));
        w.press("enter", cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    assert!(screen(&mut cx, &lab, a).contains("PASTED_OK"));
    assert!(!screen(&mut cx, &lab, b).contains("PASTED_OK"));
    // Moving the existing entity must retain its grid and process, and the layout goes to disk.
    let entity = cx.update(|cx| lab.read(cx).terminal(a).unwrap().entity_id());
    cx.update_window(window, |_, w, cx| {
        w.click(SharedString::from(format!("move-{a}")), cx)
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    assert_eq!(
        cx.update(|cx| lab.read(cx).terminal(a).unwrap().entity_id()),
        entity
    );
    // A real program's alternate screen and modes must survive renderer reconstruction.
    cx.update(|cx| {
        lab.read(cx).terminal(a).unwrap().update(cx, |t, cx| {
            t.send_text("printf '\\033[?1049h\\033[2J\\033[HALT_SCREEN'".into(), cx)
        })
    });
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(a).unwrap();
        w.focus(&t.read(cx).focus_handle(cx), cx);
        w.press("enter", cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    assert!(screen(&mut cx, &lab, a).contains("ALT_SCREEN"));
    assert!(cx.update(|cx| {
        let t = lab.read(cx).terminal(a).unwrap();
        t.read(cx)
            .pane()
            .read(cx)
            .view()
            .read(cx)
            .grid()
            .is_alternate_screen()
    }));
    // Select it in the destination, detach, and attach to the same environment.
    cx.update_window(window, |_, w, cx| {
        w.click(SharedString::from(format!("tab-{a}")), cx);
        w.click(SharedString::from(format!("detach-{a}")), cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    cx.update_window(window, |_, w, cx| {
        w.click(SharedString::from(format!("resume-{a}")), cx)
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    assert!(screen(&mut cx, &lab, a).contains("ALT_SCREEN"));
    assert!(cx.update(|cx| {
        let t = lab.read(cx).terminal(a).unwrap();
        t.read(cx)
            .pane()
            .read(cx)
            .view()
            .read(cx)
            .grid()
            .is_alternate_screen()
    }));
    cx.update(|cx| {
        let t = lab.read(cx).terminal(a).unwrap();
        t.update(cx, |t, cx| {
            t.send_text("printf 'SAME_PROCESS=%s\\n' \"$LAB_STILL\"".into(), cx)
        });
    });
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(a).unwrap();
        w.focus(&t.read(cx).focus_handle(cx), cx);
        w.press("enter", cx);
        w.resize(size(px(700.), px(600.)));
        w.bounds_changed(cx);
    })
    .unwrap();
    wait(&runtime, &mut cx, &lab, window);
    assert!(screen(&mut cx, &lab, a).contains("SAME_PROCESS=original"));
    cx.update_window(window, |_, w, cx| {
        w.resize(size(px(1200.), px(760.)));
        w.bounds_changed(cx);
    })
    .unwrap();
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
    cx.update(|cx| {
        for id in [a, b] {
            lab.read(cx)
                .terminal(id)
                .unwrap()
                .update(cx, |t, _| t.stop());
        }
    });
    wait(&runtime, &mut cx, &lab, window);
    // The host's creation button starts another real shell, locally or remotely.
    cx.update_window(window, |_, window, cx| {
        window.click(if remote { "new-ssh" } else { "new-local" }, cx)
    })
    .unwrap();
    wait_until(&runtime, &mut cx, &lab, window, "the new shell's terminal", |cx| {
        cx.update(|cx| lab.read(cx).terminal(3).is_some())
    });
    cx.update(|cx| {
        lab.read(cx)
            .terminal(3)
            .unwrap()
            .update(cx, |terminal, cx| {
                terminal.send_text("printf 'CREATED_%s\\n' OK".into(), cx)
            })
    });
    cx.update_window(window, |_, window, cx| {
        let terminal = lab.read(cx).terminal(3).unwrap();
        window.focus(&terminal.read(cx).focus_handle(cx), cx);
        window.press("enter", cx);
    })
    .unwrap();
    wait_until(&runtime, &mut cx, &lab, window, "the new shell's output", |cx| {
        screen(cx, &lab, 3).contains("CREATED_OK")
    });
    cx.update(|cx| {
        lab.read(cx)
            .terminal(3)
            .unwrap()
            .update(cx, |terminal, _| terminal.stop())
    });
    wait(&runtime, &mut cx, &lab, window);
}
