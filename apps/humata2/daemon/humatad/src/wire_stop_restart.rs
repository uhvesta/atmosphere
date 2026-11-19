//! Wiring adapters connecting `humata2_svc_stop_restart` port traits to
//! `humatad`'s concrete services (`Guard`, `Inbox`, `SessionSvc`, `Tell`,
//! `Identities`, `HostVars`, `RelayAuth`, `Uplinks`, `Relay`, `Runner`, `Work`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use humata2_access::{AccessRequest, Call};
use humata2_daemon_ports::{Caller, Deferred, Gated, PeerLinkState, Run, TabIdentity};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Invocation, ManageError};
use humata2_sessions::Starter;
use humata2_store::{Restored, SessionRecord, Store};
use humata2_svc_stop_restart::daemon::Daemon;
use humata2_svc_stop_restart::doctor::Doctor;
use humata2_svc_stop_restart::ports::{
    Checked, CutRebuild, DaemonLinkPort, DaemonRelayAuthPort, DaemonUpgradePort,
    DaemonUplinksPort, DoctorSessionsPort, PeerFacts, RemoteStopPort, RestartSessionsPort,
    StopGuardPort, StopInboxPort, TabEnvPort, UplinkStream,
};
use humata2_svc_stop_restart::restart::{self, Operations, Started};
use humata2_svc_stop_restart::stop::StopFolder;
use humata2_svc_stop_restart::stop::boot::{AfterStop, Settle};
use humata2_svc_stop_restart::stop::remote::HostStops;
use humata2_svc_stop_restart::stop::service::{Relaunch, StopSvc};
use humata2_svc_stop_restart::stop::sweep::Sweep;
use tokio::sync::{Notify, watch};
use tonic::transport::Channel;
use tonic::{Request, Response, Status, Streaming};

use crate::away::Uplinks;
use crate::guard::{Checked as HumatadChecked, Guard};
use humata2_svc_machines::ports::LinkState;
use crate::host_vars::HostVars;
use crate::identity::{self, Identities};
use crate::inbox::Inbox;
use crate::machine_service::Work;
use crate::relay::Relay;
use crate::relay_auth::{ConnId as HumatadConnId, PeerFacts as HumatadPeerFacts, RelayAuth};
use crate::session_service::{self, SessionSvc, Tell};
use crate::status;
use crate::upgrade::run::Runner as UpgradeRunner;

/// Adapter implementing [`StopGuardPort`] over `humatad::guard::Guard`.
pub(crate) struct GuardAdapter(pub(crate) Arc<Guard>);

impl StopGuardPort for GuardAdapter {
    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        match self.0.check(caller, call)? {
            HumatadChecked::Allowed => Ok(Checked::Allowed),
            HumatadChecked::Ask(asked) => Ok(Checked::Ask(asked)),
        }
    }
}

/// Adapter implementing [`StopInboxPort`] over `humatad::inbox::Inbox`.
pub(crate) struct InboxAdapter(pub(crate) Arc<Inbox>);

#[tonic::async_trait]
impl StopInboxPort for InboxAdapter {
    async fn ask_deferred(
        &self,
        caller: &Caller,
        asked: AccessRequest,
        _response_type: &'static str,
        run: Run,
    ) -> Result<Deferred, Status> {
        Err(self.0.defer(caller, asked, None, run).await)
    }

    async fn withdraw_all_pending(&self, reason: pb::WithdrawReason) -> Result<(), Status> {
        self.0
            .withdraw_all_pending(reason)
            .await
            .map_err(|err| status::internal(status::chain(&err), ""))
    }
}

/// Adapter implementing [`DoctorSessionsPort`] over `humatad::session_service::SessionSvc`.
pub(crate) struct DoctorSessionsAdapter(pub(crate) SessionSvc);

#[tonic::async_trait]
impl DoctorSessionsPort for DoctorSessionsAdapter {
    async fn repair_session(
        &self,
        caller: &Caller,
        id: String,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        self.0.repair_session(caller, id, approved).await
    }

    async fn repair_tab(
        &self,
        caller: &Caller,
        id: String,
        approved: bool,
    ) -> Result<Gated<()>, Status> {
        self.0.repair_tab(caller, id, approved).await
    }
}

/// Adapter implementing [`RestartSessionsPort`] over `humatad::session_service`.
pub(crate) struct RestartSessionsAdapter {
    tell: tokio::sync::Mutex<Option<Tell>>,
}

impl RestartSessionsAdapter {
    pub(crate) fn new(tell: Tell) -> Self {
        Self {
            tell: tokio::sync::Mutex::new(Some(tell)),
        }
    }
}

