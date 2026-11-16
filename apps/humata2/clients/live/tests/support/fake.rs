//! The fake daemon over a workspace with only This Mac.

use std::sync::Arc;

use humata2_clients::model::{Agent, Machine, MachineId, MachineKind};
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
        }],
        ..Workspace::default()
    };
    let fake = Arc::new(FakeDaemon::new(workspace, Arc::new(SystemClock::new())));
    let daemon: Arc<dyn Daemon> = fake.clone();
    let terminals: Arc<dyn Terminals> = fake;
    (FakeWorld, daemon, terminals)
}
