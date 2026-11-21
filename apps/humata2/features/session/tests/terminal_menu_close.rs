//! Closing a tab or split tab group must clear `terminal_menu` so a phantom
//! menu on a removed group cannot swallow Escape.
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::SessionId;
use humata2_session::{SessionAction as A, SessionState, SessionView, Sessions, Tab, TabGroup};
use tca::{TestClock, TestStore};

fn split_store() -> TestStore<Sessions> {
    let workspace = fixture::design();
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let id = SessionId::new("ledger");
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        open: Some(id.clone()),
        terminal_menu: Some(1),
        ..Default::default()
    };
    state.views.insert(
        id,
        SessionView {
            groups: vec![
                TabGroup {
                    tabs: vec![Tab::Terminal(0)],
                    selected: 0,
                },
                TabGroup {
                    tabs: vec![Tab::Terminal(1)],
                    selected: 0,
                },
            ],
            focused_group: 1,
            shells_placed: Some(1),
            ..Default::default()
        },
    );
    TestStore::new(state, Sessions::new(clients)).non_exhaustive()
}

#[test]
fn close_tab_clears_terminal_menu() {
    let mut store = split_store();
    assert_eq!(store.state().terminal_menu, Some(1));
    store.send(A::CloseTab { group: 1, index: 0 }, |_| {});
    assert_eq!(store.state().terminal_menu, None);
}

#[test]
fn close_group_clears_terminal_menu() {
    let mut store = split_store();
    assert_eq!(store.state().terminal_menu, Some(1));
    store.send(A::CloseGroup(1), |_| {});
    assert_eq!(store.state().terminal_menu, None);
}
