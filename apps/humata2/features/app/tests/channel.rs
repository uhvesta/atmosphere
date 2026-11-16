//! CH-9/12/13 and PER-25: filtering keeps context, folds save per channel.
use humata2_app::{
    App, AppAction as A, AppState, Route,
    channel::{self, Action, History, Tab, View},
    saved,
};
use humata2_clients::{
    Clients, fixture,
    model::{ChannelId, MachineId, SessionId},
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

#[test]
fn channel_filters_keep_ancestors_and_the_page_restores_after_restart() {
    let workspace = Arc::new(fixture::design());
    let (clients, _) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let id = ChannelId::new("payments-rewrite");
    let mut state = AppState::launch(&clients);
    state.workspace = workspace;
    state.route = Route::Channel(id.clone());
    let mut store = TestStore::new(state, App::new(clients.clone()));
    store.send(A::Channel(Action::Fold(SessionId::new("ledger"))), |s| {
        s.channel_pages
            .entry(id.clone())
            .or_default()
            .folded
            .insert(SessionId::new("ledger"));
    });
    let rows = channel::rows(
        &store.state().workspace,
        &id,
        &store.state().channel_pages[&id],
    );
    assert_eq!(
        rows.iter()
            .map(|r| r.session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["ledger", "refund", "rate"]
    );
    store.send(A::Channel(Action::Filter("PARTIAL".into())), |s| {
        s.channel_pages.get_mut(&id).unwrap().query = "PARTIAL".into()
    });
    let rows = channel::rows(
        &store.state().workspace,
        &id,
        &store.state().channel_pages[&id],
    );
    assert_eq!(
        rows.iter()
            .map(|r| r.session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["ledger", "brin", "partial-brin"]
    );
    store.send(A::Channel(Action::Tab(Tab::Activity)), |s| {
        s.channel_pages.get_mut(&id).unwrap().tab = Tab::Activity
    });
    saved::Saver::new(clients.storage.clone())
        .save(1, &store.state().saved())
        .expect("save channel view");
    assert_eq!(
        AppState::launch(&clients).channel_pages,
        store.state().channel_pages
    );
    store.finish();
}

#[test]
fn archived_and_clone_filters_stay_in_their_channel_with_tree_context() {
    let mut workspace = fixture::design();
    let id = ChannelId::new("payments-rewrite");
    let partial = workspace
        .sessions
        .iter_mut()
        .find(|s| s.id.as_str() == "partial-brin")
        .unwrap();
    partial.archived = true;
    // The clone filter below picks gpu-box's clone, so this test puts one
    // session there itself rather than lean on where the fixture puts it.
    partial.machine = MachineId::new("gpu-box");
    let rows = channel::rows(
        &workspace,
        &id,
        &View {
            history: History::Archived,
            ..Default::default()
        },
    );
    assert_eq!(
        rows.iter()
            .map(|r| r.session.id.as_str())
            .collect::<Vec<_>>(),
        vec!["ledger", "brin", "partial-brin"]
    );
    let rows = channel::rows(
        &workspace,
        &id,
        &View {
            history: History::All,
            clones: Some((MachineId::new("gpu-box"), "acme/ledger".into())),
            ..Default::default()
        },
    );
    assert!(rows.iter().all(|r| r.session.channel == id));
    assert_eq!(
        rows.iter()
            .filter(|r| r.session.machine.as_str() == "gpu-box")
            .count(),
        1
    );
    let ledger = rows
        .iter()
        .find(|r| r.session.id.as_str() == "ledger")
        .unwrap();
    // Its own Move PAY-3 plus its fork's Push try-brin (frames 01–06).
    assert_eq!(ledger.pending, 2);
}
