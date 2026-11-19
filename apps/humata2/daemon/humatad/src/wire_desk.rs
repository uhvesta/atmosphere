//! Adapters wiring `humata2_svc_desk` port traits to `humatad`'s concrete
//! services (`Guard`, `Operations`, `Identities`, `Tokens`, `Inbox`, `Tell`,
//! `Bin`, `Core`, `RemoteWork`, `Migrations`, `ChannelSvc`, `Started`, `Feed`).

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use humata2_access::{AccessRequest, Call, Capability, ChannelId, ChannelRules, Decision, Grant, Policy, SessionId};
use humata2_daemon_ports::{Caller, Gated, Run};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::Starter;
use humata2_store::TabRecord;
use humata2_svc_desk::ports::{
    Awaited, Checked, DeskChannelPort, DeskFeedPort, DeskGuardPort, DeskIdentityPort,
    DeskInboxPort, DeskMigrationsPort, DeskOperationsPort, DeskRemotePort, DeskRestartPort,
    DeskSessionPort, DeskTellPort, DeskTerminalPort, DeskTokensPort, DeskTrashPort,
    FolderMigrator, HostOp, LinkState, LinkWatch, Loaded, Moving, OperationState, Origin,
    Progress, ProgressPort, RelayError, RelayedClients, RemoteError, RestartOperationsPort, Scope,
    Started, TrashOps,
};
use humata2_svc_desk::{DeskDeps, DeskSvc, server};
use tokio::sync::{broadcast, watch};
use tonic::Status;

use crate::channel_service::ChannelSvc;
use crate::events::Feed;
use crate::guard::Guard;
use crate::identity::Identities;
use crate::inbox::Inbox;
use crate::operations::Operations;
use crate::remote_work::RemoteWork;
use crate::restart::{Started as RestartStarted, Tokens};
use crate::session_service::Tell;
use crate::terminal::Core;
use crate::trash_service::Bin;
use crate::upgrade::items::Migrations;

pub(crate) struct DeskGuardAdapter(pub(crate) Arc<Guard>);

impl DeskGuardPort for DeskGuardAdapter {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        match self.0.check(caller, call)? {
            crate::guard::Checked::Allowed => Ok(Checked::Allowed),
            crate::guard::Checked::Ask(req) => Ok(Checked::Ask(req)),
        }
    }

    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        self.0.require(caller, call)
    }

    fn end_grants_of(&self, session: &SessionId) -> Vec<Grant> {
        self.0.with_policy(|policy, _| policy.end_grants_of(session))
    }

    fn channel_decision(&self, channel: &ChannelId, capability: Capability) -> Option<Decision> {
        self.0.with_policy(|policy, _| {
            policy
                .channel_rules(channel)
                .and_then(|rules| rules.decision(capability))
        })
    }

    fn channel_rules(&self, channel: &ChannelId) -> Option<ChannelRules> {
        self.0
            .with_policy(|policy, _| policy.channel_rules(channel).cloned())
    }

    fn policy(&self) -> Policy {
        self.0.with_policy(|policy, _| policy.clone())
    }
}

pub(crate) struct DeskIdentityAdapter(pub(crate) Arc<Identities>);

impl DeskIdentityPort for DeskIdentityAdapter {
    fn revoke(&self, tab_id: &str) {
        self.0.revoke(tab_id);
    }
}

pub(crate) struct DeskTokensAdapter(pub(crate) Arc<Tokens>);

impl DeskTokensPort for DeskTokensAdapter {
    fn env(&self, tab: &TabRecord) -> Vec<(OsString, OsString)> {
        self.0.env(tab)
    }
}

pub(crate) struct DeskInboxAdapter(pub(crate) Arc<Inbox>);

#[tonic::async_trait]
impl DeskInboxPort for DeskInboxAdapter {
    async fn ask_read(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        operation: &str,
        run: Option<Run>,
    ) -> Result<(), Status> {
        self.0.ask_read(caller, asked, operation, run).await
    }

    async fn withdraw_pending_of(
        &self,
        session: &str,
        reason: pb::WithdrawReason,
    ) -> Result<(), Status> {
        self.0.withdraw_pending_of(session, reason).await
    }

    async fn revoked(&self, id: &str, at: SystemTime) -> Result<(), Status> {
        self.0.revoked(id, at).await
    }

    async fn defer(&self, caller: &Caller, asked: AccessRequest, key: Option<String>, run: Run) -> Status {
        self.0.defer(caller, asked, key, run).await
    }
}

pub(crate) struct DeskTerminalAdapter(pub(crate) Arc<Core>);

impl DeskTerminalPort for DeskTerminalAdapter {
    fn forget_tab(&self, tab_id: &str) {
        self.0.forget_tab(tab_id);
    }
}

