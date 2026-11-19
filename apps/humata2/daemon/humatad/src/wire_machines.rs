//! Adapters wiring `humata2_svc_machines` port traits onto `humatad`'s
//! concrete services, plus constructors for `Work`, `MachineSvc`, and
//! `MachineScripts`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use humata2_access::Call;
use humata2_daemon_ports::{Caller, Run};
use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Builds, Machines, OpStep, StepEvent};
use humata2_store::Store;
use humata2_svc_machines::ports::{
    Checked, HostVarsError, LinkState, MachineChannelsPort, MachineFeedPort, MachineGuardPort,
    MachineHoldersPort, MachineHostProbePort, MachineHostVarsPort, MachineInboxPort,
    MachineLinksPort, MachineOperationsPort, MachinePeerReportsPort, MachineRestartPort,
    MachineSessionsPort, MachineSkillsPort, MachineStopsPort, MachineStoragePort,
    MachineTerminalPort, MachineTrashPort, MachineUpgradePort, MoveError, OperationOrigin,
    OperationProgress, OperationStarted, RelayError, RestartOpState, ScriptSessionView, StopCopy,
};
use humata2_svc_machines::{Clock, MachineScripts, MachineSvc, Parts, Work};
use tokio::sync::{broadcast, watch};
use tonic::Status;

use crate::channel_service;
use crate::events::Feed;
use crate::guard::{self, Guard};
use crate::host_relay::{LinkState as RelayLinkState, Links};
use crate::host_vars::HostVars;
use crate::inbox::Inbox;
use crate::operations::{Operations, Origin, Progress, Started};
use crate::peer_reports::PeerReports;
use crate::restart::{OperationState as RestartState, Started as RestartStarted};
use crate::session_service::SessionSvc;
use crate::skills_service::SkillsHost;
use crate::stop::remote::HostStops;
use crate::storage::Storage;
use crate::terminal::Core as TerminalCore;
use crate::trash_service::Bin;
use crate::upgrade::items::Watch as UpgradeWatch;

// ---------------------------------------------------------------------------
// Guard
// ---------------------------------------------------------------------------

pub(crate) struct GuardAdapter(pub(crate) Arc<Guard>);

impl MachineGuardPort for GuardAdapter {
    fn machine(&self) -> &str {
        self.0.machine()
    }

    fn check(&self, caller: &Caller, call: &Call) -> Result<Checked, Status> {
        match self.0.check(caller, call)? {
            guard::Checked::Allowed => Ok(Checked::Allowed),
            guard::Checked::Ask(ask) => Ok(Checked::Ask(ask)),
        }
    }

    fn require(&self, caller: &Caller, call: &Call) -> Result<(), Status> {
        self.0.require(caller, call)
    }

    fn principal(&self, caller: &Caller) -> String {
        format!("{:?}", guard::principal(caller))
    }

    fn is_present_user(&self, caller: &Caller) -> bool {
        let principal = guard::principal(caller);
        self.0
            .with_policy(|_, context| principal.present_user(context))
            .is_some()
    }
}

// ---------------------------------------------------------------------------
// Feed
// ---------------------------------------------------------------------------

pub(crate) struct FeedAdapter(pub(crate) Arc<Feed>);

impl MachineFeedPort for FeedAdapter {
    fn publish(&self, change: pb::Change, payload: pb::event::Payload, by: Option<pb::Actor>) {
        self.0.publish(change, payload, by);
    }

    fn subscribe(&self) -> broadcast::Receiver<pb::Event> {
        let (_, mut rx) = self.0.subscribe(None);
        let (tx, out) = broadcast::channel(64);
        tokio::spawn(async move {
            while let Ok(published) = rx.recv().await {
                if tx.send((**published).clone()).is_err() {
                    break;
                }
            }
        });
        out
    }
}

// ---------------------------------------------------------------------------
// Inbox
// ---------------------------------------------------------------------------

pub(crate) struct InboxAdapter(pub(crate) Arc<Inbox>);

#[tonic::async_trait]
impl MachineInboxPort for InboxAdapter {
    async fn ask(
        &self,
        caller: &Caller,
        asked: humata2_access::AccessRequest,
        operation: &str,
        run: Option<Run>,
    ) -> Result<(), Status> {
        self.0.ask_read(caller, asked, operation, run).await
    }

    async fn post_host_ask(
        &self,
        key: &str,
        card: pb::InboxRequest,
        run: Run,
    ) -> Result<(), Status> {
        self.0.post_host_ask(key, card, run).await
    }

