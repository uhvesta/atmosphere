use std::path::PathBuf;

use humata2_store::{Store, TabKind, TabRecord, TabReplies};

fn tab(id: &str) -> TabRecord {
    TabRecord {
        id: id.to_owned(),
        session: None,
        kind: TabKind::Shell,
        title: "zsh".to_owned(),
        holder_socket: PathBuf::from(format!("/tmp/{id}.sock")),
        holder_pid: 1,
        created_unix_ms: 1,
        holder_generation: 2,
    }
}

fn record() -> TabReplies {
    TabReplies {
        generation: 2,
        answered_through: 4096,
        pending: vec![3, 4],
        uncertain: true,
    }
}

#[test]
fn a_reply_record_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_tab(&tab("tb_a")).unwrap();
        store.put_tab_replies("tb_a", &record()).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.tab_replies("tb_a").unwrap(), Some(record()));
}

#[test]
fn a_later_record_replaces_the_earlier_and_an_empty_pending_list_reads_back_empty() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_tab(&tab("tb_a")).unwrap();
    store.put_tab_replies("tb_a", &record()).unwrap();
    let later = TabReplies {
        generation: 2,
        answered_through: 9000,
        pending: Vec::new(),
        uncertain: false,
    };
    store.put_tab_replies("tb_a", &later).unwrap();
    assert_eq!(store.tab_replies("tb_a").unwrap(), Some(later));
}

#[test]
fn a_tab_that_never_had_a_record_reads_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_tab(&tab("tb_a")).unwrap();
    assert_eq!(store.tab_replies("tb_a").unwrap(), None);
    assert_eq!(store.tab_replies("tb_missing").unwrap(), None);
}

#[test]
fn a_tab_saved_by_an_older_build_reads_none() {
    let dir = tempfile::tempdir().unwrap();
    drop(Store::open(dir.path()).unwrap());
    // A row with only the columns an older schema had given.
    rusqlite::Connection::open(dir.path().join("state.sqlite"))
        .unwrap()
        .execute(
            "INSERT INTO tabs (id, session, kind, title, holder_socket, holder_pid, created_unix_ms)
             VALUES ('tb_old', NULL, 'shell', 'zsh', ?1, 9, 1)",
            [b"/tmp/tb_old.sock".as_slice()],
        )
        .unwrap();
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.tab_replies("tb_old").unwrap(), None);
}

#[test]
fn saving_the_tab_again_keeps_its_record_and_removing_it_removes_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_tab(&tab("tb_a")).unwrap();
    store.put_tab_replies("tb_a", &record()).unwrap();

    let mut renamed = tab("tb_a");
    renamed.title = "fish".to_owned();
    store.put_tab(&renamed).unwrap();
    assert_eq!(store.tab_replies("tb_a").unwrap(), Some(record()));

    store.remove_tab("tb_a").unwrap();
    assert_eq!(store.tab_replies("tb_a").unwrap(), None);
    // Saving for a tab that is gone is not an error and keeps nothing.
    store.put_tab_replies("tb_a", &record()).unwrap();
    assert_eq!(store.tab_replies("tb_a").unwrap(), None);
}