struct TrashOpsAdapter(Arc<humata2_transfers::Trash>);

impl TrashOps for TrashOpsAdapter {
    fn put(
        &self,
        folder: &Path,
        item: humata2_transfers::NewItem,
    ) -> Result<humata2_transfers::TrashItem, Status> {
        self.0
            .put(folder, item)
            .map_err(|e| crate::status::internal("move to trash", &e.to_string()))
    }
}

pub(crate) struct DeskTrashAdapter(pub(crate) Arc<Bin>);

impl DeskTrashPort for DeskTrashAdapter {
    fn get(&self) -> Result<Arc<dyn TrashOps>, Status> {
        Ok(Arc::new(TrashOpsAdapter(self.0.get()?)))
    }
}

pub(crate) struct DeskFeedAdapter(pub(crate) Arc<Feed>);

impl DeskFeedPort for DeskFeedAdapter {
    fn publish_in(
        &self,
        change: pb::Change,
        payload: pb::event::Payload,
        scope: Scope,
        by: Option<pb::Actor>,
    ) {
        self.0.publish_in(
            change,
            payload,
            crate::events::Scope {
                channel: scope.channel,
                session: scope.session,
            },
            by,
        );
    }

    fn subscribe(&self, _after: Option<u64>) -> (u64, broadcast::Receiver<pb::Event>) {
        let (_tx, rx) = broadcast::channel(64);
        (0, rx)
    }
}

pub(crate) struct DeskTellAdapter(pub(crate) Tell);

#[tonic::async_trait]
impl DeskTellPort for DeskTellAdapter {
    async fn session(&self, session_id: &str, change: pb::Change, by: Option<pb::Actor>) {
        self.0.session(session_id, change, by).await;
    }

    async fn channel(&self, _channel_id: &str, _change: pb::Change, _by: Option<pb::Actor>) {}
}

pub(crate) struct DeskChannelAdapter(pub(crate) ChannelSvc);

#[tonic::async_trait]
impl DeskChannelPort for DeskChannelAdapter {
    async fn add_repo_to_channel(
        &self,
        caller: &Caller,
        req: pb::AddRepoRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        self.0.add_repo_to_channel(caller, req, approved).await
    }

    fn record_added(&self, owner: &str, name: &str, place: &humata2_repo_library::Place) {
        self.0.record_added(owner, name, place);
    }

    fn watch_aliases(&self) -> watch::Receiver<HashMap<String, PathBuf>> {
        let (_tx, rx) = watch::channel(HashMap::new());
        rx
    }
}

struct ProgressAdapter(std::sync::Mutex<Option<crate::operations::Progress>>);

impl ProgressAdapter {
    fn new(p: crate::operations::Progress) -> Self {
        Self(std::sync::Mutex::new(Some(p)))
    }

    /// Takes the operation's own handle out, for work that ends it itself.
    fn take(&self) -> Option<crate::operations::Progress> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    }
}

#[tonic::async_trait]
impl ProgressPort for ProgressAdapter {
    fn id(&self) -> String {
        self.0
            .lock()
            .ok()
            .and_then(|g| g.as_ref().and_then(|p| p.operation().id.map(|i| i.value)))
            .unwrap_or_default()
    }

    fn operation(&self) -> pb::Operation {
        self.0
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|p| p.operation()))
            .unwrap_or_default()
    }

    fn origin(&self) -> Origin {
        self.0
            .lock()
            .ok()
            .and_then(|g| {
                g.as_ref().map(|p| {
                    let o = p.origin();
                    Origin {
                        owner: o.owner,
                        scope: o.scope,
                    }
                })
            })
            .unwrap_or(Origin {
                owner: String::new(),
                scope: None,
            })
    }

    fn set_step(&self, key: &str, state: pb::StepState, output_tail: Option<String>) {
        if let Ok(g) = self.0.lock()
            && let Some(p) = g.as_ref()
        {
            p.set_step(key, state, output_tail);
        }
    }

    fn succeed(&self, result: Option<pb::operation::Result>) {
        if let Ok(mut g) = self.0.lock()
            && let Some(p) = g.take()
        {
            p.succeed(result);
        }
    }

    fn fail(&self, error: pb::Error) {
        if let Ok(mut g) = self.0.lock()
            && let Some(p) = g.take()
        {
            p.fail(error);
        }
    }

    fn end_cancelled(&self) {
        if let Ok(mut g) = self.0.lock()
            && let Some(p) = g.take()
        {
            p.end_cancelled();
        }
    }

    fn watch(&self) -> watch::Receiver<pb::Operation> {
        self.0
            .lock()
            .ok()
            .and_then(|g| g.as_ref().map(|p| p.watch()))
            .unwrap_or_else(|| watch::channel(pb::Operation::default()).1)
    }
}