    async fn withdraw_host_ask(&self, key: &str, reason: pb::WithdrawReason) {
        self.0.withdraw_host_ask(key, reason).await;
    }

    async fn withdraw_machine(
        &self,
        machine: &str,
        reason: pb::WithdrawReason,
    ) -> Result<(), Status> {
        self.0.withdraw_pending_from(machine, reason).await
    }

    async fn forget_journal(&self, machine: &str) {
        self.0.forget_journal(machine).await;
    }

    fn forget_journal_now(&self, machine: &str) {
        self.0.forget_journal_now(machine);
    }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

pub(crate) struct ProgressAdapter(pub(crate) Mutex<Option<Progress>>);

impl ProgressAdapter {
    fn new(progress: Progress) -> Self {
        Self(Mutex::new(Some(progress)))
    }
}

impl OperationProgress for ProgressAdapter {
    fn id(&self) -> String {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .and_then(|p| p.operation().id)
            .map(|id| id.value)
            .unwrap_or_default()
    }

    fn step(&self, index: usize, state: pb::StepState, detail: String) {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.as_ref() {
            let op = p.operation();
            if let Some(step) = op.steps.get(index) {
                p.set_step(&step.key, state, Some(detail));
            }
        }
    }

    fn set_step(&self, key: &str, state: pb::StepState, detail: Option<String>) {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.as_ref() {
            p.set_step(key, state, detail);
        }
    }

    fn step_event(&self, event: &StepEvent) {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.as_ref() {
            p.step(event);
        }
    }

    fn detail(&self, _detail: String) {}

    fn cancelled(&self) -> bool {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.as_ref().is_some_and(Progress::cancelled)
    }

    fn succeed(&self, result: Option<pb::operation::Result>) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.take() {
            p.succeed(result);
        }
    }

    fn end_cancelled(&self) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.take() {
            p.end_cancelled();
        }
    }

    fn fail(&self, error: pb::Error) {
        let mut guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.take() {
            p.fail(error);
        }
    }

    fn rerun_with(&self, rerun: Arc<dyn Fn(Arc<dyn OperationProgress>) + Send + Sync>) {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = guard.as_ref() {
            p.rerun_with(Arc::new(move |progress| {
                rerun(Arc::new(ProgressAdapter::new(progress)));
            }));
        }
    }

    fn finish_ok(&self, _result_type: &str, _result_bytes: Vec<u8>) {
        self.succeed(None);
    }

    fn finish_err(&self, error: pb::Error) {
        self.fail(error);
    }

    fn snapshot(&self) -> pb::Operation {
        let guard = self.0.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .as_ref()
            .map(Progress::operation)
            .unwrap_or_default()
    }
}

pub(crate) struct OperationsAdapter(pub(crate) Arc<Operations>);

impl MachineOperationsPort for OperationsAdapter {
    fn existing(
        &self,
        caller: &Caller,
        key: Option<&pb::IdempotencyKey>,
    ) -> Result<Option<pb::Operation>, Status> {
        self.0.existing(caller, key)
    }

    fn start(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        key: Option<pb::IdempotencyKey>,
        steps: Vec<String>,
    ) -> Result<OperationStarted, Status> {
        let specs: Vec<(String, String)> = steps
            .into_iter()
            .enumerate()
            .map(|(i, title)| (format!("step-{i}"), title))
            .collect();
        match self
            .0
            .start_steps(caller, kind, pb::EntityRef::default(), &specs, false, key)?
        {
            Started::New(p) => Ok(OperationStarted::Started(Arc::new(ProgressAdapter::new(p)))),
            Started::Existing(op) => Ok(OperationStarted::Existing(op)),
        }
    }

