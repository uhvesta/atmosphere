//! Wiring adapters connecting `humata2_svc_stop_restart` port traits to
//! `humatad`'s concrete services (`Guard`, `Inbox`, `SessionSvc`, `Tell`,
//! `Identities`, `HostVars`, `RelayAuth`, `Uplinks`, `Relay`, `Runner`, `Work`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use humata2_access::{AccessRequest, Call};
use humata2_config_compose::{AgentKind, ProtectedHumataMcp};
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
use crate::config_sync::ConfigSync;
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

    fn session_compose_env(&self, session: &SessionRecord, agent: &str) -> Result<Vec<(OsString, OsString)>, String> {
        match installed_compose() {
            Some(compose) => compose.env(session, agent),
            None => Ok(Vec::new()),
        }
    }

    fn session_home(&self, session: &str, agent: &str) -> Option<std::path::PathBuf> {
        let compose = installed_compose()?;
        crate::config_sync::composed_home_of(compose.sync.machine_root(), session, agent).map(|(_, dir)| dir)
    }
}

/// D895: gives a local agent tab its session's composed config home. The
/// real dotfiles are only read; a tab already running is never touched.
pub(crate) struct ComposeEnv {
    sync: Arc<ConfigSync>,
    protected: ProtectedHumataMcp,
    /// The FUSE tree is mounted; without it agents get no `HUMATA_FUSE_ROOT`
    /// and use the files tools instead.
    fuse_mounted: Arc<AtomicBool>,
}

/// The agent a session's agent name stands for, if it has a config home to
/// compose.
fn compose_kind(agent: &str) -> Option<AgentKind> {
    match agent {
        "claude" => Some(AgentKind::Claude),
        "codex" => Some(AgentKind::Codex),
        "copilot" => Some(AgentKind::Copilot),
        "agy" => Some(AgentKind::Agy),
        _ => None,
    }
}

/// Whether a launch records the agent's conversation file in its config home.
fn records_conversations(kind: AgentKind) -> bool {
    !matches!(kind, AgentKind::Agy)
}

impl ComposeEnv {
    /// `humatad` and `socket` make the `humata` MCP entry no composing may
    /// change, the same one the session's MCP config files hold.
    pub(crate) fn new(
        sync: Arc<ConfigSync>,
        humatad: &Path,
        socket: &Path,
        fuse_mounted: Arc<AtomicBool>,
    ) -> Self {
        let protected = ProtectedHumataMcp::new(
            humatad.to_string_lossy().into_owned(),
            vec!["mcp".to_owned(), "--socket".to_owned(), socket.to_string_lossy().into_owned()],
        );
        Self { sync, protected, fuse_mounted }
    }

    /// Decides where `session`'s `agent` keeps its config home, once, for
    /// the launch record: composes it and returns it, or None when composing
    /// failed, so the record names the agent's usual (global) home. None too
    /// for a session whose agent has no conversations to record there.
    /// A home that was already there is kept when its refresh fails; one this
    /// plan began making is removed (that folder only), so the record and
    /// what [`Self::env`] finds on disk agree. [`Self::env`] follows the disk.
    fn plan_home(&self, session: &str, agent: &str) -> Option<PathBuf> {
        let kind = compose_kind(agent).filter(|kind| records_conversations(*kind))?;
        let dir = crate::config_sync::session_home(self.sync.machine_root(), session, kind);
        let existed = dir.is_dir();
        match self.sync.compose(session, kind, self.protected.clone()) {
            Ok(report) => Some(report.session_config_dir),
            Err(error) if existed => {
                crate::log::say!(
                    "humatad2: session {session}'s {agent} keeps its config home as it was: {}",
                    status::chain(&error)
                );
                Some(dir)
            }
            Err(error) => {
                crate::log::say!(
                    "humatad2: session {session}'s {agent} is recorded in its usual config home: {}",
                    status::chain(&error)
                );
                if let Err(removal) = std::fs::remove_dir_all(&dir)
                    && dir.exists()
                {
                    crate::log::say!(
                        "humatad2: couldn't clear the half-made config home {}, so it is used: {removal}",
                        dir.display()
                    );
                    return Some(dir);
                }
                None
            }
        }
    }

