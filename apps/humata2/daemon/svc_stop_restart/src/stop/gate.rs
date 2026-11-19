//! Whether this daemon's sessions are stopped (D261, D264). A daemon is
//! stopped or running for the whole of its life: a stop or a start makes it
//! start itself again, so the answer is set once at boot and read from
//! anywhere that would launch, resume or type.

use humata2_daemon_ports::Caller;
use tonic::Status;

pub use humata2_daemon_runtime::stop_gate::{enter, is_stopped, stopped_machine};

/// Refuses a call that would launch, resume, attach or type while the
/// sessions are stopped. Agents and scripts are refused earlier and more
/// firmly by the guard, so they pass here.
pub fn require_running(caller: &Caller) -> Result<(), Status> {
    humata2_daemon_runtime::stop_gate::require_running_for(matches!(
        caller,
        Caller::Agent(_) | Caller::Script(_)
    ))
}