    fn start_op(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        target: pb::EntityRef,
        steps: &[OpStep],
        retryable: bool,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<OperationStarted, Status> {
        match self.0.start(caller, kind, target, steps, retryable, key)? {
            Started::New(p) => Ok(OperationStarted::Started(Arc::new(ProgressAdapter::new(p)))),
            Started::Existing(op) => Ok(OperationStarted::Existing(op)),
        }
    }

    fn start_steps(
        &self,
        caller: &Caller,
        kind: pb::OperationKind,
        key: Option<pb::IdempotencyKey>,
        steps: Vec<pb::OperationStep>,
    ) -> Result<OperationStarted, Status> {
        let specs: Vec<(String, String)> = steps
            .into_iter()
            .map(|s| (s.key, s.title))
            .collect();
        match self
            .0
            .start_steps(caller, kind, pb::EntityRef::default(), &specs, false, key)?
        {
            Started::New(p) => Ok(OperationStarted::Started(Arc::new(ProgressAdapter::new(p)))),
            Started::Existing(op) => Ok(OperationStarted::Existing(op)),
        }
    }

    fn restore(
        &self,
        operation: pb::Operation,
        origin: &OperationOrigin,
    ) -> Option<Arc<dyn OperationProgress>> {
        let mapped = Origin {
            owner: origin.caller_data.clone(),
            scope: None,
        };
        self.0
            .restore(operation, &mapped)
            .map(|p| Arc::new(ProgressAdapter::new(p)) as Arc<dyn OperationProgress>)
    }
}

// ---------------------------------------------------------------------------
// Storage, Trash, Channels, HostVars, HostProbe
// ---------------------------------------------------------------------------

pub(crate) struct TrashAdapter(pub(crate) Arc<Bin>);

impl MachineTrashPort for TrashAdapter {
    fn move_to_folder(&self, folder: PathBuf) {
        self.0.move_to(folder);
    }
}

pub(crate) struct StorageAdapter(pub(crate) Arc<Storage>);

impl MachineStoragePort for StorageAdapter {
    fn store(&self) -> &Store {
        self.0.store().as_ref()
    }

    fn holders(&self) -> PathBuf {
        self.0.holders()
    }

    fn root(&self) -> PathBuf {
        self.0.channels_root()
    }

    fn data_dir(&self) -> PathBuf {
        self.0.home().to_path_buf()
    }

    fn layout(&self) -> humata2_sessions::Layout {
        self.0.layout().clone()
    }

    fn storage_summary(&self) -> Result<pb::StorageReport, Status> {
        self.0.report(Vec::new())
    }

    fn report(&self, extra: Vec<pb::RetentionSkip>) -> Result<pb::StorageReport, Status> {
        self.0.report(extra)
    }

    fn machine_of_session(&self, session: &str) -> Result<Option<String>, Status> {
        self.0.machine_of_session(session)
    }

    fn clear_build_cache(
        &self,
        request: &pb::ClearBuildCacheRequest,
    ) -> Result<pb::ClearBuildCacheResponse, Status> {
        self.0.clear_build_cache(request)
    }

    fn trim_logs(&self, request: &pb::TrimLogsRequest) -> Result<pb::TrimLogsResponse, Status> {
        self.0.trim_logs(request)
    }

    fn running_agents(&self) -> Result<u32, Status> {
        self.0.running_agents()
    }

    fn trash_bin(&self) -> Option<Arc<dyn MachineTrashPort>> {
        Some(Arc::new(TrashAdapter(self.0.bin().clone())))
    }
}

pub(crate) struct ChannelsAdapter;

impl MachineChannelsPort for ChannelsAdapter {
    fn carry_conversations(
        &self,
        _store: &Store,
        _from_root: &Path,
        _to_root: &Path,
    ) -> Result<(), MoveError> {
        Ok(())
    }

    fn rewrite_folders(
        &self,
        _store: &Store,
        _from_root: &Path,
        _to_root: &Path,
    ) -> Result<(), Status> {
        Ok(())
    }
}

pub(crate) struct HostVarsAdapter;

/// The port's error for `error`, with the kind clients branch on (the
/// field a refused name or value is in) kept.
fn host_vars_error(error: &crate::host_vars::HostVarsError) -> HostVarsError {
    let status = error.status();
    let said = crate::status::error_of(&status);
    HostVarsError {
        message: status.message().to_owned(),
        fix: said.as_ref().map(|said| said.fix.clone()).unwrap_or_default(),
        code: status.code(),
        kind: said.and_then(|said| said.kind),
    }
}

impl MachineHostVarsPort for HostVarsAdapter {
    fn names(&self) -> Result<Vec<String>, HostVarsError> {
        HostVars::installed()
            .and_then(|vars| vars.names())
            .map_err(|err| host_vars_error(&err))
    }

    fn check(&self, name: &str, value: &str) -> Result<(), HostVarsError> {
        HostVars::check(name, value).map_err(|err| host_vars_error(&err))
    }

    fn set(&self, name: &str, value: &str) -> Result<(), HostVarsError> {
        HostVars::installed()
            .and_then(|vars| vars.set(name, value))
            .map_err(|err| host_vars_error(&err))
    }

