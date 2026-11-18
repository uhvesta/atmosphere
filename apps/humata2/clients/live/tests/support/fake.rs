//! The fake daemon over a workspace with only This Mac.

use std::sync::Arc;

use humata2_clients::model::{Agent, Machine, MachineId, MachineKind};
use humata2_clients::comments::{CommentsClient, FakeComments};
use humata2_clients::desk::DeskClient;
use humata2_clients::{Daemon, FakeDaemon, Terminals, Workspace};
use tca::SystemClock;

use super::cases::World;

pub struct FakeWorld;

impl World for FakeWorld {
    fn repo(&self, owner: &str, name: &str) -> String {
        format!("{owner}/{name}")
    }

    fn agent(&self) -> Agent {
        Agent::Claude
    }

    fn answer_to_ping(&self) -> &'static str {
        "Message received by the fake"
    }

    fn paths_exist(&self) -> bool {
        false
    }
}

pub fn start() -> (FakeWorld, Arc<dyn Daemon>, Arc<dyn Terminals>) {
    let workspace = Workspace {
        machines: vec![Machine {
            id: MachineId::new("this-mac"),
            name: "This Mac".to_owned(),
            kind: MachineKind::ThisMac,
            online: true,
            detail: String::new(),
            last_seen: "now".to_owned(),
            lost_at: None,
            stop: None,
            root: "~/Humata".to_owned(),
        }],
        ..Workspace::default()
    };
    let fake = Arc::new(FakeDaemon::new(workspace, Arc::new(SystemClock::new())));
    let daemon: Arc<dyn Daemon> = fake.clone();
    let terminals: Arc<dyn Terminals> = fake;
    (FakeWorld, daemon, terminals)
}

pub fn start_comments() -> (FakeWorld, Arc<dyn Daemon>, Arc<dyn CommentsClient>) {
    let (world, daemon, _terminals) = start();
    (world, daemon, Arc::new(FakeComments::new()))
}

// shared test support: not every test binary uses it
#[allow(dead_code)]
pub fn start_desk() -> (FakeWorld, Arc<dyn Daemon>, Arc<dyn DeskClient>) {
    let workspace = Workspace {
        machines: vec![Machine {
            id: MachineId::new("this-mac"),
            name: "This Mac".to_owned(),
            kind: MachineKind::ThisMac,
            online: true,
            detail: String::new(),
            last_seen: "now".to_owned(),
            lost_at: None,
            stop: None,
            root: "~/Humata".to_owned(),
        }],
        ..Workspace::default()
    };
    let fake = Arc::new(FakeDaemon::new(workspace, Arc::new(SystemClock::new())));
    let daemon: Arc<dyn Daemon> = fake.clone();
    let desk: Arc<dyn DeskClient> = fake;
    (FakeWorld, daemon, desk)
}
