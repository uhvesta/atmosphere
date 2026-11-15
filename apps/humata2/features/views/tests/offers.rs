//! VW-4/5: Later hides until return; Not now stays dismissed.
use humata2_clients::{fixture, model::SessionId};
use humata2_views::{Views, ViewsAction as A, ViewsState};
use std::sync::Arc;
use tca::TestStore;

#[test]
fn returning_shows_a_deferred_offer_but_never_a_dismissed_one() {
    let workspace = Arc::new(fixture::design());
    let title = "Review: ledger schema".to_owned();
    let mut store = TestStore::new(
        ViewsState {
            workspace,
            ..Default::default()
        },
        Views,
    );
    assert_eq!(store.state().offers().len(), 1);
    store.send(A::Later(title.clone()), |s| {
        s.later.insert(title.clone());
    });
    assert!(store.state().offers().is_empty());
    store.send(A::Return(SessionId::new("ledger")), |s| s.later.clear());
    assert_eq!(store.state().offers().len(), 1);
    store.send(A::Dismiss(title.clone()), |s| {
        s.decisions.dismissed.insert(title.clone());
    });
    store.send(A::Return(SessionId::new("ledger")), |_| {});
    assert!(store.state().offers().is_empty());
    store.finish();
}