pub(crate) struct DeskOperationsAdapter(pub(crate) Arc<Operations>);

#[tonic::async_trait]
impl DeskOperationsPort for DeskOperationsAdapter {
    fn existing(
        &self,
        caller: &Caller,
        key: Option<&pb::IdempotencyKey>,
    ) -> Result<Option<pb::Operation>, Status> {
        self.0.existing(caller, key)
    }

    fn progress_of(&self, caller: &Caller, id: &str) -> Result<Progress, Status> {
        self.0
            .progress_of(caller, id)
            .map(|p| Progress::new(Arc::new(ProgressAdapter::new(p))))
    }

    fn restore(&self, op: pb::Operation, origin: &Origin) -> Option<Progress> {
        let o = crate::operations::Origin {
            owner: origin.owner.clone(),
            scope: origin.scope.clone(),
        };
        self.0
            .restore(op, &o)
            .map(|p| Progress::new(Arc::new(ProgressAdapter::new(p))))
    }

    fn start_steps(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        target: pb::EntityRef,
        steps: &[(String, String)],
        cancellable: bool,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<Started, Status> {
        match self
            .0
            .start_steps(caller, kind, target, steps, cancellable, key)?
        {
            crate::operations::Started::New(p) => {
                Ok(Started::New(Progress::new(Arc::new(ProgressAdapter::new(p)))))
            }
            crate::operations::Started::Existing(op) => Ok(Started::Existing(op)),
        }
    }
}

/// The restart's record of the operations it found cut short (D241).
struct RestartOpsAdapter(crate::restart::Operations);

impl RestartOperationsPort for RestartOpsAdapter {
    fn record(&self, session: &str, kind: &str, step: &str, state: OperationState) {
        use crate::restart::OperationState as Kept;
        let state = match state {
            OperationState::Pending { waits_for } => Kept::Pending { waits_for },
            OperationState::Running | OperationState::Succeeded | OperationState::Finished => Kept::Finished,
            OperationState::Failed(reason) | OperationState::Stuck { reason } => Kept::Stuck { reason },
            OperationState::Undone { reason } => Kept::Undone { reason },
        };
        self.0.record(session, kind, step, state);
    }
}

pub(crate) struct DeskRestartAdapter(pub(crate) Arc<RestartStarted>);

#[tonic::async_trait]
impl DeskRestartPort for DeskRestartAdapter {
    async fn wait_for(&self, awaited: Awaited) -> Result<(), Status> {
        let a = match awaited {
            Awaited::DeskStart | Awaited::DeskMove => crate::restart::Awaited::DeskStart,
        };
        self.0.wait_for(a).await
    }

    async fn wait(&self) -> pb::LastStart {
        self.0.wait().await;
        self.0.get().cloned().unwrap_or_default()
    }

    fn get(&self) -> Option<pb::LastStart> {
        self.0.get().cloned()
    }

    fn operations(&self) -> Arc<dyn RestartOperationsPort> {
        Arc::new(RestartOpsAdapter(self.0.operations().clone()))
    }

    async fn operation_state(&self, _id: &str) -> Option<OperationState> {
        None
    }

    async fn record_operation(&self, _id: String, _state: OperationState) {}
}

pub(crate) struct DeskMigrationsAdapter(pub(crate) Migrations);

#[tonic::async_trait]
impl DeskMigrationsPort for DeskMigrationsAdapter {
    fn machine_ready(&self, machine: &pb::MachineId) -> Result<(), Status> {
        crate::upgrade::gate::machine_ready(&self.0, machine)
    }

    async fn observe(
        &self,
        thing: pb::MigrateThing,
        machine: &pb::MachineId,
        host: pb::BuildInfo,
        peer: pb::BuildInfo,
    ) -> Result<(), Status> {
        self.0
            .observe(thing, machine, host, peer)
            .await
            .map(|_| ())
            .map_err(|e| crate::status::internal("migrations", &e.to_string()))
    }

    fn set_desk_folders(&self, _migrator: Arc<dyn FolderMigrator>) {}

    async fn resolved(&self, thing: pb::MigrateThing, machine: &pb::MachineId) -> Result<(), Status> {
        self.0
            .resolved(thing, machine)
            .await
            .map_err(|e| crate::status::internal("migrations", &e.to_string()))
    }
}

/// The session service's work a Desk session shares with a channel session.
pub(crate) struct DeskSessionAdapter {
    pub(crate) starter: Starter,
    pub(crate) tell: Tell,
    pub(crate) tokens: Arc<Tokens>,
}

#[tonic::async_trait]
impl DeskSessionPort for DeskSessionAdapter {
    async fn agent_ready(&self, agents: &humata2_sessions::Agents, agent: &str) -> Result<(), Status> {
        crate::agent_gate::ready(agents, agent).await.map_err(Status::from)
    }

