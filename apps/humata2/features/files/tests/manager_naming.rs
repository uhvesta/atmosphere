//! The name prompt, the machine picker and the times shown from listings,
//! on the fake file manager.
use std::sync::Arc;

use humata2_clients::model::{Machine, MachineId, MachineKind};
use humata2_clients::{Clients, FakeFileManager, Folder, Workspace, fixture};
use humata2_files::{Files, FilesAction as A, FilesState, NamingKind, Side};
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

fn workspace() -> Arc<Workspace> {
    let mut workspace = fixture::design();
    workspace.machines = vec![machine("this-mac", true), machine("gpu-box", true), machine("ci-runner", false)];
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
    fake: FakeFileManager,
}

/// A page opened on This Mac, after `prepare` set the fake up.
fn page_with(prepare: impl FnOnce(&FakeFileManager)) -> Page {
    let workspace = workspace();
    let (clients, fakes) = Clients::fake((*workspace).clone(), Arc::new(TestClock::new()));
    let fake = fakes.file_manager.clone();
    prepare(&fake);
    let mut store = TestStore::new(FilesState::default(), Files::new(clients)).non_exhaustive();
    store.send(A::Show(workspace), |_| {});
    store.send(A::Opened, |_| {});
    let mut page = Page { store, fake };
    page.settle();
    page
}

fn page() -> Page {
    page_with(|_| {})
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

#[test]
fn renaming_needs_exactly_one_selected_name() {
    let mut page = page();
    page.fake.put_file(&home(&mac()), "a.txt", 1);
    page.fake.put_file(&home(&mac()), "b.txt", 1);
    page.send(A::Navigate(Side::Left, home(&mac())));
    page.send(A::BeginRename);
    assert!(page.state().naming.is_none());

    page.send(A::Select(Side::Left, "a.txt".to_owned(), false));
    page.send(A::Select(Side::Left, "b.txt".to_owned(), true));
    page.send(A::BeginRename);
    assert!(page.state().naming.is_none());

    page.send(A::Select(Side::Left, "b.txt".to_owned(), true));
    page.send(A::BeginRename);
    let naming = page.state().naming.clone();
    assert_eq!(naming.as_ref().map(|n| n.kind), Some(NamingKind::Rename));
    assert_eq!(naming.map(|n| n.initial), Some("a.txt".to_owned()));
}

#[test]
fn a_prompt_is_refused_on_a_stale_pane() {
    let mut page = page();
    let mut offline = (*workspace()).clone();
    offline.machines[0].online = false;
    page.send(A::Show(Arc::new(offline)));
    page.send(A::BeginNewFolder);
    assert!(page.state().naming.is_none());
}

#[test]
fn a_bad_name_keeps_the_prompt_open_with_an_error_and_calls_nothing() {
    let mut page = page();
    page.send(A::BeginNewFolder);
    for bad in ["", "   ", "a/b", ".", ".."] {
        page.send(A::CommitName(bad.to_owned()));
        let naming = page.state().naming.clone();
        assert!(naming.as_ref().is_some_and(|n| n.error.is_some()), "{bad:?} should be refused");
    }
    assert!(page.fake.names_in(&home(&mac())).is_empty());
    assert!(page.state().error.is_none());

    page.send(A::NewFolder("old".to_owned()));
    page.send(A::Select(Side::Left, "old".to_owned(), false));
    page.send(A::BeginRename);
    page.send(A::CommitName("old".to_owned()));
    assert!(page.state().naming.as_ref().is_some_and(|n| n.error.is_some()));
    assert_eq!(page.fake.names_in(&home(&mac())), ["old"]);
}

#[test]
fn a_good_name_makes_the_folder_and_closes_the_prompt() {
    let mut page = page();
    page.send(A::BeginNewFolder);
    page.send(A::CommitName("made".to_owned()));
    assert!(page.state().naming.is_none());
    assert_eq!(page.names(Side::Left), ["made"]);
}

#[test]
fn a_good_name_renames_and_closes_the_prompt() {
    let mut page = page();
    page.send(A::NewFolder("old".to_owned()));
    page.send(A::Select(Side::Left, "old".to_owned(), false));
    page.send(A::BeginRename);
    page.send(A::CommitName("new".to_owned()));
    assert!(page.state().naming.is_none());
    assert_eq!(page.names(Side::Left), ["new"]);
}

#[test]
fn a_taken_name_closes_the_prompt_and_lands_in_the_error() {
    let mut page = page();
    page.send(A::NewFolder("dup".to_owned()));
    page.send(A::BeginNewFolder);
    page.send(A::CommitName("dup".to_owned()));
    assert!(page.state().naming.is_none());
    assert!(page.state().error.is_some());
}

#[test]
fn cancelling_closes_the_prompt() {
    let mut page = page();
    page.send(A::BeginNewFolder);
    page.send(A::CancelName);
    assert!(page.state().naming.is_none());
}

#[test]
fn opening_one_picker_closes_the_other_and_picking_closes_it() {
    let mut page = page();
    page.send(A::TogglePicker(Side::Left));
    assert_eq!(page.state().picker, Some(Side::Left));
    page.send(A::TogglePicker(Side::Right));
    assert_eq!(page.state().picker, Some(Side::Right));
    page.send(A::PickMachine(Side::Right, gpu()));
    assert_eq!(page.state().picker, None);
    page.send(A::TogglePicker(Side::Left));
    page.send(A::TogglePicker(Side::Left));
    assert_eq!(page.state().picker, None);
    page.send(A::TogglePicker(Side::Left));
    page.send(A::ClosePicker);
    assert_eq!(page.state().picker, None);
}

const MONDAY_MORNING: i64 = 1_704_100_320;

#[test]
fn an_entry_reads_from_the_listings_read_time_not_the_system_clock() {
    let first = page_with(|fake| {
        fake.set_now(MONDAY_MORNING);
        fake.put_file(&home(&mac()), "a.txt", 1);
    });
    let entry = first.state().left.entries[0].clone();
    let early = first.state().modified(Side::Left, &entry);

    let mut later = page_with(|fake| {
        fake.set_now(MONDAY_MORNING);
        fake.put_file(&home(&mac()), "a.txt", 1);
    });
    later.fake.set_now(MONDAY_MORNING + 7 * 86_400);
    later.send(A::Navigate(Side::Left, home(&mac())));
    let entry = later.state().left.entries[0].clone();
    assert_ne!(later.state().modified(Side::Left, &entry), early);

    // The same state always reads the same.
    assert_eq!(first.state().modified(Side::Left, &entry), first.state().modified(Side::Left, &entry));
}

#[test]
fn the_listings_utc_offset_shifts_the_label() {
    let utc = page_with(|fake| {
        fake.set_now(MONDAY_MORNING);
        fake.put_file(&home(&mac()), "a.txt", 1);
    });
    let shifted = page_with(|fake| {
        fake.set_now(MONDAY_MORNING);
        fake.set_utc_offset(3600);
        fake.put_file(&home(&mac()), "a.txt", 1);
    });
    let entry = utc.state().left.entries[0].clone();
    assert_ne!(utc.state().modified(Side::Left, &entry), shifted.state().modified(Side::Left, &entry));
}
