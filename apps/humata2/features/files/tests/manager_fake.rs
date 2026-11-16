//! The in-memory file manager behaves as the real one is meant to.
use std::future::Future;
use std::path::PathBuf;
use std::task::{Context, Poll, Waker};

use humata2_clients::model::{MachineId, SessionId};
use humata2_clients::{
    FakeFileManager, FileManager, FileManagerError, Folder, StorageAction, StorageKind, StorageRow, TransferMode,
    TransferPhase,
};

fn block<T>(future: tca::BoxFuture<T>) -> T {
    let mut future = future;
    let mut cx = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the fake answers at once"),
    }
}

fn mac() -> MachineId {
    MachineId::new("this-mac")
}
fn gpu() -> MachineId {
    MachineId::new("gpu-box")
}
fn home(machine: &MachineId) -> Folder {
    Folder::home(machine.clone())
}
fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_listing_has_folders_first_then_files_by_name() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("b.txt", Some(1)), ("a.txt", Some(1)), ("zdir", None)]);
    let listing = block(fake.list(&home(&mac()))).unwrap_or_else(|error| panic!("{error}"));
    let seen: Vec<_> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(seen, ["zdir", "a.txt", "b.txt"]);
}

#[test]
fn every_call_on_an_offline_machine_is_offline() {
    let fake = FakeFileManager::default();
    fake.set_online(&gpu(), false);
    let expected = Err(FileManagerError::Offline { machine: gpu() });
    assert_eq!(block(fake.list(&home(&gpu()))).map(|_| ()), expected.clone());
    assert_eq!(block(fake.create_folder(&home(&gpu()), "x")), expected.clone());
    assert_eq!(block(fake.storage(&gpu())).map(|_| ()), expected);
}

#[test]
fn a_taken_name_is_refused() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("a", None)]);
    assert_eq!(
        block(fake.create_folder(&home(&mac()), "a")),
        Err(FileManagerError::NameTaken { name: "a".to_owned() })
    );
}

#[test]
fn delete_goes_to_trash_and_restore_puts_it_back() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("a.txt", Some(5))]);
    assert_eq!(block(fake.delete(&home(&mac()), names(&["a.txt"]))), Ok(()));
    assert!(fake.names_in(&home(&mac())).is_empty());
    assert_eq!(fake.trash_names(&mac()), names(&["a.txt"]));
    let id = block(fake.trash(&mac())).map(|listing| listing.items[0].id.clone());
    let id = id.unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(block(fake.restore(&mac(), &id)), Ok(()));
    assert_eq!(fake.names_in(&home(&mac())), names(&["a.txt"]));
}

#[test]
fn a_finished_copy_appears_at_the_destination_and_stays_at_the_source() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("data", None)]);
    fake.put_file(&home(&mac()).child("data"), "x.bin", 40);
    let id = block(fake.start_transfer(&home(&mac()), names(&["data"]), &home(&gpu()), TransferMode::Copy))
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(fake.names_in(&home(&gpu())).is_empty());
    fake.advance(&id, 40);
    fake.finish(&id);
    assert_eq!(fake.names_in(&home(&gpu())), names(&["data"]));
    assert_eq!(fake.names_in(&home(&gpu()).child("data")), names(&["x.bin"]));
    assert_eq!(fake.names_in(&home(&mac())), names(&["data"]));
}

#[test]
fn a_finished_move_puts_the_source_in_its_machines_trash() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("a.txt", Some(5))]);
    let id = block(fake.start_transfer(&home(&mac()), names(&["a.txt"]), &home(&gpu()), TransferMode::Move))
        .unwrap_or_else(|error| panic!("{error}"));
    fake.finish(&id);
    assert_eq!(fake.names_in(&home(&gpu())), names(&["a.txt"]));
    assert!(fake.names_in(&home(&mac())).is_empty());
    assert_eq!(fake.trash_names(&mac()), names(&["a.txt"]));
}

#[test]
fn a_transfer_that_does_not_fit_fails_at_start_with_disk_full() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("big", Some(100))]);
    fake.set_free_bytes(&gpu(), 10);
    let result = block(fake.start_transfer(&home(&mac()), names(&["big"]), &home(&gpu()), TransferMode::Copy));
    assert_eq!(
        result,
        Err(FileManagerError::DiskFull {
            machine: gpu(),
            needs_bytes: 100,
            free_bytes: 10
        })
    );
}

