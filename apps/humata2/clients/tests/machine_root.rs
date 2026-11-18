//! A machine's Humata folder (PL-1): where the fake daemon puts what it makes.

use std::sync::Arc;

use humata2_clients::desk::{
    Access, DEFAULT_ROOT, Folder, FolderChoice, NewDeskSession, root_of, scratch_path_in,
};
use humata2_clients::model::{Agent, MachineId};
use humata2_clients::{Daemon, DeskClient, FakeDaemon, fixture};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn scratch(name: &str, machine: &str) -> NewDeskSession {
    NewDeskSession {
        title: "Plan the quarter".to_owned(),
        agent: Agent::Claude,
        machine: MachineId::new(machine),
        folder: FolderChoice::Scratch { name: name.to_owned() },
        repos: Vec::new(),
        access: Access::default(),
    }
}

#[test]
fn a_scratch_folder_goes_under_the_machines_own_root() {
    let mut workspace = fixture::design();
    for machine in &mut workspace.machines {
        if machine.id.as_str() == "this-mac" {
            machine.root = "~/Work".to_owned();
        }
    }
    let daemon = FakeDaemon::new(workspace, Arc::new(TestClock::new()));
    let started = block_on(daemon.start(scratch("quarter", "this-mac"))).expect("the session starts");
    let workspace = daemon.workspace();
    let Some(record) = workspace.desk.sessions.get(&started) else {
        panic!("the Desk knows the session");
    };
    assert_eq!(
        record.folder,
        Folder::Scratch { path: "~/Work/desk/quarter".to_owned(), kept: false }
    );
}

#[test]
fn a_machine_the_workspace_lacks_has_the_default_root() {
    let workspace = fixture::design();
    assert_eq!(root_of(&workspace.machines, &MachineId::new("nowhere")), DEFAULT_ROOT);
    assert_eq!(scratch_path_in("~/Work", "plan"), "~/Work/desk/plan");
}

#[test]
fn a_machine_added_in_the_fake_starts_on_the_default_root() {
    let daemon = FakeDaemon::new(fixture::design(), Arc::new(TestClock::new()));
    let id = block_on(daemon.add_machine("dev@box")).expect("it is added");
    let workspace = daemon.workspace();
    assert_eq!(root_of(&workspace.machines, &id), DEFAULT_ROOT);
}