#[tonic::async_trait]
impl RestartSessionsPort for RestartSessionsAdapter {
    fn cut_rebuilds(&self, _starter: &Starter) -> Vec<CutRebuild> {
        Vec::new()
    }

    async fn after_restart(&self) {
        if let Some(tell) = self.tell.lock().await.take() {
            session_service::after_restart(tell).await;
        }
    }

    async fn undo_cut_rebuilds(
        &self,
        starter: &Starter,
        _cut: Vec<CutRebuild>,
        operations: &Operations,
    ) {
        let cut = session_service::cut_rebuilds(starter);
        for rebuild in &cut {
            rebuild.note(operations);
        }
        session_service::undo_cut_rebuilds(starter, cut, operations).await;
    }
}

/// Direct [`TabEnvPort`] implementation on `humatad::identity::Identities`.
impl TabEnvPort for Identities {
    fn host_variables(&self, _tab_id: &str) -> Result<Vec<(OsString, OsString)>, String> {
        HostVars::installed()
            .and_then(|vars| vars.env())
            .map_err(|err| status::chain(&err))
    }

    fn revoke(&self, tab_id: &str) -> Result<(), String> {
        self.revoke(tab_id).map_err(|err| status::chain(&err))
    }

    fn mint_tab_env(
        &self,
        tab_id: &str,
        session: &SessionRecord,
        kind: TabIdentity,
        socket: &Path,
    ) -> Result<Vec<(OsString, OsString)>, String> {
        let id_kind = match kind {
            TabIdentity::Session => identity::TabIdentity::Session,
            TabIdentity::AddedAgent => identity::TabIdentity::AddedAgent,
        };
        let token = self
            .mint_as(tab_id, session, id_kind)
            .map_err(|err| status::chain(&err))?;
        Ok(identity::tab_env(token, socket))
    }

    fn tools_path(&self, host_path: Option<OsString>) -> Option<OsString> {
        identity::tools_path(host_path)
    }
}

/// Adapter implementing [`TabEnvPort`] over `humatad::identity::Identities` and `HostVars`.
pub(crate) struct TabEnvAdapter(pub(crate) Arc<Identities>);

impl TabEnvPort for TabEnvAdapter {
    fn host_variables(&self, tab_id: &str) -> Result<Vec<(OsString, OsString)>, String> {
        TabEnvPort::host_variables(self.0.as_ref(), tab_id)
    }

    fn revoke(&self, tab_id: &str) -> Result<(), String> {
        TabEnvPort::revoke(self.0.as_ref(), tab_id)
    }

    fn mint_tab_env(
        &self,
        tab_id: &str,
        session: &SessionRecord,
        kind: TabIdentity,
        socket: &Path,
    ) -> Result<Vec<(OsString, OsString)>, String> {
        TabEnvPort::mint_tab_env(self.0.as_ref(), tab_id, session, kind, socket)
    }

    fn tools_path(&self, host_path: Option<OsString>) -> Option<OsString> {
        TabEnvPort::tools_path(self.0.as_ref(), host_path)
    }
}

/// Adapter implementing [`DaemonRelayAuthPort`] over `humatad::relay_auth::RelayAuth`.
pub(crate) struct RelayAuthAdapter(pub(crate) Arc<RelayAuth>);

impl DaemonRelayAuthPort for RelayAuthAdapter {
    fn begin(&self, conn: u64, now: Instant) -> [u8; 32] {
        self.0.begin(HumatadConnId(conn), now)
    }

    fn complete(
        &self,
        conn: u64,
        facts: &PeerFacts,
        host_nonce: &[u8],
        proof: &[u8],
        now: Instant,
    ) -> Result<(), Status> {
        let peer = HumatadPeerFacts {
            pid: facts.pid.and_then(|p| i32::try_from(p).ok()),
            uid: facts.uid,
        };
        self.0
            .complete(HumatadConnId(conn), &peer, host_nonce, proof, now)
            .map_err(|refusal| refusal.status())
    }
}

/// Adapter implementing [`DaemonUplinksPort`] over `humatad::away::Uplinks`.
pub(crate) struct UplinksAdapter(pub(crate) Arc<Uplinks>);

impl DaemonUplinksPort for UplinksAdapter {
    fn reachable(&self) -> bool {
        self.0.reachable()
    }

    fn open(
        &self,
        caller: Option<&Caller>,
        stream: Streaming<pb::HostToMachine>,
    ) -> Result<UplinkStream, Status> {
        let inner = self.0.open(caller, stream)?;
        Ok(Box::pin(inner))
    }
}

