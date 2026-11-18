//! The host's stack settings on a machine whose link just came up (D411):
//! `config/stack.toml` from the host's humata home, secrets taken out, goes
//! to `~/.humata2/config/stack.toml` on the machine when the machine's copy
//! differs. Adding or reconnecting a machine sends it too, in setup; this
//! catches a file that changed while the machine was linked or away. A host
//! with no such file sends nothing and leaves the machine's own alone.

use std::sync::Arc;

use humata2_machines::{StackConfigError, SystemRunner};

use super::Work;
use crate::status;

/// Sends the settings to `machine` in the background. A failure is logged
/// and the machine keeps the copy it has; the next link sends again.
pub(super) fn link_up(work: &Arc<Work>, machine: &str) {
    let (work, machine) = (Arc::clone(work), machine.to_owned());
    tokio::spawn(async move {
        let named = machine.clone();
        match tokio::task::spawn_blocking(move || send(&work, &machine)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::log::say!(
                "humatad2: couldn't put the host's stack settings on {named}, which keeps the copy it has: {}",
                status::chain(&error)
            ),
            Err(error) => crate::log::say!("humatad2: sending the stack settings to {named} stopped: {error}"),
        }
    });
}

/// Reads the host's file afresh and sends it to `machine`.
fn send(work: &Work, machine: &str) -> Result<(), StackConfigError> {
    // REPOS (RL-12): `config/repos.toml` goes the same way, to its own file;
    // so does MACHINESCRIPTS' `config/machine-scripts.toml`.
    for config in [
        work.builds.stack_config()?,
        work.builds.repos_config()?,
        work.builds.agents_config()?,
        work.builds.machine_scripts_config()?,
    ]
    .into_iter()
    .flatten()
    {
        for key in config.dropped() {
            crate::log::say!(
                "humatad2: the host's setting `{key}` looks like a secret, so it isn't sent to {machine}"
            );
        }
        work.machines.push_stack_config(machine, &config, &SystemRunner::new(&()))?;
    }
    Ok(())
}