    /// The variables for `session`'s agent tab: its home variable pointing
    /// at the composed folder, the FUSE root and the machine's name. None
    /// for a session on another machine, or an agent with no config home.
    /// An agent whose conversations are recorded follows its record: it gets
    /// a home only if [`Self::plan_home`] made one (refreshed here, and kept
    /// if the refresh fails), else it runs in its usual home.
    fn env(&self, session: &SessionRecord, agent: &str) -> Result<Vec<(OsString, OsString)>, String> {
        if !session.machine.is_empty() && session.machine != self.sync.machine_name() {
            return Ok(Vec::new());
        }
        let Some(kind) = compose_kind(agent) else {
            // D916: a shell (or any tab with no composed home) still reaches
            // the mount and knows its machine.
            return Ok(self.place_env());
        };
        let existing = crate::config_sync::composed_home_of(self.sync.machine_root(), &session.id, agent).map(|(_, dir)| dir);
        let mut env = Vec::new();
        if existing.is_some() || !records_conversations(kind) {
            match self.sync.compose(&session.id, kind, self.protected.clone()) {
                Ok(report) => {
                    // SMB-078: apply the full composed env_plan (including
                    // CLAUDE_SECURESTORAGE_CONFIG_DIR="" on macOS).
                    for (k, v) in report.env_plan.env_vars {
                        env.push((OsString::from(k), OsString::from(v)));
                    }
                }
                Err(error) => match existing {
                    Some(dir) => {
                        crate::log::say!(
                            "humatad2: session {}'s {agent} keeps its config home as it was: {}",
                            session.id,
                            status::chain(&error)
                        );
                        env.push((OsString::from(kind.config_env_var()), dir.into_os_string()));
                        if kind == AgentKind::Claude && cfg!(target_os = "macos") {
                            env.push((OsString::from("CLAUDE_SECURESTORAGE_CONFIG_DIR"), OsString::new()));
                        }
                    }
                    None => return Err(status::chain(&error)),
                },
            }
        }
        env.extend(self.place_env());
        Ok(env)
    }

    /// The FUSE root, only while the tree is mounted, and the machine's name.
    fn place_env(&self) -> Vec<(OsString, OsString)> {
        let mut env = Vec::new();
        if self.fuse_mounted.load(Ordering::SeqCst) {
            env.push((OsString::from("HUMATA_FUSE_ROOT"), self.sync.fuse_root().as_os_str().to_owned()));
        }
        env.push((OsString::from("HUMATA_MACHINE_NAME"), OsString::from(self.sync.machine_name())));
        env
    }

    #[allow(dead_code, reason = "called once archive and delete have one place to say a session is gone")]
    fn forget(&self, session: &str) {
        self.sync.unregister(session);
    }
}

/// The composing this daemon does, set once at its start, before any tab is
/// brought back.
static COMPOSE: Mutex<Option<Arc<ComposeEnv>>> = Mutex::new(None);

pub(crate) fn use_compose_env(compose: Arc<ComposeEnv>) {
    crate::config_sync::use_machine_root(compose.sync.machine_root().to_path_buf());
    *COMPOSE.lock().unwrap_or_else(PoisonError::into_inner) = Some(compose);
}

/// The seam a launch records its conversation file through: the composed
/// home of `session`'s `agent`, made now, or None when composing failed or
/// this daemon composes nothing, so the record names the global home.
pub(crate) fn launch_home(session: &str, agent: &str) -> Option<PathBuf> {
    installed_compose()?.plan_home(session, agent)
}

