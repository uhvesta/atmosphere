//! A file manager row's menu (PL-10): Reveal in Finder on This Mac, Open
//! terminal here on another machine, Copy path on both.
use std::sync::Arc;

use humata2_clients::model::{Machine, MachineId, MachineKind, SessionClone, SessionId};
use humata2_clients::{Clients, FakeFileManager, Fakes, FileRoot, Folder, Workspace, fixture};
use humata2_files::{Delegate, Files, FilesAction as A, FilesState, RowChoice, Side};
use tca::{TestClock, TestStore};

const LOCAL_CLONE: &str = "/Users/me/Humata/pay/sessions/one/api";
const REMOTE_CLONE: &str = "/home/dev/Humata/pay/sessions/two/api";

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
        root: "~/Humata".to_owned(),
        stop: None,
    }
}

fn mac() -> MachineId {
    MachineId::new("this-mac")
}
fn gpu() -> MachineId {
    MachineId::new("gpu-box")
}
fn bare() -> MachineId {
    MachineId::new("bare-box")
}

/// Two sessions with clones, one on This Mac and one on gpu-box; every other
/// session is on This Mac, and nothing runs on bare-box.
struct World {
    workspace: Arc<Workspace>,
    local: SessionId,
    remote: SessionId,
}

fn world(gpu_online: bool) -> World {
    let mut workspace = fixture::design();
    workspace.machines = vec![machine("this-mac", true), machine("gpu-box", gpu_online), machine("bare-box", true)];
    for session in &mut workspace.sessions {
        session.machine = mac();
    }
    let clone = |path: &str| {
        vec![SessionClone {
            repo: "acme/api".to_owned(),
            branch: "work".to_owned(),
            path: path.to_owned(),
        }]
    };
    workspace.sessions[0].clones = clone(LOCAL_CLONE);
    let local = workspace.sessions[0].id.clone();
    workspace.sessions[1].machine = gpu();
    workspace.sessions[1].clones = clone(REMOTE_CLONE);
    let remote = workspace.sessions[1].id.clone();
    World {
        workspace: Arc::new(workspace),
        local,
        remote,
    }
}

fn clone_folder(machine: MachineId, session: &SessionId) -> Folder {
    Folder::root(
        machine,
        FileRoot::SessionClone {
            session: session.clone(),
            repo: "acme/api".to_owned(),
        },
    )
}

struct Page {
    store: TestStore<Files>,
    fakes: Fakes,
    fake: FakeFileManager,
}

fn page(world: &World) -> Page {
    let (_, fakes) = humata2_clients::fake_clients((*world.workspace).clone(), Arc::new(TestClock::new()));
    let (clients, fakes) = humata2_clients::clients_with_fakes(fakes, Arc::new(TestClock::new()));
    let fake = fakes.file_manager.clone();
    let mut store = TestStore::new(FilesState::default(), Files::new(clients)).non_exhaustive();
    store.send(A::Show(world.workspace.clone()), |_| {});
    store.send(A::Opened, |_| {});
    let mut page = Page { store, fakes, fake };
    page.settle();
    page
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
    /// Shows `folder` in `side`, holding a file `main.rs` and a folder `src`.
    fn show(&mut self, side: Side, folder: &Folder) {
        self.fake.put_file(folder, "main.rs", 3);
        self.fake.put_folder(&folder.child("src"));
        self.send(A::Navigate(side, folder.clone()));
    }
}

#[test]
fn copy_path_puts_the_rows_absolute_path_on_the_clipboard() {
    let world = world(true);
    let mut page = page(&world);
    page.show(Side::Left, &clone_folder(mac(), &world.local));
    page.send(A::OpenRowMenu(Side::Left, "main.rs".to_owned()));
    page.send(A::CopyPath(Side::Left, "main.rs".to_owned()));
    assert_eq!(page.fakes.clipboard.copied(), Some(format!("{LOCAL_CLONE}/main.rs")));
    assert_eq!(page.state().row_menu, None);
}

#[test]
fn copy_path_works_on_a_remote_row_too() {
    let world = world(true);
    let mut page = page(&world);
    page.show(Side::Right, &clone_folder(gpu(), &world.remote));
    page.send(A::CopyPath(Side::Right, "src".to_owned()));
    assert_eq!(page.fakes.clipboard.copied(), Some(format!("{REMOTE_CLONE}/src")));
}

