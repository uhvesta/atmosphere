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
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
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
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
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
fn a_restart_restores_both_tab_groups_and_the_selected_file() {
    use humata2_session::{Tab, TabGroup};
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
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
fn board_view_scope_and_filters_restore_together() {
    use humata2_board::{BoardAction as B, Filter, Mode};
    use humata2_clients::model::{ChannelId, ItemStatus, Scope};
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.route = Route::Board(Scope::Global);
    let mut store = TestStore::new(state, App::new(clients.clone()));
    let scope = Scope::Channel(ChannelId::new("infra"));
    store.send(AppAction::Board(B::SetScope(scope.clone())), |s| {
        s.route = Route::Board(scope.clone());
        s.board.scope = scope.clone();
    });
    store.send(
        AppAction::Board(B::Drill(Filter::Status(ItemStatus::InProgress))),
        |s| {
            s.board.view.mode = Mode::Table;
            s.board.view.filter = Filter::Status(ItemStatus::InProgress);
        },
    );
    saved::Saver::new(clients.storage.clone())
        .save(1, &store.state().saved())
        .expect("saved board");
    let restored = AppState::launch(&clients);
    assert_eq!(restored.route, Route::Board(scope.clone()));
    assert_eq!(restored.board.scope, scope);
    assert_eq!(restored.board.view, store.state().board.view);
    store.finish();
}