#[test]
fn a_transfer_pauses_for_an_offline_machine_and_goes_on_when_it_returns() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("a", Some(100))]);
    let id = block(fake.start_transfer(&home(&mac()), names(&["a"]), &home(&gpu()), TransferMode::Copy))
        .unwrap_or_else(|error| panic!("{error}"));
    fake.advance(&id, 30);
    fake.set_online(&gpu(), false);
    fake.advance(&id, 30);
    let rows = block(fake.transfers()).unwrap_or_else(|error| panic!("{error}"));
    assert!(matches!(&rows[0].phase, TransferPhase::Paused { waiting_for: Some(m), .. } if m == &gpu()));
    assert_eq!(rows[0].done_bytes, 30);
    fake.set_online(&gpu(), true);
    fake.advance(&id, 70);
    let rows = block(fake.transfers()).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(rows[0].done_bytes, 100);
}

#[test]
fn every_transfer_change_reaches_a_subscriber() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("a", Some(10))]);
    let mut events = fake.transfer_events();
    let id = block(fake.start_transfer(&home(&mac()), names(&["a"]), &home(&gpu()), TransferMode::Copy))
        .unwrap_or_else(|error| panic!("{error}"));
    fake.advance(&id, 4);
    fake.finish(&id);
    let mut seen = Vec::new();
    while let Ok(row) = events.try_recv() {
        seen.push(row.done_bytes);
    }
    assert_eq!(seen, [0, 4, 10]);
}

#[test]
fn a_machine_to_machine_transfer_goes_through_the_host() {
    let fake = FakeFileManager::default().with_tree(&home(&gpu()), &[("a", Some(1))]);
    let other = MachineId::new("ci-runner");
    let id = block(fake.start_transfer(&home(&gpu()), names(&["a"]), &home(&other), TransferMode::Copy))
        .unwrap_or_else(|error| panic!("{error}"));
    let rows = block(fake.transfers()).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(rows[0].id, id);
    assert!(rows[0].via_host);
}

#[test]
fn an_upload_puts_the_dropped_names_in_the_folder() {
    let fake = FakeFileManager::default();
    let id = block(fake.upload(vec![PathBuf::from("/nowhere/report.pdf")], &home(&gpu())))
        .unwrap_or_else(|error| panic!("{error}"));
    fake.finish(&id);
    assert_eq!(fake.names_in(&home(&gpu())), names(&["report.pdf"]));
}

#[test]
fn emptying_the_trash_frees_what_the_plan_said() {
    let fake = FakeFileManager::default().with_tree(&home(&mac()), &[("a", Some(70)), ("b", Some(30))]);
    assert_eq!(block(fake.delete(&home(&mac()), names(&["a", "b"]))), Ok(()));
    let before = fake.free_bytes(&mac());
    let plan = block(fake.plan_empty_trash(&mac())).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!((plan.items, plan.frees_bytes), (2, 100));
    assert_eq!(block(fake.empty_trash(plan)), Ok(100));
    assert_eq!(fake.free_bytes(&mac()), before + 100);
    assert!(fake.trash_names(&mac()).is_empty());
}

#[test]
fn clearing_a_build_cache_zeroes_its_row_and_storage_is_measured_per_call() {
    let fake = FakeFileManager::default();
    let session = SessionId::new("s1");
    fake.set_storage(
        &gpu(),
        vec![StorageRow {
            kind: StorageKind::BuildCache,
            what: "build".to_owned(),
            note: None,
            bytes: 900,
            last_used_unix_s: None,
            action: StorageAction::ClearBuildCache(session.clone()),
            asks_first: false,
        }],
        Vec::new(),
    );
    let first = block(fake.storage(&gpu())).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(first.rows[0].bytes, 900);
    assert_eq!(block(fake.clear_build_cache(&gpu(), &session)), Ok(()));
    let second = block(fake.storage(&gpu())).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(second.rows[0].bytes, 0);
}

#[test]
fn the_next_call_fails_with_the_error_the_test_chose() {
    let fake = FakeFileManager::default();
    fake.fail_next(FileManagerError::NotAllowed { what: "no".to_owned() });
    assert!(block(fake.list(&home(&mac()))).is_err());
    assert!(block(fake.list(&home(&mac()))).is_ok());
}