#[test]
fn a_row_on_this_mac_offers_reveal_and_copy_but_not_a_terminal() {
    let world = world(true);
    let mut page = page(&world);
    page.show(Side::Left, &clone_folder(mac(), &world.local));
    assert_eq!(
        page.state().row_choices(Side::Left, "main.rs"),
        [RowChoice::RevealInFinder, RowChoice::CopyPath]
    );
}

#[test]
fn reveal_on_a_this_mac_row_asks_finder_for_its_path() {
    let world = world(true);
    let mut page = page(&world);
    page.show(Side::Left, &clone_folder(mac(), &world.local));
    page.send(A::OpenRowMenu(Side::Left, "main.rs".to_owned()));
    assert!(page.state().row_menu.is_some());
    page.send(A::RevealInFinder(Side::Left, "main.rs".to_owned()));
    assert_eq!(page.fakes.reveal.revealed(), [format!("{LOCAL_CLONE}/main.rs")]);
    assert_eq!(page.state().row_menu, None);
}

#[test]
fn a_remote_row_offers_a_terminal_not_reveal_and_reveal_does_nothing_there() {
    let world = world(true);
    let mut page = page(&world);
    page.show(Side::Right, &clone_folder(gpu(), &world.remote));
    assert_eq!(
        page.state().row_choices(Side::Right, "main.rs"),
        [RowChoice::OpenTerminalHere, RowChoice::CopyPath]
    );
    page.send(A::RevealInFinder(Side::Right, "main.rs".to_owned()));
    assert!(page.fakes.reveal.revealed().is_empty());
}

#[test]
fn a_terminal_for_a_remote_folder_row_starts_in_that_folder_on_its_session() {
    let world = world(true);
    let mut page = page(&world);
    page.show(Side::Right, &clone_folder(gpu(), &world.remote));
    page.store.send(A::OpenTerminalHere(Side::Right, "src".to_owned()), |_| {});
    page.store.receive(
        A::Delegate(Delegate::OpenTerminal {
            session: world.remote.clone(),
            folder: format!("{REMOTE_CLONE}/src"),
        }),
        |_| {},
    );
}

#[test]
fn a_terminal_for_a_remote_file_row_starts_in_the_folder_it_is_in() {
    let world = world(true);
    let mut page = page(&world);
    page.show(Side::Right, &clone_folder(gpu(), &world.remote));
    page.store.send(A::OpenTerminalHere(Side::Right, "main.rs".to_owned()), |_| {});
    page.store.receive(
        A::Delegate(Delegate::OpenTerminal {
            session: world.remote.clone(),
            folder: REMOTE_CLONE.to_owned(),
        }),
        |_| {},
    );
}

#[test]
fn a_terminal_from_a_home_folder_uses_a_session_on_that_machine() {
    let world = world(true);
    let mut page = page(&world);
    let home = Folder::home(gpu());
    page.fake.put_file(&home, "notes.txt", 1);
    page.send(A::PickMachine(Side::Right, gpu()));
    page.store.send(A::OpenTerminalHere(Side::Right, "notes.txt".to_owned()), |_| {});
    page.store.receive(
        A::Delegate(Delegate::OpenTerminal {
            session: world.remote.clone(),
            folder: "~".to_owned(),
        }),
        |_| {},
    );
}

#[test]
fn a_machine_with_no_session_says_so_instead_of_opening_a_terminal() {
    let world = world(true);
    let mut page = page(&world);
    page.fake.put_file(&Folder::home(bare()), "notes.txt", 1);
    page.send(A::PickMachine(Side::Right, bare()));
    page.send(A::OpenTerminalHere(Side::Right, "notes.txt".to_owned()));
    assert!(page.state().error.is_some());
}

#[test]
fn an_offline_machines_row_offers_only_copy_path() {
    let world = world(true);
    let mut page = page(&world);
    page.show(Side::Right, &clone_folder(gpu(), &world.remote));
    page.send(A::Show(self::world(false).workspace));
    assert_eq!(page.state().row_choices(Side::Right, "main.rs"), [RowChoice::CopyPath]);
}

#[test]
fn a_row_with_no_known_path_has_no_menu() {
    let world = world(true);
    let mut page = page(&world);
    let pastes = Folder::root(mac(), FileRoot::SessionPastes(world.local.clone()));
    page.show(Side::Left, &pastes);
    assert!(page.state().row_choices(Side::Left, "main.rs").is_empty());
    page.send(A::OpenRowMenu(Side::Left, "main.rs".to_owned()));
    assert_eq!(page.state().row_menu, None);
}
