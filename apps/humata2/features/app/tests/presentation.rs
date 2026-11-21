//! N1, SE-12/13: full-screen areas and narrow columns preserve saved layout.
use humata2_app::layout::{FullScreen, Layout};
use humata2_app::{App, AppAction, AppState, Route, saved};
use humata2_clients::model::SessionId;
use humata2_clients::{Clients, fixture};
use humata2_session::{Panel, SessionAction, SessionView};
use std::sync::Arc;
use tca::{TestClock, TestStore};

#[test]
fn expanding_either_area_and_returning_preserves_the_columns() {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    let id = SessionId::new("ledger");
    state.route = Route::Session(id.clone());
    state.session.open = Some(id.clone());
    state.session.views.insert(
        id.clone(),
        SessionView {
            panel: Some(Panel::Board),
            ..Default::default()
        },
    );
    let layout = state.layout;
    let mut store = TestStore::new(state, App::new(clients.clone()));
    store.send(AppAction::ToggleFullScreen(FullScreen::Terminal), |s| {
        s.full_screens.insert(id.clone(), FullScreen::Terminal);
    });
    assert_eq!(store.state().full_screen(), Some(FullScreen::Terminal));
    assert_eq!(store.state().layout, layout);
    saved::Saver::new(clients.storage.clone())
        .save(1, &store.state().saved())
        .expect("save terminal presentation");
    assert_eq!(
        AppState::launch(&clients).full_screen(),
        Some(FullScreen::Terminal)
    );
    store.send(AppAction::Session(SessionAction::ToggleWiden), |s| {
        s.full_screens.insert(id.clone(), FullScreen::Panel);
        s.session.views.get_mut(&id).expect("view").widened = true;
    });
    assert_eq!(store.state().full_screen(), Some(FullScreen::Panel));
    let saved = store.state().saved();
    saved::Saver::new(clients.storage.clone())
        .save(2, &saved)
        .expect("save");
    let restored = AppState::launch(&clients);
    assert_eq!(restored.full_screen(), Some(FullScreen::Panel));
    assert_eq!(restored.layout, layout);
    store.send(AppAction::ToggleFullScreen(FullScreen::Panel), |s| {
        s.full_screens.remove(&id);
        s.session.views.get_mut(&id).expect("view").widened = false;
    });
    assert_eq!(store.state().full_screen(), None);
    assert_eq!(store.state().layout, layout);
    store.finish();
}

#[test]
fn closing_the_full_screen_panel_returns_to_the_terminal() {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    let id = SessionId::new("ledger");
    state.route = Route::Session(id.clone());
    state.session.open = Some(id.clone());
    state.full_screens.insert(id.clone(), FullScreen::Panel);
    state.session.views.insert(
        id.clone(),
        SessionView {
            panel: Some(Panel::Board),
            widened: true,
            ..Default::default()
        },
    );
    let mut store = TestStore::new(state, App::new(clients));
    store.send(AppAction::Session(SessionAction::SetPanel(None)), |s| {
        s.full_screens.remove(&id);
        let view = s.session.views.get_mut(&id).expect("view");
        view.panel = None;
        view.widened = false;
    });
    assert_eq!(store.state().full_screen(), None);
    store.finish();
}

#[test]
fn half_screen_and_wide_again_restore_sizes_without_saving_automatic_folds() {
    let layout = Layout::default();
    let wide = layout.columns(1280., true, true);
    assert!(!wide.sidebar_hidden && !wide.panel_hidden);
    let half = layout.columns(720., true, true);
    assert!(half.sidebar_hidden && !half.panel_hidden);
    assert!(half.panel >= 200.);
    assert!(720. - 14. - 64. - 1. - half.panel >= 360.);
    let small = layout.columns(560., true, true);
    assert!(small.sidebar_hidden && small.panel_hidden);
    assert_eq!(layout.columns(1280., true, true), wide);
    let no_panel = layout.columns(720., true, false);
    assert!(!no_panel.sidebar_hidden);
    assert_eq!(no_panel.panel, 0.);
}

#[test]
fn side_panel_can_resize_up_to_half_the_window_width_while_keeping_main_min() {
    use humata2_app::layout::{MAIN_MIN, PaneId, panel_max_width};

    let mut layout = Layout::default();
    // On a 1600px window, 50% is 800px; dragging there should not clamp at 420px.
    layout.drag(PaneId::Panel, 800.);
    assert_eq!(layout.panel.width, 800.);
    let cols = layout.columns(1600., true, true);
    assert!(!cols.panel_hidden);
    assert_eq!(cols.panel, 800.);
    let main = 1600. - 64. - cols.sidebar - 2. - cols.panel;
    assert!(main >= MAIN_MIN, "main pane kept at least MAIN_MIN: {main}");

    // Dragging beyond 50% of a 1440px window is clamped to 50% (720px) in columns().
    layout.drag(PaneId::Panel, 1200.);
    assert_eq!(panel_max_width(1440.), 720.);
    let cols_1440 = layout.columns(1440., true, true);
    assert_eq!(cols_1440.panel, 720.);
    let main_1440 = 1440. - 64. - cols_1440.sidebar - 2. - cols_1440.panel;
    assert!(main_1440 >= MAIN_MIN, "main pane kept at least MAIN_MIN: {main_1440}");

    // On a narrower 1280px window with sidebar open, panel scales so main >= MAIN_MIN.
    let cols_1280 = layout.columns(1280., true, true);
    assert!(cols_1280.panel <= 640.);
    let main_1280 = 1280. - 64. - cols_1280.sidebar - 2. - cols_1280.panel;
    assert!(
        main_1280 >= MAIN_MIN - 0.5,
        "main pane never collapses below MAIN_MIN: {main_1280}"
    );

    // With sidebar collapsed on 1280px, panel reaches full 50% (640px).
    layout.sidebar.collapsed = true;
    let cols_collapsed = layout.columns(1280., true, true);
    assert_eq!(cols_collapsed.panel, 640.);
    let main_collapsed = 1280. - 64. - 14. - 1. - cols_collapsed.panel;
    assert!(main_collapsed >= MAIN_MIN);
}

