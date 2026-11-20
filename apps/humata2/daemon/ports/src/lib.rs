//! Cross-service port traits and tiny value types for `humatad2` (SPLIT §3.1,
//! §6.2; SPLIT_PHASE3_DAEMON §2.1–2.2).
//!
//! Service crates (`svc_*`) depend on these narrow traits rather than
//! importing sibling services directly, breaking the 29 cross-service cycles
//! and keeping compilation and Bazel test caching isolated per domain.

use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use humata2_access::{
    AccessRequest, ChannelDeskAccess, DeskAccess, PresenceMethod, ScriptToken, SessionId,
    SessionKind, SessionRef,
};
#[allow(unused_imports, ambiguous_glob_reexports)]
mod pb {
    pub use humata2_grpc_core::humata::v1::*;
    pub use humata2_grpc_inbox::humata::v1::*;
    pub use humata2_grpc_machines::humata::v1::*;
    pub use humata2_grpc_peer::humata::v1::*;
    pub use humata2_grpc_review::humata::v1::*;
    pub use humata2_grpc_sessions::humata::v1::*;
}
use tokio::sync::watch;
use tonic::Status;

pub mod release;

pub use release::{
    BUNDLE_HEADER, Release, host_files, installed_tools, names_file, names_whole_entry,
    set_log_sink, short,
};

/// What Unix sockets allow in a path, without its terminating NUL.
#[cfg(target_os = "linux")]
pub const SOCKET_PATH_MAX: usize = 107;
#[cfg(not(target_os = "linux"))]
pub const SOCKET_PATH_MAX: usize = 103;

/// This API's version. `API_MINOR` grows with every addition once Humata 2
/// ships; until then the protos change in place.
pub const API_MAJOR: u32 = 1;
pub const API_MINOR: u32 = 0;

/// Whether a daemon runs as the host (the Mac) or a managed machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Role {
    /// The Mac: the source of truth.
    Host,
    /// Any other machine.
    Machine,
}

/// Whose calls with no credential an admin test daemon takes for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admin {
    /// Anyone's (`--admin`).
    Anyone,
    /// Only this process's (`--admin-pid`), so a test can play the app
    /// while every other process is judged as by a release daemon.
    Process(i32),
}

/// Explicit cross-service port error type that maps cleanly to and from
/// [`tonic::Status`].
#[derive(Clone, Debug, thiserror::Error)]
pub enum PortError {
    #[error("{0}")]
    Status(#[from] Status),
    #[error("{message}")]
    Message { message: String },
}

impl PortError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message {
            message: message.into(),
        }
    }
}

impl From<PortError> for Status {
    fn from(error: PortError) -> Self {
        match error {
            PortError::Status(status) => status,
            PortError::Message { message } => Status::internal(message),
        }
    }
}

/// What a guarded handler body got to: the answer, or the rule that sent
/// the call to Ask me before it could run (D38.6).
#[derive(Clone, Debug)]
pub enum Gated<T> {
    Done(T),
    Ask(Box<AccessRequest>),
}

/// What an approved call produced: its response, encoded, and the response
/// message's full name (e.g. `"humata.v1.ListChannelsResponse"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deferred {
    pub result: Vec<u8>,
    pub result_type: String,
}

/// What the card of a deferred board call shows in place of the access
/// preview (BD-15): the change field by field, or the new item, and the
/// agent's reason.
#[derive(Clone, Debug)]
pub struct BoardAsk {
    pub preview: pb::request_preview::Preview,
    pub reason: String,
}

/// The call, to run once the user approves it. It is given the id of the
/// request that was approved, for a call that records which approval it ran
/// under (D268).
pub type Run = Box<
    dyn FnOnce(String) -> Pin<Box<dyn Future<Output = Result<Deferred, Status>> + Send>> + Send,
>;

/// What a machine is called in text a person reads (D500). Ids stay in the
/// structured fields. The host's own machine is "This Mac", as the app
/// labels it. Another machine's id is its name until machines get ids of
/// their own (D61), so it reads as its name; an id with no name known reads
/// as the id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Names {
    this_mac: Option<String>,
    named: HashMap<String, String>,
}

impl Names {
    /// The host's own machine, `machine`, is "This Mac".
    pub fn of_host(machine: &str) -> Self {
        Self {
            this_mac: Some(machine.to_owned()),
            named: HashMap::new(),
        }
    }