    fn remove(&self, name: &str) -> Result<(), HostVarsError> {
        HostVars::installed()
            .and_then(|vars| vars.remove(name).map(|_| ()))
            .map_err(|err| host_vars_error(&err))
    }
}

pub(crate) struct HostProbeAdapter;

impl MachineHostProbePort for HostProbeAdapter {
    fn read_probe(&self, include_folders: bool) -> pb::ProbeResponse {
        crate::peer::host_probe::read(include_folders)
    }
}

// ---------------------------------------------------------------------------
// Links, PeerReports, Stops, Skills, Upgrade, Holders, Sessions, Terminal, Restart
// ---------------------------------------------------------------------------

pub(crate) struct LinksAdapter(pub(crate) Links);

fn map_link_state(links: &Links, machine: &str, state: &RelayLinkState) -> LinkState {
    match state {
        RelayLinkState::Up { .. } => match links.channel(machine) {
            Ok(channel) => LinkState::Up { channel },
            Err(_) => LinkState::Down,
        },
        RelayLinkState::Down => LinkState::Down,
    }
}

impl MachineLinksPort for LinksAdapter {
    fn channel(&self, machine: &str) -> Result<tonic::transport::Channel, RelayError> {
        self.0
            .channel(machine)
            .map_err(|_| RelayError {
                machine: machine.to_owned(),
            })
    }

    fn up(&self, machine: &str, channel: tonic::transport::Channel) {
        self.0.up(machine, channel);
    }

    fn down(&self, machine: &str) {
        self.0.down(machine);
    }

    fn forget(&self, machine: &str) {
        self.0.forget(machine);
    }

    fn watch(&self, machine: &str) -> watch::Receiver<LinkState> {
        let mut rx = self.0.watch(machine);
        let links = self.0.clone();
        let name = machine.to_owned();
        let init = map_link_state(&links, &name, &rx.borrow());
        let (tx, out) = watch::channel(init);
        tokio::spawn(async move {
            while rx.changed().await.is_ok() {
                if tx.send(map_link_state(&links, &name, &rx.borrow())).is_err() {
                    break;
                }
            }
        });
        out
    }

    fn watch_all(&self) -> watch::Receiver<HashMap<String, LinkState>> {
        let mut follow = self.0.watch_all();
        let links = self.0.clone();
        let (tx, out) = watch::channel(HashMap::new());
        tokio::spawn(async move {
            let mut current = HashMap::new();
            while let Some((machine, state)) = follow.next().await {
                let mapped = map_link_state(&links, &machine, &state);
                current.insert(machine, mapped);
                if tx.send(current.clone()).is_err() {
                    break;
                }
            }
        });
        out
    }
}

pub(crate) struct PeerReportsAdapter(pub(crate) PeerReports);

impl MachinePeerReportsPort for PeerReportsAdapter {
    fn link_up(&self, machine: &str) {
        self.0.link_up(machine);
    }

    fn link_down(&self, machine: &str, at: SystemTime) {
        self.0.link_down(machine, at);
    }

    fn forget(&self, machine: &str) {
        self.0.forget(machine);
    }

    fn accept(&self, machine: &str, message: pb::MachineToHost) {
        self.0.accept(machine, message);
    }
}

pub(crate) struct SkillsAdapter(pub(crate) Arc<SkillsHost>);

#[tonic::async_trait]
impl MachineSkillsPort for SkillsAdapter {
    fn health(&self, machine: &str) -> (bool, Option<pb::Error>) {
        self.0.health(machine)
    }

    fn link_up(&self, machine: &str, channel: tonic::transport::Channel) {
        self.0.link_up(machine, channel);
    }

    fn link_down(&self, machine: &str) {
        self.0.link_down(machine);
    }

    async fn forget(&self, machine: &str) -> Result<(), Status> {
        self.0
            .forget(machine)
            .await
            .map_err(|err| Status::internal(err.to_string()))
    }

    fn forget_now(&self, machine: &str) -> Result<(), Status> {
        self.0
            .forget_now(machine)
            .map_err(|err| Status::internal(err.to_string()))
    }
}

pub(crate) struct UpgradeAdapter(pub(crate) UpgradeWatch);

impl MachineUpgradePort for UpgradeAdapter {
    fn link_up(&self, _machine: &str, _daemon_version: &str) {}

