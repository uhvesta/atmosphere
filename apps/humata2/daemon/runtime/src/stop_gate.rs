//! Whether this daemon's sessions are stopped (D261, D264). A daemon is
//! stopped or running for the whole of its life: a stop or a start makes it
//! start itself again, so the answer is set once at boot and read from
//! anywhere that would launch, resume or type.

use std::sync::OnceLock;

use humata2_daemon_ports::Caller;
use humata2_rpc_kit::failed;
use tonic::{Code, Status};

/// The machine whose sessions are stopped and what a person calls it, set
/// once by a stopped boot.
static STOPPED: OnceLock<(String, String)> = OnceLock::new();

#[derive(Debug, thiserror::Error)]
#[error(
    "the sessions on {machine} are stopped, so nothing can be started, attached to or typed into until they are started again"
)]
struct SessionsStopped {
    machine: String,
}

/// Marks this daemon's sessions stopped. Called once, by a stopped boot.
/// `name` is what a refusal calls the machine (D500).
pub fn enter(machine: &str, name: &str) {
    let _ = STOPPED.set((machine.to_owned(), name.to_owned()));
}

/// Whether this daemon's sessions are stopped.
pub fn is_stopped() -> bool {
    STOPPED.get().is_some()
}

/// The machine whose sessions are stopped, if they are.
pub fn stopped_machine() -> Option<&'static str> {
    STOPPED.get().map(|(machine, _)| machine.as_str())
}

/// Refuses a call that would launch, resume, attach or type while the
/// sessions are stopped. Agents and scripts are refused earlier and more
/// firmly by the guard, so they pass here.
pub fn require_running(caller: &Caller) -> Result<(), Status> {
    require_running_for(matches!(caller, Caller::Agent(_) | Caller::Script(_)))
}

/// As [`require_running`] given whether the caller is an agent or script.
pub fn require_running_for(is_agent_or_script: bool) -> Result<(), Status> {
    let Some((_, name)) = STOPPED.get() else {
        return Ok(());
    };
    if is_agent_or_script {
        return Ok(());
    }
    Err(refusal(name))
}

fn refusal(name: &str) -> Status {
    failed(
        Code::FailedPrecondition,
        &SessionsStopped {
            machine: name.to_owned(),
        },
        &format!(
            "Start sessions on {name} first, with the Start button or `humata start`, then try again"
        ),
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_a_failed_precondition() {
        assert_eq!(refusal("ma_1").code(), Code::FailedPrecondition);
    }

    #[test]
    fn a_refusal_calls_the_machine_by_the_name_it_was_given() {
        let refused = refusal("This Mac");

        assert!(
            refused.message().contains("This Mac"),
            "{}",
            refused.message()
        );
    }

    #[test]
    fn a_daemon_that_was_never_stopped_lets_every_call_through() {
        assert!(!is_stopped());
        assert!(require_running(&Caller::Peer).is_ok());
    }
}
