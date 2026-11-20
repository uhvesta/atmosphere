//! Narrow plugin seams for the optional daemon services (`svc_catalog`,
//! `svc_board`, `svc_review`) so `humatad_core` compiles once without depending
//! on any of those crates, preserving scoped Bazel test caching.

use std::sync::Arc;

use humata2_daemon_runtime::agent_switches::OnChange;
use humata2_sessions::{Agents, Starter};
use humata2_store::Store;
use humata2_svc_peer::ports::{PeerBoardPort, PeerReviewPort};
use tonic::Status;
use tonic::service::RoutesBuilder;

use crate::away::Forwarder;
use crate::events::Feed;
use crate::files::Files;
use crate::guard::Guard;
use crate::inbox::Inbox;
use crate::machine_service::MachineSvc;
use crate::operations::Operations;
use crate::remote_work::RemoteWork;
use crate::repo_service::RepoSvc;
use crate::restart::Started;
use crate::session_service::{SessionSvc, Tell};
use crate::terminal::Core as TerminalCore;
use crate::transfer_service::TransferSvc;

/// Deferred registration of a plugin's gRPC services onto tonic's router.
pub type RegisterRoutes = Box<dyn FnOnce(&mut RoutesBuilder) + Send>;

/// Narrow port for `SessionSvc::plan_archive_of` to count unsubmitted PR
/// drafts without `humatad_core` depending on `humata2_svc_review`.
pub trait ReviewDraftsPort: Send + Sync {
    fn unsubmitted_pr_drafts(&self, session: &str) -> Result<u32, Status>;
}

/// Optional `AgentService` (`svc_catalog`).
pub trait CatalogPlugin: Send + Sync {
    fn build(&self, agents: Agents, use_builtin: bool) -> (OnChange, RegisterRoutes);
}

/// Concrete core handles passed to [`BoardPlugin::build`].
pub struct BoardContext {
    pub guard: Arc<Guard>,
    pub store: Arc<Store>,
    pub feed: Arc<Feed>,
    pub inbox: Arc<Inbox>,
    pub machine: String,
    pub tell: Option<Tell>,
    pub repos: Option<RepoSvc>,
    pub forwarder: Option<Arc<Forwarder>>,
    /// TODO2WIRE: the session service, set once it exists (it is built after
    /// the board), so To-do automation can wake, archive and restore sessions.
    pub sessions: Arc<std::sync::OnceLock<SessionSvc>>,
}

/// A live `BoardSvc` instance wired by `humatad_plugin_board`.
pub trait BoardInstance: Send + Sync {
    fn peer_board(&self) -> Arc<dyn PeerBoardPort>;
    fn answer_host_calls(
        &self,
        machines: MachineSvc,
        files: Arc<Files>,
        transfers: Arc<TransferSvc>,
    );
    fn register(self: Box<Self>, routes: &mut RoutesBuilder);
}

/// Optional `BoardService` (`svc_board`).
pub trait BoardPlugin: Send + Sync {
    fn build(&self, ctx: BoardContext) -> Box<dyn BoardInstance>;
}

/// Concrete core handles passed to [`ReviewPlugin::build`].
pub struct ReviewContext {
    pub store: Arc<Store>,
    pub guard: Arc<Guard>,
    pub machine: String,
    pub feed: Arc<Feed>,
    pub terminal_core: Arc<TerminalCore>,
    pub files: Arc<Files>,
    pub stopped_now: bool,
    pub last_start: Arc<Started>,
    pub starter: Starter,
    pub session_tell: Tell,
    pub default_agent: String,
    pub operations: Arc<Operations>,
    pub remote_work: Option<RemoteWork>,
}

/// A live `Review` + `PullRequests` instance wired by `humatad_plugin_review`.
pub trait ReviewInstance: Send + Sync {
    fn drafts_port(&self) -> Arc<dyn ReviewDraftsPort>;
    fn use_sessions(&self, sessions: Arc<SessionSvc>);
    fn spawn_reconcile_submits(&self);
    fn peer_review(&self) -> Arc<dyn PeerReviewPort>;
    fn register(self: Box<Self>, routes: &mut RoutesBuilder);
}

/// Optional review / PR / comment / diff / view services (`svc_review`).
pub trait ReviewPlugin: Send + Sync {
    fn build(&self, ctx: ReviewContext) -> Box<dyn ReviewInstance>;
}

/// The optional service plugins a binary profile injects into [`crate::serve::run`].
#[derive(Default)]
pub struct Plugins {
    pub catalog: Option<Box<dyn CatalogPlugin>>,
    pub board: Option<Box<dyn BoardPlugin>>,
    pub review: Option<Box<dyn ReviewPlugin>>,
}
