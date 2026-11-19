//! Narrow port traits and helpers decoupling `humata2_svc_sessions_transfer`
//! from `humatad`-internal services (`Guard`, `Inbox`, `Carry`, `Links`,
//! `Operations`, `Holders`, `Scripts`, `Migrations`, `Builds`, etc.).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use humata2_access::{AccessRequest, Call, Capability, Target};
use humata2_daemon_ports::{BoardAsk, Caller, Deferred, Run, actor};
use humata2_svc_sessions_common::{Loaded, SessionCore, pb};
use tokio::sync::broadcast;
use tonic::Status;

pub use humata2_daemon_ports::{PortError, SessionTransferPort, UserVia};
pub use humata2_rpc_kit::{
    Answers, blocking, chain, error, failed, failure, internal, invalid_field, lock, missing,
    not_found, now_ms,
};
pub use humata2_svc_sessions_common::{
    Tell, clean_title, publish_family, require_user, resume_failed, session_not_found, start_error,
};

/// Result of a guard check: allowed or must ask the user.
#[derive(Debug)]
pub enum Checked {
    Allowed,
    Ask(AccessRequest),
}

/// Whether a machine's link is up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Down,
    Up { generation: u64 },
}

/// Progress handle for a long-running operation.
#[tonic::async_trait]
pub trait OpProgress: Send + Sync + 'static {
    fn id(&self) -> &str;
    async fn start_step(&self, key: &str, title: &str);
    async fn step_progress(&self, key: &str, done: u64, total: u64);
    async fn finish_step(&self, key: &str);
    async fn fail_step(&self, key: &str, tail: &str);
    async fn succeed(&self);
    async fn fail(&self, message: &str, fix: &str);
}

/// Narrow environment port providing everything the 15 transfer/roam/push/remote
/// handlers need from `humatad` without importing `humatad` internals.
#[tonic::async_trait]
pub trait TransferEnv: Send + Sync + 'static {
    /// Checks access for `caller` performing `call`.
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status>;

    /// Checks access for `caller` performing `(capability, target)`.
    fn check_capability(
        &self,
        caller: &Caller,
        capability: Capability,
        target: Target,
    ) -> Result<Checked, Status>;

    /// Requires access for `caller` performing `call`.
    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status>;

    /// Requires access for `caller` performing `(capability, target)`.
    fn require_capability(
        &self,
        caller: &Caller,
        capability: Capability,
        target: Target,
    ) -> Result<(), Status>;

    /// Defers a gated call to the Inbox.
    async fn ask(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
        board: Option<BoardAsk>,
    ) -> Result<(), Status>;

    /// Starts a tracked operation and returns its initial proto + progress handle.
    async fn start_operation(
        &self,
        kind: pb::OperationKind,
        steps: Vec<(String, String)>,
    ) -> Result<(pb::Operation, Arc<dyn OpProgress>), Status>;

    /// Whether `machine` is this host.
    fn is_host(&self, machine: &str) -> bool;

    /// Human-readable label for `machine`.
    fn machine_label(&self, machine: &str) -> String;

    /// Subscribes to link state changes across all machines.
    fn watch_links(&self) -> broadcast::Receiver<(String, LinkState)>;

    /// Loads a session record and its detail, returning `NotFound` if missing.
    fn load_session(&self, core: &SessionCore, id: &str) -> Result<Loaded, Status>;

    /// Returns the state directory for move/fork journals.
    fn state_dir(&self) -> PathBuf;

    /// Host-level variables for a machine.
    fn host_vars(&self, machine: &str) -> BTreeMap<String, String>;

    /// Returns the actor proto for `caller`.
    fn actor_of(&self, caller: &Caller) -> Option<pb::Actor> {
        Some(actor(caller))
    }

    /// Marks a session as moving or clears its moving state.
    fn set_moving(&self, session: &str, moving: bool);

    /// Whether a session is currently moving.
    fn is_moving(&self, session: &str) -> bool;

    /// Publishes a session update to the event feed.
    async fn publish_session(&self, session: &str, change: pb::Change, by: Option<pb::Actor>);

    /// Watches an agent tab on this machine.
    fn watch_agent(&self, session: String);

    /// Executes a deferred result wrapper.
    fn deferred(result: Vec<u8>, result_type: &str) -> Deferred
    where
        Self: Sized,
    {
        Deferred {
            result,
            result_type: result_type.to_owned(),
        }
    }

    /// Helper for unimplemented/not-yet-wired remote calls during migration.
    fn not_wired(&self, what: &str) -> Status {
        humata2_rpc_kit::not_built_status(what)
    }
}
