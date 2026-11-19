//! Machine scripts on the machine list (D681): which link-up is `added` and
//! which is `connected`, and the engine the machine service's calls and
//! triggers reach. The engine is set once by `serve` before any link starts.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use humata2_machines::Trigger;
use tokio::runtime::Handle;

use super::{Work, lock};
use crate::machine_scripts::{Context, MachineScripts};

/// The engine, and the humata home its rules file and scripts are in.
pub(crate) struct Installed {
    pub(crate) scripts: Arc<MachineScripts>,
    pub(crate) home: PathBuf,
}

/// What a `Work` keeps for machine scripts.
#[derive(Default)]
pub(super) struct Hook {
    installed: OnceLock<Installed>,
    /// Machines added since their link last came up: that first link-up is
    /// `added`, not `connected`.
    just_added: Mutex<HashSet<String>>,
}

impl Hook {
    pub(super) fn get(&self) -> Option<&Installed> {
        self.installed.get()
    }

    /// The first link-up after `machine` was added: true once.
    fn take_added(&self, machine: &str) -> bool {
        lock(&self.just_added).remove(machine)
    }
}

impl Work {
    /// Gives the machine service the engine. Once.
    pub fn install_scripts(&self, scripts: Arc<MachineScripts>, home: &Path) {
        let _ = self.scripts.installed.set(Installed {
            scripts,
            home: home.to_owned(),
        });
    }

    /// `machine` was just added: its first link-up fires `added` (D681).
    /// Nothing runs yet, since the machine's session service is reached
    /// over its link.
    pub(super) fn note_added(&self, machine: &str) {
        lock(&self.scripts.just_added).insert(machine.to_owned());
    }

    /// `machine`'s link came up: `added` for the first after its add, else
    /// `connected`. In the background; a flapping link queues at most one
    /// more run (D681).
    pub(super) fn fire_link_up(&self, machine: &str) {
        let trigger = if self.scripts.take_added(machine) {
            Trigger::Added
        } else {
            Trigger::Connected
        };
        self.fire_script(trigger, machine);
    }

    /// A Migrate of `machine` finished: its `upgraded` rules run (D689).
    pub fn fire_upgraded(&self, machine: &str) {
        self.fire_script(Trigger::Upgraded, machine);
    }

    fn fire_script(&self, trigger: Trigger, machine: &str) {
        let Some(installed) = self.scripts.get() else {
            return;
        };
        let Ok(runtime) = Handle::try_current() else {
            humata2_rpc_kit::log::say!(
                "humatad2: machine scripts for {machine} didn't run: no async runtime is running here; restart Humata"
            );
            return;
        };
        let _entered = runtime.enter();
        installed.scripts.fire(Context {
            machine: machine.to_owned(),
            trigger,
            session: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_link_up_after_an_add_is_the_added_one() {
        let hook = Hook::default();
        assert!(!hook.take_added("box"), "a machine never added here has only connected");
        hook.just_added.lock().unwrap().insert("box".to_owned());
        assert!(hook.take_added("box"));
        assert!(!hook.take_added("box"), "the second link-up is connected");
    }
}