#[test]
fn a_restart_restores_both_tab_groups_and_the_selected_file() {
    use humata2_session::{Tab, TabGroup};
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    let id = SessionId::new("ledger");
    state.route = Route::Session(id.clone());
    let view = SessionView {
        groups: vec![
            TabGroup {
                tabs: vec![Tab::Terminal(0), Tab::Terminal(1)],
                selected: 1,
            },
            TabGroup {
                tabs: vec![Tab::File {
                    repo: "api".into(),
                    path: "src/client.rs".into(),
                }],
                selected: 0,
            },
        ],
        focused_group: 1,
        tab: 1,
        ..Default::default()
    };
    state.session.views.insert(id.clone(), view.clone());
    saved::Saver::new(clients.storage.clone())
        .save(1, &state.saved())
        .expect("save tabs");
    let restored = AppState::launch(&clients);
    assert_eq!(restored.session.views.get(&id), Some(&view));
    assert_eq!(restored.session.open, Some(id));
}

#[test]
fn an_earlier_builds_saved_board_opens_the_todo_list() {
    // The board page is gone (D874): a saved board route, a board in the
    // trail, and a Go to Board recent pick from an earlier build still
    // restore, as the To-do list and without the pick.
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let earlier = br#"{
        "route": {"Board": {"Channel": "infra"}},
        "trail": ["Inbox", {"Board": "Global"}],
        "recent": ["Board", "Inbox", {"ChannelBoard": "infra"}],
        "board": {"mode": "Table"}
    }"#;
    clients
        .storage
        .save(saved::KEY, earlier)
        .expect("save an earlier build's state");
    let restored = saved::restore(clients.storage.as_ref())
        .expect("an earlier build's state reads")
        .expect("something was saved");
    assert_eq!(restored.route, Route::Todo);
    assert_eq!(restored.trail, vec![Route::Inbox, Route::Todo]);
    assert_eq!(restored.recent, vec![humata2_palette::Command::Inbox]);
}

#[test]
fn selected_host_and_layout_preferences_restore_without_closing_saved_groups() {
    use humata2_clients::model::MachineId;
    use humata2_settings::{SettingsAction as T, Theme};
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.hosts.selected = Some(MachineId::new("devbox-1"));
    let mut store = TestStore::new(state, App::new(clients.clone()));
    store.send(AppAction::Settings(T::SetTheme(Theme::Light)), |s| {
        s.settings.preferences.theme = Theme::Light
    });
    store.send(AppAction::Settings(T::TogglePinned), |s| {
        s.settings.preferences.show_pinned = false;
        s.pinned.shown = false;
    });
    saved::Saver::new(clients.storage.clone())
        .save(1, &store.state().saved())
        .expect("save preferences");
    let restored = AppState::launch(&clients);
    assert_eq!(restored.hosts.selected, Some(MachineId::new("devbox-1")));
    assert_eq!(
        restored.settings.preferences,
        store.state().settings.preferences
    );
    store.finish();
}

/// Flow 26.14 steps 3–5, ST-8: Long lines is kept across a restart, and the
/// diff and the editor take the restored choice at launch.
#[test]
fn long_lines_survive_a_restart_both_ways() {
    use humata2_settings::SettingsAction as T;
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let first = state.review.wrap;
    let mut store = TestStore::new(state, App::new(clients.clone()));
    store.send(AppAction::Settings(T::ToggleWrap), |s| {
        s.settings.preferences.wrap_diffs = !first;
        s.review.wrap = !first;
    });
    saved::Saver::new(clients.storage.clone())
        .save(1, &store.state().saved())
        .expect("save preferences");
    let restored = AppState::launch(&clients);
    assert_eq!(restored.settings.preferences.wrap_diffs, !first);
    assert_eq!(restored.review.wrap, !first);

    store.send(AppAction::Settings(T::ToggleWrap), |s| {
        s.settings.preferences.wrap_diffs = first;
        s.review.wrap = first;
    });
    saved::Saver::new(clients.storage.clone())
        .save(2, &store.state().saved())
        .expect("save preferences");
    let restored = AppState::launch(&clients);
    assert_eq!(restored.review.wrap, first);
    store.finish();
}
