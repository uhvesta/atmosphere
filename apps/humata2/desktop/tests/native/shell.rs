//! The window shell: sidebar navigation, both appearances, columns and full screen.
use super::*;

/// NV-2, CH-16: navigation and folding use the real sidebar and store.
#[test]
fn sidebar_navigation_and_folds_act_on_the_open_workspace() {
    let mut app = App::new(fixture::design());
    assert!(app.contains("sidebar-session-partial-brin"));
    app.click("sidebar-session-partial-brin");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("partial-brin"))
    );
    app.click("rail-home");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Home);
    assert!(app.ui.read(|s| s.trail.is_empty()));
    app.click("rail-inbox");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Inbox);
    app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
        humata2_clients::model::ChannelId::new("payments-rewrite"),
    ))));
    assert!(!app.contains("sidebar-session-partial-brin"));
    app.click("sidebar-channel-payments-rewrite");
    assert_eq!(
        app.ui.read(|s| s.route.clone()),
        Route::Channel(humata2_clients::model::ChannelId::new("payments-rewrite"))
    );
    app.click("rail-settings");
    assert_eq!(app.ui.read(|s| s.route.clone()), Route::Settings);
}

/// N1: full-window reference images, never replacing the approved frames.
#[test]
fn the_real_shell_in_both_appearances_and_sidebar_states() {
    let snapshots = folder();
    for (name, mode, pinned, folded, selected) in [
        (
            "01-session-working-light",
            ThemeMode::Light,
            false,
            false,
            "ledger",
        ),
        (
            "02-session-working-dark",
            ThemeMode::Dark,
            false,
            false,
            "ledger",
        ),
        ("15-sidebar-pinned", ThemeMode::Light, true, false, "ledger"),
        (
            "16-sidebar-fork-selected",
            ThemeMode::Light,
            false,
            false,
            "partial-brin",
        ),
        (
            "17-sidebar-channels-folded",
            ThemeMode::Dark,
            false,
            true,
            "ledger",
        ),
    ] {
        let mut workspace = fixture::design_session(if pinned {
            Status::NeedsYou
        } else {
            Status::Working
        });
        let ledger = workspace
            .sessions
            .iter_mut()
            .find(|s| s.id.as_str() == "ledger")
            .expect("ledger fixture");
        ledger.status = if pinned {
            Status::NeedsYou
        } else {
            Status::Working
        };
        ledger.pinned = pinned;
        if pinned {
            workspace
                .sessions
                .iter_mut()
                .find(|s| s.id.as_str() == "pr-482")
                .expect("PR fixture")
                .pinned = true;
        }
        let mut app = App::new(workspace);
        app.cx.update(|cx| Theme::change(mode, None, cx));
        // D128.5: frame 16 shows the selected fork itself (SE-6), so the
        // header reads Partial BRIN on posted_at and the rail has no Inbox
        // badge.
        app.open(selected, Panel::Changes);
        app.file_tab();
        if pinned {
            // Golden 15: two messages wait while Ledger needs you.
            for message in [
                "Keep both indexes until the backfill ends.",
                "Then check the read path before dropping the old index.",
            ] {
                app.send(AppAction::Session(SessionAction::Draft(message.into())));
                app.send(AppAction::Session(SessionAction::Submit));
            }
        }
        app.send(AppAction::Review(
            humata2_review::ReviewAction::SelectFile {
                repo: "api".into(),
                path: "src/client.rs".into(),
            },
        ));
        app.write_comment(
            "ledger",
            "api",
            43,
            "Keep insert_legacy until the backfill ends.",
        );
        app.send(AppAction::Sidebar(SidebarAction::SetCurrent(Some(
            RowKey::Session(SessionId::new(selected)),
        ))));
        for id in if folded {
            vec!["payments-rewrite", "infra", "pr-reviews", "web"]
        } else {
            vec!["pr-reviews", "web"]
        } {
            app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
        app.draw();
        let tree = app
            .cx
            .update_window(app.window, |_, window, _| view::capture(window))
            .expect("window")
            .expect("view");
        snapshots.assert_text(&format!("{name}.view"), &tree);
        let image = app
            .cx
            .capture_screenshot(app.window)
            .expect("headless image");
        snapshots.assert_image(name, &image, image_tolerance(&snapshots));
    }
}

/// N1: expanding either area uses the same real header/panel controls.
#[test]
fn terminal_and_panel_full_screen_return_to_the_same_columns() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Board);
    let layout = app.ui.read(|s| s.layout);
    app.click("terminal-full-screen");
    assert_eq!(app.ui.read(|s| s.full_screen()), Some(FullScreen::Terminal));
    assert!(!app.contains("sidebar"));
    assert!(!app.contains("side-panel"));
    app.click("terminal-full-screen");
    assert!(app.contains("sidebar"));
    assert!(app.contains("side-panel"));
    app.click("expand-panel");
    assert_eq!(app.ui.read(|s| s.full_screen()), Some(FullScreen::Panel));
    assert!(app.contains("panel-full-screen"));
    assert!(!app.contains("terminal"));
    app.click("show-terminal");
    assert!(app.contains("terminal"));
    assert!(app.contains("side-panel"));
    assert_eq!(app.ui.read(|s| s.layout), layout);
}

/// The owner's half-screen request: resize the same live headless window,
/// navigate through its compact sidebar, then widen it again.
#[test]
fn a_half_screen_window_can_navigate_and_restore_its_columns() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    let saved = app.ui.read(|s| s.layout);
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(720.), px(760.)));
            window.bounds_changed(cx);
        })
        .expect("resize");
    app.settle();
    app.draw();
    let width = app
        .cx
        .update_window(app.window, |_, window, _| {
            f32::from(window.viewport_size().width)
        })
        .expect("viewport");
    assert_eq!(width, 720.);
    assert!(app.contains("sidebar-strip"));
    assert!(app.contains("side-panel"));
    app.click("sidebar-strip");
    assert!(app.contains("sidebar-session-brin"));
    app.click("sidebar-session-brin");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("brin"))
    );
    assert!(!app.ui.read(|s| s.navigation_overlay));
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(1280.), px(760.)));
            window.bounds_changed(cx);
        })
        .expect("resize back");
    app.settle();
    app.draw();
    assert!(app.contains("sidebar"));
    assert!(!app.contains("sidebar-strip"));
    assert_eq!(app.ui.read(|s| s.layout), saved);
}