    fn agent_enabled(&self, agents: &humata2_sessions::Agents, agent: &str) -> Result<(), Status> {
        crate::agent_gate::enabled(agents, agent).map_err(Status::from)
    }

    fn watch_agent(&self, session: String) {
        crate::session_service::watch_agent(self.tell.clone(), session);
    }

    fn running_turn(&self, session: &str, holder_pid: u32) -> pb::SessionStatus {
        crate::session_service::running_turn(session, holder_pid)
    }

    fn moving(&self, session: &str, to: &str, operation: &str) -> Moving {
        Moving::new(Box::new(crate::session_service::Moving::begin(session, to, operation)))
    }

    fn session_follows(
        &self,
        store: &humata2_store::Store,
        agents: &humata2_sessions::Agents,
        home: &Path,
        channels: &Path,
        session: &str,
        from: &Path,
        to: &Path,
    ) -> Result<Vec<humata2_revive::FollowStep>, humata2_store::StoreError> {
        crate::channel_service::rename_folder::session_follows(store, agents, home, channels, session, from, to)
    }

    async fn run_resume(&self, by: pb::Actor, loaded: Loaded, progress: Progress) {
        let Some(own) = progress
            .port()
            .downcast_ref::<ProgressAdapter>()
            .and_then(ProgressAdapter::take)
        else {
            progress.fail(resume_error(
                "the Desk session's resume was handed an operation this daemon didn't start",
                "resume the session again",
            ));
            return;
        };
        let (store, key) = (self.starter.store.clone(), loaded.session.id.clone());
        let read = crate::channel_service::blocking(move || crate::session_service::Loaded::read(&store, &key)).await;
        let full = match read {
            Ok(Ok(Some(full))) => full,
            Ok(Ok(None)) => {
                own.fail(resume_error(
                    format!("Desk session {} is gone from this machine", loaded.session.id),
                    "list the Desk's sessions to see which are here",
                ));
                return;
            }
            Ok(Err(error)) => {
                own.fail(resume_error(
                    format!("couldn't read Desk session {} to resume it: {error}", loaded.session.id),
                    "check the data folder's disk, then resume the session again",
                ));
                return;
            }
            Err(error) => {
                own.fail(resume_error(error.message(), "resume the session again"));
                return;
            }
        };
        crate::session_service::run_resume(self.starter.clone(), self.tell.clone(), by, self.tokens.clone(), full, own)
            .await;
    }
}

/// A Desk resume that couldn't begin: what failed and how to fix it.
fn resume_error(message: impl Into<String>, fix: impl Into<String>) -> pb::Error {
    pb::Error {
        message: message.into(),
        fix: fix.into(),
        retry: None,
        kind: None,
    }
}

/// Construct `humata2_svc_desk::server` wired to `humatad`'s concrete services.
#[allow(clippy::too_many_arguments)]
pub(crate) fn desk_server(
    guard: Arc<Guard>,
    operations: Arc<Operations>,
    starter: Starter,
    identities: Arc<Identities>,
    tokens: Arc<Tokens>,
    inbox: Arc<Inbox>,
    tell: Tell,
    machine: String,
    socket: PathBuf,
    default_agent: String,
    bin: Arc<Bin>,
    terminals: Arc<Core>,
    state: PathBuf,
    handle: humata2_svc_desk::desk_service::channel_desk_close::DeskHandle,
    remote: Option<Arc<dyn DeskRemotePort>>,
    migrations: Migrations,
    channels: ChannelSvc,
    last_start: Arc<RestartStarted>,
) -> pb::desk_service_server::DeskServiceServer<DeskSvc> {
    let sessions = Arc::new(DeskSessionAdapter {
        starter: starter.clone(),
        tell: tell.clone(),
        tokens: tokens.clone(),
    });
    server(DeskDeps {
        guard: Arc::new(DeskGuardAdapter(guard)),
        operations: Arc::new(DeskOperationsAdapter(operations)),
        starter,
        identities: Arc::new(DeskIdentityAdapter(identities)),
        tokens: Arc::new(DeskTokensAdapter(tokens)),
        inbox: Arc::new(DeskInboxAdapter(inbox)),
        tell: Arc::new(DeskTellAdapter(tell)),
        machine,
        socket,
        default_agent,
        bin: Arc::new(DeskTrashAdapter(bin)),
        terminals: Arc::new(DeskTerminalAdapter(terminals)),
        state,
        handle,
        remote,
        migrations: Arc::new(DeskMigrationsAdapter(migrations)),
        channels: Arc::new(DeskChannelAdapter(channels)),
        last_start: Arc::new(DeskRestartAdapter(last_start)),
        sessions,
    })
}
