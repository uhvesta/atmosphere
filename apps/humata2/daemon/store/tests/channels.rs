use std::path::PathBuf;

use humata2_store::{
    ChannelRecord, ChannelRepo, MainClone, SessionDetail, SessionRecord, SessionRepo, StartStep,
    Store, StoreError,
};

fn repo(owner: &str, name: &str) -> ChannelRepo {
    ChannelRepo {
        owner: owner.to_owned(),
        name: name.to_owned(),
        main: MainClone::Cloning,
    }
}

fn channel(id: &str, created: i64) -> ChannelRecord {
    ChannelRecord {
        id: id.to_owned(),
        name: format!("Channel {id}"),
        folder: format!("/work/{id}"),
        branch_prefix: Some("ava/".to_owned()),
        default_agent: "claude".to_owned(),
        repos: vec![repo("zed", "web"), repo("acme", "api")],
        machines: vec!["ma_2".to_owned(), "ma_1".to_owned()],
        revision: 1,
        created_unix_ms: created,
    }
}

fn session(id: &str) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Fix".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work/ch_1"),
        created_unix_ms: 1,
    }
}

fn detail(session: &str) -> SessionDetail {
    SessionDetail {
        session: session.to_owned(),
        repos: vec![
            SessionRepo {
                owner: "zed".to_owned(),
                name: "web".to_owned(),
                branch: "ava/fix".to_owned(),
                path: PathBuf::from("/work/ch_1/web"),
            },
            SessionRepo {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
                branch: "ava/fix".to_owned(),
                path: PathBuf::from("/work/ch_1/api"),
            },
        ],
        model: Some("opus".to_owned()),
        first_message: Some("hello".to_owned()),
        start: StartStep::Cloned,
        operation: Some("op_1".to_owned()),
        revision: 3,
    }
}

#[test]
fn a_channel_round_trips_with_repos_and_machines_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.channel("ch_1").unwrap(), None);
    let saved = channel("ch_1", 5);
    store.put_channel(&saved).unwrap();
    assert_eq!(store.channel("ch_1").unwrap(), Some(saved));
}

#[test]
fn saving_a_channel_again_replaces_its_repos_and_machines() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_channel(&channel("ch_1", 5)).unwrap();
    let mut changed = channel("ch_1", 5);
    changed.repos = vec![repo("acme", "docs")];
    changed.machines = vec!["ma_3".to_owned()];
    changed.revision = 2;
    store.put_channel(&changed).unwrap();
    assert_eq!(store.channel("ch_1").unwrap(), Some(changed));
}

#[test]
fn set_main_clone_changes_one_repo_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_channel(&channel("ch_1", 5)).unwrap();
    store
        .set_main_clone(
            "ch_1",
            "acme",
            "api",
            &MainClone::Failed {
                reason: "no network".to_owned(),
            },
        )
        .unwrap();
    let got = store.channel("ch_1").unwrap().unwrap();
    assert_eq!(got.repos[0].main, MainClone::Cloning);
    assert_eq!(
        got.repos[1].main,
        MainClone::Failed {
            reason: "no network".to_owned()
        }
    );
    store
        .set_main_clone("ch_1", "acme", "api", &MainClone::Ready)
        .unwrap();
    assert_eq!(
        store.channel("ch_1").unwrap().unwrap().repos[1].main,
        MainClone::Ready
    );
}

#[test]
fn set_main_clone_of_an_unknown_channel_or_repo_errors_apart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(matches!(
        store.set_main_clone("ch_x", "acme", "api", &MainClone::Ready),
        Err(StoreError::NoChannel { .. })
    ));
    store.put_channel(&channel("ch_1", 5)).unwrap();
    assert!(matches!(
        store.set_main_clone("ch_1", "acme", "nope", &MainClone::Ready),
        Err(StoreError::NoChannelRepo { .. })
    ));
}

