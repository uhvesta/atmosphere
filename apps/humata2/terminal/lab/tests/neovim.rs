//! Optional system-Neovim acceptance test; no user config or swap/shada writes.
use ghostty_vt::{Color, Grid};
use gpui_kit::{
    AnyWindowHandle, Bounds, Entity, Focusable, HeadlessAppContext, WindowBounds, WindowOptions,
    point, prelude::*, px, size, test::TestWindowExt,
};
use humata2_engine::Resources;
use std::{os::unix::fs::PermissionsExt, time::Duration};
use terminal_lab::{Assets, Lab};
use terminal_session::{Endpoint, SessionSpec, SessionStore};
fn settle(
    runtime: &tokio::runtime::Runtime,
    cx: &mut HeadlessAppContext,
    lab: &Entity<Lab>,
    window: AnyWindowHandle,
) {
    for _ in 0..35 {
        runtime.block_on(async {
            tokio::time::sleep(Duration::from_millis(10)).await;
        });
        cx.update(|cx| lab.update(cx, |lab, cx| lab.poll(cx)));
        cx.run_until_parked();
        cx.update_window(window, |_, w, cx| {
            w.render_frame(cx);
            w.render_frame(cx);
        })
        .unwrap();
    }
}
fn screen(cx: &mut HeadlessAppContext, lab: &Entity<Lab>) -> String {
    cx.update(|cx| {
        let t = lab.read(cx).terminal(1).unwrap();
        let grid = t.read(cx).pane().read(cx).view().read(cx).grid();
        (0..grid.size().rows)
            .filter_map(|y| grid.row(y))
            .map(|r| r.cells.iter().map(|c| c.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    })
}
#[test]
fn neovim_colors_mouse_keyboard_resize_and_resume() {
    let nvim = std::env::var_os("TERMINAL_LAB_NVIM").expect(
        "Set TERMINAL_LAB_NVIM to an installed Neovim binary for this manual acceptance test",
    );
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::Builder::new()
        .prefix("tnvim-")
        .tempdir_in("/tmp")
        .unwrap();
    let state = dir.path().join("state");
    std::fs::create_dir(&state).unwrap();
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
    let file = dir.path().join("terminal.rs");
    std::fs::write(
        &file,
        r#"// A real Neovim buffer inside Humata
use std::time::Duration;

struct Session {
    title: String,
    connected: bool,
}

impl Session {
    fn resume(&mut self) -> Result<(), &'static str> {
        if self.connected {
            return Err("already connected");
        }
        self.connected = true;
        println!("Connected: {}", self.title);
        Ok(())
    }
}

fn main() {
    let timeout = Duration::from_secs(30);
    println!("Ready in {:?}", timeout);
}
"#,
    )
    .unwrap();
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let mut store = SessionStore::open(&state, holder).unwrap();
    store
        .create(SessionSpec {
            title: "Neovim".into(),
            program: nvim.into(),
            args: vec![
                "--clean".into(),
                "-n".into(),
                "-i".into(),
                "NONE".into(),
                "--cmd".into(),
                "set termguicolors background=dark".into(),
                "-c".into(),
                "set number cursorline mouse=a laststatus=2 noshowmode".into(),
                "-c".into(),
                "syntax on".into(),
                "terminal.rs".into(),
            ],
            cwd: dir.path().into(),
            cols: 110,
            rows: 32,
            endpoint: Endpoint::Local,
        })
        .unwrap();
    let mut cx = humata2_engine::headless(&Resources::new(Assets)).unwrap();
    let (window, lab) = cx
        .update(|cx| {
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1200.), px(800.)),
                    ))),
                    show: false,
                    focus: false,
                    ..Default::default()
                },
                cx,
                |w, cx| cx.new(|cx| Lab::new(store, runtime.handle().clone(), None, w, cx)),
            )
        })
        .unwrap();
    cx.update_window(window, |_, w, _| w.activate_window())
        .unwrap();
    settle(&runtime, &mut cx, &lab, window);
    assert!(screen(&mut cx, &lab).contains("struct Session"));
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(1).unwrap();
        w.focus(&t.read(cx).focus_handle(cx), cx);
        t.read(cx).pane().clone().update(cx, |p, cx| {
            p.view()
                .clone()
                .update(cx, |v, cx| v.set_cursor_blink(false, cx))
        });
        w.render_frame(cx);
    })
    .unwrap();
    settle(&runtime, &mut cx, &lab, window);
    // Syntax colors must be actual 24-bit terminal cell colors, not host labels.
    let colors = cx.update(|cx| {
        let t = lab.read(cx).terminal(1).unwrap();
        let g = t.read(cx).pane().read(cx).view().read(cx).grid();
        (0..g.size().rows)
            .filter_map(|y| g.row(y))
            .flat_map(|r| r.cells.iter())
            .filter_map(|c| match c.fg {
                Color::Rgb { r, g, b } => Some((r, g, b)),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>()
    });
    assert!(
        colors.len() >= 5,
        "Neovim should draw varied RGB syntax colors: {colors:?}"
    );
    let snapshots = humata2_snapshot::snapshots!().unwrap();
    let suffix = if gpui_kit::GPUI_FAST { "-fast" } else { "" };
    snapshots.assert_image(
        &format!("neovim{suffix}"),
        &cx.capture_screenshot(window).unwrap(),
        humata2_snapshot::Tolerance::default(),
    );
    // A real mouse click moves Neovim's cursor; it does not select terminal text.
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(1).unwrap();
        let p = t.read(cx).pane();
        let cell = p.read(cx).view().read(cx).cell_size().unwrap();
        let bounds = w.find("terminal").bounds();
        let at = point(
            bounds.origin.x + px(cell.width * 12.5),
            bounds.origin.y + px(cell.height * 4.5),
        );
        w.drag(at, at, cx);
    })
    .unwrap();
    settle(&runtime, &mut cx, &lab, window);
    assert_eq!(
        cx.update(|cx| {
            let t = lab.read(cx).terminal(1).unwrap();
            t.read(cx).pane().read(cx).view().read(cx).grid().cursor().y
        }),
        4
    );
    cx.update_window(window, |_, w, cx| {
        w.press("g", cx);
        w.press("g", cx);
        w.press("0", cx);
        w.press("i", cx);
        w.input("// unsaved edit ", cx);
        w.press("escape", cx);
    })
    .unwrap();
    settle(&runtime, &mut cx, &lab, window);
    assert!(screen(&mut cx, &lab).contains("unsaved edit"));
    // Font zoom and window resize trigger real Neovim redraws through SIGWINCH.
    cx.update_window(window, |_, w, cx| {
        w.press("cmd-=", cx);
        w.resize(size(px(700.), px(600.)));
        w.bounds_changed(cx);
    })
    .unwrap();
    settle(&runtime, &mut cx, &lab, window);
    assert!(screen(&mut cx, &lab).contains("unsaved edit"));
    cx.update_window(window, |_, w, cx| {
        w.press("cmd-0", cx);
        w.resize(size(px(1200.), px(800.)));
        w.bounds_changed(cx);
        w.click("detach-1", cx);
    })
    .unwrap();
    settle(&runtime, &mut cx, &lab, window);
    cx.update_window(window, |_, w, cx| w.click("resume-1", cx))
        .unwrap();
    settle(&runtime, &mut cx, &lab, window);
    assert!(
        screen(&mut cx, &lab).contains("unsaved edit"),
        "resuming must keep Neovim's unsaved buffer"
    );
    cx.update_window(window, |_, w, cx| {
        let t = lab.read(cx).terminal(1).unwrap();
        w.focus(&t.read(cx).focus_handle(cx), cx);
        w.press(":", cx);
        w.input("write", cx);
        w.press("enter", cx);
    })
    .unwrap();
    settle(&runtime, &mut cx, &lab, window);
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("unsaved edit")
    );
    cx.update(|cx| {
        lab.read(cx)
            .terminal(1)
            .unwrap()
            .update(cx, |t, _| t.stop())
    });
    settle(&runtime, &mut cx, &lab, window);
}
