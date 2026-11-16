//! The file manager's panes, transfers, trash and Storage, on the fake file manager.
use std::path::PathBuf;
use std::sync::Arc;

use humata2_clients::model::{Machine, MachineId, MachineKind, SessionId};
use humata2_clients::{
    Clients, FakeFileManager, Fakes, Folder, StorageAction, StorageKind, StorageRow, TransferPhase, Workspace,
    fixture,
};
use humata2_files::{Delegate, Files, FilesAction as A, FilesState, Side};
use tca::{TestClock, TestStore};

fn machine(id: &str, online: bool) -> Machine {
    Machine {
        id: MachineId::new(id),
        name: id.to_owned(),
        kind: if id == "this-mac" {
            MachineKind::ThisMac
        } else {
            MachineKind::Ssh {
                address: format!("{id}.example"),
            }
        },
        online,
        detail: String::new(),
        last_seen: String::new(),
        lost_at: None,
    }
}

fn workspace(gpu_online: bool) -> Arc<Workspace> {
    let mut workspace = fixture::design();
    workspace.machines = vec![machine("this-mac", true), machine("gpu-box", gpu_online), machine("ci-runner", false)];
    Arc::new(workspace)
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

struct Page {
    store: TestStore<Files>,
    fakes: Fakes,
    fake: FakeFileManager,
}

fn launch(fakes: Fakes, workspace: &Arc<Workspace>) -> Page {
    let (clients, fakes) = Clients::with_fakes(fakes, Arc::new(TestClock::new()));
    let fake = fakes.file_manager.clone();
    let mut store = TestStore::new(FilesState::default(), Files::new(clients)).non_exhaustive();
    store.send(A::Show(workspace.clone()), |_| {});
    store.send(A::Opened, |_| {});
    let mut page = Page { store, fakes, fake };
    page.settle();
    page
}

fn page() -> Page {
    let workspace = workspace(true);
    let (_, fakes) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    launch(fakes, &workspace)
}

impl Page {
    fn settle(&mut self) {
        self.store.skip_received();
        self.store.skip_received();
    }
    fn send(&mut self, action: A) {
        self.store.send(action, |_| {});
        self.settle();
    }
    fn state(&self) -> &FilesState {
        self.store.state()
    }
    fn names(&self, side: Side) -> Vec<String> {
        self.state().pane(side).entries.iter().map(|entry| entry.name.clone()).collect()
    }
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|name| (*name).to_owned()).collect()
}

fn row(kind: StorageKind, what: &str, bytes: u64, action: StorageAction, asks_first: bool) -> StorageRow {
    StorageRow {
        kind,
        what: what.to_owned(),
        note: None,
        bytes,
        last_used_unix_s: None,
        action,
        asks_first,
    }
}

#[test]
fn both_panes_start_in_this_macs_home() {
    let page = page();
    assert_eq!(page.state().left.folder, Some(home(&mac())));
    assert_eq!(page.state().right.folder, Some(home(&mac())));
    assert!(page.state().left.loaded);
}

#[test]
fn picking_an_offline_machine_is_refused() {
    let mut page = page();
    let before = page.state().clone();
    page.send(A::PickMachine(Side::Right, MachineId::new("ci-runner")));
    assert_eq!(page.state(), &before);
}

#[test]
fn a_pane_goes_stale_and_refuses_actions_then_reloads_when_the_machine_returns() {
    let mut page = page();
    page.fake.put_file(&home(&gpu()), "model.bin", 5);
    page.send(A::PickMachine(Side::Right, gpu()));
    assert_eq!(page.names(Side::Right), names(&["model.bin"]));

    page.send(A::Show(workspace(false)));
    assert!(page.state().right.stale);
    assert_eq!(page.names(Side::Right), names(&["model.bin"]));

    // The fake would still answer, so a refused action shows the reducer did it.
    page.send(A::Activate(Side::Right));
    page.send(A::NewFolder("made".to_owned()));
    assert_eq!(page.fake.names_in(&home(&gpu())), names(&["model.bin"]));
    page.send(A::Select(Side::Right, "model.bin".to_owned(), false));
    assert!(page.state().right.selected.is_empty());

    page.fake.put_file(&home(&gpu()), "later.bin", 5);
    page.send(A::Show(workspace(true)));
    assert!(!page.state().right.stale);
    assert_eq!(page.names(Side::Right), names(&["later.bin", "model.bin"]));
}

#[test]
fn panes_come_back_after_a_relaunch_on_the_same_storage() {
    let mut first = page();
    first.fake.put_file(&home(&gpu()).child("runs"), "log.txt", 1);
    first.send(A::PickMachine(Side::Right, gpu()));
    first.send(A::OpenFolder(Side::Right, "runs".to_owned()));
    let wanted = home(&gpu()).child("runs");
    assert_eq!(first.state().right.folder, Some(wanted.clone()));

    let second = launch(first.fakes.clone(), &workspace(true));
    assert_eq!(second.state().right.folder, Some(wanted));
    assert_eq!(second.names(Side::Right), names(&["log.txt"]));
}

