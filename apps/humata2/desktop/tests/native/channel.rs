//! The channel page (frames 18-20, 35).
use super::*;

/// N1: the real channel's three approved page bodies.
#[test]
fn channel_page_reference_images() {
    use humata2_app::channel::{Action, Tab};
    for (name, tab, mode) in [
        ("18-channel-sessions", Tab::Sessions, ThemeMode::Light),
        ("19-channel-activity", Tab::Activity, ThemeMode::Dark),
        ("20-channel-repos-machines", Tab::Repos, ThemeMode::Light),
    ] {
        let mut app = App::new(fixture::design());
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.send(AppAction::Switch(Route::Channel(
            humata2_clients::model::ChannelId::new("payments-rewrite"),
        )));
        app.send(AppAction::Channel(Action::Tab(tab)));
        for id in ["pr-reviews", "web"] {
            app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
        app.snapshot(name);
    }
}

/// Frame 35: a channel with nothing in it lists its first steps, and they act.
#[test]
fn an_empty_channel_offers_a_session_and_a_machine() {
    let mut app = App::new(fixture::fresh_channel());
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.send(AppAction::Switch(Route::Channel(
        humata2_clients::model::ChannelId::new("web-revamp"),
    )));
    app.snapshot("35-empty-channel");
    app.click("channel-empty-new-session");
    assert!(
        app.ui
            .read(|s| matches!(s.overlay(), Some(humata2_app::Overlay::NewSession(_)))),
        "overlay after New session: {:?}",
        app.ui.read(|s| s.overlays.clone())
    );
    assert!(app.contains("new-session-create"));
    app.send(AppAction::Escape);
    app.click("channel-empty-add-machine");
    assert!(app.contains("host-add-connect"));
}

/// CH-12/13: filters/folds retain fork context; an Inbox count opens the right session.
#[test]
fn channel_sessions_filter_fold_and_open_their_own_inbox() {
    use humata2_app::channel::Action;
    let mut app = App::new(fixture::design());
    app.click("sidebar-channel-payments-rewrite");
    assert!(app.contains("channel-session-partial-brin"));
    app.click("channel-fold-ledger");
    assert!(!app.contains("channel-session-partial-brin"));
    app.click("channel-expand-all");
    assert!(app.contains("channel-session-partial-brin"));
    app.click("channel-session-filter");
    app.cx
        .update_window(app.window, |_, window, cx| window.input("PARTIAL", cx))
        .expect("filter input");
    app.settle();
    assert!(app.contains("channel-session-ledger"));
    assert!(app.contains("channel-session-partial-brin"));
    assert!(!app.contains("channel-session-refund"));
    app.send(AppAction::Channel(Action::Filter(String::new())));
    app.click("channel-inbox-ledger");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("ledger"))
    );
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Inbox));
}

/// CH-9: clone cells filter the channel table and Activity links open their session.
#[test]
fn channel_clone_cells_and_activity_are_actionable() {
    use humata2_app::channel::Tab;
    let mut app = App::new(fixture::design());
    app.click("sidebar-channel-payments-rewrite");
    app.click("channel-tab-Repos and machines");
    app.click("channel-clones-acme/ledger-devbox-1");
    assert!(app.contains("channel-sessions"));
    assert!(app.ui.read(|s| {
        s.channel_pages
            .values()
            .any(|v| v.tab == Tab::Sessions && v.clones.is_some())
    }));
    assert!(!app.contains("channel-session-refund"));
    app.click("channel-clear-clones");
    assert!(app.contains("channel-session-refund"));
    app.click("channel-tab-Inbox");
    assert!(app.contains("global-request move-pay-3"));
    assert!(!app.contains("global-request benchmark-command"));
    app.click("rail-home");
    app.click("sidebar-channel-payments-rewrite");
    assert!(app.contains("global-request move-pay-3"));
    assert!(!app.contains("global-request benchmark-command"));
    app.click("channel-tab-Activity");
    app.click("channel-event-0");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("ledger"))
    );
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Inbox));
}