fn installed_compose() -> Option<Arc<ComposeEnv>> {
    COMPOSE.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// A session that is archived or deleted stops following the sync settings;
/// its folder is left as it is.
#[allow(dead_code, reason = "called once archive and delete have one place to say a session is gone")]
pub(crate) fn forget_session_home(session: &str) {
    if let Some(compose) = installed_compose() {
        compose.forget(session);
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

    fn session_compose_env(&self, session: &SessionRecord, agent: &str) -> Result<Vec<(OsString, OsString)>, String> {
        TabEnvPort::session_compose_env(self.0.as_ref(), session, agent)
    }

    fn session_home(&self, session: &str, agent: &str) -> Option<std::path::PathBuf> {
        TabEnvPort::session_home(self.0.as_ref(), session, agent)
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

#[cfg(test)]
mod compose_tests {
    use super::*;

    struct Rig {
        dir: tempfile::TempDir,
        compose: ComposeEnv,
    }

    fn rig() -> Rig {
        rig_mounted(true)
    }

    fn rig_mounted(mounted: bool) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        std::fs::write(home.join(".codex/auth.json"), "{\"token\":\"x\"}").unwrap();
        let state = dir.path().join("humata/state");
        std::fs::create_dir_all(&state).unwrap();
        let sync = Arc::new(ConfigSync::new(
            &state,
            dir.path().join("machine"),
            home,
            "box".to_owned(),
            dir.path().join("mnt"),
        ));
        let compose = ComposeEnv::new(sync, Path::new("/opt/humata/humatad2"), &dir.path().join("d.sock"), Arc::new(AtomicBool::new(mounted)));
        Rig { dir, compose }
    }

    fn session(machine: &str, agent: &str) -> SessionRecord {
        SessionRecord {
            id: "se_a".to_owned(),
            channel: "ch_a".to_owned(),
            machine: machine.to_owned(),
            title: "t".to_owned(),
            agent: agent.to_owned(),
            folder: PathBuf::from("/unused"),
            created_unix_ms: 0,
        }
    }

    fn value<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
        env.iter().find(|(var, _)| var == name).map(|(_, value)| value)
    }

    #[test]
    fn a_local_agent_runs_in_a_composed_home_under_its_session_and_the_real_home_is_unchanged() {
        let rig = rig();
        let real = rig.dir.path().join("home/.codex/auth.json");
        let before = std::fs::read(&real).unwrap();

        rig.compose.plan_home("se_a", "codex").unwrap();
        let env = rig.compose.env(&session("", "codex"), "codex").unwrap();

        let composed = PathBuf::from(value(&env, "CODEX_HOME").expect("the home variable"));
        assert_eq!(composed, rig.dir.path().join("machine/.sessions/se_a/config/codex"));
        assert!(composed.is_dir());
        assert_eq!(value(&env, "HUMATA_FUSE_ROOT"), Some(&rig.dir.path().join("mnt").into_os_string()));
        assert_eq!(value(&env, "HUMATA_MACHINE_NAME"), Some(&OsString::from("box")));
        assert_eq!(std::fs::read(&real).unwrap(), before);
        let real_home: Vec<_> = std::fs::read_dir(rig.dir.path().join("home/.codex")).unwrap().collect();
        assert_eq!(real_home.len(), 1, "composing adds nothing to the real home");
    }

    #[test]
    fn an_unmounted_tree_leaves_the_fuse_root_out_of_the_env() {
        let rig = rig_mounted(false);
        rig.compose.plan_home("se_a", "codex").unwrap();
        let env = rig.compose.env(&session("", "codex"), "codex").unwrap();
        assert!(value(&env, "HUMATA_FUSE_ROOT").is_none());
        assert!(value(&env, "CODEX_HOME").is_some());
        assert!(value(&env, "HUMATA_MACHINE_NAME").is_some());
    }

    #[test]
    fn each_agent_gets_its_own_home_variable() {
        let rig = rig();
        for (agent, var) in [
            ("claude", "CLAUDE_CONFIG_DIR"),
            ("codex", "CODEX_HOME"),
            ("copilot", "COPILOT_HOME"),
            ("agy", "GEMINI_CLI_HOME"),
        ] {
            rig.compose.plan_home("se_a", agent);
            let env = rig.compose.env(&session("box", agent), agent).unwrap();
            assert!(value(&env, var).is_some(), "{agent}");
        }
    }

    #[test]
    fn a_session_on_another_machine_gets_nothing_and_a_shell_gets_no_home() {
        let rig = rig();
        assert!(rig.compose.env(&session("elsewhere", "codex"), "codex").unwrap().is_empty());
        assert!(rig.compose.env(&session("elsewhere", "shell"), "shell").unwrap().is_empty());
        for agent in ["shell", "zsh"] {
            let env = rig.compose.env(&session("", agent), agent).unwrap();
            assert_eq!(value(&env, "HUMATA_FUSE_ROOT"), Some(&rig.dir.path().join("mnt").into_os_string()), "{agent}");
            assert!(value(&env, "HUMATA_MACHINE_NAME").is_some(), "{agent}");
            assert_eq!(env.len(), 2, "{agent} gets no config home");
        }
        assert!(!rig.dir.path().join("machine/.sessions").exists());
    }

    #[test]
    fn an_unmounted_tree_leaves_the_fuse_root_out_of_a_shell_too() {
        let rig = rig_mounted(false);
        let env = rig.compose.env(&session("", "shell"), "shell").unwrap();
        assert!(value(&env, "HUMATA_FUSE_ROOT").is_none());
        assert!(value(&env, "HUMATA_MACHINE_NAME").is_some());
    }

    /// A rig whose machine root is a file, so composing a session home fails.
    fn broken_rig() -> Rig {
        let rig = rig();
        let root = rig.dir.path().join("machine");
        std::fs::write(&root, "not a folder").unwrap();
        rig
    }

    fn conversation_file(compose: &Arc<ComposeEnv>, session: &str) -> PathBuf {
        let compose = compose.clone();
        let agents = humata2_sessions::Agents::builtin()
            .with_session_homes(move |session, agent| compose.plan_home(session, agent));
        agents
            .get_for_session(session, "codex")
            .unwrap()
            .launch(Path::new("/home/me"), Path::new("/u/me/folder"), "abc")
            .conversation_file
    }

    #[test]
    fn a_failed_compose_records_the_global_home_and_the_agent_gets_no_home_variable() {
        let rig = broken_rig();
        let compose = Arc::new(rig.compose);

        let recorded = conversation_file(&compose, "se_a");
        assert!(recorded.starts_with("/home/me/.codex"), "{recorded:?}");

        let env = compose.env(&session("", "codex"), "codex").unwrap_or_default();
        assert!(value(&env, "CODEX_HOME").is_none());
    }

    #[test]
    fn a_compose_that_would_only_work_at_spawn_does_not_move_the_agent_off_its_record() {
        let rig = broken_rig();
        let compose = Arc::new(rig.compose);
        let recorded = conversation_file(&compose, "se_a");
        assert!(recorded.starts_with("/home/me/.codex"), "{recorded:?}");

        // The root is fixed before the tab starts: composing would now work.
        let root = rig.dir.path().join("machine");
        std::fs::remove_file(&root).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let env = compose.env(&session("", "codex"), "codex").unwrap();
        assert!(value(&env, "CODEX_HOME").is_none());
        assert!(!root.join(".sessions").exists());
    }

    #[test]
    fn a_compose_that_works_names_the_same_home_in_the_record_and_the_env() {
        let rig = rig();
        let compose = Arc::new(rig.compose);

        let recorded = conversation_file(&compose, "se_a");
        let env = compose.env(&session("", "codex"), "codex").unwrap();

        let home = PathBuf::from(value(&env, "CODEX_HOME").expect("the home variable"));
        assert_eq!(home, rig.dir.path().join("machine/.sessions/se_a/config/codex"));
        assert!(recorded.starts_with(&home), "{recorded:?}");
    }

    #[test]
    fn a_compose_that_fails_after_making_its_folder_records_global_and_leaves_no_folder() {
        let rig = rig();
        std::fs::write(rig.dir.path().join("home/.codex/config.toml"), "= not [toml").unwrap();
        let compose = Arc::new(rig.compose);

        let recorded = conversation_file(&compose, "se_a");
        assert!(recorded.starts_with("/home/me/.codex"), "{recorded:?}");
        let session_home = rig.dir.path().join("machine/.sessions/se_a/config/codex");
        assert!(!session_home.exists());

        let env = compose.env(&session("", "codex"), "codex").unwrap();
        assert!(value(&env, "CODEX_HOME").is_none());
    }

    #[test]
    fn a_refresh_that_fails_keeps_the_record_and_the_env_on_the_existing_folder() {
        let rig = rig();
        let compose = Arc::new(rig.compose);
        let first = conversation_file(&compose, "se_a");
        let session_home = rig.dir.path().join("machine/.sessions/se_a/config/codex");
        assert!(first.starts_with(&session_home), "{first:?}");

        std::fs::write(rig.dir.path().join("home/.codex/config.toml"), "= not [toml").unwrap();
        let again = conversation_file(&compose, "se_a");
        let env = compose.env(&session("", "codex"), "codex").unwrap();

        assert!(again.starts_with(&session_home), "{again:?}");
        assert_eq!(value(&env, "CODEX_HOME"), Some(&session_home.into_os_string()));
    }

    #[test]
    fn composed_claude_env_includes_keychain_setting_on_macos() {
        // SMB-078: ComposeEnv::env must apply env_plan so composed Claude
        // homes receive CLAUDE_SECURESTORAGE_CONFIG_DIR="" on macOS.
        let rig = rig();
        rig.compose.plan_home("se_a", "claude");
        let env = rig.compose.env(&session("box", "claude"), "claude").unwrap();
        assert!(value(&env, "CLAUDE_CONFIG_DIR").is_some());
        if cfg!(target_os = "macos") {
            assert_eq!(
                value(&env, "CLAUDE_SECURESTORAGE_CONFIG_DIR"),
                Some(&OsString::new()),
                "composed Claude env must include CLAUDE_SECURESTORAGE_CONFIG_DIR=\"\", got {env:?}"
            );
        }
    }
}
