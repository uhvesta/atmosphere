//! Whether this daemon's sessions are stopped (D261, D264). A daemon is
//! stopped or running for the whole of its life: a stop or a start makes it
//! start itself again, so the answer is set once at boot and read from
//! anywhere that would launch, resume or type.

use std::sync::OnceLock;

use tonic::{Code, Status};

use crate::channel_service::failed;
use crate::identity::Caller;

/// The machine whose sessions are stopped, set once by a stopped boot.
static STOPPED: OnceLock<String> = OnceLock::new();

#[derive(Debug, thiserror::Error)]
#[error(
    "the sessions on {machine} are stopped, so nothing can be started, attached to or typed into until they are started again"
)]
struct SessionsStopped {
    machine: String,
}

/// Marks this daemon's sessions stopped. Called once, by a stopped boot.
pub(crate) fn enter(machine: &str) {
    let _ = STOPPED.set(machine.to_owned());
}

/// Whether this daemon's sessions are stopped.
pub(crate) fn is_stopped() -> bool {
    STOPPED.get().is_some()
}

/// The machine whose sessions are stopped, if they are.
pub(crate) fn stopped_machine() -> Option<&'static str> {
    STOPPED.get().map(String::as_str)
}

/// Refuses a call that would launch, resume, attach or type while the
/// sessions are stopped. Agents and scripts are refused earlier and more
/// firmly by the guard, so they pass here.
pub(crate) fn require_running(caller: &Caller) -> Result<(), Status> {
    let Some(machine) = STOPPED.get() else {
        return Ok(());
    };
    if matches!(caller, Caller::Agent(_) | Caller::Script(_)) {
        return Ok(());
    }
    Err(refusal(machine))
}

fn refusal(machine: &str) -> Status {
    failed(
        Code::FailedPrecondition,
        &SessionsStopped {
            machine: machine.to_owned(),
        },
        &format!("Start sessions on {machine} first, with the Start button or `humata start`, then try again"),
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
    fn a_daemon_that_was_never_stopped_lets_every_call_through() {
        assert!(!is_stopped());
        assert!(require_running(&Caller::Peer).is_ok());
    }
}
