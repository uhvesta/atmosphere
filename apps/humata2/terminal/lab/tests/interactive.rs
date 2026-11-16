//! Manual acceptance with the owner's real zsh config, or a disposable SSH fixture.
use ghostty_vt::Grid;
use gpui_kit::{
    AnyWindowHandle, Bounds, ClipboardItem, Entity, Focusable, HeadlessAppContext, WindowBounds,
    WindowOptions, point, prelude::*, px, size, test::TestWindowExt,
};
use humata2_engine::Resources;
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};
use terminal_lab::{Assets, Lab};
use terminal_session::{Endpoint, SessionSpec, SessionStore};
fn pump(
    runtime: &tokio::runtime::Runtime,
    cx: &mut HeadlessAppContext,
    lab: &Entity<Lab>,
    window: AnyWindowHandle,
) {
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
fn text(cx: &mut HeadlessAppContext, lab: &Entity<Lab>, id: u64) -> String {
    cx.update(|cx| {
        let t = lab.read(cx).terminal(id).unwrap();
        let g = t.read(cx).pane().read(cx).view().read(cx).grid();
        (0..g.size().rows)
            .filter_map(|y| g.row(y))
            .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect::<String>())
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
    needle: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        pump(runtime, cx, lab, window);
        if cx.update(|cx| lab.read(cx).terminal(id).is_some()) && text(cx, lab, id).contains(needle)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "waiting for {needle}: {}",
            if cx.update(|cx| lab.read(cx).terminal(id).is_some()) {
                text(cx, lab, id)
            } else {
                "connecting".into()
            }
        );
    }
}
fn command(
    cx: &mut HeadlessAppContext,
    lab: &Entity<Lab>,
    window: AnyWindowHandle,
    id: u64,
    command: &str,
) {
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(id).unwrap();
        w.focus(&t.read(cx).focus_handle(cx), cx);
        w.input(command, cx);
        w.press("enter", cx);
    })
    .unwrap();
}
fn wait_cursor(
    runtime: &tokio::runtime::Runtime,
    cx: &mut HeadlessAppContext,
    lab: &Entity<Lab>,
    window: AnyWindowHandle,
    needle: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(25);
    loop {
        pump(runtime, cx, lab, window);
        if cursor_text(cx, lab, 2).contains(needle) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "cursor did not reach {needle}: {}",
            text(cx, lab, 2)
        );
    }
}
fn cursor_text(cx: &mut HeadlessAppContext, lab: &Entity<Lab>, id: u64) -> String {
    cx.update(|cx| {
        let t = lab.read(cx).terminal(id).unwrap();
        let g = t.read(cx).pane().read(cx).view().read(cx).grid();
        g.row(g.cursor().y)
            .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect::<String>())
            .unwrap_or_default()
    })
}
fn scrollbar(cx: &mut HeadlessAppContext, lab: &Entity<Lab>, id: u64) -> ghostty_vt::Scrollbar {
    cx.update(|cx| {
        lab.read(cx)
            .terminal(id)
            .unwrap()
            .read(cx)
            .pane()
            .read(cx)
            .view()
            .read(cx)
            .grid()
            .scrollbar()
            .unwrap()
    })
}
#[test]
fn actual_zsh_commands_clipboard_movement_and_twelve_thousand_history_lines() {
    let remote = std::env::var_os("TERMINAL_LAB_SSH_CONFIG").is_some();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::Builder::new()
        .prefix("tinteractive-")
        .tempdir_in("/tmp")
        .unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let mut store = SessionStore::open(&state, holder).unwrap();
    let endpoint = std::env::var("TERMINAL_LAB_SSH_CONFIG")
        .map(|config| Endpoint::Ssh {
            config: config.into(),
            destination: "lima-h2-terminal-c1".into(),
            holder: "/tmp/h2-terminal-holder".into(),
            root: format!("/var/tmp/interactive-{}", std::process::id()),
        })
        .unwrap_or(Endpoint::Local);
    let local_repo = PathBuf::from(
        std::env::var_os("TERMINAL_LAB_REPO")
            .expect("Set TERMINAL_LAB_REPO to the real checkout for this manual test"),
    );
    let cwd = if remote {
        PathBuf::from("/var/tmp/h2-terminal-acceptance")
    } else {
        local_repo.clone()
    };
    let file = dir.path().join("terminal.rs");
    std::fs::write(
        &file,
        "// Editing in an actual terminal\nfn main() {\n    println!(\"Hello from Humata\");\n}\n",
    )
    .unwrap();
    let nvim = if remote {
        PathBuf::from("/usr/bin/nvim")
    } else {
        PathBuf::from(std::env::var_os("TERMINAL_LAB_NVIM").expect("Set TERMINAL_LAB_NVIM"))
    };
    store
        .create(SessionSpec {
            title: if remote { "SSH · Neovim" } else { "Neovim" }.into(),
            program: nvim,
            args: vec![
                "--clean".into(),
                "-n".into(),
                "-i".into(),
                "NONE".into(),
                "--cmd".into(),
                "set termguicolors background=dark".into(),
                "-c".into(),
                "set number mouse=a laststatus=2".into(),
                if remote {
                    "terminal.rs".into()
                } else {
                    file.to_string_lossy().into_owned()
                },
            ],
            cwd: if remote {
                cwd.clone()
            } else {
                dir.path().into()
            },
            cols: 70,
            rows: 30,
            endpoint: endpoint.clone(),
        })
        .unwrap();
    let mut args = vec!["DISABLE_AUTO_UPDATE=true".into()];
    if remote {
        args.push("ZDOTDIR=/tmp/h2-terminal-zdot".into());
        args.push(format!("HISTFILE=/tmp/h2-terminal-zdot/history-{}",std::process::id()));
    } else {
        // `bazel test` intentionally supplies a temporary HOME. This manual
        // case explicitly opts into the owner's real shell home/config.
        let zsh_home=std::env::var("TERMINAL_LAB_ZSH_HOME").expect("Set TERMINAL_LAB_ZSH_HOME to the owner's home for the manual real-zsh acceptance test");
        args.push(format!("HOME={zsh_home}"));
        args.push(format!("HISTFILE={}/history",dir.path().display()));
        args.push(format!("ZDOTDIR={zsh_home}"));
        args.push(format!("ZSH_CACHE_DIR={}/cache", dir.path().display()));
        args.push(format!("ZSH_COMPDUMP={}/zcompdump", dir.path().display()));
    }
    args.extend(["/bin/zsh".into(), "-i".into()]);
    store
        .create(SessionSpec {
            title: if remote { "SSH · zsh" } else { "zsh" }.into(),
            program: "/usr/bin/env".into(),
            args,
            cwd,
            cols: 70,
            rows: 30,
            endpoint: endpoint.clone(),
        })
        .unwrap();
    store.move_to(2, 1, 0).unwrap();
    let mut cx = humata2_engine::headless(&Resources::new(Assets)).unwrap();
    let (window, lab) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1440.), px(900.)),
                    ))),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                cx,
                |w, cx| {
                    cx.new(|cx| {
                        Lab::new(
                            store,
                            runtime.handle().clone(),
                            remote.then_some(endpoint),
                            w,
                            cx,
                        )
                    })
                },
            )
        })
        .unwrap();
    cx.update_window(window, |_, w, _| w.activate_window())
        .unwrap();
    until(&runtime, &mut cx, &lab, window, 1, "println!");
    // Exercise real TUI mouse reporting and Up/Down in the editor, including SSH.
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(1).unwrap();
        let cell = t
            .read(cx)
            .pane()
            .read(cx)
            .view()
            .read(cx)
            .cell_size()
            .unwrap();
        let bounds = w.within("group-0").find("terminal").bounds();
        let at = point(
            bounds.origin.x + px(cell.width * 12.5),
            bounds.origin.y + px(cell.height * 1.5),
        );
        w.drag(at, at, cx);
    })
    .unwrap();
    for _ in 0..15 {
        pump(&runtime, &mut cx, &lab, window);
    }
    let cursor_y = |cx: &mut HeadlessAppContext| {
        cx.update(|cx| {
            let t = lab.read(cx).terminal(1).unwrap();
            t.read(cx).pane().read(cx).view().read(cx).grid().cursor().y
        })
    };
    assert_eq!(cursor_y(&mut cx), 1);
    cx.update_window(window, |_, w, cx| w.press("down", cx))
        .unwrap();
    for _ in 0..15 {
        pump(&runtime, &mut cx, &lab, window);
    }
    assert_eq!(cursor_y(&mut cx), 2);
    cx.update_window(window, |_, w, cx| w.press("up", cx))
        .unwrap();
    for _ in 0..15 {
        pump(&runtime, &mut cx, &lab, window);
    }
    assert_eq!(cursor_y(&mut cx), 1);

    for _ in 0..80 {
        pump(&runtime, &mut cx, &lab, window);
    }
    command(
        &mut cx,
        &lab,
        window,
        2,
        "printf '\\nTHEME=%s ZSH=%s\\n' \"$ZSH_THEME\" \"$ZSH_VERSION\"",
    );
    until(&runtime, &mut cx, &lab, window, 2, "THEME=af-magic");
    command(
        &mut cx,
        &lab,
        window,
        2,
        if remote {
            "ls; cd /var/tmp/h2-terminal-acceptance; git status --short; printf 'GIT_%s\\n' DONE"
        } else {
            "ls apps/humata2/terminal; cd apps/humata2; git status --short --untracked-files=no | head -n 5; printf 'GIT_%s\\n' DONE"
        },
    );
    until(&runtime, &mut cx, &lab, window, 2, "GIT_DONE");
    // Real paste shortcut, including the shell's bracketed-paste negotiation.
    cx.update_window(window, |_, w, cx| {
        cx.write_to_clipboard(ClipboardItem::new_string(
            "printf 'PASTE_%s\\n' VERIFIED".into(),
        ));
        w.press("cmd-v", cx);
        w.press("enter", cx);
    })
    .unwrap();
    until(&runtime, &mut cx, &lab, window, 2, "PASTE_VERIFIED");
    // Wait for the real prompt, including any Git/theme hooks, before history keys.
    wait_cursor(&runtime, &mut cx, &lab, window, "»");
    let snapshots = humata2_snapshot::snapshots!().unwrap();
    let name = if remote {
        "ssh-neovim-zsh"
    } else {
        "neovim-oh-my-zsh"
    };
    let backend = if gpui_kit::GPUI_FAST { "-fast" } else { "" };
    // These captures intentionally contain the owner's changing prompt/Git state.
    if std::env::var("HUMATA2_SNAPSHOT").as_deref() == Ok("record") {
        snapshots.assert_image(
            &format!("{name}{backend}"),
            &cx.capture_screenshot(window).unwrap(),
            humata2_snapshot::Tolerance::EXACT,
        );
    }
    // Up retrieves the command, Down returns to the unsubmitted empty draft.
    let prompt_x = cx.update(|cx| {
        let t = lab.read(cx).terminal(2).unwrap();
        t.read(cx).pane().read(cx).view().read(cx).grid().cursor().x
    });
    cx.update_window(window, |_, w, cx| w.press("up", cx))
        .unwrap();
    for _ in 0..20 {
        pump(&runtime, &mut cx, &lab, window);
    }
    wait_cursor(&runtime, &mut cx, &lab, window, "VERIFIED");
    cx.update_window(window, |_, w, cx| w.press("down", cx))
        .unwrap();
    for _ in 0..20 {
        pump(&runtime, &mut cx, &lab, window);
    }
    assert_eq!(
        cx.update(|cx| {
            let t = lab.read(cx).terminal(2).unwrap();
            t.read(cx).pane().read(cx).view().read(cx).grid().cursor().x
        }),
        prompt_x
    );
    // Select the actual output cells and copy via Cmd+C to the frontend clipboard.
    let row = cx.update(|cx| {
        let t = lab.read(cx).terminal(2).unwrap();
        let g = t.read(cx).pane().read(cx).view().read(cx).grid();
        (0..g.size().rows)
            .find(|y| {
                g.row(*y).is_some_and(|r| {
                    r.cells
                        .iter()
                        .map(|c| c.text.as_str())
                        .collect::<String>()
                        .trim()
                        == "PASTE_VERIFIED"
                })
            })
            .unwrap()
    });
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(2).unwrap();
        let cell = t
            .read(cx)
            .pane()
            .read(cx)
            .view()
            .read(cx)
            .cell_size()
            .unwrap();
        let bounds = w.within("group-1").find("terminal").bounds();
        let at = |x: f32| {
            point(
                bounds.origin.x + px((x + 0.5) * cell.width),
                bounds.origin.y + px((f32::from(row) + 0.5) * cell.height),
            )
        };
        w.drag(at(0.), at(13.), cx);
        w.press("cmd-c", cx);
    })
    .unwrap();
    assert_eq!(
        cx.update(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
            .as_deref(),
        Some("PASTE_VERIFIED")
    );
    let entity = cx.update(|cx| lab.read(cx).terminal(2).unwrap().entity_id());
    cx.update_window(window, |_, w, cx| {
        let tab = w.find("tab-2").bounds();
        let target = w.find("group-0").bounds();
        w.drag(
            tab.center(),
            point(target.origin.x + px(40.), target.origin.y + px(110.)),
            cx,
        );
    })
    .unwrap();
    for _ in 0..30 {
        pump(&runtime, &mut cx, &lab, window);
    }
    assert!(cx.update(|cx| lab.read(cx).group(0).unwrap().contains(&2)));
    assert_eq!(
        cx.update(|cx| lab.read(cx).terminal(2).unwrap().entity_id()),
        entity
    );
    assert!(text(&mut cx, &lab, 2).contains("PASTE_VERIFIED"));
    // Reconnect the same zsh, preserving its directory and environment.
    command(
        &mut cx,
        &lab,
        window,
        2,
        "export RESUME_PROOF=original; printf 'RESUME_%s\\n' READY",
    );
    until(&runtime, &mut cx, &lab, window, 2, "RESUME_READY");
    cx.update_window(window, |_, w, cx| w.click("detach-2", cx))
        .unwrap();
    for _ in 0..20 {
        pump(&runtime, &mut cx, &lab, window);
    }
    cx.update_window(window, |_, w, cx| w.click("resume-2", cx))
        .unwrap();
    for _ in 0..70 {
        pump(&runtime, &mut cx, &lab, window);
    }
    command(
        &mut cx,
        &lab,
        window,
        2,
        "printf 'SAME_%s\\n' \"$RESUME_PROOF\"",
    );
    until(&runtime, &mut cx, &lab, window, 2, "SAME_original");
    let started = Instant::now();
    command(
        &mut cx,
        &lab,
        window,
        2,
        "/usr/bin/awk 'BEGIN {for(i=1;i<=12000;i++) printf \"HISTORY_%05d\\n\",i}'",
    );
    until(&runtime, &mut cx, &lab, window, 2, "HISTORY_12000");
    let output_ms = started.elapsed().as_secs_f64() * 1000.;
    let bottom = scrollbar(&mut cx, &lab, 2);
    assert!(bottom.total >= 12000);
    assert!(bottom.at_bottom());
    cx.update_window(window, |_, w, cx| w.press("cmd-pageup", cx))
        .unwrap();
    for _ in 0..10 {
        pump(&runtime, &mut cx, &lab, window);
    }
    assert!(scrollbar(&mut cx, &lab, 2).offset < bottom.offset);
    // Scroll through the real 12k-line history repeatedly, measuring UI work.
    let mut times = Vec::new();
    for _ in 0..50 {
        let start = Instant::now();
        cx.update_window(window, |_, w, cx| {
            let key=gpui_kit::Keystroke::parse_with_os("cmd-pageup",cx.platform_info().os).unwrap();
            w.dispatch_keystroke(key,cx);
            w.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        times.push(start.elapsed().as_secs_f64() * 1000.);
    }
    cx.update_window(window, |_, w, cx| w.press("cmd-home", cx))
        .unwrap();
    for _ in 0..10 {
        pump(&runtime, &mut cx, &lab, window);
    }
    assert_eq!(scrollbar(&mut cx, &lab, 2).offset, 0);
    cx.update_window(window, |_, w, cx| w.press("cmd-end", cx))
        .unwrap();
    for _ in 0..10 {
        pump(&runtime, &mut cx, &lab, window);
    }
    assert!(scrollbar(&mut cx, &lab, 2).at_bottom());
    assert!(text(&mut cx, &lab, 2).contains("HISTORY_12000"));
    let stats = cx.update(|cx| {
        let t = lab.read(cx).terminal(2).unwrap();
        t.read(cx).pane().read(cx).view().read(cx).stats()
    });
    assert_eq!(stats.paint_failures, 0);
    assert_eq!(
        stats.rows_rebuilt, 0,
        "idle 12k-line history should reuse its visible rows"
    );
    times.sort_by(f64::total_cmp);
    println!(
        "{{\"case\":\"12000-line-real-pty-history\",\"ssh\":{remote},\"fast\":{},\"output_ms\":{output_ms:.2},\"scroll_frame_p50_ms\":{:.3},\"scroll_frame_p95_ms\":{:.3},\"retained_rows\":{}}}",
        gpui_kit::GPUI_FAST,
        times[25],
        times[47],
        bottom.total
    );
    // Restore the large display from disk while retaining the actual remote/local shell.
    let old=cx.update(|cx|lab.read(cx).terminal(2).unwrap().entity_id());
    cx.update_window(window,|_,w,cx|w.click("detach-2",cx)).unwrap();
    for _ in 0..15{pump(&runtime,&mut cx,&lab,window);}
    let replay_started=Instant::now();
    cx.update_window(window,|_,w,cx|w.click("resume-2",cx)).unwrap();
    let deadline=Instant::now()+Duration::from_secs(15);
    while cx.update(|cx|lab.read(cx).terminal(2).unwrap().entity_id())==old {
        pump(&runtime,&mut cx,&lab,window);assert!(Instant::now()<deadline,"large-history resume must replace the detached view");
    }
    until(&runtime,&mut cx,&lab,window,2,"HISTORY_12000");
    assert!(scrollbar(&mut cx,&lab,2).total>=12000);
    println!("large_history_replay_ms={:.2} ssh={remote} fast={}",replay_started.elapsed().as_secs_f64()*1000.,gpui_kit::GPUI_FAST);
    for id in [1, 2] {
        cx.update(|cx| {
            lab.read(cx)
                .terminal(id)
                .unwrap()
                .update(cx, |t, _| t.stop())
        });
    }
    for _ in 0..50 {
        pump(&runtime, &mut cx, &lab, window);
    }
}