#[test]
fn a_saved_machine_that_no_longer_exists_is_ignored() {
    let mut first = page();
    first.send(A::PickMachine(Side::Right, gpu()));
    let mut smaller = (*workspace(true)).clone();
    smaller.machines.retain(|machine| machine.id != gpu());
    let second = launch(first.fakes.clone(), &Arc::new(smaller));
    assert_eq!(second.state().right.folder, Some(home(&mac())));
}

#[test]
fn a_copy_shows_in_the_tray_and_the_name_appears_on_the_other_side_when_done() {
    let mut page = page();
    page.fake.put_file(&home(&mac()), "notes.txt", 10);
    page.send(A::PickMachine(Side::Right, gpu()));
    page.send(A::Navigate(Side::Left, home(&mac())));
    page.send(A::Select(Side::Left, "notes.txt".to_owned(), false));
    page.send(A::Copy);
    assert_eq!(page.state().transfers.len(), 1);
    assert_eq!(page.state().transfers[0].phase, TransferPhase::Copying);
    assert!(page.names(Side::Right).is_empty());

    let id = page.state().transfers[0].id.clone();
    page.fake.advance(&id, 10);
    page.fake.finish(&id);
    page.settle();
    assert_eq!(page.state().transfers[0].phase, TransferPhase::Done);
    assert_eq!(page.names(Side::Right), names(&["notes.txt"]));
}

#[test]
fn a_move_leaves_the_source_in_that_machines_trash() {
    let mut page = page();
    page.fake.put_file(&home(&mac()), "notes.txt", 10);
    page.send(A::PickMachine(Side::Right, gpu()));
    page.send(A::Navigate(Side::Left, home(&mac())));
    page.send(A::Select(Side::Left, "notes.txt".to_owned(), false));
    page.send(A::Move);
    let id = page.state().transfers[0].id.clone();
    page.fake.finish(&id);
    page.settle();
    assert_eq!(page.fake.trash_names(&mac()), names(&["notes.txt"]));
    assert_eq!(page.names(Side::Right), names(&["notes.txt"]));
    assert!(page.names(Side::Left).is_empty());
}

#[test]
fn a_transfer_paused_for_an_offline_machine_says_which_machine() {
    let mut page = page();
    page.fake.put_file(&home(&mac()), "notes.txt", 10);
    page.send(A::PickMachine(Side::Right, gpu()));
    page.send(A::Navigate(Side::Left, home(&mac())));
    page.send(A::Select(Side::Left, "notes.txt".to_owned(), false));
    page.send(A::Copy);
    page.fake.set_online(&gpu(), false);
    page.settle();
    assert!(matches!(
        &page.state().transfers[0].phase,
        TransferPhase::Paused { waiting_for: Some(machine), .. } if machine == &gpu()
    ));
}

#[test]
fn a_transfer_can_be_cancelled_from_the_tray() {
    let mut page = page();
    page.fake.put_file(&home(&mac()), "notes.txt", 10);
    page.send(A::PickMachine(Side::Right, gpu()));
    page.send(A::Navigate(Side::Left, home(&mac())));
    page.send(A::Select(Side::Left, "notes.txt".to_owned(), false));
    page.send(A::Copy);
    let id = page.state().transfers[0].id.clone();
    page.send(A::CancelTransfer(id));
    assert_eq!(page.state().transfers[0].phase, TransferPhase::Cancelled);
}

#[test]
fn a_dropped_file_uploads_into_the_pane() {
    let mut page = page();
    page.send(A::PickMachine(Side::Right, gpu()));
    page.send(A::Dropped(Side::Right, vec![PathBuf::from("/nowhere/report.pdf")]));
    assert_eq!(page.state().transfers.len(), 1);
    assert_eq!(page.state().transfers[0].to, gpu());
    let id = page.state().transfers[0].id.clone();
    page.fake.finish(&id);
    page.settle();
    assert_eq!(page.names(Side::Right), names(&["report.pdf"]));
}

#[test]
fn a_new_folder_and_a_rename_show_in_the_pane() {
    let mut page = page();
    page.send(A::NewFolder("old".to_owned()));
    assert_eq!(page.names(Side::Left), names(&["old"]));
    page.send(A::Select(Side::Left, "old".to_owned(), false));
    page.send(A::Rename { new_name: "new".to_owned() });
    assert_eq!(page.names(Side::Left), names(&["new"]));
}

#[test]
fn a_taken_name_lands_in_the_error() {
    let mut page = page();
    page.send(A::NewFolder("dup".to_owned()));
    page.send(A::NewFolder("dup".to_owned()));
    assert!(page.state().error.is_some());
}

#[test]
fn delete_then_restore_puts_the_file_back() {
    let mut page = page();
    page.fake.put_file(&home(&mac()), "a.txt", 5);
    page.send(A::Navigate(Side::Left, home(&mac())));
    page.send(A::Select(Side::Left, "a.txt".to_owned(), false));
    page.send(A::Delete);
    assert!(page.names(Side::Left).is_empty());
    assert_eq!(page.state().trash_count, 1);

    page.send(A::OpenTrash(mac()));
    let id = page.state().trash.as_ref().and_then(|trash| trash.listing.as_ref()).map(|listing| listing.items[0].id.clone());
    page.send(A::Restore(id.unwrap_or_default()));
    assert_eq!(page.names(Side::Left), names(&["a.txt"]));
    assert_eq!(page.state().trash_count, 0);
}

