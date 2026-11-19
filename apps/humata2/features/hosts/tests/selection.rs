//! A host selection follows loaded machines and ignores stale links.
use humata2_clients::{Clients, fixture, model::MachineId};
use humata2_hosts::{Hosts, HostsAction as A, HostsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};
#[test]
fn selected_host_survives_refresh_and_falls_back_when_removed() {
    let workspace = Arc::new(fixture::design());
    let (clients, _) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(HostsState::default(), Hosts::new(clients));
    store.send(A::Show(workspace.clone()), |s| {
        s.workspace = workspace.clone();
        s.selected = Some(MachineId::new("this-mac"));
    });
    // The fake's default detail is an error, which leaves the state as it was;
    // the two online machines' holder reads find no orphans (D784, frame 28).
    for _ in 0..3 {
        store.receive_matching("Health or Holders", |a| matches!(a, A::Health(..) | A::Holders(..)), |_| {});
    }
    store.send(A::Select(MachineId::new("devbox-1")), |s| {
        s.selected = Some(MachineId::new("devbox-1"))
    });
    store.receive_matching("Health", |a| matches!(a, A::Health(..)), |_| {});
    store.send(A::Select(MachineId::new("missing")), |_| {});
    store.send(A::Show(workspace.clone()), |_| {});
    let mut removed = (*workspace).clone();
    removed.machines.retain(|m| m.id.as_str() != "devbox-1");
    let removed = Arc::new(removed);
    store.send(A::Show(removed.clone()), |s| {
        s.workspace = removed.clone();
        s.selected = Some(MachineId::new("this-mac"));
    });
    store.receive_matching("Health", |a| matches!(a, A::Health(..)), |_| {});
    store.finish();
}
