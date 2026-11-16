use humata2_store::{
    BoardHistoryKind, BoardItemChange, BoardKey, BoardWrite, NewBoardItem, PrefixClaim, Store,
    StoreError,
};

fn new_item(board: BoardKey, channel: &str, title: &str) -> NewBoardItem {
    NewBoardItem {
        board,
        channel: channel.to_owned(),
        title: title.to_owned(),
        description: String::new(),
        column: 1,
        rank: "m".to_owned(),
        created_by: String::new(),
        at_unix_ms: 10,
        sessions: Vec::new(),
        depends_on: Vec::new(),
    }
}

fn change(key: &str) -> BoardItemChange {
    BoardItemChange {
        key: key.to_owned(),
        expected_revision: None,
        by: String::new(),
        at_unix_ms: 20,
        title: None,
        description: None,
        column: None,
        rank: None,
        sessions: None,
        depends_on: None,
        delete: false,
        cause: String::new(),
    }
}

fn open() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.claim_board_prefix("ch_1", "PAY").unwrap(), PrefixClaim::Claimed);
    (dir, store)
}

fn make(store: &Store, title: &str) -> String {
    store.create_board_item(&new_item(BoardKey::Channel("ch_1".into()), "ch_1", title)).unwrap().key
}

#[test]
fn keys_count_up_and_are_never_reused_after_a_delete() {
    let (_dir, store) = open();
    assert_eq!(make(&store, "a"), "PAY-1");
    assert_eq!(make(&store, "b"), "PAY-2");
    let mut gone = change("PAY-2");
    gone.delete = true;
    assert!(matches!(store.change_board_item(&gone).unwrap(), BoardWrite::Applied(_)));
    assert_eq!(make(&store, "c"), "PAY-3");
}

#[test]
fn an_owner_without_a_prefix_cannot_make_items() {
    let (_dir, store) = open();
    let error = store
        .create_board_item(&new_item(BoardKey::Channel("ch_2".into()), "ch_2", "x"))
        .unwrap_err();
    assert!(matches!(error, StoreError::NoBoardPrefix { .. }));
}

#[test]
fn a_prefix_belongs_to_one_owner() {
    let (_dir, store) = open();
    assert_eq!(store.claim_board_prefix("ch_2", "PAY").unwrap(), PrefixClaim::TakenByAnother);
    assert_eq!(store.claim_board_prefix("ch_2", "PAY2").unwrap(), PrefixClaim::Claimed);
    assert_eq!(
        store.claim_board_prefix("ch_1", "OTHER").unwrap(),
        PrefixClaim::AlreadyYours("PAY".into())
    );
    assert_eq!(store.board_prefix("ch_1").unwrap().as_deref(), Some("PAY"));
    assert_eq!(store.board_prefix("ch_9").unwrap(), None);
}

#[test]
fn listings_pick_the_right_boards_in_order_and_skip_tombstones() {
    let (_dir, store) = open();
    store.claim_board_prefix("global", "GLB").unwrap();
    let global = store.create_board_item(&new_item(BoardKey::Global, "", "g")).unwrap();
    assert_eq!(global.key, "GLB-1");
    let mut late = new_item(BoardKey::Channel("ch_1".into()), "ch_1", "late");
    late.rank = "z".into();
    let late = store.create_board_item(&late).unwrap();
    let early = make(&store, "early");
    let session =
        store.create_board_item(&new_item(BoardKey::Session("se_1".into()), "ch_1", "s")).unwrap();
    let dead = make(&store, "dead");
    let mut gone = change(&dead);
    gone.delete = true;
    store.change_board_item(&gone).unwrap();

    let keys = |items: Vec<humata2_store::BoardItemRecord>| {
        items.into_iter().map(|item| item.key).collect::<Vec<_>>()
    };
    assert_eq!(
        keys(store.board_items(&[BoardKey::Channel("ch_1".into())]).unwrap()),
        [early.clone(), late.key.clone()]
    );
    assert_eq!(keys(store.board_items(&[BoardKey::Session("se_1".into())]).unwrap()), [session.key.clone()]);
    assert_eq!(keys(store.board_items(&[BoardKey::Global]).unwrap()), ["GLB-1"]);
    assert_eq!(
        keys(store.board_items_in_channel("ch_1").unwrap()),
        [early.clone(), session.key.clone(), late.key.clone()]
    );
    assert_eq!(
        keys(store.all_board_items_but_sessions().unwrap()),
        ["GLB-1".to_owned(), early, late.key]
    );
    assert!(store.board_item(&dead).unwrap().unwrap().deleted);
}