/// Adapter implementing [`DaemonLinkPort`] over `humatad::relay::Relay`.
pub(crate) struct RelayLinkAdapter(pub(crate) Relay);

impl DaemonLinkPort for RelayLinkAdapter {
    fn proven_channel(&self) -> Option<Channel> {
        self.0.proven_channel()
    }
}

/// Adapter implementing [`DaemonUpgradePort`] over `humatad::upgrade::run::Runner`.
pub(crate) struct UpgradeAdapter(pub(crate) UpgradeRunner);

#[tonic::async_trait]
impl DaemonUpgradePort for UpgradeAdapter {
    async fn upgrade(
        &self,
        request: Request<pb::UpgradeRequest>,
    ) -> Result<Response<pb::UpgradeResponse>, Status> {
        self.0.upgrade(request).await
    }
}

/// Adapter implementing [`RemoteStopPort`] over `humatad::machine_service::Work`.
pub(crate) struct RemoteStopAdapter {
    work: Arc<Work>,
    stops: Option<HostStops>,
}

impl RemoteStopAdapter {
    pub(crate) fn new(work: Arc<Work>, stops: Option<HostStops>) -> Self {
        Self { work, stops }
    }
}

impl RemoteStopPort for RemoteStopAdapter {
    fn stops(&self) -> Option<&HostStops> {
        self.stops.as_ref()
    }

    fn publish_machines(&self) {
        self.work.publish_machines();
    }

    fn watch_link(&self, machine: &str) -> watch::Receiver<PeerLinkState> {
        let mut inner = self.work.links().watch(machine);
        let initial = map_link_state(&*inner.borrow());
        let (tx, rx) = watch::channel(initial);
        tokio::spawn(async move {
            while inner.changed().await.is_ok() {
                if tx.send(map_link_state(&*inner.borrow())).is_err() {
                    break;
                }
            }
        });
        rx
    }

    fn channel(&self, machine: &str) -> Result<Channel, String> {
        self.work
            .links()
            .channel(machine)
            .map_err(|err| status::chain(&err))
    }

    fn link_up(&self, machine: &str, runtime: &tokio::runtime::Handle) {
        self.work.link_up(machine, runtime);
    }

    fn stop_sessions_command(
        &self,
        machine: &str,
        stop_id: &str,
        by_machine: &str,
        reason: &str,
    ) -> Result<Invocation, ManageError> {
        self.work
            .machines
            .stop_sessions_command(machine, stop_id, by_machine, reason)
    }
}

fn map_link_state(state: &LinkState) -> PeerLinkState {
    match state {
        LinkState::Up { .. } => PeerLinkState::Connected,
        LinkState::Down => PeerLinkState::Disconnected,
    }
}

// ---------------------------------------------------------------------------
// Constructors for `humatad::serve`
// ---------------------------------------------------------------------------

pub(crate) fn new_stop_svc(
    folder: StopFolder,
    machine: String,
    relaunch: Relaunch,
    guard: Arc<Guard>,
    inbox: Arc<Inbox>,
) -> StopSvc {
    StopSvc::new(
        folder,
        machine,
        relaunch,
        Arc::new(GuardAdapter(guard)),
        Arc::new(InboxAdapter(inbox)),
    )
}

pub(crate) fn new_doctor(
    sessions: SessionSvc,
    inbox: Arc<Inbox>,
    store: Arc<Store>,
    machine: String,
    leave: Arc<Notify>,
) -> Doctor {
    Doctor::new(
        Arc::new(DoctorSessionsAdapter(sessions)),
        Arc::new(InboxAdapter(inbox)),
        store,
        machine,
        leave,
    )
}

pub(crate) fn new_settle(
    folder: StopFolder,
    record: Option<humata2_svc_stop_restart::stop::Record>,
    sweep: Sweep,
    store: Arc<Store>,
    inbox: Arc<Inbox>,
) -> Settle {
    Settle {
        folder,
        record,
        sweep,
        store,
        inbox: Arc::new(InboxAdapter(inbox)),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn restart_in_background(
    store: Arc<Store>,
    identities: Arc<Identities>,
    starter: Starter,
    restored: Option<Restored>,
    tell: Tell,
    socket: PathBuf,
    after_stop: Option<AfterStop>,
) -> Arc<Started> {
    restart::in_background(
        store,
        Arc::new(TabEnvAdapter(identities)),
        starter,
        restored,
        Arc::new(RestartSessionsAdapter::new(tell)),
        socket,
        after_stop,
    )
}

pub(crate) fn new_daemon(machine: String, role: pb::DaemonRole) -> Daemon {
    Daemon::new(machine, role)
}
