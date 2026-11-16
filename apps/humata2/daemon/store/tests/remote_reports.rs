//! Needs the `remote_session_reports` migration and `pub use remote_reports::RemoteReportRow`.

use humata2_store::{RemoteReportRow, Store};

fn row(machine: &str, session: &str, reported: i32) -> RemoteReportRow {
    RemoteReportRow {
        machine: machine.to_owned(),
        session: session.to_owned(),
        reported,
        exit_code: 0,
        problem: None,
        pointers: vec![1, 2, 3],
        pointers_at_ms: Some(10),
        reported_unix_ms: 20,
        last_seen_unix_ms: None,
    }
}

#[test]
fn a_report_is_replaced_by_the_next_one_and_survives_reopening() {
    let dir = tempfile::tempdir().unwrap();
    {
        let store = Store::open(dir.path()).unwrap();
        store.put_remote_report(&row("box-1", "se_1", 2)).unwrap();
        store.put_remote_report(&row("box-1", "se_1", 3)).unwrap();
        store.put_remote_report(&row("box-2", "se_1", 2)).unwrap();
    }
    let store = Store::open(dir.path()).unwrap();
    assert_eq!(
        store.remote_reports().unwrap(),
        vec![row("box-1", "se_1", 3), row("box-2", "se_1", 2)]
    );
}

#[test]
fn forgetting_one_session_keeps_the_others_and_a_missing_one_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_remote_report(&row("box-1", "se_1", 2)).unwrap();
    store.put_remote_report(&row("box-1", "se_2", 2)).unwrap();
    store.delete_remote_report("box-1", "se_1").unwrap();
    store.delete_remote_report("box-1", "se_9").unwrap();
    assert_eq!(store.remote_reports().unwrap(), vec![row("box-1", "se_2", 2)]);
}