    /// `machine` is called `name`.
    pub fn naming(mut self, machine: &str, name: &str) -> Self {
        self.named.insert(machine.to_owned(), name.to_owned());
        self
    }

    /// What `machine` is called: its name, "This Mac" for the host's own
    /// machine, and the id when neither is known.
    pub fn of(&self, machine: &str) -> String {
        if self.this_mac.as_deref() == Some(machine) {
            return "This Mac".to_owned();
        }
        match self.named.get(machine).filter(|name| !name.is_empty()) {
            Some(name) => name.clone(),
            None => machine.to_owned(),
        }
    }
}

/// A script token's holder: who the policy treats the call as.
#[derive(Clone, Debug)]
pub struct ScriptCaller {
    /// The token's id, `st_<hex>`.
    pub id: String,
    pub name: String,
    pub token: ScriptToken,
}

/// Why a script token doesn't make a call a script's.
#[derive(Debug, thiserror::Error)]
pub enum ScriptTokenUnusable {
    #[error(
        "its script token isn't one this daemon has: it was revoked, or another daemon made it; make a new one with `humata token create`"
    )]
    Unknown,
    #[error(
        "the script token {name:?} has expired; make a new one with `humata token create` and revoke this one"
    )]
    Expired { name: String },
}

/// How the user reached the daemon.
#[derive(Clone, Debug)]
pub enum UserVia {
    /// The app.
    App,
    /// A process in the terminal session that this `humata login` bound.
    CliLogin {
        login: String,
        on_host: bool,
        method: PresenceMethod,
    },
    /// A local same-UID CLI process outside every Humata session calling a
    /// strictly read-only inspection RPC without `humata login` (SEC-9 split).
    LocalReadOnly {
        on_host: bool,
    },
    /// The host, through the peer link's relay (D139, D144).
    Host,
}

/// A session's agent, or anything else running in one of its tabs.
#[derive(Clone, Debug)]
pub struct AgentCaller {
    pub session: SessionRef,
    pub agent: String,
    pub title: String,
    pub method: String,
    pub context_only_parent: Option<SessionId>,
    pub desk: Option<DeskAccess>,
    pub review_push: Option<bool>,
    pub channel_desk: Option<ChannelDeskAccess>,
}

impl AgentCaller {
    /// The kind of session the policy judges this agent's calls by.
    pub fn session_kind(&self) -> SessionKind {
        if let Some(push_allowed) = self.review_push {
            return SessionKind::Review { push_allowed };
        }
        if let Some(access) = &self.channel_desk {
            return SessionKind::ChannelDesk(access.clone());
        }
        if let Some(access) = self.desk {
            return SessionKind::Desk(access);
        }
        match &self.context_only_parent {
            Some(parent) => SessionKind::ContextOnlyFork {
                parent: parent.clone(),
            },
            None => SessionKind::Ordinary,
        }
    }

    /// The dotted operation id of the call being made (TP-16).
    pub fn operation(&self) -> String {
        operation_of(&self.method)
    }

    pub fn actor(&self) -> pb::AgentActor {
        pb::AgentActor {
            session: Some(pb::SessionId {
                value: self.session.id.to_string(),
            }),
            machine: Some(pb::MachineId {
                value: self.session.machine.to_string(),
            }),
            agent: Some(pb::AgentId {
                value: self.agent.clone(),
            }),
            session_title: self.title.clone(),
        }
    }
}

/// Who made a call, as its credential established it.
#[derive(Clone, Debug)]
pub enum Caller {
    User(UserVia),
    Agent(AgentCaller),
    Script(ScriptCaller),
    Peer,
}

/// What a token minted for a tab says its holder is (SE-29, SEC-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabIdentity {
    Session,
    AddedAgent,
}

impl TabIdentity {
    pub fn word(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::AddedAgent => "added",
        }
    }

    pub fn of_word(word: Option<&str>) -> Self {
        match word {
            Some("added") => Self::AddedAgent,
            _ => Self::Session,
        }
    }
}

/// The dotted operation id (TP-16) of the gRPC method at `path`
/// (`/humata.v1.ChannelService/ListChannels` → `channel.list`).
pub fn operation_of(path: &str) -> String {
    let Some((service, method)) = path.trim_start_matches('/').split_once('/') else {
        return String::new();
    };
    let service = service.rsplit('.').next().unwrap_or(service);
    let area = snake(service.strip_suffix("Service").unwrap_or(service));
    let words = snake(method);
    let plural = format!("{area}s");
    let verb: Vec<&str> = words
        .split('_')
        .filter(|word| *word != area && *word != plural)
        .collect();
    if verb.is_empty() {
        return format!("{area}.{words}");
    }
    format!("{area}.{}", verb.join("_"))
}

fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (at, letter) in name.chars().enumerate() {
        if letter.is_ascii_uppercase() {
            if at > 0 {
                out.push('_');
            }
            out.push(letter.to_ascii_lowercase());
        } else {
            out.push(letter);
        }
    }
    out
}

/// How the user reached this daemon, as records and whoami both name it.
pub fn user_via(via: &UserVia) -> pb::user_actor::Via {
    match via {
        UserVia::App | UserVia::Host => pb::user_actor::Via::App,
        UserVia::CliLogin { on_host: true, .. } | UserVia::LocalReadOnly { on_host: true } => {
            pb::user_actor::Via::CliOnHost
        }
        UserVia::CliLogin { on_host: false, .. } | UserVia::LocalReadOnly { on_host: false } => {
            pb::user_actor::Via::CliOnMachine
        }
    }
}

/// Who `caller` is, as events and records name them.
pub fn actor(caller: &Caller) -> pb::Actor {
    use pb::actor::Actor;
    let actor = match caller {
        Caller::User(via) => Actor::User(pb::UserActor {
            via: user_via(via) as i32,
            machine: None,
        }),
        Caller::Agent(agent) => Actor::Agent(pb::AgentActor {
            session: Some(pb::SessionId {
                value: agent.session.id.as_str().to_owned(),
            }),
            machine: Some(pb::MachineId {
                value: agent.session.machine.as_str().to_owned(),
            }),
            agent: Some(pb::AgentId {
                value: agent.agent.clone(),
            }),
            session_title: agent.title.clone(),
        }),
        Caller::Script(script) => Actor::Script(pb::ScriptActor {
            token_id: script.id.clone(),
            name: script.name.clone(),
        }),
        Caller::Peer => Actor::System(pb::SystemActor { reason: 0 }),
    };
    pb::Actor { actor: Some(actor) }
}

// ---------------------------------------------------------------------------
// Port value types
// ---------------------------------------------------------------------------

/// A permission ask posted to [`InboxPort`].
#[derive(Clone, Debug)]
pub struct PermissionAsk {
    pub session: String,
    pub title: String,
    pub detail: String,
    pub choices: Vec<String>,
}

/// The user's decision on a [`PermissionAsk`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionDecision {
    Approved { choice: Option<String> },
    Denied { note: String },
    Withdrawn,
}

/// Which forwarded method an [`AwayPort`] call invokes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AwayMethod {
    Named(String),
    Operation(String),
}

/// Request payload forwarded over [`AwayPort`].
#[derive(Clone, Debug, Default)]
pub struct ForwardRequest {
    pub session: Option<String>,
    pub payload: Vec<u8>,
}

/// Reply payload returned by [`AwayPort::forward`].
#[derive(Clone, Debug, Default)]
pub struct ForwardReply {
    pub payload: Vec<u8>,
}

/// Abstract policy-copy source exposed by [`AwayPort`].
pub trait PolicySourcePort: Send + Sync {
    fn revision(&self) -> u64;
}

/// Opaque handle to a daemon's active uplinks.
#[derive(Clone, Default)]
pub struct UplinksHandle(pub Option<Arc<dyn Send + Sync>>);

/// Current state of a peer link to a machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerLinkState {
    Disconnected,
    Connecting,
    Connected,
    Failed { reason: String },
}

/// Specification for a remote operation dispatched through [`PeerRelayPort`].
#[derive(Clone, Debug)]
pub struct RemoteOpSpec {
    pub operation: String,
    pub payload: Vec<u8>,
}

/// Outcome of a remote machine operation.
#[derive(Clone, Debug)]
pub enum MachineOutcome {
    Ready,
    Gone,
    Failed(pb::Error),
}

/// Reference to a stage location for [`TransferPort`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlaceRef {
    pub machine: String,
    pub path: PathBuf,
}

/// Handle for tracking attached terminal writers.
pub trait WritersHandle: Send + Sync {
    fn has_writer(&self, tab: &str) -> bool;
}

/// Operation on a running holder via [`TerminalPort`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HolderOp {
    Stop,
    Resume,
    End,
}

