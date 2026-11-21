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

#[test]
fn repeated_events_ended_backs_off_and_toasts_once_per_outage() {
    // SMB-003: while the daemon is down, repeated EventsEnded(Failed) must
    // back off via the clock instead of spinning in a tight loop, and toast
    // only once per outage.
    let clock = Arc::new(TestClock::new());
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), clock.clone());
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    let err = || humata2_clients::DaemonError::Offline {
        machine: humata2_clients::model::MachineId::new("this-mac"),
    };
    // First disconnect: toasts once, schedules immediate reconnect.
    let _hold = fakes.daemon.hold_next_subscribe();
    store.send(A::EventsEnded(StreamEnd::Failed(err())), |_| {});
    let toasts_after_first = store.state().toasts.len();
    assert_eq!(toasts_after_first, 1, "first disconnect shows one toast");

    // Second failure while still disconnected: must NOT add a second toast.
    store.send(A::EventsEnded(StreamEnd::Failed(err())), |_| {});
    assert_eq!(
        store.state().toasts.len(),
        1,
        "repeated reconnect failures must not spam duplicate toasts"
    );
    store.finish();
}

#[test]
fn resync_restarts_pinned_watch() {
    // SMB-032: after a daemon restart (Event::Resync), Pinned must be started
    // again so pinned items keep updating without relaunching the app.
    let clock = Arc::new(TestClock::new());
    let (clients, _fakes) = humata2_clients::fake_clients(fixture::design(), clock);
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();

    store.send(A::Event(humata2_clients::Event::Resync), |_| {});
    store.receive_matching(
        "Resync reloads workspace",
        |a| matches!(a, A::Loaded(Ok(_))),
        |_| {},
    );
    store.receive_matching(
        "Resync restarts pinned watch",
        |a| matches!(a, A::Pinned(_)),
        |_| {},
    );
    store.settle();
    store.skip_received();
    store.finish();
}