    fn forget(&self, machine: &str) {
        let watch = self.0.clone();
        let machine = machine.to_owned();
        tokio::spawn(async move {
            watch.removed(&machine).await;
        });
    }
}

pub(crate) struct HoldersAdapter {
    pub(crate) storage: Arc<Storage>,
    pub(crate) sessions: Arc<OnceLock<SessionSvc>>,
}

#[tonic::async_trait]
impl MachineHoldersPort for HoldersAdapter {
    async fn list(&self, machine: &str) -> Result<Vec<pb::RunningHolder>, Status> {
        let Some(sessions) = self.sessions.get() else {
            return Err(Status::internal("session service is not wired"));
        };
        crate::running_holders::list(
            self.storage.store(),
            &self.storage.holders(),
            machine,
            &crate::wire_terminal::SessionsAdapter(sessions.clone()),
            &crate::wire_terminal::LifecycleAdapter,
        )
        .await
        .map_err(|err| err.into_status(machine))
    }

    async fn stop(&self, machine: &str, tab: &str) -> Result<(), Status> {
        crate::running_holders::stop(self.storage.store(), tab)
            .await
            .map_err(|err| err.into_status(machine))
    }

    async fn resume(&self, machine: &str, caller: &Caller, tab: &str) -> Result<(), Status> {
        let Some(sessions) = self.sessions.get() else {
            return Err(Status::internal("session service is not wired"));
        };
        crate::running_holders::resume(
            self.storage.store(),
            &crate::wire_terminal::SessionsAdapter(sessions.clone()),
            caller,
            tab,
        )
        .await
        .map_err(|err| err.into_status(machine))
    }

    async fn end(&self, machine: &str, tab: &str) -> Result<(), Status> {
        crate::running_holders::end(
            self.storage.store(),
            &crate::wire_terminal::LifecycleAdapter,
            tab,
        )
        .await
        .map_err(|err| err.into_status(machine))
    }

    async fn adopt(
        &self,
        machine: &str,
        _caller: &Caller,
        tab: &str,
        channel: &str,
    ) -> Result<String, Status> {
        let builtin = humata2_sessions::Agents::builtin();
        let sessions = self.sessions.get();
        let (agents, home) =
            sessions.map_or((&builtin, Path::new("")), |s| s.agents_and_home());
        let session = crate::running_holders::adopt(
            self.storage.store(),
            &self.storage.holders(),
            agents,
            home,
            tab,
            channel,
            true,
        )
        .await
        .map_err(|err| err.into_status(machine))?;
        if let Some(sessions) = sessions {
            sessions.adopted(&session).await;
        }
        Ok(session)
    }

    async fn let_go(&self, sessions: Vec<String>, channels: Vec<String>) -> Result<u32, Status> {
        let let_go = crate::running_holders::let_go(self.storage.store(), sessions, channels)
            .await
            .map_err(|err| err.into_status("this"))?;
        Ok(u32::try_from(let_go.len()).unwrap_or(u32::MAX))
    }
}

pub(crate) struct SessionsAdapter(pub(crate) SessionSvc);

#[tonic::async_trait]
impl MachineSessionsPort for SessionsAdapter {
    async fn start_for_script(
        &self,
        machine: &str,
        agent: &str,
        channel: &str,
        title: &str,
        key: &str,
    ) -> Result<String, Status> {
        self.0
            .start_for_script(machine, agent, channel, title, key)
            .await
    }