#[test]
fn emptying_the_trash_frees_what_the_plan_said() {
    let mut page = page();
    page.fake.put_file(&home(&mac()), "a.txt", 100);
    page.send(A::Navigate(Side::Left, home(&mac())));
    page.send(A::Select(Side::Left, "a.txt".to_owned(), false));
    page.send(A::Delete);
    page.send(A::OpenTrash(mac()));
    page.send(A::PlanEmptyTrash);
    let planned = page.state().empty_plan.as_ref().map(|plan| plan.frees_bytes);
    assert_eq!(planned, Some(100));
    let before = page.fake.free_bytes(&mac());
    page.send(A::ConfirmEmptyTrash);
    assert_eq!(page.state().last_freed, Some(100));
    assert_eq!(page.fake.free_bytes(&mac()), before + 100);
    assert_eq!(page.state().trash_count, 0);
}

fn disk_full_page() -> Page {
    let mut page = page();
    page.fake.put_file(&home(&mac()), "big.bin", 100);
    page.fake.set_free_bytes(&gpu(), 10);
    page.fake.set_storage(
        &gpu(),
        vec![
            row(StorageKind::Logs, "Logs", 5, StorageAction::TrimLogs, false),
            row(StorageKind::SessionClones, "Clones", 900, StorageAction::Archive(SessionId::new("s1")), false),
            row(StorageKind::MainClones, "Main", 300, StorageAction::None, false),
        ],
        Vec::new(),
    );
    page.send(A::PickMachine(Side::Right, gpu()));
    page.send(A::Navigate(Side::Left, home(&mac())));
    page.send(A::Select(Side::Left, "big.bin".to_owned(), false));
    page.send(A::Copy);
    page
}

#[test]
fn disk_full_opens_the_low_disk_state_with_the_largest_rows_first() {
    let page = disk_full_page();
    let Some(low) = &page.state().low_disk else {
        panic!("low disk should be open");
    };
    assert_eq!((low.machine.clone(), low.needs_bytes, low.free_bytes), (gpu(), 100, 10));
    let sizes: Vec<u64> = low.largest.iter().map(|row| row.bytes).collect();
    assert_eq!(sizes, [900, 300, 5]);
    assert!(page.state().error.is_none());
}

#[test]
fn freeing_room_then_retrying_starts_the_transfer() {
    let mut page = disk_full_page();
    page.send(A::LowDiskAct(2));
    page.fake.set_free_bytes(&gpu(), 1_000);
    page.send(A::RetryAfterLowDisk);
    assert!(page.state().low_disk.is_none());
    assert_eq!(page.state().transfers.len(), 1);
}

#[test]
fn a_storage_page_is_measured_when_opened_and_again_on_request() {
    let mut page = page();
    page.fake.set_storage(&gpu(), vec![row(StorageKind::Logs, "Logs", 50, StorageAction::TrimLogs, false)], Vec::new());
    page.send(A::OpenStorage(gpu()));
    let bytes = |page: &Page| page.state().storage.as_ref().and_then(|s| s.report.as_ref()).map(|r| r.rows[0].bytes);
    assert_eq!(bytes(&page), Some(50));
    page.fake.set_storage(&gpu(), vec![row(StorageKind::Logs, "Logs", 80, StorageAction::TrimLogs, false)], Vec::new());
    assert_eq!(bytes(&page), Some(50));
    page.send(A::MeasureAgain);
    assert_eq!(bytes(&page), Some(80));
}

#[test]
fn a_storage_archive_row_asks_the_parent_to_archive() {
    let mut page = page();
    let session = SessionId::new("s1");
    page.fake.set_storage(&gpu(), vec![row(StorageKind::SessionClones, "Clones", 5, StorageAction::Archive(session.clone()), false)], Vec::new());
    page.send(A::OpenStorage(gpu()));
    page.store.send(A::StorageAct(0), |_| {});
    page.store.receive(A::Delegate(Delegate::ArchiveSession(session)), |_| {});
}

#[test]
fn clearing_a_running_sessions_build_cache_asks_first() {
    let mut page = page();
    let session = SessionId::new("s1");
    page.fake.set_storage(&gpu(), vec![row(StorageKind::BuildCache, "build", 400, StorageAction::ClearBuildCache(session), true)], Vec::new());
    page.send(A::OpenStorage(gpu()));
    page.send(A::StorageAct(0));
    assert!(page.state().pending_clear.is_some());
    let bytes = |page: &Page| page.state().storage.as_ref().and_then(|s| s.report.as_ref()).map(|r| r.rows[0].bytes);
    assert_eq!(bytes(&page), Some(400));
    page.send(A::ConfirmClear);
    assert_eq!(bytes(&page), Some(0));
    assert!(page.state().pending_clear.is_none());
}