#[test]
fn a_stale_revision_loses_when_its_field_changed_since() {
    let (_dir, store) = open();
    let key = make(&store, "a");
    let mut first = change(&key);
    first.title = Some("b".into());
    store.change_board_item(&first).unwrap();
    let mut stale = change(&key);
    stale.expected_revision = Some(1);
    stale.title = Some("c".into());
    let BoardWrite::Stale { current } = store.change_board_item(&stale).unwrap() else {
        panic!("expected stale");
    };
    assert_eq!((current.title.as_str(), current.revision), ("b", 2));
    assert_eq!(store.board_item(&key).unwrap().unwrap(), current);
    let history = store.board_item_history(&key, 0, 10).unwrap();
    let last = history.last().unwrap();
    assert!(last.lost);
    assert_eq!((last.before.as_str(), last.after.as_str()), ("b", "c"));
}

#[test]
fn a_stale_revision_applies_when_only_other_fields_changed() {
    let (_dir, store) = open();
    let key = make(&store, "a");
    let mut first = change(&key);
    first.description = Some("notes".into());
    first.sessions = Some(vec!["se_1".into()]);
    store.change_board_item(&first).unwrap();
    let mut stale = change(&key);
    stale.expected_revision = Some(1);
    stale.column = Some(3);
    let BoardWrite::Applied(item) = store.change_board_item(&stale).unwrap() else {
        panic!("expected applied");
    };
    assert_eq!((item.column, item.description.as_str(), item.revision), (3, "notes", 3));
    assert_eq!(item.sessions, vec!["se_1".to_owned()]);
}

#[test]
fn history_names_the_session_that_caused_a_change() {
    let (_dir, store) = open();
    let key = make(&store, "a");
    let mut start = change(&key);
    start.column = Some(3);
    start.cause = "se_9".into();
    store.change_board_item(&start).unwrap();
    let history = store.board_item_history(&key, 0, 10).unwrap();
    assert_eq!(history.last().unwrap().cause, "se_9");
    assert_eq!(history.first().unwrap().cause, "");
}

#[test]
fn a_write_without_a_revision_wins_and_history_keeps_the_old_value() {
    let (_dir, store) = open();
    let key = make(&store, "old");
    let mut edit = change(&key);
    edit.title = Some("new".into());
    edit.by = "se_1".into();
    let BoardWrite::Applied(item) = store.change_board_item(&edit).unwrap() else {
        panic!("expected applied");
    };
    assert_eq!((item.title.as_str(), item.revision), ("new", 2));
    let history = store.board_item_history(&key, 1, 10).unwrap();
    assert_eq!(history.len(), 1);
    let entry = &history[0];
    assert_eq!(entry.kind, BoardHistoryKind::FieldChanged);
    assert_eq!((entry.before.as_str(), entry.after.as_str(), entry.by.as_str()), ("old", "new", "se_1"));
    assert!(!entry.lost);
}

#[test]
fn a_change_that_changes_nothing_keeps_the_revision() {
    let (_dir, store) = open();
    let key = make(&store, "a");
    let mut same = change(&key);
    same.title = Some("a".into());
    let BoardWrite::Applied(item) = store.change_board_item(&same).unwrap() else {
        panic!("expected applied");
    };
    assert_eq!(item.revision, 1);
    assert_eq!(store.board_item_history(&key, 0, 10).unwrap().len(), 1);
}

#[test]
fn a_change_to_a_deleted_item_is_kept_as_lost_and_does_not_revive_it() {
    let (_dir, store) = open();
    let key = make(&store, "a");
    let mut gone = change(&key);
    gone.delete = true;
    store.change_board_item(&gone).unwrap();
    let mut edit = change(&key);
    edit.title = Some("back".into());
    let BoardWrite::Gone { tombstone } = store.change_board_item(&edit).unwrap() else {
        panic!("expected gone");
    };
    assert!(tombstone.deleted);
    let item = store.board_item(&key).unwrap().unwrap();
    assert!(item.deleted);
    assert_eq!(item.title, "a");
    let history = store.board_item_history(&key, 0, 10).unwrap();
    let last = history.last().unwrap();
    assert!(last.lost);
    assert_eq!(last.after, "back");
}

#[test]
fn a_stale_delete_leaves_the_item_and_is_kept_as_lost() {
    let (_dir, store) = open();
    let key = make(&store, "a");
    let mut edit = change(&key);
    edit.title = Some("b".into());
    store.change_board_item(&edit).unwrap();
    let mut gone = change(&key);
    gone.delete = true;
    gone.expected_revision = Some(1);
    assert!(matches!(store.change_board_item(&gone).unwrap(), BoardWrite::Stale { .. }));
    assert!(!store.board_item(&key).unwrap().unwrap().deleted);
    let last = store.board_item_history(&key, 0, 10).unwrap().pop().unwrap();
    assert!(last.lost);
    assert_eq!(last.kind, BoardHistoryKind::Deleted);
}

