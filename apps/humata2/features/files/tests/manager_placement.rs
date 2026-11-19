//! Where files sent to a machine land, what the places are called, and the
//! Copy to / Move to actions that use them (PL-3, PL-4, PL-5, PL-11, PL-13).
use std::sync::Arc;

use humata2_clients::model::{ChannelId, Machine, MachineId, MachineKind, SessionId};
use humata2_clients::{Clients, FakeFileManager, FileRoot, Fakes, Folder, TransferMode, TransferPhase, Workspace, fixture};
use humata2_files::{Files, FilesAction as A, FilesState, Pane, Side, crumbs, landing, landing_label, session_slug};
use tca::{TestClock, TestStore};

fn machine(id: &str, name: &str, online: bool) -> Machine {
    Machine {
        id: MachineId::new(id),
        name: name.to_owned(),
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

/// This Mac, a runner outside the `web` channel and gpu-box inside it, in
/// that order; `ci-runner` is offline when `runner_online` is false.
fn workspace(runner_online: bool) -> Arc<Workspace> {
    let mut workspace = fixture::standard();
    fixture::desk(&mut workspace);
    workspace.machines = vec![
        machine("this-mac", "This Mac", true),
        machine("ci-runner", "ci-runner", runner_online),
        machine("gpu-box", "gpu-box", true),
    ];
    Arc::new(workspace)
}

fn mac() -> MachineId {
    MachineId::new("this-mac")
}
fn gpu() -> MachineId {
    MachineId::new("gpu-box")
}
fn runner() -> MachineId {
    MachineId::new("ci-runner")
}
fn web() -> ChannelId {
    ChannelId::new("web")
}
fn login() -> SessionId {
    SessionId::new("s-login")
}
fn received(on: &MachineId, from: &MachineId) -> Folder {
    Folder::root(on.clone(), FileRoot::Received { from: from.clone() })
}
fn shared(on: &MachineId) -> Folder {
    Folder::root(on.clone(), FileRoot::ChannelShared(web()))
}
fn clone_of_login() -> Folder {
    Folder::root(
        mac(),
        FileRoot::SessionClone {
            session: login(),
            repo: "acme/web".to_owned(),
        },
    )
}
fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn a_channels_shared_folder_lands_in_the_same_channels_shared_folder_on_a_machine_of_the_channel() {
    let workspace = workspace(true);
    let source = shared(&mac()).child("datasets");
    assert_eq!(landing(&source, &gpu(), &workspace), shared(&gpu()));
}

#[test]
fn a_sessions_clone_and_folder_land_under_a_folder_for_the_session_in_the_channels_shared_folder() {
    let workspace = workspace(true);
    let slug = session_slug(&workspace, &login());
    assert_eq!(landing(&clone_of_login(), &gpu(), &workspace), shared(&gpu()).child(&slug));
    let folder = Folder::root(mac(), FileRoot::SessionFolder { session: login() });
    assert_eq!(landing(&folder, &gpu(), &workspace), shared(&gpu()).child(&slug));
}

#[test]
fn a_session_without_a_known_folder_is_named_by_its_title_in_lower_case_with_dashes() {
    let mut workspace = (*workspace(true)).clone();
    for session in &mut workspace.sessions {
        session.folder.clear();
    }
    let title = workspace.session(&login()).map(|session| session.title.clone()).unwrap_or_default();
    let expected: String = title
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    assert!(!expected.is_empty());
    assert_eq!(session_slug(&workspace, &login()), expected);
}

#[test]
fn a_renamed_session_still_sends_into_the_folder_it_was_made_with() {
    let mut workspace = (*workspace(true)).clone();
    let named = workspace
        .sessions
        .iter_mut()
        .find(|session| session.id == login())
        .expect("the fixture has the login session");
    named.folder = "/Users/me/Humata/web/sessions/login-v2".to_owned();
    named.title = "Sign-in rewrite".to_owned();
    assert_eq!(session_slug(&workspace, &login()), "login-v2");
    assert_eq!(landing(&clone_of_login(), &gpu(), &workspace), shared(&gpu()).child("login-v2"));
}

#[test]
fn a_session_without_a_title_is_named_by_its_id() {
    let workspace = workspace(true);
    let unknown = SessionId::new("not-a-session");
    assert_eq!(session_slug(&workspace, &unknown), unknown.to_string());
}

#[test]
fn anything_else_lands_in_the_targets_received_place_for_the_source_machine() {
    let workspace = workspace(true);
    let sources = [
        Folder::home(mac()).child("notes"),
        Folder::root(mac(), FileRoot::SessionPastes(login())),
        Folder::root(mac(), FileRoot::ChannelFolder(web())),
        Folder::root(
            mac(),
            FileRoot::SessionFolder {
                session: SessionId::new("desk-plan-q4"),
            },
        ),
        received(&mac(), &runner()),
    ];
    for source in sources {
        assert_eq!(
            landing(&source, &gpu(), &workspace),
            received(&gpu(), &source.machine),
            "{source:?}"
        );
    }
}

#[test]
fn a_machine_outside_the_channel_gets_the_received_place_even_for_channel_files() {
    let workspace = workspace(true);
    assert_eq!(landing(&shared(&mac()), &runner(), &workspace), received(&runner(), &mac()));
    assert_eq!(landing(&clone_of_login(), &runner(), &workspace), received(&runner(), &mac()));
}

#[test]
fn breadcrumbs_are_names_from_the_machine_down() {
    let workspace = workspace(true);
    assert_eq!(crumbs(&Folder::home(mac()), &workspace), names(&["This Mac", "Home"]));
    assert_eq!(
        crumbs(&Folder::home(gpu()).child("a/b"), &workspace),
        names(&["gpu-box", "Home", "a", "b"])
    );
    assert_eq!(
        crumbs(&shared(&gpu()).child("datasets"), &workspace),
        names(&["gpu-box", "# web", "shared", "datasets"])
    );
    assert_eq!(
        crumbs(&received(&gpu(), &mac()), &workspace),
        names(&["gpu-box", "From This Mac"])
    );
}

#[test]
fn a_session_clones_breadcrumbs_name_the_channel_the_session_and_the_repository() {
    let workspace = workspace(true);
    let title = workspace.session(&login()).map(|session| session.title.clone()).unwrap_or_default();
    let found = crumbs(&clone_of_login().child("src"), &workspace);
    assert_eq!(found, vec!["This Mac".to_owned(), "# web".to_owned(), title, "web".to_owned(), "src".to_owned()]);
}

#[test]
fn a_landing_label_names_the_machine_and_the_place() {
    let workspace = workspace(true);
    assert_eq!(landing_label(&shared(&gpu()), &workspace), "gpu-box \u{b7} web shared");
    assert_eq!(
        landing_label(&received(&runner(), &mac()), &workspace),
        "ci-runner \u{b7} From This Mac"
    );
}

fn state_in(folder: Folder, workspace: &Arc<Workspace>) -> FilesState {
    FilesState {
        workspace: workspace.clone(),
        left: Pane {
            folder: Some(folder),
            ..Pane::default()
        },
        ..FilesState::default()
    }
}

fn order(state: &FilesState) -> Vec<MachineId> {
    state.send_targets().into_iter().map(|target| target.machine).collect()
}

#[test]
fn the_send_menu_lists_the_channels_machines_first() {
    let workspace = workspace(true);
    let state = state_in(clone_of_login(), &workspace);
    assert_eq!(order(&state), vec![mac(), gpu(), runner()]);
}

#[test]
fn the_send_menu_leaves_out_the_machine_a_file_would_only_be_received_from() {
    let workspace = workspace(true);
    let state = state_in(Folder::home(mac()), &workspace);
    assert_eq!(order(&state), vec![runner(), gpu()]);
}

#[test]
fn the_send_menu_marks_offline_machines() {
    let workspace = workspace(false);
    let state = state_in(Folder::home(mac()), &workspace);
    let online: Vec<_> = state.send_targets().into_iter().map(|target| (target.machine, target.online)).collect();
    assert_eq!(online, vec![(runner(), false), (gpu(), true)]);
}

struct Page {
    store: TestStore<Files>,
    fake: FakeFileManager,
}

fn page() -> Page {
    let workspace = workspace(false);
    let (_, fakes): (_, Fakes) = humata2_clients::fake_clients((*workspace).clone(), Arc::new(TestClock::new()));
    let (clients, fakes) = humata2_clients::clients_with_fakes(fakes, Arc::new(TestClock::new()));
    let fake = fakes.file_manager.clone();
    let mut store = TestStore::new(FilesState::default(), Files::new(clients)).non_exhaustive();
    store.send(A::Show(workspace), |_| {});
    store.send(A::Opened, |_| {});
    let mut page = Page { store, fake };
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
    fn select_in_home(&mut self, name: &str) {
        self.send(A::Navigate(Side::Left, Folder::home(mac())));
        self.send(A::Select(Side::Left, name.to_owned(), false));
    }
    fn finish_last(&mut self) {
        let id = self.state().transfers[0].id.clone();
        self.fake.finish(&id);
        self.settle();
    }
}

#[test]
fn copy_to_a_machine_puts_the_file_in_the_received_place_and_keeps_the_source() {
    let mut page = page();
    page.fake.put_file(&Folder::home(mac()), "notes.txt", 10);
    page.select_in_home("notes.txt");
    page.send(A::CopyTo(gpu()));
    assert_eq!(page.state().transfers[0].phase, TransferPhase::Copying);
    page.finish_last();
    assert_eq!(page.fake.names_in(&received(&gpu(), &mac())), names(&["notes.txt"]));
    assert_eq!(page.fake.names_in(&Folder::home(mac())), names(&["notes.txt"]));
}

#[test]
fn move_to_a_machine_sends_the_source_to_its_trash() {
    let mut page = page();
    page.fake.put_file(&Folder::home(mac()), "notes.txt", 10);
    page.select_in_home("notes.txt");
    page.send(A::MoveTo(gpu()));
    page.finish_last();
    assert_eq!(page.fake.names_in(&received(&gpu(), &mac())), names(&["notes.txt"]));
    assert_eq!(page.fake.trash_names(&mac()), names(&["notes.txt"]));
}

#[test]
fn a_name_taken_where_a_machine_only_send_lands_keeps_both() {
    let mut page = page();
    page.fake.put_file(&Folder::home(mac()), "notes.txt", 10);
    page.fake.put_file(&received(&gpu(), &mac()), "notes.txt", 3);
    page.select_in_home("notes.txt");
    page.send(A::CopyTo(gpu()));
    assert!(page.state().error.is_none());
    page.finish_last();
    let mut found = page.fake.names_in(&received(&gpu(), &mac()));
    found.sort();
    assert_eq!(found.len(), 2);
    assert!(found.contains(&"notes.txt".to_owned()));
}

#[test]
fn a_session_clone_send_lands_in_the_channels_shared_folder_under_the_session() {
    let mut page = page();
    let clone = clone_of_login();
    page.fake.put_file(&clone, "README.md", 4);
    page.send(A::Navigate(Side::Left, clone.clone()));
    page.send(A::Select(Side::Left, "README.md".to_owned(), false));
    page.send(A::CopyTo(gpu()));
    page.finish_last();
    let slug = session_slug(&workspace(false), &login());
    assert_eq!(page.fake.names_in(&shared(&gpu()).child(&slug)), names(&["README.md"]));
}

#[test]
fn sending_to_an_offline_machine_starts_nothing() {
    let mut page = page();
    page.fake.put_file(&Folder::home(mac()), "notes.txt", 10);
    page.select_in_home("notes.txt");
    page.send(A::CopyTo(runner()));
    assert!(page.state().transfers.is_empty());
}

#[test]
fn somewhere_specific_still_copies_into_the_other_pane_and_refuses_a_taken_name() {
    let mut page = page();
    page.fake.put_file(&Folder::home(mac()), "notes.txt", 10);
    page.fake.put_file(&Folder::home(gpu()), "notes.txt", 3);
    page.send(A::PickMachine(Side::Right, gpu()));
    page.select_in_home("notes.txt");
    page.send(A::Copy);
    assert!(page.state().error.is_some());
    assert!(page.state().transfers.is_empty());
}

#[test]
fn the_send_menu_opens_for_one_mode_and_closes_when_asked_again_or_after_sending() {
    let mut page = page();
    page.fake.put_file(&Folder::home(mac()), "notes.txt", 10);
    page.select_in_home("notes.txt");
    page.send(A::ToggleSendMenu(TransferMode::Copy));
    assert_eq!(page.state().send_menu, Some(TransferMode::Copy));
    page.send(A::ToggleSendMenu(TransferMode::Move));
    assert_eq!(page.state().send_menu, Some(TransferMode::Move));
    page.send(A::ToggleSendMenu(TransferMode::Move));
    assert_eq!(page.state().send_menu, None);
    page.send(A::ToggleSendMenu(TransferMode::Copy));
    page.send(A::CopyTo(gpu()));
    assert_eq!(page.state().send_menu, None);
}
