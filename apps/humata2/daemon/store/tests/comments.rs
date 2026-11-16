use std::path::PathBuf;

use humata2_store::{
    CommentChange, CommentDeliveryRecord, CommentDeliveryState, CommentNumbers, CommentRecord,
    SessionComments, SessionRecord, Store, StoreError,
};

fn session(id: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Fix the build".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work/web"),
        created_unix_ms: 1,
    }
}

fn comment(n: u64, state: CommentDeliveryState) -> CommentRecord {
    CommentRecord {
        id: format!("c-{n}"),
        archived: false,
        delivery_state: state,
        comment: vec![n as u8; 5],
        updated_unix_ms: n as i64,
    }
}

fn delivery(n: u64, state: CommentDeliveryState) -> CommentDeliveryRecord {
    CommentDeliveryRecord {
        key: format!("d-{n}"),
        state,
        delivery: vec![n as u8; 3],
    }
}

fn change(session: &str) -> CommentChange {
    CommentChange {
        session: session.to_owned(),
        put: Vec::new(),
        removed: Vec::new(),
        put_deliveries: Vec::new(),
        removed_deliveries: Vec::new(),
        numbers: CommentNumbers::default(),
    }
}

#[test]
fn a_change_round_trips_in_number_order_and_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a")).unwrap();
    assert_eq!(store.comments("se_a").unwrap(), SessionComments::default());

    let first = CommentChange {
        put: vec![
            comment(10, CommentDeliveryState::None),
            comment(2, CommentDeliveryState::Sending),
            comment(9, CommentDeliveryState::None),
        ],
        put_deliveries: vec![delivery(10, CommentDeliveryState::Sending), delivery(2, CommentDeliveryState::Unconfirmed)],
        numbers: CommentNumbers { last_comment: 10, last_delivery: 10 },
        ..change("se_a")
    };
    store.apply_comments(&first).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    let saved = store.comments("se_a").unwrap();
    assert_eq!(
        saved.comments,
        vec![
            comment(2, CommentDeliveryState::Sending),
            comment(9, CommentDeliveryState::None),
            comment(10, CommentDeliveryState::None)
        ]
    );
    assert_eq!(
        saved.deliveries,
        vec![delivery(2, CommentDeliveryState::Unconfirmed), delivery(10, CommentDeliveryState::Sending)]
    );
    assert_eq!(saved.numbers, CommentNumbers { last_comment: 10, last_delivery: 10 });
}

#[test]
fn a_second_change_touches_only_what_it_names() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a")).unwrap();
    store
        .apply_comments(&CommentChange {
            put: vec![
                comment(1, CommentDeliveryState::None),
                comment(2, CommentDeliveryState::None),
                comment(3, CommentDeliveryState::None),
            ],
            put_deliveries: vec![delivery(1, CommentDeliveryState::Sending)],
            numbers: CommentNumbers { last_comment: 3, last_delivery: 1 },
            ..change("se_a")
        })
        .unwrap();

    let archived = CommentRecord {
        archived: true,
        comment: vec![42],
        ..comment(2, CommentDeliveryState::None)
    };
    store
        .apply_comments(&CommentChange {
            put: vec![archived.clone()],
            removed: vec!["c-3".to_owned()],
            removed_deliveries: vec!["d-1".to_owned()],
            numbers: CommentNumbers { last_comment: 3, last_delivery: 1 },
            ..change("se_a")
        })
        .unwrap();

    let saved = store.comments("se_a").unwrap();
    assert_eq!(saved.comments, vec![comment(1, CommentDeliveryState::None), archived]);
    assert_eq!(saved.deliveries, Vec::new());
    assert_eq!(saved.numbers.last_comment, 3, "a removed comment's number is not reused");
}

#[test]
fn sessions_with_sending_comments_lists_only_those_with_a_sending_batch() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    for id in ["se_a", "se_b", "se_c"] {
        store.put_session(&session(id)).unwrap();
    }
    assert_eq!(store.sessions_with_sending_comments().unwrap(), Vec::<String>::new());
    let with = |id: &str, state| CommentChange {
        put_deliveries: vec![delivery(1, state)],
        ..change(id)
    };
    store.apply_comments(&with("se_a", CommentDeliveryState::Sending)).unwrap();
    store.apply_comments(&with("se_b", CommentDeliveryState::Unconfirmed)).unwrap();
    store
        .apply_comments(&CommentChange {
            put: vec![comment(1, CommentDeliveryState::None)],
            ..change("se_c")
        })
        .unwrap();
    assert_eq!(store.sessions_with_sending_comments().unwrap(), vec!["se_a".to_owned()]);
}

#[test]
fn comments_need_their_session_a_wellformed_id_and_go_with_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let refused = store.apply_comments(&CommentChange {
        put: vec![comment(1, CommentDeliveryState::None)],
        ..change("se_gone")
    });
    assert!(matches!(refused, Err(StoreError::NoSession { id }) if id == "se_gone"));

    store.put_session(&session("se_a")).unwrap();
    let odd = CommentRecord {
        id: "note-1".to_owned(),
        ..comment(1, CommentDeliveryState::None)
    };
    let refused = store.apply_comments(&CommentChange { put: vec![odd], ..change("se_a") });
    assert!(matches!(refused, Err(StoreError::InvalidId { .. })));
    assert_eq!(store.comments("se_a").unwrap(), SessionComments::default());

    store
        .apply_comments(&CommentChange {
            put: vec![comment(1, CommentDeliveryState::None)],
            put_deliveries: vec![delivery(1, CommentDeliveryState::Sending)],
            numbers: CommentNumbers { last_comment: 1, last_delivery: 1 },
            ..change("se_a")
        })
        .unwrap();
    store.remove_session("se_a").unwrap();
    assert_eq!(store.comments("se_a").unwrap(), SessionComments::default());
    assert_eq!(store.sessions_with_sending_comments().unwrap(), Vec::<String>::new());
}