#[test]
fn a_dependency_loop_is_refused_and_named() {
    let (_dir, store) = open();
    let a = make(&store, "a");
    let b = make(&store, "b");
    let mut first = change(&a);
    first.depends_on = Some(vec![b.clone()]);
    assert!(matches!(store.change_board_item(&first).unwrap(), BoardWrite::Applied(_)));
    let mut closing = change(&b);
    closing.depends_on = Some(vec![a.clone()]);
    let BoardWrite::Loop { cycle } = store.change_board_item(&closing).unwrap() else {
        panic!("expected loop");
    };
    assert_eq!(cycle, [b.clone(), a.clone(), b.clone()]);
    let item = store.board_item(&b).unwrap().unwrap();
    assert_eq!((item.revision, item.depends_on.len()), (1, 0));

    let mut itself = change(&a);
    itself.depends_on = Some(vec![a.clone()]);
    let BoardWrite::Loop { cycle } = store.change_board_item(&itself).unwrap() else {
        panic!("expected loop");
    };
    assert_eq!(cycle, [a.clone(), a]);
}

#[test]
fn a_missing_dependency_is_refused() {
    let (_dir, store) = open();
    let a = make(&store, "a");
    let mut edit = change(&a);
    edit.depends_on = Some(vec!["PAY-99".into()]);
    assert!(matches!(
        store.change_board_item(&edit).unwrap(),
        BoardWrite::MissingDependency { key } if key == "PAY-99"
    ));
    let mut with = new_item(BoardKey::Channel("ch_1".into()), "ch_1", "x");
    with.depends_on = vec!["PAY-99".into()];
    assert!(matches!(store.create_board_item(&with), Err(StoreError::NoBoardItem { .. })));
}

#[test]
fn deleting_an_item_drops_the_edges_to_it_and_tells_its_dependents() {
    let (_dir, store) = open();
    let a = make(&store, "a");
    let b = make(&store, "b");
    let mut link = change(&a);
    link.depends_on = Some(vec![b.clone()]);
    store.change_board_item(&link).unwrap();
    let mut gone = change(&b);
    gone.delete = true;
    gone.by = "se_1".into();
    store.change_board_item(&gone).unwrap();
    assert!(store.board_item(&a).unwrap().unwrap().depends_on.is_empty());
    let last = store.board_item_history(&a, 0, 10).unwrap().pop().unwrap();
    assert_eq!(last.kind, BoardHistoryKind::DependenciesChanged);
    assert_eq!((last.before, last.after, last.by), (b, String::new(), "se_1".to_owned()));
}

#[test]
fn sessions_keep_their_order_and_new_ones_go_last() {
    let (_dir, store) = open();
    let mut item = new_item(BoardKey::Channel("ch_1".into()), "ch_1", "a");
    item.sessions = vec!["se_1".into(), "se_2".into()];
    let key = store.create_board_item(&item).unwrap().key;
    let mut edit = change(&key);
    edit.sessions = Some(vec!["se_3".into(), "se_2".into()]);
    let BoardWrite::Applied(item) = store.change_board_item(&edit).unwrap() else {
        panic!("expected applied");
    };
    assert_eq!(item.sessions, ["se_2", "se_3"]);
}

#[test]
fn moving_into_done_sets_the_done_time_and_out_clears_it() {
    let (_dir, store) = open();
    let key = make(&store, "a");
    let mut done = change(&key);
    done.column = Some(5);
    done.at_unix_ms = 77;
    let BoardWrite::Applied(item) = store.change_board_item(&done).unwrap() else {
        panic!("expected applied");
    };
    assert_eq!(item.done_unix_ms, Some(77));
    let mut back = change(&key);
    back.column = Some(3);
    let BoardWrite::Applied(item) = store.change_board_item(&back).unwrap() else {
        panic!("expected applied");
    };
    assert_eq!((item.done_unix_ms, item.column), (None, 3));
}

#[test]
fn everything_survives_reopening() {
    let (dir, store) = open();
    let key = make(&store, "a");
    let mut edit = change(&key);
    edit.title = Some("b".into());
    store.change_board_item(&edit).unwrap();
    let before = store.board_item(&key).unwrap();
    let history = store.board_item_history(&key, 0, 10).unwrap();
    drop(store);
    let again = Store::open(dir.path()).unwrap();
    assert_eq!(again.board_item(&key).unwrap(), before);
    assert_eq!(again.board_item_history(&key, 0, 10).unwrap(), history);
    assert_eq!(make(&again, "c"), "PAY-2");
}