/// Execution context for a machine script via [`MachinePort`].
#[derive(Clone, Debug)]
pub struct ScriptContext {
    pub machine: String,
    pub script_id: String,
}

/// What a caller waiting on daemon startup is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Awaited {
    Session,
    Terminal,
    Daemon,
}

/// Handle to host-side stop records for remote machines.
pub trait HostStopsHandle: Send + Sync {
    fn is_stopped(&self, machine: &str) -> bool;
}

/// A loaded session ready to be resumed via [`SessionPort`].
#[derive(Clone, Debug)]
pub struct LoadedSession {
    pub id: String,
    pub channel: String,
    pub machine: String,
    pub folder: PathBuf,
}

// ---------------------------------------------------------------------------
// Phase 4 + Phase 2 Cross-Service Port Traits (14 core ports + gate traits)
// ---------------------------------------------------------------------------

/// Port into `svc_inbox` (breaks 24 cross-service callers into `inbox`).
#[tonic::async_trait]
pub trait InboxPort: Send + Sync {
    async fn approved_on(&self, machine: &str, id: &str, session: &str) -> Option<pb::InboxRequest>;
    async fn pending_ids_of(&self, sessions: &[String]) -> Vec<String>;
    async fn withdraw_pending_of(
        &self,
        session: &str,
        reason: pb::WithdrawReason,
    ) -> Result<(), PortError>;
    async fn withdraw_all_pending(&self, reason: pb::WithdrawReason) -> Result<(), PortError>;
    async fn answer_relayed(
        &self,
        relayed: &pb::InboxAnswerRelay,
    ) -> Option<pb::InboxAnswerRefused>;
    async fn post_board_ask(&self, ask: BoardAsk) -> Result<Deferred, PortError>;
    async fn ask_permission(&self, req: PermissionAsk) -> Result<PermissionDecision, PortError>;
    fn screen_output(&self, session: &str, holder_pid: u32);
    fn screen_quiet(&self, session: &str, holder_pid: u32);
    fn screen_gone(&self, session: &str);
}

