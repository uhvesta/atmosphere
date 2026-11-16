//! The item drawer (frame 22, BD-3, BD-14–16, D177): the ask in place,
//! history read on open, and the one picker that adds sessions and
//! dependencies.
use humata2_board::{Ask, Board, BoardAction as A, BoardState, Choice, Delegate, Picker, PickerKind};
use humata2_clients::{
    Clients, Fakes, fixture,
    model::{
        ChannelId, FieldChange, Item, ItemChange, ItemId, ItemKind, ItemStatus as S, Scope,
        SessionId, Workspace,
    },
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store_with(workspace: Workspace) -> (TestStore<Board>, Fakes) {
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let store = TestStore::new(
        BoardState {
            workspace: Arc::new(workspace),
            ..Default::default()
        },
        Board::new(clients),
    );
    (store, fakes)
}

fn id(text: &str) -> ItemId {
    ItemId::new(text)
}

/// Opens `item`'s drawer and takes the history read it starts.
fn open(store: &mut TestStore<Board>, item: &str) {
    store.send(A::Open(id(item)), |s| s.drawer = Some(id(item)));
    store.receive_matching(
        "the item's history",
        |a| matches!(a, A::History(found, Ok(_)) if found == &id(item)),
        |s| {
            s.history = s
                .workspace
                .item_details
                .get(&id(item))
                .map(|d| d.history.clone())
                .unwrap_or_default();
        },
    );
}

fn item(store: &TestStore<Board>, item: &str) -> Item {
    store
        .state()
        .workspace
        .items
        .iter()
        .find(|i| i.id == id(item))
        .cloned()
        .expect("the item is on the board")
}

#[test]
fn the_ask_names_the_change_it_waits_on() {
    let mut workspace = fixture::design();
    let (store, _) = store_with(workspace.clone());
    let pay3 = item(&store, "PAY-3");
    let Some(Ask::Move { to, keep, .. }) = store.state().ask(&pay3) else {
        panic!("PAY-3's waiting move shows as a move, got {:?}", store.state().ask(&pay3))
    };
    assert_eq!((to, keep), (S::Done, S::InProgress));
    assert_eq!(store.state().ask(&item(&store, "PAY-7")), None, "no request, no ask");

    let edit = vec![FieldChange {
        field: "Title".into(),
        before: "Ledger schema v2".into(),
        after: "Ledger schema v3".into(),
    }];
    let details = workspace.item_details.get_mut(&id("PAY-3")).expect("PAY-3 details");
    details.requested_status = None;
    details.requested_change = Some(ItemChange::Edit(edit.clone()));
    let (store, _) = store_with(workspace.clone());
    assert!(
        matches!(store.state().ask(&pay3), Some(Ask::Edit { changes, .. }) if changes == edit),
        "an edit shows every changed field"
    );

    let details = workspace.item_details.get_mut(&id("PAY-3")).expect("PAY-3 details");
    details.requested_change = Some(ItemChange::Delete);
    let (store, _) = store_with(workspace.clone());
    assert!(matches!(store.state().ask(&pay3), Some(Ask::Delete { .. })));

    // Once the request is answered elsewhere, the ask goes.
    let request = workspace.item_details[&id("PAY-3")].request.clone().expect("a request");
    workspace.requests.retain(|r| r.id != request);
    let (store, _) = store_with(workspace);
    assert_eq!(store.state().ask(&pay3), None);
}

#[test]
fn opening_an_item_reads_its_history_and_late_reads_are_dropped() {
    let (mut store, _) = store_with(fixture::design());
    open(&mut store, "PAY-3");
    assert!(!store.state().history.is_empty());
    // A read for an item that's no longer open changes nothing.
    store.send(A::History(id("PAY-9"), Ok(vec!["Created by you".into()])), |_| {});
    store.send(A::Close, |s| {
        s.drawer = None;
        s.history = Vec::new();
    });
    store.finish();
}

#[test]
fn the_session_picker_offers_the_channels_other_sessions_and_stays_open() {
    let workspace = fixture::design();
    let channel = ChannelId::new("payments-rewrite");
    let (mut store, fakes) = store_with(workspace.clone());
    open(&mut store, "PAY-3");
    store.send(A::OpenPicker(PickerKind::Sessions), |s| {
        s.picker = Some(Picker {
            kind: PickerKind::Sessions,
            query: String::new(),
            cursor: 0,
        });
    });
    let working = &workspace.item_details[&id("PAY-3")].sessions;
    let expected: Vec<Choice> = workspace
        .sessions
        .iter()
        .filter(|s| s.channel == channel && !s.archived && !working.contains(&s.id))
        .map(|s| Choice::Session(s.id.clone()))
        .collect();
    assert!(expected.len() >= 2, "the fixture has spare sessions in the channel");
    assert_eq!(store.state().choices(), expected);

    // Arrows move, Enter adds the highlighted one, and the picker stays.
    store.send(A::PickerDown, |s| {
        if let Some(p) = &mut s.picker {
            p.cursor = 1;
        }
    });
    let Choice::Session(second) = expected[1].clone() else {
        panic!("sessions only")
    };
    store.send(A::PickerEnter, |_| {});
    store.receive(A::Done(Ok(())), |_| {});
    assert!(store.state().picker.is_some(), "the picker stays open for more adds");
    let after = fakes.daemon.workspace();
    assert!(after.item_details[&id("PAY-3")].sessions.contains(&second));

    // The filter narrows by title.
    let title = workspace
        .session(&second)
        .map(|s| s.title.clone())
        .expect("the session");
    store.send(A::PickerQuery(title.to_uppercase()), |s| {
        if let Some(p) = &mut s.picker {
            p.query = title.to_uppercase();
            p.cursor = 0;
        }
    });
    assert_eq!(store.state().choices(), vec![Choice::Session(second.clone())]);

    // Removing a session from the list goes to the daemon.
    store.send(A::RemoveSession(SessionId::new("brin")), |_| {});
    store.receive(A::Done(Ok(())), |_| {});
    assert!(!fakes.daemon.workspace().item_details[&id("PAY-3")]
        .sessions
        .contains(&SessionId::new("brin")));

    store.send(A::ClosePicker, |s| s.picker = None);
    store.finish();
}

#[test]
fn a_global_item_offers_every_session() {
    let mut workspace = fixture::design();
    workspace.items.push(Item {
        id: id("GLB-1"),
        scope: Scope::Global,
        kind: ItemKind::Task,
        title: "Quarterly plan".into(),
        status: S::Todo,
        depends_on: Vec::new(),
        session: None,
        parent: None,
    });
    let (mut store, _) = store_with(workspace.clone());
    open(&mut store, "GLB-1");
    store.send(A::OpenPicker(PickerKind::Sessions), |s| {
        s.picker = Some(Picker {
            kind: PickerKind::Sessions,
            query: String::new(),
            cursor: 0,
        });
    });
    let offered = store.state().choices();
    let every: Vec<_> = workspace
        .sessions
        .iter()
        .filter(|s| !s.archived)
        .map(|s| Choice::Session(s.id.clone()))
        .collect();
    assert_eq!(offered.len(), every.len());
    assert!(every.iter().all(|each| offered.contains(each)));
    store.finish();
}

#[test]
fn the_dependency_picker_leaves_out_itself_existing_edges_and_loops() {
    let mut workspace = fixture::design();
    // PAY-7 waits for PAY-3, so PAY-3 waiting for PAY-7 would close a loop.
    workspace
        .items
        .iter_mut()
        .find(|i| i.id == id("PAY-7"))
        .expect("PAY-7")
        .depends_on
        .push(id("PAY-3"));
    let (mut store, fakes) = store_with(workspace.clone());
    open(&mut store, "PAY-3");
    store.send(A::OpenPicker(PickerKind::Dependencies), |s| {
        s.picker = Some(Picker {
            kind: PickerKind::Dependencies,
            query: String::new(),
            cursor: 0,
        });
    });
    let offered: Vec<_> = store.state().choices();
    for left_out in ["PAY-3", "PAY-2", "PAY-7"] {
        assert!(
            !offered.contains(&Choice::Item(id(left_out))),
            "{left_out} is not offered"
        );
    }
    // Any board (BD-14): the infra channel's items are offered too.
    for offered_item in ["PAY-12", "PAY-9", "INF-4"] {
        assert!(offered.contains(&Choice::Item(id(offered_item))));
    }

    store.send(A::Pick(Choice::Item(id("INF-4"))), |_| {});
    store.receive(A::Done(Ok(())), |_| {});
    let pay3 = |w: &Workspace| {
        w.items
            .iter()
            .find(|i| i.id == id("PAY-3"))
            .map(|i| i.depends_on.clone())
            .unwrap_or_default()
    };
    assert!(pay3(&fakes.daemon.workspace()).contains(&id("INF-4")));
    assert!(store.state().picker.is_some());

    store.send(A::RemoveDependency(id("PAY-2")), |_| {});
    store.receive(A::Done(Ok(())), |_| {});
    assert!(!pay3(&fakes.daemon.workspace()).contains(&id("PAY-2")));
    store.finish();
}

#[test]
fn a_refused_edit_shows_why_refreshes_and_keeps_the_picker() {
    let mut workspace = fixture::design();
    workspace
        .items
        .iter_mut()
        .find(|i| i.id == id("PAY-7"))
        .expect("PAY-7")
        .depends_on
        .push(id("PAY-3"));
    let (store, fakes) = store_with(workspace.clone());
    let mut store = store.non_exhaustive();
    open(&mut store, "PAY-3");
    store.send(A::OpenPicker(PickerKind::Dependencies), |_| {});
    // Another edit closed the loop after the picker opened: the daemon
    // refuses, the board says why and asks for fresh data.
    store.send(A::Pick(Choice::Item(id("PAY-7"))), |_| {});
    store.receive_matching(
        "the refusal",
        |a| matches!(a, A::Done(Err(_))),
        |_| {},
    );
    store.receive(A::Delegate(Delegate::Refresh), |_| {});
    assert!(store.state().picker.is_some(), "the picker stays open");
    assert!(store.state().error.as_deref().is_some_and(|e| !e.is_empty()));
    let pay3 = fakes
        .daemon
        .workspace()
        .items
        .iter()
        .find(|i| i.id == id("PAY-3"))
        .map(|i| i.depends_on.clone())
        .unwrap_or_default();
    assert!(!pay3.contains(&id("PAY-7")));
    store.finish();
}

#[test]
fn closing_the_drawer_closes_the_picker() {
    let (mut store, _) = store_with(fixture::design());
    open(&mut store, "PAY-3");
    store.send(A::OpenPicker(PickerKind::Sessions), |s| {
        s.picker = Some(Picker {
            kind: PickerKind::Sessions,
            query: String::new(),
            cursor: 0,
        });
    });
    store.send(A::Close, |s| {
        s.drawer = None;
        s.picker = None;
        s.history = Vec::new();
    });
    store.finish();
}
