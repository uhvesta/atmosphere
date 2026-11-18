//! Moving a Desk session into a channel in the store (DK-15, DK-18, D305):
//! one commit files it under the channel, keeps who it is, drops its Desk
//! row and saves its repos on their new branches.

use std::path::PathBuf;

use humata2_store::{
    DeskFolderKind, DeskRow, SessionDetail, SessionRecord, SessionRepo, StartStep, Store,
    StoreError,
};

fn desk_session(id: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "desk".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Plan Q4".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/h/desk/plan-q4"),
        created_unix_ms: 1,
    }
}

fn scratch(id: &str) -> DeskRow {
    DeskRow {
        session_id: id.to_owned(),
        kind: DeskFolderKind::Scratch { kept: false },
        boards: true,
        channels_and_machines: false,
        last_used_unix_ms: 10,
        closed_unix_ms: None,
        deleted_unix_ms: None,
        held_work: String::new(),
    }
}

fn moved(id: &str) -> SessionRecord {
    SessionRecord {
        channel: "ch_pay".to_owned(),
        folder: PathBuf::from("/h/ch/payments/sessions/plan-q4"),
        ..desk_session(id)
    }
}

fn detail(id: &str) -> SessionDetail {
    let repo = |name: &str| SessionRepo {
        owner: "acme".to_owned(),
        name: name.to_owned(),
        branch: "me/plan-q4".to_owned(),
        path: PathBuf::from(format!("/h/ch/payments/sessions/plan-q4/{name}")),
    };
    SessionDetail {
        session: id.to_owned(),
        repos: vec![repo("api"), repo("web")],
        model: None,
        first_message: None,
        start: StartStep::Done,
        operation: None,
        revision: 0,
    }
}

#[test]
fn a_moved_session_is_filed_under_the_channel_with_its_repos_and_no_desk_row() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_session(&desk_session("se_1")).unwrap();
        store.put_desk(&scratch("se_1")).unwrap();
        store.desk_to_channel(&moved("se_1"), &detail("se_1")).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.session("se_1").unwrap(), Some(moved("se_1")));
    assert_eq!(store.desk("se_1").unwrap(), None);
    assert_eq!(store.session_detail("se_1").unwrap(), Some(detail("se_1")));
}

#[test]
fn moving_an_unknown_session_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&desk_session("se_1")).unwrap();
    store.put_desk(&scratch("se_1")).unwrap();

    let refused = store.desk_to_channel(&moved("se_9"), &detail("se_9"));

    assert!(matches!(refused, Err(StoreError::NoSession { .. })), "{refused:?}");
    assert_eq!(store.session("se_9").unwrap(), None);
    assert_eq!(store.session_detail("se_9").unwrap(), None);
    assert_eq!(store.session("se_1").unwrap(), Some(desk_session("se_1")));
    assert_eq!(store.desk("se_1").unwrap(), Some(scratch("se_1")));
}