/// Port into `svc_away` (breaks 14 cross-service callers into `away`).
#[tonic::async_trait]
pub trait AwayPort: Send + Sync {
    async fn forward(
        &self,
        method: AwayMethod,
        req: ForwardRequest,
    ) -> Result<ForwardReply, PortError>;
    fn record_journal_now(&self, kind: &'static str, payload: serde_json::Value);
    fn policy_source(&self) -> Arc<dyn PolicySourcePort>;
    fn uplinks(&self) -> UplinksHandle;
    fn check_passphrase(&self, passphrase: &str) -> Result<(), PortError>;
    fn set_approval_passphrase(&self, passphrase: &str) -> Result<(), PortError>;
    fn verifier(&self) -> Option<String>;
    fn kept_policy_stamp(&self) -> Option<u64>;
}

/// Port into `svc_peer` (breaks 17 cross-service callers into `host_relay` / `peer_client`).
#[tonic::async_trait]
pub trait PeerRelayPort: Send + Sync {
    fn channel(&self, machine: &str) -> Result<tonic::transport::Channel, PortError>;
    async fn channel_for_machine(
        &self,
        machine: &str,
    ) -> Result<tonic::transport::Channel, PortError> {
        self.channel(machine)
    }
    fn link_state(&self, machine: &str) -> PeerLinkState;
    fn watch_link(&self, machine: &str) -> watch::Receiver<PeerLinkState>;
    fn states(&self) -> Vec<(String, PeerLinkState)>;
    async fn create_channel_on(
        &self,
        machine: &str,
        channel: &str,
        req: pb::CreateChannelRequest,
    ) -> Result<(), PortError>;
    async fn run_remote_op(
        &self,
        machine: &str,
        op: RemoteOpSpec,
    ) -> Result<MachineOutcome, PortError>;
    fn proven_channel(&self) -> Option<tonic::transport::Channel>;
}

/// Port into `svc_transfers` (`carry`, `carry_service`, `transfer_service`).
#[tonic::async_trait]
pub trait TransferPort: Send + Sync {
    async fn pack(
        &self,
        machine: &str,
        op: &str,
        session: &str,
    ) -> Result<pb::CarryPackResponse, PortError>;
    async fn unpack(
        &self,
        machine: &str,
        op: &str,
        session: &str,
        channel: &str,
    ) -> Result<pb::CarryUnpackResponse, PortError>;
    async fn copy_stage(&self, id: &str, from: PlaceRef, to: PlaceRef) -> Result<(), PortError>;
    async fn release_stage(&self, machine: &str, op: &str, session: &str) -> Result<(), PortError>;
    async fn abort_stage(&self, machine: &str, op: &str, session: &str) -> Result<(), PortError>;
}

/// Port into `svc_terminal` (`terminal`, `remote_terminal`, `running_holders`).
#[tonic::async_trait]
pub trait TerminalPort: Send + Sync {
    fn writers(&self) -> Arc<dyn WritersHandle>;
    fn forget_tab(&self, tab: &str);
    async fn type_for_script(
        &self,
        session: &str,
        text: &str,
        enter: bool,
        key: &str,
    ) -> Result<(), PortError>;
    async fn list_running_holders(
        &self,
        machine: &str,
    ) -> Result<Vec<pb::RunningHolder>, PortError>;
    async fn control_running_holder(&self, tab: &str, op: HolderOp) -> Result<(), PortError>;
}

/// Port into `svc_machines` (`machine_service`, `machine_scripts`).
#[tonic::async_trait]
pub trait MachinePort: Send + Sync {
    fn saw(&self, machine: &str, reach: pb::Reachability);
    fn link_up(&self, machine: &str, runtime: &tokio::runtime::Handle);
    fn link_down(&self, machine: &str);
    fn is_forgetting(&self, machine: &str) -> bool;
    async fn run_machine_script(&self, ctx: ScriptContext) -> Result<(), PortError>;
}

/// Port into `svc_lifecycle` (`upgrade`, `stop`, `restart`, `doctor`, `daemon`).
#[tonic::async_trait]
pub trait LifecyclePort: Send + Sync {
    fn require_running(&self, caller: &Caller) -> Result<(), PortError>;
    fn machine_ready(&self, machine: &pb::MachineId) -> Result<(), PortError>;
    async fn wait_started(&self, doing: Awaited) -> Result<(), PortError>;
    fn tab_token_env(
        &self,
        tab_id: &str,
        session_id: Option<&str>,
    ) -> Vec<(OsString, OsString)>;
    async fn approve_migration(&self, id: &str) -> Result<pb::InboxRequest, PortError>;
    fn host_stops(&self) -> Arc<dyn HostStopsHandle>;
}

/// Narrow gate for checking the current release and upgrade state (SPLIT §3.1, §6.2).
pub trait UpgradeGate: Send + Sync {
    fn current_release(&self) -> &Release;
    fn check_not_upgrading(&self) -> Result<(), PortError>;
}

/// Narrow gate for checking whether sessions on this daemon are stopped (SPLIT §3.1).
pub trait StopGate: Send + Sync {
    fn is_stopped(&self) -> bool;
    fn stopped_machine(&self) -> Option<&str>;
    fn require_running(&self, caller: &Caller) -> Result<(), PortError>;
}

/// Port into `svc_channels` (`channel_service`, `desk_service`).
#[tonic::async_trait]
pub trait ChannelPort: Send + Sync {
    fn tell_channel(&self, channel: &str, change: pb::Change, by: Option<pb::Actor>);
    async fn refuse_desk_channel_action(
        &self,
        session: &str,
        action: &str,
    ) -> Result<(), PortError>;
    async fn close_channel_desks(&self, channel: &str) -> Result<(), PortError>;
    async fn migrate_desk_folders(&self) -> Result<(), PortError>;
    async fn rewrite_channel_folders(&self, from: &Path, to: &Path) -> Result<(), PortError>;
}

/// Port into `svc_sessions_*` (breaks 21 cross-service callers into `session_service`).
#[tonic::async_trait]
pub trait SessionPort: Send + Sync {
    async fn publish_session(&self, session: &str, change: pb::Change, by: Option<pb::Actor>);
    fn watch_agent(&self, session: String);
    fn is_moving(&self, session: &str) -> bool;
    fn is_asking(&self, session: &str) -> bool;
    async fn resume_loaded(&self, caller: &Caller, loaded: LoadedSession) -> Result<(), PortError>;
    async fn begin_review(
        &self,
        caller: &Caller,
        channel_id: &str,
        req: &pb::StartReviewRequest,
    ) -> Result<pb::StartSessionResponse, PortError>;
    async fn repair_session(
        &self,
        caller: &Caller,
        id: String,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, PortError>;
    async fn adopted(&self, id: &str);
    async fn restart_session_shells(&self, session: &str) -> Result<(), PortError>;
}

/// Port implemented by `svc_sessions_transfer` so `SessionService` can delegate
/// transfer/roam/push RPCs without cross-crate inherent `impl SessionSvc` blocks.
#[tonic::async_trait]
pub trait SessionTransferPort: Send + Sync {
    async fn move_session(
        &self,
        caller: &Caller,
        req: pb::MoveSessionRequest,
    ) -> Result<pb::Operation, PortError>;
    async fn rebuild_session(
        &self,
        caller: &Caller,
        req: pb::RebuildSessionRequest,
    ) -> Result<pb::Operation, PortError>;
}

/// Port implemented by `svc_sessions_archive` so `SessionService` can delegate
/// archive/delete/forget RPCs without cross-crate inherent `impl SessionSvc` blocks.
#[tonic::async_trait]
pub trait SessionArchivePort: Send + Sync {
    async fn archive_session(
        &self,
        caller: &Caller,
        req: pb::ArchiveSessionRequest,
    ) -> Result<pb::Operation, PortError>;
}

/// Phase 2 port into `svc_board` (`board_service`).
#[tonic::async_trait]
pub trait BoardPort: Send + Sync {
    async fn replay_journal_entry(&self, machine: &str, payload: &[u8]) -> Result<(), PortError>;
    async fn forward_away_call(&self, payload: &[u8]) -> Result<Vec<u8>, PortError>;
    async fn remove_machine_boards(&self, machine: &str) -> Result<(), PortError>;
}

/// Phase 2 port into `svc_files` (`files`).
#[tonic::async_trait]
pub trait FilesPort: Send + Sync {
    fn publish_file_change(&self, session: &str, path: &Path);
    async fn resolve_root(&self, location: &pb::FileLocation) -> Result<PathBuf, PortError>;
}

/// Phase 2 port into `svc_review` (`review`).
#[tonic::async_trait]
pub trait ReviewPort: Send + Sync {
    fn forget_session(&self, session: &str);
    async fn replay_view_offer(&self, machine: &str, payload: &[u8]) -> Result<(), PortError>;
    async fn pull_request_binding(
        &self,
        session: &str,
    ) -> Result<Option<pb::PullRequestBinding>, PortError>;
}

/// Phase 2 port into `trash_service` (`Bin` / `TrashSvc`).
#[tonic::async_trait]
pub trait TrashBinPort: Send + Sync {
    fn move_to_folder(&self, folder: PathBuf);
    fn sweep(&self);
    async fn move_path_to_trash(
        &self,
        path: &Path,
        description: &str,
        caller_data: &str,
    ) -> Result<String, PortError>;
}

/// Phase 2 port into `svc_catalog` (`repo_service`, `skills_service`, `settings_service`, etc.).
#[tonic::async_trait]
pub trait CatalogPort: Send + Sync {
    fn default_agent_for(&self, channel: &str) -> Result<Option<String>, PortError>;
    async fn record_ready_repo(&self, owner: &str, name: &str) -> Result<(), PortError>;
    async fn forget_machine_skills(&self, machine: &str) -> Result<(), PortError>;
    fn skills_health(&self, machine: &str) -> (bool, String);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_method_path_names_its_dotted_operation() {
        for (path, operation) in [
            ("/humata.v1.ChannelService/ListChannels", "channel.list"),
            ("/humata.v1.SessionService/StartSession", "session.start"),
            ("/humata.v1.InboxService/RequestAccess", "inbox.request_access"),
            ("/humata.v1.TerminalService/OpenShellTab", "terminal.open_shell_tab"),
            ("/humata.v1.ChannelService/GetChannel", "channel.get"),
            ("not a path", ""),
        ] {
            assert_eq!(operation_of(path), operation, "{path}");
        }
    }

    #[test]
    fn names_maps_host_to_this_mac_and_named_machines_to_their_labels() {
        let names = Names::of_host("ma_host").naming("ma_box", "Build Box");
        assert_eq!(names.of("ma_host"), "This Mac");
        assert_eq!(names.of("ma_box"), "Build Box");
        assert_eq!(names.of("ma_unknown"), "ma_unknown");
    }

    #[test]
    fn port_error_converts_round_trip_with_status() {
        let err = PortError::from(Status::not_found("missing"));
        let status: Status = err.into();
        assert_eq!(status.code(), tonic::Code::NotFound);
    }
}
