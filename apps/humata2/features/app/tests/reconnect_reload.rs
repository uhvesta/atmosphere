//! A reconnect never leaves the app on a failed load: the daemon the app
//! follows again says `Resync` once the subscription is up, and the app
//! loads again, even when the load that ran beside the subscribe hit a
//! daemon that was going away or not yet bound.

use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Load};
use humata2_clients::model::RequestId;
use humata2_clients::{Operation, fixture};
use tca::{StreamEnd, TestClock, TestStore};

#[test]
fn a_follow_that_lands_after_a_failed_load_ends_in_a_reload() {
    let clock = Arc::new(TestClock::new());
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), clock);
    fakes.daemon.resync_on_subscribe();
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    // The subscribe is held until the failed load has landed.
    let subscribe = fakes.daemon.hold_next_subscribe();
    fakes.daemon.fail_next(Operation::Load, "the daemon was not bound yet");
    store.send(A::EventsEnded(StreamEnd::Finished), |_| {});
    store.receive_matching(
        "the load hit the dying daemon",
        |a| matches!(a, A::Loaded(Err(_))),
        |state| assert!(matches!(state.load, Load::Failed(_))),
    );

    // What only the new daemon holds: the app sees it only by loading again.
    let mut fresh = fakes.daemon.workspace().requests[0].clone();
    fresh.id = RequestId::new("held-by-the-new-daemon");
    fresh.answer = None;
    fakes.daemon.put_request(fresh.clone());

    // The follow now lands; nothing but its Resync can reload.
    drop(subscribe);
    store.receive_matching(
        "the new subscription said to resync",
        |a| matches!(a, A::Event(humata2_clients::Event::Resync)),
        |_| {},
    );
    store.receive_matching(
        "the resync's load succeeded",
        |a| matches!(a, A::Loaded(Ok(_))),
        |_| {},
    );
    store.settle();
    store.skip_received();

    let state = store.state();
    assert_eq!(state.load, Load::Ready);
    assert!(
        state.workspace.requests.iter().any(|r| r.id == fresh.id),
        "the ready state is the resync's load, which saw the new daemon's data"
    );
}