    async fn script_view(&self, session: &str) -> Result<ScriptSessionView, Status> {
        let view = self.0.script_view(session).await?;
        Ok(ScriptSessionView {
            agent: view.agent,
            turn_end: view.turn_end,
            status: view.status,
        })
    }
}

pub(crate) struct TerminalAdapter(pub(crate) Arc<TerminalCore>);

#[tonic::async_trait]
impl MachineTerminalPort for TerminalAdapter {
    async fn type_for_script(
        &self,
        session: &str,
        text: &str,
        enter: bool,
        key: &str,
    ) -> Result<(), Status> {
        self.0.type_for_script(session, text, enter, key).await
    }
}

pub(crate) struct RestartAdapter(pub(crate) Arc<RestartStarted>);

impl MachineRestartPort for RestartAdapter {
    fn set_operation_state(&self, name: &str, state: RestartOpState) {
        let mapped = match state {
            RestartOpState::Pending { waits_for } => RestartState::Pending { waits_for },
            RestartOpState::Running => RestartState::Pending {
                waits_for: "running".to_owned(),
            },
            RestartOpState::Done => RestartState::Finished,
            RestartOpState::Failed { reason } => RestartState::Stuck { reason },
        };
        self.0.operations().record(name, "forget", "", mapped);
    }
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

pub(crate) fn new_work(
    machines: Machines,
    builds: Box<dyn Builds + Send + Sync>,
    feed: Arc<Feed>,
    links: Links,
    reports: PeerReports,
    clock: Clock,
) -> Arc<Work> {
    let work = Arc::new(Work::new(
        machines,
        builds,
        Arc::new(FeedAdapter(feed)),
        Arc::new(LinksAdapter(links)),
        Arc::new(PeerReportsAdapter(reports)),
        clock,
    ));
    let _ = work.host_probe.set(Arc::new(HostProbeAdapter));
    work
}

pub(crate) fn new_machine_svc(
    guard: Arc<Guard>,
    work: Arc<Work>,
    operations: Arc<Operations>,
    inbox: Arc<Inbox>,
    storage: Arc<Storage>,
    sessions: Arc<OnceLock<SessionSvc>>,
) -> MachineSvc {
    MachineSvc::new(
        Arc::new(GuardAdapter(guard)),
        work,
        Arc::new(OperationsAdapter(operations)),
        Arc::new(InboxAdapter(inbox)),
        Arc::new(StorageAdapter(storage.clone())),
        Arc::new(ChannelsAdapter),
        Arc::new(HostVarsAdapter),
        Arc::new(HoldersAdapter { storage, sessions }),
    )
}

pub(crate) fn new_machine_scripts(
    work: Arc<Work>,
    sessions: SessionSvc,
    terminal: Arc<TerminalCore>,
    feed: Arc<Feed>,
    inbox: Arc<Inbox>,
    humata_home: PathBuf,
    state: PathBuf,
) -> Arc<MachineScripts> {
    humata2_svc_machines::build(Parts {
        work,
        sessions: Arc::new(SessionsAdapter(sessions)),
        terminal: Arc::new(TerminalAdapter(terminal)),
        feed: Arc::new(FeedAdapter(feed)),
        inbox: Arc::new(InboxAdapter(inbox)),
        humata_home,
        state,
    })
}

pub(crate) struct AwayAdapter(pub(crate) Arc<crate::away::Forwarder>);

#[tonic::async_trait]
impl humata2_svc_machines::ports::MachineAwayPort for AwayAdapter {
    async fn forward(
        &self,
        method: &str,
        caller: &Caller,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, Status> {
        let session = match caller {
            Caller::Agent(agent) => agent.session.id.as_str(),
            _ => return Err(Status::permission_denied("only agents can forward away calls")),
        };
        let row = match method {
            "list_machines"
            | "humata.v1.MachineService/ListMachines"
            | "humata.v1.MachineService.ListMachines" => crate::away::Method::ListMachines.row(),
            "get_machine"
            | "humata.v1.MachineService/GetMachine"
            | "humata.v1.MachineService.GetMachine" => {
                crate::away::Method::GetMachine.row()
            }
            other => return Err(Status::unimplemented(format!("unknown forwarded method {other}"))),
        };
        self.0.call(session, row, payload, None).await
    }
}

pub(crate) fn new_box_machine_svc(
    storage: Arc<Storage>,
    machine: String,
    operations: Arc<Operations>,
    forwarder: Arc<crate::away::Forwarder>,
    sessions: Arc<OnceLock<SessionSvc>>,
) -> humata2_svc_machines::machine_service::BoxMachineSvc {
    humata2_svc_machines::machine_service::BoxMachineSvc::new(
        Arc::new(StorageAdapter(storage.clone())),
        Arc::new(ChannelsAdapter),
        machine,
        Arc::new(OperationsAdapter(operations)),
        Arc::new(AwayAdapter(forwarder)),
        Arc::new(HostVarsAdapter),
        Arc::new(HoldersAdapter { storage, sessions }),
    )
}

// ---------------------------------------------------------------------------
// Peer links: the host dials each machine's `serve --stdio`
// ---------------------------------------------------------------------------

/// Boards for a daemon built without them: nothing a machine journals as
/// `board.` is its own, and a forwarded call has no board to run on.
pub(crate) struct NoBoards;

#[tonic::async_trait]
impl humata2_svc_peer::ports::PeerBoardPort for NoBoards {
    async fn replay_at(
        &self,
        _machine: &str,
        _entry: &pb::JournalEntry,
        _link_up: i64,
    ) -> humata2_svc_peer::ports::Replayed {
        humata2_svc_peer::ports::Replayed::Rejected
    }

