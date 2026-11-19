//! Fakes and readers the daemon's own unit tests share. The crates the
//! daemon is wired from keep theirs under `#[cfg(test)]`, which this crate's
//! tests can't reach.

use std::collections::HashMap;
use std::io;
use std::path::Path;

use crate::terminal_session::{Process, Processes};

/// The ids a report folder's `operations.jsonl` has a `pending` line for.
pub(crate) fn pending_in(folder: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(folder.join("operations.jsonl")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|value| value["state"] == "pending")
        .filter_map(|value| value["id"].as_str().map(str::to_owned))
        .collect()
}

/// Processes by pid, as a test lays them out.
#[derive(Default)]
pub(crate) struct Fake {
    pub(crate) processes: HashMap<i32, Process>,
}

impl Fake {
    pub(crate) fn add(&mut self, pid: i32, parent: i32, session: i32, terminal: Option<u64>) {
        self.processes.insert(
            pid,
            Process {
                parent,
                session,
                terminal,
                started: u64::try_from(pid).unwrap() * 10,
                name: format!("p{pid}"),
            },
        );
    }
}

impl Processes for Fake {
    fn process(&self, pid: i32) -> io::Result<Process> {
        self.processes
            .get(&pid)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }

    fn terminal_name(&self, device: u64) -> Option<String> {
        Some(format!("tty{device}"))
    }
}

/// A terminal app (10) that opened session 20 on terminal 7, a shell (21)
/// in it, and a command (22) the shell ran.
pub(crate) fn terminal() -> Fake {
    let mut fake = Fake::default();
    fake.add(1, 0, 1, None);
    fake.add(10, 1, 10, None);
    fake.add(20, 10, 20, Some(7));
    fake.add(21, 20, 20, Some(7));
    fake.add(22, 21, 20, Some(7));
    fake
}