#[test]
fn renaming_a_channel_changes_its_name_and_revision_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_channel(&channel("ch_1", 5)).unwrap();
    store
        .set_main_clone("ch_1", "acme", "api", &MainClone::Ready)
        .unwrap();
    let before = store.channel("ch_1").unwrap().unwrap();
    let renamed = store.rename_channel("ch_1", "New name", Some(1)).unwrap();
    assert_eq!(renamed.name, "New name");
    assert_eq!(renamed.revision, 2);
    assert_eq!(store.channel("ch_1").unwrap(), Some(renamed.clone()));
    assert_eq!(renamed.repos, before.repos);
    assert_eq!(renamed.machines, before.machines);
    assert_eq!(renamed.folder, before.folder);
    // Without an expected revision it applies to whatever is saved.
    assert_eq!(
        store
            .rename_channel("ch_1", "Again", None)
            .unwrap()
            .revision,
        3
    );
}

#[test]
fn renaming_against_a_stale_revision_is_refused_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_channel(&channel("ch_1", 5)).unwrap();
    let saved = store.channel("ch_1").unwrap();
    assert!(matches!(
        store.rename_channel("ch_1", "New name", Some(7)),
        Err(StoreError::StaleChannel {
            expected: 7,
            actual: 1,
            ..
        })
    ));
    assert_eq!(store.channel("ch_1").unwrap(), saved);
}

#[test]
fn renaming_an_unknown_channel_errors_apart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(matches!(
        store.rename_channel("ch_x", "Name", None),
        Err(StoreError::NoChannel { .. })
    ));
}

#[test]
fn channels_list_oldest_first_then_by_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_channel(&channel("ch_c", 1)).unwrap();
    store.put_channel(&channel("ch_b", 2)).unwrap();
    store.put_channel(&channel("ch_a", 2)).unwrap();
    let ids: Vec<_> = store
        .channels()
        .unwrap()
        .into_iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(ids, ["ch_c", "ch_a", "ch_b"]);
}

#[test]
fn removing_a_channel_removes_it_and_is_repeatable() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_channel(&channel("ch_1", 1)).unwrap();
    store.put_channel(&channel("ch_2", 2)).unwrap();
    store.remove_channel("ch_1").unwrap();
    store.remove_channel("ch_1").unwrap();
    assert_eq!(store.channel("ch_1").unwrap(), None);
    assert_eq!(store.channels().unwrap().len(), 1);
}

#[test]
fn a_session_detail_round_trips_and_a_rewrite_replaces_its_repos() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1")).unwrap();
    assert_eq!(store.session_detail("se_1").unwrap(), None);
    store.put_session_detail(&detail("se_1")).unwrap();
    assert_eq!(store.session_detail("se_1").unwrap(), Some(detail("se_1")));

    let mut next = detail("se_1");
    next.repos.truncate(1);
    next.start = StartStep::Done;
    next.operation = None;
    next.model = None;
    store.put_session_detail(&next).unwrap();
    assert_eq!(store.session_detail("se_1").unwrap(), Some(next.clone()));
    assert_eq!(store.session_details().unwrap(), vec![next]);
}

#[test]
fn a_session_detail_needs_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert!(matches!(
        store.put_session_detail(&detail("se_missing")),
        Err(StoreError::NoSession { .. })
    ));
    assert_eq!(store.session_detail("se_missing").unwrap(), None);
}

#[test]
fn removing_a_session_removes_its_detail() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_1")).unwrap();
    store.put_session_detail(&detail("se_1")).unwrap();
    store.remove_session("se_1").unwrap();
    assert_eq!(store.session_detail("se_1").unwrap(), None);
    assert_eq!(store.session_details().unwrap(), Vec::new());
}

#[test]
fn everything_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_channel(&channel("ch_1", 1)).unwrap();
        store
            .set_main_clone("ch_1", "zed", "web", &MainClone::Ready)
            .unwrap();
        store.put_session(&session("se_1")).unwrap();
        store.put_session_detail(&detail("se_1")).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    let mut expected = channel("ch_1", 1);
    expected.repos[0].main = MainClone::Ready;
    assert_eq!(store.channels().unwrap(), vec![expected]);
    assert_eq!(store.session_detail("se_1").unwrap(), Some(detail("se_1")));
}