    async fn forwarded(&self, _machine: &str, call: pb::ForwardedCall) -> pb::ForwardedResult {
        pb::ForwardedResult {
            call_id: call.call_id,
            outcome: Some(pb::forwarded_result::Outcome::Error(pb::Error {
                message: "this daemon is built without boards, so it can't run the call".to_owned(),
                ..pb::Error::default()
            })),
        }
    }
}

/// Views for a daemon built without them.
pub(crate) struct NoViews;

#[tonic::async_trait]
impl humata2_svc_peer::ports::PeerReviewPort for NoViews {
    async fn replay_offered(
        &self,
        _machine: &str,
        _entry: &pb::JournalEntry,
    ) -> humata2_svc_peer::ports::Replayed {
        humata2_svc_peer::ports::Replayed::Rejected
    }
}

/// What the host does with a link besides keeping it up (see
/// [`crate::peer_client::HostSide`]).
pub(crate) struct HostParts {
    pub(crate) policy: Arc<dyn humata2_svc_peer::ports::PolicySourcePort>,
    pub(crate) inbox: Arc<dyn humata2_svc_peer::ports::PeerInboxPort>,
    pub(crate) boards: Arc<dyn humata2_svc_peer::ports::PeerBoardPort>,
    pub(crate) views: Arc<dyn humata2_svc_peer::ports::PeerReviewPort>,
    pub(crate) upgrade: Option<Arc<dyn humata2_svc_peer::ports::PeerUpgradeWatchPort>>,
}

/// Dials each machine through its link command and keeps the link up.
pub(crate) struct PeerDialAdapter {
    work: std::sync::Weak<Work>,
    runtime: tokio::runtime::Handle,
    reports: PeerReports,
    host: HostParts,
}

impl PeerDialAdapter {
    /// Must be made inside the daemon's runtime: links are spawned on it,
    /// whichever thread asks for one.
    pub(crate) fn new(work: &Arc<Work>, reports: PeerReports, host: HostParts) -> Self {
        Self {
            work: Arc::downgrade(work),
            runtime: tokio::runtime::Handle::current(),
            reports,
            host,
        }
    }

    fn host_side(&self) -> crate::peer_client::HostSide {
        crate::peer_client::HostSide {
            policy: self.host.policy.clone(),
            inbox: self.host.inbox.clone(),
            boards: self.host.boards.clone(),
            views: self.host.views.clone(),
            upgrade: self.host.upgrade.clone(),
        }
    }
}

fn identity_wrong(error: &crate::peer_client::LinkError) -> bool {
    use crate::peer_client::LinkError;
    matches!(
        error,
        LinkError::MachineProofWrong { .. }
            | LinkError::MachineRefused {
                reason: pb::peer_unproven::Reason::HostProofWrong,
                ..
            }
    )
}

impl humata2_svc_machines::ports::MachinePeerDialPort for PeerDialAdapter {
    fn spawn_supervisor(
        &self,
        entry: humata2_machines::HostEntry,
        on_event: Arc<dyn Fn(humata2_svc_machines::ports::LinkEvent) + Send + Sync>,
    ) -> tokio::task::AbortHandle {
        use crate::peer_client::{CommandDial, LinkEvent as Event};
        use humata2_svc_machines::ports::LinkEvent as Told;
        let name = entry.name;
        let Some(work) = self.work.upgrade() else {
            return self.runtime.spawn(async {}).abort_handle();
        };
        // A login that just finished hands over its terminal; otherwise the
        // link dials its command.
        let dial: Box<dyn crate::peer_client::Dial> = match crate::login_service::one_shot(&name) {
            Some(dial) => dial,
            None => match work.machines.peer_link(&name) {
                Ok(invocation) => Box::new(CommandDial::new(name.clone(), invocation)),
                Err(error) => {
                    crate::log::say!(
                        "humatad2: couldn't make the peer link command for {name}: {}",
                        crate::status::chain(&error)
                    );
                    return self.runtime.spawn(async {}).abort_handle();
                }
            },
        };
        let key_file = work.machines.peer_key(&name);
        let sink: crate::peer_client::Sink = {
            let (reports, name) = (self.reports.clone(), name.clone());
            Box::new(move |message| reports.accept(&name, message))
        };
        let report: crate::peer_client::Report = Box::new(move |event: Event<'_>| {
            on_event(match event {
                Event::Up {
                    daemon_version,
                    channel,
                    stopped,
                } => Told::Up {
                    daemon_version: daemon_version.to_owned(),
                    channel: channel.clone(),
                    stopped: stopped.cloned(),
                },
                Event::Down { error, retry_in } => {
                    let reason = format!("{}; {}", crate::status::chain(error), error.fix());
                    if identity_wrong(error) {
                        Told::IdentityMismatch { reason }
                    } else {
                        Told::Down {
                            reason,
                            next_retry_at: retry_in.map(|wait| SystemTime::now() + wait),
                        }
                    }
                }
            });
        });
        self.runtime
            .spawn(crate::peer_client::supervise(
                dial,
                key_file,
                name,
                report,
                sink,
                Some(self.host_side()),
            ))
            .abort_handle()
    }
}

// ---------------------------------------------------------------------------
// Stops kept for other machines (D262)
// ---------------------------------------------------------------------------

/// Adapts the host's stop copies and `stop::remote` to [`MachineStopsPort`].
pub(crate) struct StopsAdapter {
    stops: HostStops,
    remote: Arc<dyn humata2_svc_stop_restart::ports::RemoteStopPort>,
}

impl StopsAdapter {
    pub(crate) fn new(work: Arc<Work>, stops: HostStops) -> Self {
        let remote = Arc::new(crate::wire_stop_restart::RemoteStopAdapter::new(
            work,
            Some(stops.clone()),
        ));
        Self { stops, remote }
    }
}

fn stop_copy(copy: &crate::stop::host_copy::HostCopy) -> StopCopy {
    StopCopy {
        pb_record: crate::stop::boot::to_pb(&copy.record),
        by_user: copy.record.by.kind == crate::stop::ByKind::User,
        delivered: copy.delivered,
    }
}

#[tonic::async_trait]
impl MachineStopsPort for StopsAdapter {
    fn get(&self, machine: &str) -> Option<StopCopy> {
        self.stops.get(machine).as_ref().map(stop_copy)
    }

    fn this_machine(&self) -> Option<pb::SessionsStopped> {
        self.stops.this_machine()
    }

    fn link_up(&self, machine: &str, told: Option<&pb::SessionsStopped>, channel: tonic::transport::Channel) {
        crate::stop::remote::link_up(&self.remote, machine, told, &channel);
    }

    fn remove(&self, machine: &str) -> Result<(), Status> {
        self.stops
            .remove(machine)
            .map_err(|error| Status::internal(crate::status::chain(&error)))
    }

    fn forget(&self, machine: &str) {
        if let Err(error) = self.stops.remove(machine) {
            crate::log::say!(
                "humatad2: couldn't drop the stop kept for {machine}: {}",
                crate::status::chain(&error)
            );
        }
    }

    async fn stop_remote(
        &self,
        machine: &str,
        stop_id: String,
        reason: String,
        via: pb::StopVia,
        by: String,
    ) -> Result<StopCopy, Status> {
        use crate::stop::Via;
        let via = match via {
            pb::StopVia::Unspecified | pb::StopVia::App => Via::App,
            pb::StopVia::Cli | pb::StopVia::Ssh | pb::StopVia::Direct => Via::Cli,
        };
        let ask = crate::stop::remote::Ask {
            stop_id,
            reason,
            via,
            by,
        };
        let reach = crate::stop::remote::stop(&self.remote, machine, ask)
            .await
            .map_err(|error| error.status())?;
        Ok(StopCopy {
            pb_record: crate::stop::boot::to_pb(&reach.record),
            by_user: reach.record.by.kind == crate::stop::ByKind::User,
            delivered: reach.delivered,
        })
    }

    async fn start_remote(&self, machine: &str) -> Result<bool, Status> {
        let started = crate::stop::remote::start(&self.remote, machine)
            .await
            .map_err(|error| error.status())?;
        Ok(matches!(started, crate::stop::remote::Started::Cancelled))
    }
}

/// Shows a machine whose link is down and whose reconnects need the user as
/// NeedsLogin (MP-16).
pub(crate) struct LoginAdapter {
    hosts_file: PathBuf,
}

impl LoginAdapter {
    pub(crate) fn new(work: &Work) -> Self {
        Self {
            hosts_file: work.machines.hosts_file().to_owned(),
        }
    }
}

impl humata2_svc_machines::ports::MachineLoginPort for LoginAdapter {
    fn needs_login(&self, entry: &humata2_machines::HostEntry, reachability: pb::Reachability) -> pb::Reachability {
        if reachability == pb::Reachability::Offline
            && humata2_machines::is_interactive(&self.hosts_file, &entry.name)
        {
            pb::Reachability::NeedsLogin
        } else {
            reachability
        }
    }

    fn login_needs_user(&self, machine: &str, _reason: &str) -> pb::LoginNeedsUser {
        pb::LoginNeedsUser {
            machine: Some(pb::MachineId {
                value: machine.to_owned(),
            }),
            ..pb::LoginNeedsUser::default()
        }
    }
}
