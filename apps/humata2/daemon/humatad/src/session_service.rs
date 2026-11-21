//! SessionService (SE-1, SE-4, SE-13..SE-15, PER-13..PER-15, FC-4, FC-15):
//! start a session on this machine, list and read the sessions, rename,
//! stop and resume one. The work (record, clones, launching the agent,
//! bringing it back) is `humata2_sessions`' and `humata2_revive`'s; this
//! wires it to the API: the access checks, an `Operation` whose steps
//! change as the work does, and the errors in the API's terms. The rest of
//! the service answers "not built".
//!
//! A session's status comes from its agent's record: launching or
//! resuming is STARTING, running is what its turn says (`turns.rs`:
//! WORKING, IDLE, NEEDS_INPUT or PROBABLY_IDLE, D169), exited is EXITED,
//! stopped is IDLE and a failed resume is RESUME_FAILED. A session whose start hasn't reached its end
//! is STARTING whatever its agent says.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use humata2_access::{AgentId, Call, Capability, ChannelId, Refusal, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_revive::{Revived, resume_agent, resume_agent_asked, resume_moved_agent};
use humata2_sessions::{Agents, ReviewStart, StartError, StartRequest, Starter, branch_slug};
use humata2_store::{
    AgentRecord, AgentState, ChannelRecord, MainClone, ResumeFailure, SessionDetail, SessionRecord,
    StartStep, StoreError, TabRecord,
};
use pb::invalid_field::Problem;
use tokio::sync::{broadcast, oneshot};
use tonic::{Code, Request, Response, Status};

use crate::desk_service;
use crate::events::{Feed, Scope};
use crate::channel_service::{Gated, answer_gated, answer_gated_showing, answer_plan, blocking, error_of, failed, gate, timestamp_ms};
use crate::guard::{self, Checked, Guard};
use crate::holder_end;
use crate::identity::{self, Caller, Identities};
use crate::inbox::Inbox;
use crate::operations::{Operations, Progress, Started};
use crate::remote_work::RemoteWork;
use crate::restart::Tokens;
use crate::status;
use crate::upgrade::items::Migrations;

// REPOS (CH-21)
mod add_repo;
mod archive;
mod archive_remote;
pub(crate) use archive_remote::{machine_failed as peer_failed, use_upgrade as use_peer_upgrade};
mod continue_in;
mod conversation_read;
mod delete_many;
mod ended;
mod forget;
mod fork;
mod fork_card;
mod fork_remote;
mod hooks;
mod lineage;
mod move_recover;
mod move_session;
pub(crate) use move_session::paused_for_move;
mod notes;
mod own_copy;
mod pointers;
mod push;
mod rebuild;
mod rename_branch;
mod remote;
mod remote_review;
mod remote_start;
mod script_steps;
mod start_fresh;
pub(crate) mod status_since;
mod stop_launching;
mod stop_running;
pub(crate) mod turns;
mod turns_list;
pub(crate) mod watch;

pub(crate) use push::gh_program;

pub(crate) use archive_remote::Removal;
pub(crate) use archive::undo_window_ms as archive_undo_window_ms;

pub(crate) use remote::{TellSink, stuck_start};
pub(crate) use turns::{
    Asking, HooksError, changes as turn_changes, install_hooks, running as running_turn,
    set_sources, tell_asks,
};
#[cfg(test)]
pub(crate) use turns::asking as turns_asking;
pub(crate) use hooks::{install_agent_hooks, install_mcp_configs};
pub(crate) use pointers_start::after_restart;
pub(crate) use move_session::{Moving, is_moving, moving_changes, refuse_input_while_moving};
pub(crate) use rebuild::{CutRebuild, cut_rebuilds, undo_cut_rebuilds};

mod pointers_start {
    use super::Tell;

    /// Starts the pointer loop, then the restart's own work.
    pub(crate) async fn after_restart(tell: Tell) {
        super::pointers::spawn(tell.clone());
        super::watch::after_restart(tell).await;
    }
}

/// Watches a Desk session's agent the way a channel session's is watched (SE-21).
pub(crate) fn watch_agent(tell: Tell, session: String) {
    watch::agent(tell, session);
}

/// The longest session title taken.
const TITLE_MAX: usize = 200;
/// The terminal size a session's agent starts with, until a client attaches.
const COLS: u16 = crate::holders::START_COLS;
const ROWS: u16 = crate::holders::START_ROWS;

#[derive(Clone)]
pub struct SessionSvc {
    guard: Arc<Guard>,
    operations: Arc<Operations>,
    starter: Starter,
    tokens: Arc<Tokens>,
    inbox: Arc<Inbox>,
    machine: String,
    tell: Tell,
    /// The host's way to other machines (D139); none on a machine's own
    /// daemon, where every session is here.
    remote: Option<RemoteWork>,
    /// Plan tokens and per-session locks of fork, archive and push.
    lifecycle: Arc<archive::Lifecycle>,
    /// The host's way to move a session between machines (D195); set once.
    carry: Arc<std::sync::OnceLock<crate::carry::Carry>>,
    /// This run's restart, set once the daemon serves: a start the host
    /// asks again waits for it, so it is answered from what recovery kept.
    restart: Arc<std::sync::OnceLock<Arc<crate::restart::Started>>>,
    /// The host's mirror's scrubbed lines, for rebuilding a lost session (SL-4).
    raw_lines: rebuild::Lines,
    /// UP-1, D281 e: what would run across releases is refused, by the Inbox's Migrate items.
    migrations: Migrations,
    /// REPOS (CH-21): the channel service, for the channel add of a repo it lacks; set once.
    channels: add_repo::Channels,
    /// PR-17: the review service, which knows a review's drafts; set once.
    review: Arc<std::sync::OnceLock<Arc<dyn crate::plugins::ReviewDraftsPort>>>,
    /// MACHINESCRIPTS (D681): the engine a move fires session triggers on; set once.
    scripts: Arc<std::sync::OnceLock<Arc<crate::machine_scripts::MachineScripts>>>,
}

/// A pull request review session's binding, as the Session shows it.
fn pr_binding(record: &humata2_store::ReviewSessionRecord) -> pb::PullRequestBinding {
    pb::PullRequestBinding {
        pull_requests: record
            .prs
            .iter()
            .map(|pr| pb::PullRequestRef {
                repo: Some(pb::RepoRef {
                    owner: pr.owner.clone(),
                    name: pr.name.clone(),
                }),
                number: pr.number,
                ..pb::PullRequestRef::default()
            })
            .collect(),
        push_allowed: record.push_allowed,
    }
}

/// Puts a session as the store holds it now on the event feed.
#[derive(Clone)]
pub struct Tell {
    feed: Arc<Feed>,
    store: Arc<humata2_store::Store>,
    machine: String,
    /// Set once the host's remote work exists: what a remote session shows is
    /// merged with what its machine reported (SL-3).
    remote: Arc<std::sync::OnceLock<RemoteWork>>,
    /// The last pointers of this machine's sessions (SL-3).
    pointers: Arc<pointers::Cache>,
    /// Who drops what they keep for a session once it ends.
    endings: ended::Ended,
    /// The files of this machine's session clones, told on the feed (bug 23).
    file_changes: Arc<crate::files::FileChanges>,
}

impl Tell {
    pub(crate) fn new(feed: Arc<Feed>, store: Arc<humata2_store::Store>, machine: String) -> Self {
        let file_changes = Arc::new(crate::files::FileChanges::new(Arc::new(
            crate::wire_files::FeedAdapter(feed.clone()),
        )));
        Self {
            feed,
            store,
            machine,
            remote: Arc::default(),
            pointers: pointers::shared(),
            endings: ended::Ended::default(),
            file_changes,
        }
    }

    /// What tells the files of session clones; the file service shares it.
    pub(crate) fn file_changes(&self) -> Arc<crate::files::FileChanges> {
        self.file_changes.clone()
    }

    /// Calls `forget` with each session archived or freed from now on, so
    /// what it keeps for sessions stays bounded.
    pub(crate) fn on_ended(&self, forget: impl Fn(&str) + Send + Sync + 'static) {
        self.endings.listen(forget);
    }

    /// Says `session` was archived or its folder freed.
    fn ended(&self, session: &str) {
        self.endings.tell(session);
        crate::channel_service::rename_folder::wake_aliases();
    }

    /// Schedules a fresh pointer look for `session` (e.g. after restore).
    pub(super) fn poke_pointers(&self, session: &str) {
        self.pointers.poke(session);
    }

    /// The same, merging remote sessions with `remote` once it is set.
    pub(crate) fn with_remote(mut self, remote: Arc<std::sync::OnceLock<RemoteWork>>) -> Self {
        self.remote = remote;
        self
    }

    /// The session the start operation `operation` recorded, if it got that
    /// far.
    async fn session_of(&self, operation: &str) -> Option<String> {
        let (store, operation) = (self.store.clone(), operation.to_owned());
        blocking(move || -> Result<Option<String>, StoreError> {
            Ok(store
                .session_details()?
                .into_iter()
                .find(|detail| detail.operation.as_deref() == Some(operation.as_str()))
                .map(|detail| detail.session))
        })
        .await
        .ok()?
        .ok()?
    }

    /// Publishes a `FileChanged` event (e.g. forwarded from a peer machine) on the feed.
    pub(crate) fn file_changed(&self, session: &str, changed: pb::FileChanged) {
        self.feed.publish_in(
            pb::Change::Updated,
            pb::event::Payload::FileChanged(changed),
            crate::events::Scope {
                channel: String::new(),
                session: session.to_owned(),
            },
            None,
        );
    }

    /// Reads `session` back and publishes it as GetSession would show it.
    /// A session that can't be read is not told; publishing never fails
    /// the call that changed it.
    pub async fn session(&self, session: &str, change: pb::Change, by: Option<pb::Actor>) {
        self.session_when(session, change, by, |_| true).await;
    }

    /// Publishes `session` as [`Self::session`] does, only while its agent
    /// runs: an ask starting or ending changes no other session's status
    /// (D158.7), so no other is told of it.
    pub(crate) async fn running_session(&self, session: &str) {
        self.session_when(session, pb::Change::Updated, None, |loaded| {
            matches!(
                loaded.agent.as_ref().map(|agent| &agent.state),
                Some(AgentState::Running { .. })
            )
        })
        .await;
    }

    async fn session_when(
        &self,
        session: &str,
        change: pb::Change,
        by: Option<pb::Actor>,
        tell: impl FnOnce(&Loaded) -> bool,
    ) {
        let (store, key) = (self.store.clone(), session.to_owned());
        let read = blocking(move || Loaded::read(&store, &key))
        .await;
        let Ok(Ok(Some(loaded))) = read else {
            return;
        };
        if !tell(&loaded) {
            return;
        }
        let mut shown = session_pb(
            &loaded.record,
            loaded.detail.as_ref(),
            loaded.agent.as_ref(),
            loaded.summary_source,
            &self.machine,
        );
        shown.away_left_out.clone_from(&loaded.away_left_out);
        shown.pull_request = loaded.review.as_ref().map(pr_binding);
        shown.shell_tabs.clone_from(&loaded.shell_tabs);
        if let Some(remote) = self.remote.get()
            && remote::elsewhere(&loaded.record, &self.machine)
        {
            remote.merge(&mut shown);
        } else if !remote::elsewhere(&loaded.record, &self.machine) {
            self.pointers.fill(&loaded.record.id, &mut shown);
            self.pointers.poke(&loaded.record.id);
        }
        // After the machine's report, so an archived session shows archived.
        lineage::apply(&self.store, &mut shown);
        move_session::show_moving(&mut shown);
        stamp_status_since(&mut shown);
        self.feed.publish_in(
            change,
            pb::event::Payload::Session(shown),
            Scope {
                channel: loaded.record.channel.clone(),
                session: loaded.record.id.clone(),
            },
            by,
        );
    }
}

/// What the store holds of one session.
pub(crate) struct Loaded {
    record: SessionRecord,
    detail: Option<SessionDetail>,
    agent: Option<AgentRecord>,
    /// What a start fresh could summarize; set only for a failed resume.
    summary_source: pb::session::SummarySource,
    /// What the host left out of its agent's replayed writes (AW-9, D267).
    away_left_out: Option<pb::AwayWritesLeftOut>,
    /// The pull requests a review session is bound to (PR-4).
    review: Option<humata2_store::ReviewSessionRecord>,
    /// For an agent a move just landed here, the line its terminal shows
    /// first: it is started again in its conversation, not resumed as an
    /// interrupted one, so nothing is recorded and it isn't told.
    moved: Option<&'static str>,
    /// D858: extra terminal tabs (shell or added agent), oldest first.
    shell_tabs: Vec<pb::ShellTab>,
}

impl Loaded {
    /// Reads session `key`. It looks at the disk for a failed resume, so it
    /// is blocking.
    pub(crate) fn read(store: &humata2_store::Store, key: &str) -> Result<Option<Loaded>, StoreError> {
        let Some(record) = store.session(key)? else {
            return Ok(None);
        };
        let agent = store.agent(key)?;
        Ok(Some(Loaded {
            detail: store.session_detail(key)?,
            summary_source: start_fresh::summary_source(agent.as_ref()),
            away_left_out: store.away_left_out(key)?.map(away_pb),
            review: store.review_session(key)?,
            agent,
            record,
            moved: None,
            shell_tabs: humata2_svc_terminal::agent_tabs::shell_tabs(store, key)
                .unwrap_or_default()
                .into_iter()
                .map(|entry| pb::ShellTab {
                    tab: Some(pb::TabId { value: entry.tab }),
                    label: entry.label,
                    agent: entry.agent.map(|value| pb::AgentId { value }),
                })
                .collect(),
        }))
    }
}

impl SessionSvc {
    /// D784: the configured agents and home an adopted holder's session
    /// is planned with.
    pub(crate) fn agents_and_home(&self) -> (&Agents, &std::path::Path) {
        (&self.starter.agents, &self.starter.home)
    }

    /// D784: tells the feed of a session adopted from an orphan holder and
    /// watches its terminal for an exit, as a started one is.
    pub(crate) async fn adopted(&self, id: &str) {
        self.tell.session(id, pb::Change::Created, None).await;
        watch::agent(self.tell.clone(), id.to_owned());
    }

    pub(crate) fn new(
        guard: Arc<Guard>,
        operations: Arc<Operations>,
        starter: Starter,
        identities: Arc<Identities>,
        machine: String,
        inbox: Arc<Inbox>,
        remote: Option<RemoteWork>,
        tell: Tell,
        socket: PathBuf,
        migrations: Migrations,
    ) -> Self {
        let tokens = Arc::new(Tokens::new(starter.store.clone(), identities, socket));
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(tell_read_marks(tell.clone(), crate::read_marks::changes()));
            runtime.spawn(tell_read_marks(tell.clone(), SHELLS_CHANGED.subscribe()));
        }
        Self {
            tell,
            guard,
            operations,
            starter,
            tokens,
            inbox,
            machine,
            remote,
            lifecycle: Arc::default(),
            carry: Arc::default(),
            restart: Arc::default(),
            raw_lines: rebuild::Lines::default(),
            channels: Arc::default(),
            review: Arc::default(),
            scripts: Arc::default(),
            migrations,
        }
    }

    /// A session that stopped, was archived or was deleted no longer waits
    /// on anyone: its pending Inbox requests are withdrawn and can't be
    /// answered (TP-15).
    async fn withdraw_requests_of(
        &self,
        session: &str,
        reason: pb::WithdrawReason,
    ) -> Result<(), Status> {
        self.inbox.withdraw_pending_of(session, reason).await
    }

    /// Ends the grants of a session that was archived or deleted, in the
    /// policy and in the store, so a restart doesn't bring them back
    /// (IN-22). ArchiveSession calls this, then `withdraw_requests_of`.
    async fn end_grants_of(&self, session: &str) -> Result<(), Status> {
        let id = humata2_access::SessionId::new(session);
        let ended = self
            .guard
            .with_policy(|policy, _| policy.end_grants_of(&id));
        for grant in ended {
            // The request whose answer made the grant remembers it ended.
            self.inbox
                .revoked(
                    &grant.granted_from.to_string(),
                    std::time::SystemTime::now(),
                )
                .await?;
        }
        Ok(())
    }

    fn refuse_peer(caller: &Caller) -> Result<(), Status> {
        match caller {
            Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
            _ => Ok(()),
        }
    }

    /// Refuses a channel action (`action` names it) on the Desk session
    /// `session`, which belongs to no channel (D182.2).
    async fn refuse_desk(&self, session: Option<&pb::SessionId>, action: &str) -> Result<(), Status> {
        match session {
            Some(id) => {
                desk_service::refuse_channel_action(&self.starter.store, &self.machine, &id.value, action)
                    .await
            }
            None => Ok(()),
        }
    }

    /// `Some` when the rule for `capability` says Ask; an `approved` call
    /// goes on.
    fn gate(
        &self,
        caller: &Caller,
        capability: Capability,
        target: Target,
        approved: bool,
    ) -> Result<Option<Box<humata2_access::AccessRequest>>, Status> {
        Self::refuse_peer(caller)?;
        gate(
            &self.guard,
            caller,
            &Call::Use { capability, target },
            approved,
        )
    }

    /// PR-6: starts a pull request review's session; its pinned clones are
    /// `Starter::start_review`'s. The caller was vetted by the review service.
    pub async fn begin_review(
        &self,
        caller: &Caller,
        channel: ChannelRecord,
        request: &pb::StartReviewRequest,
        review: ReviewStart,
        title: String,
        first_message: String,
        // The id the host chose, on the machine a review starts on for it.
        assigned: Option<String>,
    ) -> Result<pb::Operation, Status> {
        Self::refuse_peer(caller)?;
        crate::stop::gate::require_running(caller)?;
        // SEAM: PR-4, PR-21: a review on another machine runs there.
        if let Some((machine, remote)) = self.remote_machine(request.machine.as_ref())? {
            return self
                .begin_review_remote(caller, channel, request, review, title, first_message, machine, remote)
                .await;
        }
        let agent = self.agent_for(request.agent.as_ref(), &channel)?;
        // SEAM: AGENTSET: a review runs the agent here, so it must be on and found.
        crate::agent_gate::enabled(&self.starter.agents, &agent)?;
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(operation);
        }
        crate::agent_gate::ready(&self.starter.agents, &agent).await?;
        turns::refresh_hooks().await;
        let mut keys = vec!["write_record".to_owned()];
        let mut steps = vec![("write_record".to_owned(), "Saving the session".to_owned())];
        for pr in &review.prs {
            keys.push(format!("clone:{}", pr.name));
            steps.push((format!("clone:{}", pr.name), format!("Cloning {}", pr.name)));
        }
        keys.push("launch".to_owned());
        keys.push("verify".to_owned());
        steps.push(("launch".to_owned(), "Starting the agent".to_owned()));
        steps.push(("verify".to_owned(), "Checking it started".to_owned()));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(operation),
        };
        let operation = progress.operation();
        let start = StartRequest {
            id: assigned,
            channel: channel.id,
            machine: self.machine.clone(),
            title,
            agent: Some(agent),
            model: Some(request.model.clone()).filter(|model| !model.is_empty()),
            repos: None,
            first_message: Some(first_message),
            branch: None,
            operation: operation.id.as_ref().map(|id| id.value.clone()),
            cols: COLS,
            rows: ROWS,
        };
        let (ready, recorded) = oneshot::channel();
        tokio::spawn(run_start(
            self.starter.clone(),
            self.tell.clone(),
            self.tokens.clone(),
            start,
            progress,
            keys,
            ready,
            identity::actor(caller),
            Some(review),
        ));
        match recorded.await {
            Ok(Ok(())) => {
                let started = self.started_by(operation.clone()).await?;
                if let Some(session) = &started.session {
                    self.tell
                        .session(
                            &session.value,
                            pb::Change::Created,
                            Some(identity::actor(caller)),
                        )
                        .await;
                }
                Ok(operation)
            }
            Ok(Err(refused)) => Err(refused),
            Err(_) => Err(status::internal(
                "the review's start stopped before it was recorded",
                "",
            )),
        }
    }

    async fn begin_session(
        &self,
        caller: &Caller,
        request: pb::StartSessionRequest,
        assigned: Option<String>,
        approved: bool,
    ) -> Result<Gated<pb::StartSessionResponse>, Status> {
        Self::refuse_peer(caller)?;
        let channel_id = request
            .channel
            .as_ref()
            .map(|channel| channel.value.clone())
            .filter(|channel| !channel.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        let remote_machine = self.remote_machine(request.machine.as_ref())?;
        let channel = self.find_channel(&channel_id).await?;
        let agent = self.agent_for(request.agent.as_ref(), &channel)?;
        // Starting on another machine is its own capability (D139); the host runs this
        // check before it relays anything, because the machine checks nothing more.
        let (capability, starts_on) = match &remote_machine {
            Some((machine, _)) => (Capability::SessionStartOtherMachine, machine.as_str()),
            None => (Capability::SessionStartSameMachine, self.machine.as_str()),
        };
        if let Some(asked) = self.gate(
            caller,
            capability,
            Target::Start {
                channel: ChannelId::new(channel.id.as_str()),
                machine: humata2_access::MachineId::new(starts_on),
                agent: AgentId::new(agent.as_str()),
                fork_of: None,
            },
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        // SEAM: AGENTSET: a new start of a kind that is off is refused before it is
        // relayed; a rebuild placing an existing session is not a new start.
        if request.rebuild.is_none() {
            crate::agent_gate::enabled(&self.starter.agents, &agent)?;
        }
        if let Some((machine, remote)) = remote_machine {
            return self
                .begin_remote(caller, request, channel, agent, machine, remote)
                .await;
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(self.started_by(operation).await?));
        }
        // The host placing a session it rebuilt from its own records (SL-4).
        if let Some(rebuild) = request.rebuild.clone() {
            return self
                .begin_rebuild_here(caller, request, rebuild, assigned)
                .await;
        }
        // SEAM: PR-4, PR-21: the host starting a pull request review here.
        if let Some(review) = request.review.clone() {
            return self.begin_review_here(caller, request, review, assigned, channel).await;
        }
        // A start naming a session this machine already has (the host asking again,
        // D150) answers with it and starts nothing.
        if let Some(id) = &assigned
            && let Some(known) = self.known_start(id).await?
        {
            return Ok(Gated::Done(known));
        }
        let title = clean_title(&request.title)?;
        let repos = choose_repos(&channel, &request.repos)?;
        // SEAM: AGENTSET: on the machine that runs the agent, its CLI must be found.
        // When it is missing and Humata may install it, the start shows an
        // "Installing" step and waits for it.
        let installing = crate::agent_gate::check(&self.starter.agents, &agent)?;
        let agent_name = agent.clone();
        turns::refresh_hooks().await;

        let mut keys = vec!["write_record".to_owned()];
        let mut steps = Vec::new();
        if installing {
            steps.push((
                "install".to_owned(),
                format!("Installing {}", self.starter.agents.person_name(&agent)),
            ));
        }
        steps.push(("write_record".to_owned(), "Saving the session".to_owned()));
        for (_, name) in &repos {
            keys.push(format!("clone:{name}"));
            steps.push((format!("clone:{name}"), format!("Cloning {name}")));
        }
        keys.push("launch".to_owned());
        keys.push("verify".to_owned());
        steps.push(("launch".to_owned(), "Starting the agent".to_owned()));
        steps.push(("verify".to_owned(), "Checking it started".to_owned()));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Channel(pb::ChannelId {
                value: channel.id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => {
                return Ok(Gated::Done(self.started_by(operation).await?));
            }
        };
        let operation = progress.operation();
        let start = StartRequest {
            id: assigned,
            channel: channel.id,
            machine: self.machine.clone(),
            title: if title.is_empty() {
                "Session".to_owned()
            } else {
                title
            },
            agent: Some(agent),
            model: Some(request.model).filter(|model| !model.is_empty()),
            repos: Some(repos),
            first_message: Some(request.first_message).filter(|message| !message.is_empty()),
            branch: Some(request.branch).filter(|branch| !branch.is_empty()),
            operation: operation.id.as_ref().map(|id| id.value.clone()),
            cols: COLS,
            rows: ROWS,
        };
        let (ready, recorded) = oneshot::channel();
        let install = installing.then(|| (self.starter.agents.clone(), agent_name));
        let (starter, tell, tokens, by) = (
            self.starter.clone(),
            self.tell.clone(),
            self.tokens.clone(),
            identity::actor(caller),
        );
        tokio::spawn(async move {
            if let Some((agents, kind)) = install {
                progress.set_step("install", pb::StepState::Running, None);
                match crate::agent_gate::install_now(&agents, &kind).await {
                    Ok(()) => progress.set_step("install", pb::StepState::Done, None),
                    Err(error) => {
                        let shown = error.to_error();
                        progress.set_step("install", pb::StepState::Failed, Some(shown.message.clone()));
                        let _ = ready.send(Err(status::failure(Code::FailedPrecondition, shown.clone())));
                        progress.fail(shown);
                        return;
                    }
                }
            }
            run_start(starter, tell, tokens, start, progress, keys, ready, by, None).await;
        });
        match recorded.await {
            Ok(Ok(())) => {
                let started = self.started_by(operation).await?;
                if let Some(session) = &started.session {
                    self.tell
                        .session(
                            &session.value,
                            pb::Change::Created,
                            Some(identity::actor(caller)),
                        )
                        .await;
                }
                Ok(Gated::Done(started))
            }
            Ok(Err(refused)) => Err(refused),
            Err(_) => Err(status::internal(
                "the session's start stopped before it was recorded",
                "",
            )),
        }
    }

    async fn preview_names(
        &self,
        caller: &Caller,
        request: pb::PreviewBranchNamesRequest,
        approved: bool,
    ) -> Result<Gated<pb::PreviewBranchNamesResponse>, Status> {
        let channel = request
            .channel
            .map(|channel| channel.value)
            .filter(|channel| !channel.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        if let Some(asked) = self.gate(
            caller,
            Capability::ChannelRead,
            Target::Channel(ChannelId::new(channel.as_str())),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        self.this_machine(request.machine.as_ref())?;
        self.find_channel(&channel).await?;
        let slug = branch_slug(&request.title);
        let (starter, title, id) = (self.starter.clone(), request.title, channel);
        let names = blocking(move || starter.preview_branches(&id, &title))
            .await?
            .map_err(|error| {
                let (code, failure) = start_error(&error);
                status::failure(code, failure)
            })?;
        let names = names
            .into_iter()
            .filter(|((owner, name), _)| {
                request.repos.is_empty()
                    || request
                        .repos
                        .iter()
                        .any(|r| &r.owner == owner && &r.name == name)
            })
            .map(
                |((owner, name), branch)| pb::preview_branch_names_response::Name {
                    repo: Some(pb::RepoRef { owner, name }),
                    suffix_reason: if branch.folder == slug {
                        String::new()
                    } else {
                        format!(
                            "{slug} is already used by a branch or a session, so a number was added"
                        )
                    },
                    branch: branch.name,
                },
            )
            .collect();
        Ok(Gated::Done(pb::PreviewBranchNamesResponse { names }))
    }

    async fn read_session(
        &self,
        caller: &Caller,
        request: pb::GetSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Session>, Status> {
        Self::refuse_peer(caller)?;
        let id = request
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let loaded = self.load_after_recovery(&id).await?;
        if let Some(asked) = gate(
            &self.guard,
            caller,
            &Call::ReadSession(self.session_ref(&loaded.record)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        Ok(Gated::Done(self.show(&loaded)))
    }

    async fn read_conversation_of(
        &self,
        caller: &Caller,
        request: pb::ReadConversationRequest,
        approved: bool,
    ) -> Result<Gated<pb::ReadConversationResponse>, Status> {
        Self::refuse_peer(caller)?;
        let id = request
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let loaded = self.load_after_recovery(&id).await?;
        if let Some(asked) = gate(
            &self.guard,
            caller,
            &Call::ReadSession(self.session_ref(&loaded.record)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if !loaded.record.machine.is_empty() && loaded.record.machine != self.machine {
            return Err(failed(
                Code::FailedPrecondition,
                &ReadElsewhere { session: id },
                "read it from the host's copy with `humata session read`",
                None,
            ));
        }
        let scrubber = crate::history_service::scrubber()?;
        let (store, agents, home) = (
            self.starter.store.clone(),
            self.starter.agents.clone(),
            self.starter.home.clone(),
        );
        let (session, page) = (id.clone(), request.page);
        let read = blocking(move || {
            conversation_read::read(&store, &agents, &home, &session, page.as_ref(), &scrubber)
        })
        .await?
        .map_err(|error| {
            let code = match &error {
                conversation_read::ReadError::BadToken { .. } => Code::InvalidArgument,
                conversation_read::ReadError::Store { .. }
                | conversation_read::ReadError::Unreadable { .. } => Code::Internal,
                _ => Code::FailedPrecondition,
            };
            failed(code, &error, "", None)
        })?;
        Ok(Gated::Done(read))
    }

    async fn retitle(
        &self,
        caller: &Caller,
        request: pb::RenameSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Session>, Status> {
        Self::refuse_peer(caller)?;
        let id = request
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let title = clean_title(&request.title)?;
        if title.is_empty() {
            return Err(status::missing("title"));
        }
        let mut loaded = self.load(&id).await?;
        if let Caller::Agent(agent) = caller
            && agent.session.id.as_str() != id
        {
            return Err(status::refused(&Refusal::UserOnly {
                capability: Some(Capability::OwnSessionManage),
            }));
        }
        if let Some(asked) = self.gate(
            caller,
            Capability::OwnSessionManage,
            Target::Session(self.session_ref(&loaded.record)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        let current = loaded.detail.as_ref().map_or(1, |detail| detail.revision);
        if let Some(expected) = request.expected_revision
            && expected.value != current
        {
            return Err(status::failure(
                Code::Aborted,
                pb::Error {
                    message: format!("the session is at revision {current}, not {} as the change was made against", expected.value),
                    fix: "read the session again, check the title still needs changing, and send the change again".to_owned(),
                    retry: None,
                    kind: Some(pb::error::Kind::RevisionConflict(pb::RevisionConflict {
                        kind: pb::EntityKind::Session as i32,
                        id,
                        expected: Some(expected),
                        actual: Some(pb::Revision { value: current }),
                    })),
                },
            ));
        }
        let on_machine = self.rename_branch_on_machine(&mut loaded, &title, request.rename_branch).await?;
        let rename = self.plan_branch_rename(&loaded, &title, request.rename_branch).await?;
        if loaded.record.title != title || on_machine || rename.as_ref().is_some_and(|plan| plan.changes()) {
            loaded.record.title = title;
            self.apply_branch_rename(&mut loaded, rename.as_ref()).await?;
            if let Some(detail) = loaded.detail.as_mut() {
                detail.revision += 1;
            }
            let (store, record, detail) = (
                self.starter.store.clone(),
                loaded.record.clone(),
                loaded.detail.clone(),
            );
            blocking(move || -> Result<(), StoreError> {
                store.put_session(&record)?;
                match detail {
                    Some(detail) => store.put_session_detail(&detail),
                    None => Ok(()),
                }
            })
            .await?
            .map_err(|error| status::store(error, &self.machine, "save the session's new title"))
            .inspect_err(|_| {
                if let Some(plan) = &rename {
                    plan.undo();
                }
            })?;
            self.tell
                .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
                .await;
        }
        Ok(Gated::Done(self.show(&loaded)))
    }

    async fn stop(
        &self,
        caller: &Caller,
        request: pb::StopSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Session>, Status> {
        Self::refuse_peer(caller)?;
        // D784: a stop for a move is the move's own, the host's call to the
        // machine it is taking the session from. Asked by anyone else, it is
        // the plain Stop.
        let for_move = request.for_move
            && matches!(caller, Caller::User(identity::UserVia::Host))
            && !self.guard.host_link().on_host();
        let id = request
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let mut loaded = self.load(&id).await?;
        let own = matches!(caller, Caller::Agent(agent) if agent.session.id.as_str() == id);
        if let Some(asked) = self.gate(
            caller,
            if own {
                Capability::OwnSessionEnd
            } else {
                Capability::SessionStopOther
            },
            Target::Session(self.session_ref(&loaded.record)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if let Some((machine, remote)) = self.lives_elsewhere(&loaded.record) {
            return self.stop_remote(&id, &loaded, machine, remote).await;
        }
        let Some(agent) = loaded.agent.as_mut() else {
            return Ok(Gated::Done(self.show(&loaded)));
        };
        if conversation_unknown(&agent.launch, &self.starter.agents) {
            return Err(stop_unknown_conversation(&id));
        }
        // D784: Stop keeps a session to resume, and resuming takes up the
        // agent's conversation file. A running agent that hasn't written
        // one yet (no first message, or not signed in) would leave a
        // session that can only fail to resume, so it isn't stopped. A move
        // stops it all the same: its new machine starts it fresh (D757).
        if matches!(agent.state, AgentState::Running { .. })
            && conversation_unwritten(&agent.launch, &self.starter.agents)
        {
            // D670: a CLI that picks its own conversation id has written it
            // somewhere the launch doesn't name yet; learn it first, and
            // keep it in what is saved below.
            // An agent just started may be writing it now, so it is given a
            // moment before Stop is refused. A move doesn't wait: it looks
            // once and goes on without one.
            let grace = if for_move { std::time::Duration::ZERO } else { UNWRITTEN_GRACE };
            let (store, agents, home, key, file) = (
                self.starter.store.clone(),
                self.starter.agents.clone(),
                self.starter.home.clone(),
                id.clone(),
                agent.launch.conversation_file.clone(),
            );
            let launch = agent.launch.clone();
            let learned = blocking(move || -> Result<Option<humata2_store::AgentLaunch>, StoreError> {
                let until = std::time::Instant::now() + grace;
                let agents = crate::config_sync::agents_for_session(&agents, &store, &key);
                loop {
                    if file.is_file() {
                        return Ok(Some(launch));
                    }
                    if agents.learn_conversation(&store, &home, &key)? {
                        return Ok(store.agent(&key)?.map(|found| found.launch));
                    }
                    if std::time::Instant::now() >= until {
                        return Ok(None);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            })
            .await?
            .map_err(|error| status::store(error, &self.machine, "learn the agent's conversation"))?;
            match learned {
                Some(launch) => agent.launch = launch,
                None if for_move => {}
                None => {
                    let name = self.starter.agents.person_name(&agent.launch.agent);
                    return Err(stop_unwritten_conversation(&id, &name));
                }
            }
        }
        match agent.state {
            // Saved as launching with no start of this daemon live for it:
            // its old holder may still run the agent. It is ended first, and
            // the agent is saved stopped only once that holder is gone; if
            // it can't be ended the record stays launching and the error
            // says how to go on.
            AgentState::Launching => {
                let socket = agent.launch.holder_socket.clone();
                let what = format!("session {id}'s terminal");
                stop_launching::end_old_holder(&id, humata2_revive::launch_in_flight(&id), || {
                    holder_end::end_holder_at(&socket, &what)
                })
                .await
                .map_err(|error| stop_launching::status_of(&id, &error))?;
            }
            // D784: left resuming by a run that ended mid-resume (the
            // restart named it stuck) and with no resume of it live now: its
            // old holder is ended as a launching one's is, so nothing is left
            // running. One a live operation is bringing up stays busy.
            AgentState::Resuming if self.left_resuming(&id) => {
                let socket = agent.launch.holder_socket.clone();
                let what = format!("session {id}'s terminal");
                stop_launching::end_old_holder(&id, false, || holder_end::end_holder_at(&socket, &what))
                    .await
                    .map_err(|error| stop_launching::status_of(&id, &error))?;
            }
            AgentState::Resuming => return Err(busy(&id, "being resumed")),
            AgentState::Exited { .. } | AgentState::Stopped | AgentState::ResumeFailed { .. } => {
                return Ok(Gated::Done(self.show(&loaded)));
            }
            // A running agent is saved stopped only once its holder is gone
            // (a holder already gone counts). If it can't be ended the agent
            // may still run, so the record stays running and the error says
            // how to go on.
            AgentState::Running { holder_pid } => {
                let socket = agent.launch.holder_socket.clone();
                let owner = format!("session {id}'s terminal");
                stop_running::end_running_holder(&id, || {
                    holder_end::shut_or_end(&socket, holder_pid, &owner)
                })
                .await
                .map_err(|error| stop_running::status_of(&id, &error))?;
            }
        }
        agent.state = AgentState::Stopped;
        agent.updated_unix_ms = now_ms();
        let (store, saving) = (self.starter.store.clone(), agent.clone());
        blocking(move || store.put_agent(&saving))
            .await?
            .map_err(|error| status::store(error, &self.machine, "save that the agent stopped"))?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        self.withdraw_requests_of(&id, pb::WithdrawReason::SessionStopped)
            .await?;
        Ok(Gated::Done(self.show(&loaded)))
    }

    async fn resume(
        &self,
        caller: &Caller,
        request: pb::ResumeSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        Self::refuse_peer(caller)?;
        let id = request
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let loaded = self.load(&id).await?;
        // D757: the after-move resume, which starts an agent whose
        // conversation is missing fresh, is the move's own: the host's call
        // to the machine it moved the session to. Asked by anyone else, it
        // is the strict resume.
        let moved = request.mode == pb::resume_session_request::Mode::AfterMove as i32
            && matches!(caller, Caller::User(identity::UserVia::Host))
            && !self.guard.host_link().on_host();
        let loaded = Loaded { moved: moved.then_some(MOVED), ..loaded };
        desk_service::refuse_if_closed(&self.starter.store, &self.machine, &id).await?;
        self.refuse_if_archived(&id).await?;
        let own = matches!(caller, Caller::Agent(agent) if agent.session.id.as_str() == id);
        if let Some(asked) = self.gate(
            caller,
            if own {
                Capability::OwnSessionManage
            } else {
                Capability::SessionStopOther
            },
            Target::Session(self.session_ref(&loaded.record)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        turns::refresh_hooks().await;
        if let Some((machine, remote)) = self.lives_elsewhere(&loaded.record) {
            return self
                .resume_remote(caller, request, id, machine, remote)
                .await;
        }
        if request.mode == pb::resume_session_request::Mode::StartFreshWithSummary as i32 {
            return self
                .start_fresh(caller, request.idempotency_key, loaded)
                .await;
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        if fork_remote::unlaunched(&loaded) {
            return self.launch_unlaunched(caller, request.idempotency_key, loaded).await;
        }
        // A session the restart is still bringing back reads as resuming; the
        // resume waits for the restart, but not for ever.
        if loaded.agent.as_ref().is_some_and(|agent| agent.state == AgentState::Resuming)
            && let Some(started) = self.restart.get()
        {
            started.wait_for(crate::restart::Awaited::Resume).await?;
        }
        let loaded = self.after_restart(&id, loaded).await?;
        let running = match loaded.agent.as_ref().map(|agent| agent.state) {
            None => {
                return Err(failed(
                    Code::FailedPrecondition,
                    &NoAgent { session: id },
                    "start a new session",
                    None,
                ));
            }
            Some(AgentState::Launching) => return Err(busy(&id, "still starting")),
            Some(AgentState::Resuming) if self.left_resuming(&id) => false,
            Some(AgentState::Resuming) => return Err(busy(&id, "already being resumed")),
            Some(AgentState::Running { .. }) => {
                let socket = loaded
                    .agent
                    .as_ref()
                    .map(|agent| agent.launch.holder_socket.clone());
                match socket {
                    Some(socket) => humata2_holder::attach(&socket, u64::MAX)
                        .await
                        .ok()
                        .is_some_and(|attached| attached.welcome.exited.is_none()),
                    None => false,
                }
            }
            Some(
                AgentState::Exited { .. } | AgentState::Stopped | AgentState::ResumeFailed { .. },
            ) => false,
        };
        let steps = [
            ("launch".to_owned(), "Starting the agent again".to_owned()),
            (
                "verify".to_owned(),
                "Checking it took up its conversation".to_owned(),
            ),
        ];
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        if running {
            progress.set_step("launch", pb::StepState::Skipped, None);
            progress.set_step("verify", pb::StepState::Skipped, None);
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: id,
            })));
        } else {
            let live = ResumingNow::enter(&id);
            let (starter, tell, by, tokens) = (
                self.starter.clone(),
                self.tell.clone(),
                identity::actor(caller),
                self.tokens.clone(),
            );
            tokio::spawn(async move {
                let _live = live;
                run_resume(starter, tell, by, tokens, loaded, progress).await;
            });
        }
        Ok(Gated::Done(operation))
    }

    /// The session as it stands once this run's restart is done with it. A
    /// session the restart is bringing back reads as resuming until then,
    /// and a resume asked for in the meantime would start a second agent.
    async fn after_restart(&self, id: &str, loaded: Loaded) -> Result<Loaded, Status> {
        let resuming = loaded.agent.as_ref().is_some_and(|agent| agent.state == AgentState::Resuming);
        match self.restart.get() {
            Some(started) if resuming => {
                started.wait().await;
                self.load(id).await
            }
            _ => Ok(loaded),
        }
    }

    /// Whether `id`'s agent is a resuming record with no live resume or
    /// start-fresh running for it (for example after a failed restart or a
    /// failed start-fresh rollback, SMB-046). Nothing runs for such a record,
    /// so Stop, Archive, and Resume may take it again rather than refusing it
    /// as busy until the daemon restarts.
    fn left_resuming(&self, id: &str) -> bool {
        let restart_settled = self
            .restart
            .get()
            .is_none_or(|started| started.get().is_some());
        restart_settled && !resuming_now(id)
    }

    /// The restart doctor's repair of a session (PER-30, D242): ResumeSession's
    /// own path and access rule, started again with its pinned conversation.
    pub(crate) async fn repair_session(
        &self,
        caller: &Caller,
        id: String,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        let request = pb::ResumeSessionRequest {
            id: Some(pb::SessionId { value: id }),
            ..pb::ResumeSessionRequest::default()
        };
        self.resume(caller, request, approved).await
    }

    /// The restart doctor's repair of a shell tab (PER-30, D242): started
    /// again in its folder with the output its old holder saved, as the
    /// restart does; one whose holder answers is left. A session's tab
    /// follows the rule for resuming that session; a machine's own shell is
    /// the user's.
    pub(crate) async fn repair_tab(
        &self,
        caller: &Caller,
        tab: String,
        approved: bool,
    ) -> Result<Gated<()>, Status> {
        Self::refuse_peer(caller)?;
        let store = self.starter.store.clone();
        let key = tab.clone();
        let found = blocking(move || -> Result<Option<(TabRecord, Option<SessionRecord>)>, StoreError> {
            let Some(found) = store.tab(&key)? else {
                return Ok(None);
            };
            let session = match &found.session {
                Some(id) => store.session(id)?,
                None => None,
            };
            Ok(Some((found, session)))
        })
        .await?;
        let (record, session) = match found {
            Ok(Some(found)) => found,
            Ok(None) | Err(StoreError::InvalidId { .. }) => {
                return Err(status::not_found(
                    pb::EntityKind::Tab,
                    &tab,
                    format!("no tab {tab} is on this machine"),
                    "check the tab's id in the restart report; it may have been closed or be on another machine",
                ));
            }
            Err(error) => return Err(status::store(error, &self.machine, "look up the tab")),
        };
        match &session {
            Some(session) => {
                let own = matches!(caller, Caller::Agent(agent) if agent.session.id.as_str() == session.id);
                if let Some(asked) = self.gate(
                    caller,
                    if own {
                        Capability::OwnSessionManage
                    } else {
                        Capability::SessionStopOther
                    },
                    Target::Session(self.session_ref(session)),
                    approved,
                )? {
                    return Ok(Gated::Ask(asked));
                }
            }
            None => {
                if !matches!(caller, Caller::User(_)) {
                    return Err(status::user_only_call(
                        Capability::SessionStopOther,
                        "starting a machine's own shell again",
                        "ask the user to run it, at the app or from their terminal",
                    ));
                }
            }
        }
        let tokens = self.tokens.clone();
        let env = move |tab: &TabRecord| tokens.env(tab);
        let folder = session.map(|session| session.folder);
        humata2_revive::restart_shell(
            &self.starter.store,
            &self.starter.revive,
            record,
            folder,
            &env,
            "restarted by the restart doctor",
        )
        .await
        .map_err(|error| {
            failed(
                Code::FailedPrecondition,
                &error,
                "read the tab's holder log for why, fix that, then repair it again",
                None,
            )
        })?;
        Ok(Gated::Done(()))
    }

    fn session_ref(&self, record: &SessionRecord) -> humata2_access::SessionRef {
        identity::session_ref(record, &self.machine)
    }

    /// Whether `caller` may read `record`, without asking anyone.
    fn may_read(&self, caller: &Caller, record: &SessionRecord) -> bool {
        let call = Call::ReadSession(self.session_ref(record));
        matches!(self.guard.check(caller, &call), Ok(Checked::Allowed))
    }

    async fn find_channel(&self, id: &str) -> Result<ChannelRecord, Status> {
        let store = self.starter.store.clone();
        let key = id.to_owned();
        match blocking(move || store.channel(&key)).await? {
            Ok(Some(channel)) => Ok(channel),
            Ok(None) | Err(StoreError::InvalidId { .. }) => Err(no_channel(id)),
            Err(error) => Err(status::store(error, &self.machine, "look up the channel")),
        }
    }

    async fn load(&self, id: &str) -> Result<Loaded, Status> {
        let store = self.starter.store.clone();
        let key = id.to_owned();
        let found = blocking(move || Loaded::read(&store, &key))
        .await?;
        match found {
            Ok(Some(loaded)) => Ok(loaded),
            Ok(None) | Err(StoreError::InvalidId { .. }) => Err(no_session(id)),
            Err(error) => Err(status::store(error, &self.machine, "look up the session")),
        }
    }

    fn show(&self, loaded: &Loaded) -> pb::Session {
        let mut shown = session_pb(
            &loaded.record,
            loaded.detail.as_ref(),
            loaded.agent.as_ref(),
            loaded.summary_source,
            &self.machine,
        );
        shown.away_left_out.clone_from(&loaded.away_left_out);
        shown.pull_request = loaded.review.as_ref().map(pr_binding);
        shown.shell_tabs.clone_from(&loaded.shell_tabs);
        self.merge_remote(&loaded.record, &mut shown);
        if !remote::elsewhere(&loaded.record, &self.machine) {
            self.tell.pointers.fill(&loaded.record.id, &mut shown);
        }
        self.apply_lineage(&mut shown);
        move_session::show_moving(&mut shown);
        stamp_status_since(&mut shown);
        shown
    }

    /// The session an operation started: the one whose start it drives.
    async fn started_by(
        &self,
        operation: pb::Operation,
    ) -> Result<pb::StartSessionResponse, Status> {
        let id = operation
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        let store = self.starter.store.clone();
        let session = blocking(move || -> Result<Option<String>, StoreError> {
            Ok(store
                .session_details()?
                .into_iter()
                .find(|detail| detail.operation.as_deref() == Some(id.as_str()))
                .map(|detail| detail.session))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "find the session being started"))?;
        let session = session.ok_or_else(|| {
            status::internal(
                "the operation that started a session has no session recorded",
                "",
            )
        })?;
        Ok(pb::StartSessionResponse {
            session: Some(pb::SessionId { value: session }),
            operation: Some(operation),
        })
    }

    fn this_machine(&self, wanted: Option<&pb::MachineId>) -> Result<(), Status> {
        match wanted
            .map(|machine| machine.value.as_str())
            .filter(|machine| !machine.is_empty())
        {
            Some(machine) if machine != self.machine => Err(status::failure(
                Code::Unimplemented,
                pb::Error {
                    message: format!(
                        "machine {machine} is not this machine; sessions can only start on this machine until remote machines are built (MACH)"
                    ),
                    fix: "leave the machine out, or name this machine, and start the session again"
                        .to_owned(),
                    retry: None,
                    kind: Some(pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "machine".to_owned(),
                        problem: Problem::NotAllowedHere as i32,
                    })),
                },
            )),
            _ => Ok(()),
        }
    }

    /// The agent a start would run: the one asked for. When none is asked,
    /// the first that is on of the channel's own default agent, the one in
    /// Settings (ST-15) and the one the channel was made with, else the
    /// first agent that is on (AGENTOFF); every agent off is refused.
    fn agent_for(
        &self,
        wanted: Option<&pb::AgentId>,
        channel: &ChannelRecord,
    ) -> Result<String, Status> {
        let asked = wanted.map(|agent| agent.value.clone()).filter(|agent| !agent.is_empty());
        let name = match asked {
            Some(name) => name,
            None => {
                let mut preferred = crate::settings_service::default_agent_chain(&self.starter.store, &channel.id)
                    .map_err(|error| status::store(error, "", "read the default agent"))?;
                preferred.push(channel.default_agent.clone());
                crate::agent_gate::pick(&self.starter.agents, &preferred).map_err(Status::from)?
            }
        };
        agent_known(&self.starter.agents, &name)?;
        Ok(name)
    }
}

fn agent_known(agents: &Agents, name: &str) -> Result<(), Status> {
    agents.get(name).map(|_| ()).map_err(|error| {
        failed(
            Code::InvalidArgument,
            &error,
            "pick one of the agents named, or add the agent in Settings",
            Some(pb::error::Kind::InvalidField(pb::InvalidField {
                field: "agent".to_owned(),
                problem: Problem::UnknownValue as i32,
            })),
        )
    })
}

fn no_session(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Session,
        id,
        format!("no session {id} is on this machine"),
        "list the sessions to see their ids; it may have been removed or be on another machine",
    )
}

fn no_channel(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Channel,
        id,
        format!("no channel {id} is on this machine"),
        "list the channels to see their ids",
    )
}

fn busy(session: &str, what: &str) -> Status {
    busy_running(session, what, None)
}

/// `busy`, naming the operation the session is busy with (MV-4): the person
/// waits for that operation to finish.
fn busy_running(session: &str, what: &str, running: Option<&str>) -> Status {
    status::failure(
        Code::Aborted,
        pb::Error {
            message: format!("session {session} is {what}"),
            fix: "wait for that to finish, then try again".to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::SessionBusy(pb::SessionBusy {
                session: Some(pb::SessionId {
                    value: session.to_owned(),
                }),
                running: running.map(|value| pb::OperationId { value: value.to_owned() }),
                running_title: what.to_owned(),
            })),
        },
    )
}

/// What the API calls a session, from its agent and how far its start got.
fn session_status(agent: Option<&AgentState>, start: Option<StartStep>) -> pb::SessionStatus {
    if start.is_some_and(|step| step != StartStep::Done) {
        return pb::SessionStatus::Starting;
    }
    match agent {
        Some(AgentState::Launching | AgentState::Resuming) => pb::SessionStatus::Starting,
        Some(AgentState::Running { .. }) => pb::SessionStatus::Working,
        // D784: the person stopped it; Resume brings it back. Only a running
        // agent between turns is idle.
        Some(AgentState::Stopped) => pb::SessionStatus::Stopped,
        Some(AgentState::Exited { .. }) => pb::SessionStatus::Exited,
        Some(AgentState::ResumeFailed { .. }) => pb::SessionStatus::ResumeFailed,
        None => pb::SessionStatus::Idle,
    }
}

/// `session_status`, with a running agent's turn (D169) in place of the
/// WORKING it starts as.
fn shown_status(
    session: &str,
    kind: &str,
    agent: Option<&AgentState>,
    start: Option<StartStep>,
) -> pb::SessionStatus {
    // D264: an agent a Start would bring back reads as stopped with its
    // machine; the store still says it runs.
    if crate::stop::gate::is_stopped()
        && matches!(
            agent,
            Some(AgentState::Launching | AgentState::Resuming | AgentState::Running { .. })
        )
    {
        return pb::SessionStatus::MachineStopped;
    }
    match (session_status(agent, start), agent) {
        (pb::SessionStatus::Working, Some(AgentState::Running { holder_pid })) => {
            // D784: its holder is gone while this daemon runs, so it is
            // ended until a Resume or a start brings it back.
            if watch::lost(session, *holder_pid) {
                pb::SessionStatus::Exited
            // An agent waiting on its own request needs you, turns or not
            // (D158.7); one with no turns otherwise shows none (SE-21).
            } else if turns::asking(session) {
                pb::SessionStatus::NeedsInput
            } else if turns::source_of(kind) == pb::TurnEndSource::None {
                pb::SessionStatus::RunningUnknownTurn
            } else {
                running_turn(session, *holder_pid)
            }
        }
        (status, _) => status,
    }
}

fn exit_code(agent: Option<&AgentState>) -> i32 {
    match agent {
        Some(AgentState::Exited { code }) => *code,
        Some(AgentState::ResumeFailed { exit_code, .. }) => exit_code.unwrap_or(0),
        _ => 0,
    }
}

/// What the host left out of an agent's replayed writes, as the feed shows it.
fn away_pb(left: humata2_store::AwayLeftOut) -> pb::AwayWritesLeftOut {
    pb::AwayWritesLeftOut {
        count: left.count,
        items: left
            .items
            .into_iter()
            .map(|value| pb::BoardItemId { value })
            .collect(),
        not_made: left.not_made,
        refused: left.refused,
        at: Some(timestamp_ms(left.at_unix_ms)),
    }
}

/// Records the session's final shown status (after remote merge, lineage, and
/// move state) and stamps `status_since` on `shown` (PROGRESS T5).
fn stamp_status_since(shown: &mut pb::Session) {
    stamp_status_since_at(shown, now_ms());
}

fn stamp_status_since_at(shown: &mut pb::Session, at_ms: i64) {
    let Some(id) = shown
        .id
        .as_ref()
        .map(|id| id.value.as_str())
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let status = pb::SessionStatus::try_from(shown.status)
        .unwrap_or(pb::SessionStatus::Unspecified);
    shown.status_since = Some(timestamp_ms(status_since::observe(id, status, at_ms)));
}

fn session_pb(
    record: &SessionRecord,
    detail: Option<&SessionDetail>,
    agent: Option<&AgentRecord>,
    summary_source: pb::session::SummarySource,
    here: &str,
) -> pb::Session {
    let state = agent.map(|agent| &agent.state);
    let status = shown_status(&record.id, &record.agent, state, detail.map(|detail| detail.start));
    pb::Session {
        id: Some(pb::SessionId {
            value: record.id.clone(),
        }),
        channel: Some(pb::ChannelId {
            value: record.channel.clone(),
        }),
        title: record.title.clone(),
        agent: Some(pb::AgentId {
            value: record.agent.clone(),
        }),
        model: detail
            .and_then(|detail| detail.model.clone())
            .unwrap_or_default(),
        machine: Some(pb::MachineId {
            value: if record.machine.is_empty() {
                here.to_owned()
            } else {
                record.machine.clone()
            },
        }),
        folder: record.folder.display().to_string(),
        status: status as i32,
        // Stamped by `stamp_status_since` after remote merge, lineage, and move
        // state settle on the final status shown to callers.
        status_since: None,
        repos: detail
            .map(|detail| {
                detail
                    .repos
                    .iter()
                    .map(|repo| pb::SessionRepo {
                        repo: Some(pb::RepoRef {
                            owner: repo.owner.clone(),
                            name: repo.name.clone(),
                        }),
                        branch: repo.branch.clone(),
                        path: repo.path.display().to_string(),
                        main_clone: false,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        conversation_id: agent
            .map(|agent| agent.launch.conversation_id.clone())
            .unwrap_or_default(),
        started_at: Some(timestamp_ms(record.created_unix_ms)),
        exit_code: exit_code(state),
        turn_end: turns::source_of(&record.agent) as i32,
        revision: Some(pb::Revision {
            value: detail.map_or(1, |detail| detail.revision),
        }),
        resume_failed: match state {
            Some(AgentState::ResumeFailed { reason, exit_code }) => Some(pb::ResumeFailed {
                session: Some(pb::SessionId {
                    value: record.id.clone(),
                }),
                reason: resume_reason(*reason) as i32,
                exit_code: exit_code.unwrap_or(0),
            }),
            _ => None,
        },
        // The agent record keeps every interruption, newest last.
        interrupted_at: agent
            .and_then(|agent| agent.interruptions.last())
            .map(|interruption| timestamp_ms(interruption.at_unix_ms)),
        summary_source: summary_source as i32,
        unread: crate::read_marks::is_unread(&record.id),
        pinned: crate::read_marks::is_pinned(&record.id),
        vip: crate::read_marks::is_vip(&record.id),
        ..pb::Session::default()
    }
}

/// Sessions whose shells changed outside this service, as "End for good"
/// on a shell tab from the running holders list (D784) removes one.
static SHELLS_CHANGED: std::sync::LazyLock<broadcast::Sender<String>> =
    std::sync::LazyLock::new(|| broadcast::channel(64).0);

/// Tells `session`'s feed its shells changed, so clients drop a shell
/// removed outside this service.
pub(crate) fn shells_changed(session: &str) {
    // Nobody may be following yet; the store is right all the same.
    let _ = SHELLS_CHANGED.send(session.to_owned());
}

/// CH-16, CH-17 (D392): tells the feed each session whose read mark
/// changed, so every client shows it bold or not without asking. When some
/// changes are lost, every client reloads now (D395).
/// Also tells it each session whose shells changed elsewhere.
async fn tell_read_marks(tell: Tell, mut changes: broadcast::Receiver<String>) {
    use tokio::sync::broadcast::error::RecvError;
    loop {
        match changes.recv().await {
            Ok(session) => tell.session(&session, pb::Change::Updated, None).await,
            Err(RecvError::Lagged(_)) => tell.feed.resync_all(),
            Err(RecvError::Closed) => return,
        }
    }
}

pub(crate) fn clean_title(title: &str) -> Result<String, Status> {
    let title = title.trim();
    if title.chars().count() > TITLE_MAX {
        return Err(status::invalid_field(
            "title",
            Problem::TooLong,
            format!("the title is longer than {TITLE_MAX} characters"),
            format!("use a shorter title, at most {TITLE_MAX} characters"),
        ));
    }
    Ok(title.to_owned())
}

/// The repositories a start clones, as `(owner, name)` (FC-4). `choices`
/// naming a repository with `include` lists exactly those; otherwise every
/// repository whose main clone is ready, less the ones left out.
fn choose_repos(
    channel: &ChannelRecord,
    choices: &[pb::RepoChoice],
) -> Result<Vec<(String, String)>, Status> {
    let mut included: Vec<(String, String)> = Vec::new();
    let mut left_out: Vec<(String, String)> = Vec::new();
    for (n, choice) in choices.iter().enumerate() {
        let repo = choice.repo.clone().unwrap_or_default();
        let pair = (repo.owner, repo.name);
        if !channel
            .repos
            .iter()
            .any(|r| (&r.owner, &r.name) == (&pair.0, &pair.1))
        {
            return Err(status::invalid_field(
                &format!("repos[{n}].repo"),
                Problem::UnknownValue,
                format!("the channel has no repository {}/{}", pair.0, pair.1),
                "pick one of the channel's repositories",
            ));
        }
        let list = if choice.include {
            &mut included
        } else {
            &mut left_out
        };
        if !list.contains(&pair) {
            list.push(pair);
        }
    }
    let ready = |r: &humata2_store::ChannelRepo| r.main == MainClone::Ready;
    if included.is_empty() {
        let chosen: Vec<(String, String)> = channel
            .repos
            .iter()
            .filter(|r| ready(r) && !left_out.contains(&(r.owner.clone(), r.name.clone())))
            .map(|r| (r.owner.clone(), r.name.clone()))
            .collect();
        // CH-10: a channel with no repos starts its sessions in an empty
        // folder; only repos that are all still cloning hold a start back.
        if chosen.is_empty() && !channel.repos.is_empty() {
            return Err(status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: format!("the channel {} has no repository ready to start a session in", channel.name),
                    fix: "wait for the channel's clones to finish, or add a repository, and start the session again".to_owned(),
                    retry: None,
                    kind: Some(pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "repos".to_owned(),
                        problem: Problem::Missing as i32,
                    })),
                },
            ));
        }
        return Ok(chosen);
    }
    for (owner, name) in &included {
        let repo = channel
            .repos
            .iter()
            .find(|r| &r.owner == owner && &r.name == name);
        if let Some(repo) = repo.filter(|r| !ready(r)) {
            return Err(status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: format!("{owner}/{name} isn't cloned yet ({})", describe_main(&repo.main)),
                    fix: "wait for the channel's clones to finish, or leave that repository out, and start the session again".to_owned(),
                    retry: None,
                    kind: Some(pb::error::Kind::InvalidField(pb::InvalidField {
                        field: "repos".to_owned(),
                        problem: Problem::NotAllowedHere as i32,
                    })),
                },
            ));
        }
    }
    Ok(included)
}

fn describe_main(main: &MainClone) -> String {
    match main {
        MainClone::Cloning => "still cloning".to_owned(),
        MainClone::Ready => "ready".to_owned(),
        MainClone::Failed { reason } => format!("the clone failed: {reason}"),
    }
}

/// A start's error as the API shows it, and the code it travels with.
pub(crate) fn start_error(error: &StartError) -> (Code, pb::Error) {
    let channel_not_found = |id: &str| {
        (
            Code::NotFound,
            error_of(
                error,
                "list the channels to see their ids",
                Some(pb::error::Kind::NotFound(pb::NotFound {
                    kind: pb::EntityKind::Channel as i32,
                    id: id.to_owned(),
                    in_trash: false,
                })),
            ),
        )
    };
    let field = |name: &str, problem: Problem| {
        Some(pb::error::Kind::InvalidField(pb::InvalidField {
            field: name.to_owned(),
            problem: problem as i32,
        }))
    };
    match error {
        StartError::NoChannel { id } => channel_not_found(id),
        StartError::UnknownRepo { .. } => (
            Code::InvalidArgument,
            error_of(
                error,
                "pick one of the channel's repositories",
                field("repos", Problem::UnknownValue),
            ),
        ),
        StartError::NotReady { .. } => (
            Code::FailedPrecondition,
            error_of(
                error,
                "wait for the channel's clones to finish",
                field("repos", Problem::NotAllowedHere),
            ),
        ),
        StartError::BadBranch { .. } => (
            Code::InvalidArgument,
            error_of(
                error,
                "type a branch name git accepts (no spaces, `..`, `~`, `^`, `:` or a trailing `/`), or clear it to let Humata pick one",
                field("branch", Problem::BadFormat),
            ),
        ),
        StartError::BranchTaken { branch, suggestion } => (
            Code::AlreadyExists,
            error_of(
                error,
                "use the free name suggested, or clear the branch to let Humata pick one",
                Some(pb::error::Kind::NameTaken(pb::NameTaken {
                    kind: pb::EntityKind::Session as i32,
                    name: branch.clone(),
                    suggestion: suggestion.clone(),
                })),
            ),
        ),
        StartError::Agent(_) => (
            Code::InvalidArgument,
            error_of(
                error,
                "pick one of the agents named",
                field("agent", Problem::UnknownValue),
            ),
        ),
        StartError::Git { owner, name, .. } => (
            Code::Unavailable,
            error_of(
                error,
                "the session was removed; check the repository's address, that this machine can reach it and has access, then start the session again",
                Some(pb::error::Kind::GitFailed(pb::GitFailed {
                    step: pb::git_failed::Step::Clone as i32,
                    repo: Some(pb::RepoRef {
                        owner: owner.clone(),
                        name: name.clone(),
                    }),
                    machine: None,
                    exit_code: 0,
                    stderr_tail: status::chain(error),
                })),
            ),
        ),
        StartError::FolderExists { .. } => (
            Code::AlreadyExists,
            error_of(error, "start the session with another title", None),
        ),
        StartError::Store { .. } | StartError::MakeFolder { .. } => (
            Code::Unavailable,
            error_of(
                error,
                "fix the cause the message names, then start the session again",
                None,
            ),
        ),
        StartError::Revive {
            source: humata2_revive::ReviveError::Contain { .. },
        } => (
            Code::FailedPrecondition,
            error_of(
                error,
                "the session and its folder were kept: do what the message says, or turn Contain agents off for this channel, then resume the session",
                None,
            ),
        ),
        StartError::Revive { .. } | StartError::FirstMessage { .. } => (
            Code::Unavailable,
            error_of(
                error,
                "the session and its folder were kept: open it to see what the agent printed, and resume it once the cause is fixed",
                None,
            ),
        ),
        StartError::Remove { .. } | StartError::Background { .. } => (
            Code::Internal,
            error_of(error, "restart humatad2 and try again", None),
        ),
        StartError::NoSession { id } => (
            Code::NotFound,
            error_of(
                error,
                "list the sessions to see their ids",
                Some(pb::error::Kind::NotFound(pb::NotFound {
                    kind: pb::EntityKind::Session as i32,
                    id: id.clone(),
                    in_trash: false,
                })),
            ),
        ),
        StartError::NoAgent { .. } => (
            Code::FailedPrecondition,
            error_of(error, "fork with the code only, or start the session's agent first", None),
        ),
        StartError::NoConversation { .. }
        | StartError::Conversation { .. }
        | StartError::NoConversationFormat
        | StartError::Copy { .. } => (
            Code::Unavailable,
            error_of(error, "nothing was kept; fix the cause the message names, then fork again", None),
        ),
    }
}

/// Moves the operation's steps along for a progress word from
/// `Starter::start`, and answers which step runs now. `write_record` and
/// each `clone:<repo>` report that step done; `launch` that it began;
/// `verify` that the launch is done and the check began.
fn advance(progress: &Progress, keys: &[String], event: &str) -> String {
    match event {
        "launch" => {
            progress.set_step("launch", pb::StepState::Running, None);
            "launch".to_owned()
        }
        "verify" => {
            progress.set_step("launch", pb::StepState::Done, None);
            progress.set_step("verify", pb::StepState::Running, None);
            "verify".to_owned()
        }
        done => {
            progress.set_step(done, pb::StepState::Done, None);
            match keys
                .iter()
                .position(|key| key == done)
                .and_then(|n| keys.get(n + 1))
            {
                Some(next) => {
                    progress.set_step(next, pb::StepState::Running, None);
                    next.clone()
                }
                None => done.to_owned(),
            }
        }
    }
}

/// Runs the start for `progress`. `ready` hears once the session is
/// recorded (the call answers then), or the failure when it didn't get
/// that far.
async fn run_start(
    starter: Starter,
    tell: Tell,
    tokens: Arc<Tokens>,
    request: StartRequest,
    progress: Progress,
    keys: Vec<String>,
    ready: oneshot::Sender<Result<(), Status>>,
    by: pb::Actor,
    review: Option<ReviewStart>,
) {
    let mut ready = Some(ready);
    let operation = request.operation.clone();
    let mut current = "write_record".to_owned();
    progress.set_step("write_record", pb::StepState::Running, None);
    let env = |tab: &TabRecord| tokens.env(tab);
    let started = {
        let mut note = |event: &str| {
            current = advance(&progress, &keys, event);
            if event == "write_record"
                && let Some(ready) = ready.take()
            {
                let _ = ready.send(Ok(()));
            }
        };
        let units = progress.units();
        let sink: humata2_sessions::CloneUnits = Arc::new(move |key: &str, done, total| units.set(key, done, total));
        humata2_sessions::with_clone_units(sink, async {
            match review {
                Some(review) => starter.start_review(request, review, &env, &mut note).await,
                None => starter.start(request, &env, &mut note).await,
            }
        })
        .await
    };
    match started {
        Ok(started) => {
            progress.set_step("verify", pb::StepState::Done, None);
            let session = started.session.id;
            // The watch hears the agent before the start says it runs, so
            // its first answer is never missed (CH-16).
            let _ = watch::agent_attached(tell.clone(), session.clone()).await;
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: session.clone(),
            })));
            tell.session(&session, pb::Change::Updated, Some(by)).await;
        }
        Err(error) => {
            let (code, failure) = start_error(&error);
            progress.set_step(&current, pb::StepState::Failed, Some(status::chain(&error)));
            if let Some(ready) = ready.take() {
                let _ = ready.send(Err(status::failure(code, failure.clone())));
            }
            progress.fail(failure);
            if let Some(operation) = operation
                && let Some(session) = tell.session_of(&operation).await
            {
                tell.session(&session, pb::Change::Updated, Some(by)).await;
            }
        }
    }
}

/// D784 (Fable B2): an adopted agent (not a plain shell) whose conversation
/// Humata never learned. Stopping it could not be undone by Resume.
fn conversation_unknown(launch: &humata2_store::AgentLaunch, agents: &Agents) -> bool {
    launch.conversation_id.is_empty()
        && !agents.get(&launch.agent).is_ok_and(humata2_sessions::AgentKind::is_shell)
}

/// How long Stop waits for a just-started agent to write its conversation
/// file before it is refused for not having one.
const UNWRITTEN_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether `launch`'s agent keeps a conversation file and hasn't written
/// it yet. A plain shell keeps none.
fn conversation_unwritten(launch: &humata2_store::AgentLaunch, agents: &Agents) -> bool {
    !launch.conversation_file.as_os_str().is_empty()
        && !agents.get(&launch.agent).is_ok_and(humata2_sessions::AgentKind::is_shell)
        && !launch.conversation_file.is_file()
}

/// Stop refused for an agent that hasn't written its conversation yet.
fn stop_unwritten_conversation(session: &str, agent: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        pb::Error {
            message: format!(
                "session {session}'s {agent} hasn't started its conversation yet, so a stopped session would have nothing to resume"
            ),
            fix: "send it a first message (sign it in first if it asks), then stop it; or keep it running, or End it for good".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// Stop refused for an agent whose conversation isn't known.
fn stop_unknown_conversation(session: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        pb::Error {
            message: format!(
                "session {session}'s agent was adopted and its conversation isn't known, so it can't be stopped and resumed later"
            ),
            fix: "End it for good, or keep it running".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// The reason a failed resume gives, as the API names it.
fn resume_reason(reason: ResumeFailure) -> pb::resume_failed::Reason {
    use pb::resume_failed::Reason;
    match reason {
        ResumeFailure::ConversationFileMissing => Reason::ConversationFileMissing,
        ResumeFailure::AgentStartedAnotherConversation => Reason::AgentStartedAnotherConversation,
        ResumeFailure::AgentExited => Reason::AgentExited,
        ResumeFailure::AgentNotInstalled => Reason::AgentNotInstalled,
        ResumeFailure::FolderMissing => Reason::FolderMissing,
        ResumeFailure::ConversationUnknown => Reason::ConversationUnknown,
    }
}

/// What a failed resume says and what to do about it (PER-15: never a
/// silent fresh start).
fn resume_failed(session: &str, reason: ResumeFailure, exit_code: Option<i32>) -> pb::Error {
    let (what, fix) = match reason {
        ResumeFailure::ConversationFileMissing => (
            "the agent's conversation file is missing, so its conversation can't be resumed",
            "restore the file if you have a copy and resume again; otherwise start a new session. Nothing was started in its place",
        ),
        ResumeFailure::AgentStartedAnotherConversation => (
            "the agent started a different conversation instead of the pinned one",
            "check that the agent's resume command still takes the saved conversation id, then resume again. Nothing was kept running",
        ),
        ResumeFailure::AgentExited => (
            "the agent exited as soon as it was resumed",
            "open the session's terminal to read what it printed, fix the cause, then resume again",
        ),
        ResumeFailure::AgentNotInstalled => (
            "the agent's program isn't installed on this machine",
            "install it (or fix its path in Settings > Agents), then resume again",
        ),
        ResumeFailure::ConversationUnknown => (
            "it was adopted and its agent's conversation isn't known, so there is nothing to resume",
            "End it for good, or keep it running; it can't be resumed because its conversation isn't known",
        ),
        ResumeFailure::FolderMissing => (
            "the session's folder is gone",
            "the work in it can't be recovered here; start a new session",
        ),
    };
    pb::Error {
        message: format!("couldn't resume session {session}: {what}"),
        fix: fix.to_owned(),
        retry: None,
        kind: Some(pb::error::Kind::ResumeFailed(pb::ResumeFailed {
            session: Some(pb::SessionId {
                value: session.to_owned(),
            }),
            reason: resume_reason(reason) as i32,
            exit_code: exit_code.unwrap_or(0),
        })),
    }
}

/// What the terminal of an agent a move landed here shows first.
const MOVED: &str = "Moved to this machine; the conversation continues.";

/// Brings `loaded`'s agent back for `progress`, in the background. Revive
/// resumes an agent whose record says it should run (it stops a holder
/// still answering), so the record is saved as resuming first and revive
/// does the rest for this one agent.
pub(crate) async fn run_resume(
    starter: Starter,
    tell: Tell,
    by: pb::Actor,
    tokens: Arc<Tokens>,
    loaded: Loaded,
    progress: Progress,
) {
    let Loaded { record, agent, moved, .. } = loaded;
    let session = record.id.clone();
    progress.set_step("launch", pb::StepState::Running, None);
    let Some(mut agent) = agent else {
        progress.fail(error_of(&NoAgent { session }, "start a new session", None));
        return;
    };
    // D671 (Provisional): an agent the person stopped, or that exited, and
    // is resumed on their ask was not interrupted: it isn't told so.
    let asked = matches!(agent.state, AgentState::Stopped | AgentState::Exited { .. });
    agent.state = AgentState::Resuming;
    agent.updated_unix_ms = now_ms();
    let (store, mut saving) = (starter.store.clone(), agent.clone());
    let (agents, home) = (starter.agents.clone(), starter.home.clone());
    let saved = tokio::task::spawn_blocking(move || -> Result<AgentRecord, StoreError> {
        // D670: resume the conversation the CLI wrote, once it is known,
        // not the placeholder it was started with.
        let agents = crate::config_sync::agents_for_session(&agents, &store, &saving.session);
        if agents.learn_conversation(&store, &home, &saving.session)?
            && let Some(found) = store.agent(&saving.session)?
        {
            saving.launch = found.launch;
        }
        store.put_agent(&saving)?;
        Ok(saving)
    })
    .await;
    match saved {
        Ok(Ok(saved)) => agent = saved,
        Ok(Err(error)) => {
            progress.set_step("launch", pb::StepState::Failed, Some(status::chain(&error)));
            progress.fail(error_of(
                &error,
                "fix the cause the message names, then resume again",
                None,
            ));
            return;
        }
        Err(error) => {
            progress.fail(error_of(&error, "restart humatad2 and resume again", None));
            return;
        }
    }
    tell.session(&session, pb::Change::Updated, Some(by.clone()))
        .await;
    // A test holds a resume here, saved as resuming, to act on it mid-way
    // (D784). Test daemons only.
    crate::fault::hold_point("resume-saved").await;
    let env = |tab: &TabRecord| tokens.env(tab);
    let outcome = match moved {
        Some(line) => resume_moved_agent(&starter.store, &starter.revive, agent, &env, line).await,
        None if asked => resume_agent_asked(&starter.store, &starter.revive, agent, &env).await,
        None => resume_agent(&starter.store, &starter.revive, agent, &env).await,
    };
    match outcome {
        Ok(Revived::ResumeFailed { reason, exit_code }) => {
            let failure = resume_failed(&session, reason, exit_code);
            progress.set_step(
                "launch",
                pb::StepState::Failed,
                Some(failure.message.clone()),
            );
            progress.fail(failure);
        }
        Ok(Revived::Resumed { .. } | Revived::Reattached | Revived::Relaunched) => {
            move_session::restart_session_shells(&starter, &session, &env).await;
            progress.set_step("launch", pb::StepState::Done, None);
            progress.set_step("verify", pb::StepState::Done, None);
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: session.clone(),
            })));
            watch::agent(tell.clone(), session.clone());
        }
        Ok(Revived::Exited { .. } | Revived::Left) => {
            progress.set_step("launch", pb::StepState::Failed, None);
            progress.fail(error_of(
                &NotResumed {
                    session: session.clone(),
                },
                "open the session to see its state, then resume again",
                None,
            ));
        }
        Err(error) => {
            progress.set_step("launch", pb::StepState::Failed, Some(status::chain(&error)));
            progress.fail(error_of(
                &error,
                "check humatad2's log for what failed, then resume again",
                None,
            ));
        }
    }
    tell.session(&session, pb::Change::Updated, Some(by)).await;
}

/// The sessions a resume asked of this daemon is bringing back right now.
static RESUMING_NOW: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

fn resuming_now(session: &str) -> bool {
    RESUMING_NOW
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(session)
}

/// Marks a session as being resumed by this daemon until dropped.
pub(super) struct ResumingNow(String);

impl ResumingNow {
    pub(super) fn enter(session: &str) -> Self {
        RESUMING_NOW
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session.to_owned());
        Self(session.to_owned())
    }
}

impl Drop for ResumingNow {
    fn drop(&mut self) {
        RESUMING_NOW
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

#[derive(Debug, thiserror::Error)]
#[error("session {session} has no agent to resume")]
struct NoAgent {
    session: String,
}

#[derive(Debug, thiserror::Error)]
#[error("session {session} runs on another machine, so this machine has no conversation file for it")]
struct ReadElsewhere {
    session: String,
}

#[derive(Debug, thiserror::Error)]
#[error("session {session}'s agent was not started again")]
struct NotResumed {
    session: String,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

#[tonic::async_trait]
impl pb::session_service_server::SessionService for SessionSvc {
    /// Checks the request and answers once the session is recorded; the
    /// clones and the launch run on in the operation.
    async fn start_session(
        &self,
        request: Request<pb::StartSessionRequest>,
    ) -> Result<Response<pb::StartSessionResponse>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let assigned = identity::host_assigned_id(&request, &caller)?;
        // UP-1, D281 e: nothing starts on a machine at another release.
        if let Some(machine) = &request.get_ref().machine {
            crate::upgrade::gate::machine_ready(&self.migrations, machine)?;
        }
        let bound = guard::hold_bound(&caller, request.metadata());
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        // A machine that hasn't been sent the Mac's rules can't tell
        // whether to contain the agent (D266): it waits for the copy.
        self.guard.host_rules(bound).await?;
        // Starting on another machine waits for the Mac while it is away.
        let first = self
            .guard
            .held(bound, || {
                self.begin_session(&caller, request.clone(), assigned.clone(), false)
            })
            .await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.StartSessionResponse",
            first,
            move || async move { me.begin_session(&who, request, assigned, true).await },
        )
        .await
    }

    /// The branch each repository would get for the title (FC-15).
    async fn preview_branch_names(
        &self,
        request: Request<pb::PreviewBranchNamesRequest>,
    ) -> Result<Response<pb::PreviewBranchNamesResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = None;
        let first = self.preview_names(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.PreviewBranchNamesResponse",
            first,
            move || async move { me.preview_names(&who, request, true).await },
        )
        .await
    }

    async fn get_session(
        &self,
        request: Request<pb::GetSessionRequest>,
    ) -> Result<Response<pb::Session>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = None;
        let first = self.read_session(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Session",
            first,
            move || async move { me.read_session(&who, request, true).await },
        )
        .await
    }

    async fn list_sessions(
        &self,
        request: Request<pb::ListSessionsRequest>,
    ) -> Result<Response<pb::ListSessionsResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::refuse_peer(&caller)?;
        let request = request.into_inner();
        let channel = request
            .channel
            .map(|channel| channel.value)
            .filter(|channel| !channel.is_empty());
        let machine = request
            .machine
            .map(|machine| machine.value)
            .filter(|machine| !machine.is_empty());
        if self.remote.is_none()
            && machine
                .as_ref()
                .is_some_and(|machine| machine != &self.machine)
        {
            return Ok(Response::new(pb::ListSessionsResponse::default()));
        }
        let store = self.starter.store.clone();
        let include_archived = request.include_archived;
        let (sessions, details, agents, sources, lineage, left_out, reviews, shell_tabs) = blocking(move || {
            let left_out: HashMap<String, pb::AwayWritesLeftOut> = store
                .away_left_outs()?
                .into_iter()
                .map(|(session, left)| (session, away_pb(left)))
                .collect();
            let reviews: HashMap<String, pb::PullRequestBinding> = store
                .review_sessions()?
                .iter()
                .map(|review| (review.session.clone(), pr_binding(review)))
                .collect();
            let agents = store.agents()?;
            // Read from the disk, so here and not in the handler.
            let sources: HashMap<String, pb::session::SummarySource> = agents
                .iter()
                .map(|agent| (agent.session.clone(), start_fresh::summary_source(Some(agent))))
                .collect();
            let lineage = lineage::Lineage::read(&store)?;
            // A Desk session that was deleted is hidden from every list (D182.2).
            let deleted: std::collections::HashSet<String> = store
                .desks()?
                .into_iter()
                .filter(|row| row.deleted_unix_ms.is_some())
                .map(|row| row.session_id)
                .collect();
            let shell_tabs =
                humata2_svc_terminal::agent_tabs::shell_tabs_by_session(&store).unwrap_or_default();
            Ok::<_, StoreError>((
                store
                    .sessions()?
                    .into_iter()
                    .filter(|session| !deleted.contains(&session.id))
                    .collect::<Vec<_>>(),
                store.session_details()?,
                agents,
                sources,
                lineage,
                left_out,
                reviews,
                shell_tabs,
            ))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "list the sessions"))?;
        let mut details: HashMap<String, SessionDetail> = details
            .into_iter()
            .map(|detail| (detail.session.clone(), detail))
            .collect();
        let mut agents: HashMap<String, AgentRecord> = agents
            .into_iter()
            .map(|agent| (agent.session.clone(), agent))
            .collect();
        let mut sessions: Vec<SessionRecord> = sessions
            .into_iter()
            .filter(|session| {
                channel
                    .as_ref()
                    .is_none_or(|channel| &session.channel == channel)
            })
            .filter(|session| {
                machine.as_ref().is_none_or(|machine| {
                    let lives_on = if session.machine.is_empty() {
                        &self.machine
                    } else {
                        &session.machine
                    };
                    lives_on == machine
                })
            })
            .filter(|session| self.may_read(&caller, session))
            .filter(|session| include_archived || !lineage.is_archived(&session.id))
            .collect();
        sessions.sort_by(|a, b| (a.created_unix_ms, &a.id).cmp(&(b.created_unix_ms, &b.id)));
        let sessions = sessions
            .iter()
            .map(|session| {
                let mut shown = session_pb(
                    session,
                    details.remove(&session.id).as_ref(),
                    agents.remove(&session.id).as_ref(),
                    sources
                        .get(&session.id)
                        .copied()
                        .unwrap_or(pb::session::SummarySource::Unspecified),
                    &self.machine,
                );
                shown.away_left_out = left_out.get(&session.id).cloned();
                shown.pull_request = reviews.get(&session.id).cloned();
                shown.shell_tabs = shell_tabs
                    .get(&session.id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|entry| pb::ShellTab {
                        tab: Some(pb::TabId { value: entry.tab }),
                        label: entry.label,
                        agent: entry.agent.map(|value| pb::AgentId { value }),
                    })
                    .collect();
                self.merge_remote(session, &mut shown);
                lineage.apply(&mut shown);
                move_session::show_moving(&mut shown);
                stamp_status_since(&mut shown);
                shown
            })
            .collect();
        Ok(Response::new(pb::ListSessionsResponse {
            sessions,
            page: None,
        }))
    }

    /// Changes the title shown; the branch keeps its name (FC-17). An agent
    /// may rename only its own session.
    async fn rename_session(
        &self,
        request: Request<pb::RenameSessionRequest>,
    ) -> Result<Response<pb::Session>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.retitle(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Session",
            first,
            move || async move { me.retitle(&who, request, true).await },
        )
        .await
    }

    /// Ends the agent's program and its holder and saves the agent as
    /// stopped, so nothing resumes it until the user does (SE-14). The
    /// session, its folder and its clones stay.
    async fn stop_session(
        &self,
        request: Request<pb::StopSessionRequest>,
    ) -> Result<Response<pb::Session>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.stop(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Session",
            first,
            move || async move { me.stop(&who, request, true).await },
        )
        .await
    }

    /// D801 J: pauses or resumes the agent a move is taking away. Only the
    /// host's own call counts, as `StopSession.for_move` does.
    async fn freeze_session(
        &self,
        request: Request<pb::FreezeSessionRequest>,
    ) -> Result<Response<pb::FreezeSessionResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::refuse_peer(&caller)?;
        if !matches!(caller, Caller::User(identity::UserVia::Host)) || self.guard.host_link().on_host() {
            return Err(status::refused(&Refusal::PeerCannotCall));
        }
        let request = request.into_inner();
        let id = request
            .id
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("id"))?;
        let ran = move_session::freeze_agent_here(self, &id, &self.machine, request.thaw).await?;
        Ok(Response::new(pb::FreezeSessionResponse { ran }))
    }

    /// Starts the agent again with its pinned conversation (SE-15, PER-13):
    /// an agent that is running is left as it is. Starting fresh with a
    /// summary isn't built.
    async fn resume_session(
        &self,
        request: Request<pb::ResumeSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let bound = guard::hold_bound(&caller, request.metadata());
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        // As for a start: no launch before the Mac's rules are here (D266).
        self.guard.host_rules(bound).await?;
        let first = self.resume(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.resume(&who, request, true).await },
        )
        .await
    }

    async fn list_turns(
        &self,
        request: Request<pb::ListTurnsRequest>,
    ) -> Result<Response<pb::ListTurnsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let first = self.list_turns_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            None,
            "humata.v1.ListTurnsResponse",
            first,
            move || async move { me.list_turns_of(&who, request, true).await },
        )
        .await
    }

    async fn preview_fork(
        &self,
        request: Request<pb::ForkSessionRequest>,
    ) -> Result<Response<pb::ForkPreview>, Status> {
        let caller = identity::caller(&request)?;
        self.refuse_desk(request.get_ref().parent.as_ref(), "Fork").await?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.preview_fork_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.ForkPreview",
            first,
            move || async move { me.preview_fork_of(&who, request, true).await },
        )
        .await
    }

    async fn fork_session(
        &self,
        request: Request<pb::ForkSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        self.refuse_desk(request.get_ref().parent.as_ref(), "Fork").await?;
        let bound = guard::hold_bound(&caller, request.metadata());
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self
            .guard
            .held(bound, || self.fork_session_of(&caller, request.clone(), false))
            .await;
        // TP-6, IN-10: a fork waiting on Ask me shows what it would start.
        let card = match &first {
            Ok(Gated::Ask(_)) => self.fork_card(&request).await,
            _ => None,
        };
        let (me, who) = (self.clone(), caller.clone());
        answer_gated_showing(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            card,
            move || async move { me.fork_session_of(&who, request, true).await },
        )
        .await
    }

    async fn continue_in_channel(
        &self,
        request: Request<pb::ContinueInChannelRequest>,
    ) -> Result<Response<pb::StartSessionResponse>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        self.refuse_desk(request.get_ref().session.as_ref(), "Continue in another channel")
            .await?;
        let bound = guard::hold_bound(&caller, request.metadata());
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self
            .guard
            .held(bound, || self.continue_in_channel_of(&caller, request.clone(), false))
            .await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.StartSessionResponse",
            first,
            move || async move { me.continue_in_channel_of(&who, request, true).await },
        )
        .await
    }

    async fn give_own_copy(
        &self,
        request: Request<pb::GiveOwnCopyRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        self.refuse_desk(request.get_ref().fork.as_ref(), "Give it its own copy").await?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.give_own_copy_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.give_own_copy_of(&who, request, true).await },
        )
        .await
    }

    async fn move_session(
        &self,
        request: Request<pb::MoveSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        // MV-17: a Desk session, a channel's own too, moves like any other.
        let bound = guard::hold_bound(&caller, request.metadata());
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        // INBOXHOOKS: an agent says why it moves a session, for the card.
        if matches!(caller, Caller::Agent(_)) && request.reason.trim().is_empty() {
            return Err(status::invalid_field(
                "reason",
                Problem::Missing,
                "the move has no reason, so the person who decides it cannot tell why it is wanted",
                "say why in reason and send the move again",
            ));
        }
        let first = self
            .guard
            .held(bound, || self.move_session_of(&caller, request.clone(), false))
            .await;
        // The card is worked out only when the rule says Ask.
        let card = match &first {
            Ok(Gated::Ask(_)) => self.move_card(&request).await,
            _ => None,
        };
        let (me, who) = (self.clone(), caller.clone());
        answer_gated_showing(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            card,
            move || async move { me.move_session_of(&who, request, true).await },
        )
        .await
    }

    async fn plan_archive(
        &self,
        request: Request<pb::PlanArchiveRequest>,
    ) -> Result<Response<pb::ArchivePlan>, Status> {
        let caller = identity::caller(&request)?;
        self.refuse_desk(request.get_ref().id.as_ref(), "Archive").await?;
        let request = request.into_inner();
        let first = self.plan_archive_of(&caller, request.clone(), false).await;
        answer_plan(first, || self.plan_archive_of(&caller, request, true)).await
    }

    async fn archive_session(
        &self,
        request: Request<pb::ArchiveSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        self.refuse_desk(request.get_ref().id.as_ref(), "Archive").await?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.archive_session_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.archive_session_of(&who, request, true).await },
        )
        .await
    }

    async fn restore_session(
        &self,
        request: Request<pb::RestoreSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        self.refuse_desk(request.get_ref().id.as_ref(), "Restore").await?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.restore_session_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.restore_session_of(&who, request, true).await },
        )
        .await
    }

    async fn plan_delete(
        &self,
        request: Request<pb::PlanDeleteRequest>,
    ) -> Result<Response<pb::DeletePlan>, Status> {
        let caller = identity::caller(&request)?;
        let id = request.into_inner().id;
        let key = id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let first = self
            .delete_of(&caller, id.clone(), false, || self.plan_one(&caller, &key))
            .await;
        answer_plan(first, || {
            self.delete_of(&caller, id, true, || self.plan_one(&caller, &key))
        })
        .await
    }

    async fn delete_session(
        &self,
        request: Request<pb::DeleteSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let id = request.id;
        let (plan, idempotency) = (request.plan, request.idempotency_key);
        let named = id.as_ref().map(|id| id.value.clone()).unwrap_or_default();
        let first = {
            let (me, who, named, plan, idempotency) =
                (self.clone(), caller.clone(), named.clone(), plan.clone(), idempotency.clone());
            self.delete_of(&caller, id.clone(), false, || async move {
                me.delete_one(&who, &named, plan, idempotency).await
            })
            .await
        };
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.DeleteSessionResponse",
            first,
            move || async move {
                let (again, as_who) = (me.clone(), who.clone());
                me.delete_of(&who, id, true, || async move {
                    again.delete_one(&as_who, &named, plan, idempotency).await
                })
                .await
            },
        )
        .await
    }

    async fn plan_delete_sessions(
        &self,
        request: Request<pb::PlanDeleteSessionsRequest>,
    ) -> Result<Response<pb::DeleteSessionsPlan>, Status> {
        let caller = identity::caller(&request)?;
        let scope = request.into_inner().scope;
        let first = self.plan_delete_sessions_of(&caller, scope.clone(), false).await;
        answer_plan(first, || self.plan_delete_sessions_of(&caller, scope, true)).await
    }

    async fn delete_sessions(
        &self,
        request: Request<pb::DeleteSessionsRequest>,
    ) -> Result<Response<pb::DeleteSessionsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.delete_sessions_of(&caller, request.clone(), false, false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.DeleteSessionsResponse",
            first,
            move || async move { me.delete_sessions_of(&who, request, true, false).await },
        )
        .await
    }

    /// A session lost with its machine, rebuilt on another (SL-4).
    async fn rebuild_session(
        &self,
        request: Request<pb::RebuildSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.rebuild_session_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.rebuild_session_of(&who, request, true).await },
        )
        .await
    }

    /// CH-21: a repo cloned into a running session (REPOS).
    async fn add_repo_to_session(
        &self,
        request: Request<pb::AddRepoToSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self
            .add_repo_to_session_of(&caller, request.clone(), false)
            .await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move { me.add_repo_to_session_of(&who, request, true).await },
        )
        .await
    }

    async fn plan_drop_repo(
        &self,
        _request: Request<pb::PlanDropRepoRequest>,
    ) -> Result<Response<pb::DropRepoPlan>, Status> {
        Err(status::not_built("humata.v1.SessionService.PlanDropRepo"))
    }

    async fn drop_repo(
        &self,
        _request: Request<pb::DropRepoRequest>,
    ) -> Result<Response<pb::Session>, Status> {
        Err(status::not_built("humata.v1.SessionService.DropRepo"))
    }

    async fn plan_push(
        &self,
        request: Request<pb::PlanPushRequest>,
    ) -> Result<Response<pb::PushPlan>, Status> {
        let caller = identity::caller(&request)?;
        self.refuse_desk(request.get_ref().id.as_ref(), "Push").await?;
        let bound = guard::hold_bound(&caller, request.metadata());
        let request = request.into_inner();
        let first = self
            .guard
            .held(bound, || self.plan_push_of(&caller, request.clone(), false))
            .await;
        answer_plan(first, || self.plan_push_of(&caller, request, true)).await
    }

    async fn push_and_open_pull_requests(
        &self,
        request: Request<pb::PushAndOpenPullRequestsRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        self.refuse_desk(request.get_ref().id.as_ref(), "Push").await?;
        let bound = guard::hold_bound(&caller, request.metadata());
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self
            .guard
            .held(bound, || {
                self.push_and_open_pull_requests_of(&caller, request.clone(), false)
            })
            .await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
            move || async move {
                me.push_and_open_pull_requests_of(&who, request, true)
                    .await
            },
        )
        .await
    }

    /// A session's conversation on this machine, paged (SE-3, LD-8). It is
    /// a read of the session, so the rule for `ReadSession` decides.
    async fn read_conversation(
        &self,
        request: Request<pb::ReadConversationRequest>,
    ) -> Result<Response<pb::ReadConversationResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let first = self.read_conversation_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            None,
            "humata.v1.ReadConversationResponse",
            first,
            move || async move { me.read_conversation_of(&who, request, true).await },
        )
        .await
    }

    /// TP-5: a note typed into another session once its turn is over.
    async fn send_note(
        &self,
        request: Request<pb::SendNoteRequest>,
    ) -> Result<Response<pb::SendNoteResponse>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let request = request.into_inner();
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let first = self.send_note_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.SendNoteResponse",
            first,
            move || async move { me.send_note_of(&who, request, true).await },
        )
        .await
    }

    /// TP-5: a question typed into another session; the call returns its
    /// answer.
    async fn ask_session(
        &self,
        request: Request<pb::AskSessionRequest>,
    ) -> Result<Response<pb::AskSessionResponse>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let request = request.into_inner();
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let first = self.ask_session_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.AskSessionResponse",
            first,
            move || async move { me.ask_session_of(&who, request, true).await },
        )
        .await
    }

    async fn answer_question(
        &self,
        request: Request<pb::AnswerQuestionRequest>,
    ) -> Result<Response<pb::AnswerQuestionResponse>, Status> {
        let caller = identity::caller(&request)?;
        Self::answer_question_of(&caller, request.get_ref()).map(Response::new)
    }

    /// The session's own agent says what turn it is in (SE-21, D169).
    async fn report_turn(
        &self,
        request: Request<pb::ReportTurnRequest>,
    ) -> Result<Response<pb::ReportTurnResponse>, Status> {
        self.report_turn_of(request).await
    }

    type HookStream = hooks::HookStream;

    /// The session's own agent's permission hook holds this while the
    /// Inbox card is open (HOOKS, D191).
    async fn hook(
        &self,
        request: Request<pb::HookRequest>,
    ) -> Result<Response<Self::HookStream>, Status> {
        self.hook_of(request).await
    }
}

impl SessionSvc {
    /// INBOXHOOKS: what the Agent inbox card of a move shows, in names: the
    /// session's title and channel, its repositories, the terminals that
    /// move along, and the forks that stay. What can't be read cheaply is
    /// left empty (the size, the uncommitted files, the clones needed).
    async fn move_card(&self, request: &pb::MoveSessionRequest) -> Option<crate::inbox::BoardAsk> {
        let id = request.id.as_ref()?.value.clone();
        let to = request.to.as_ref()?.clone();
        let loaded = self.load(&id).await.ok()?;
        let from = if loaded.record.machine.is_empty() {
            self.machine.clone()
        } else {
            loaded.record.machine.clone()
        };
        let channel_name = if loaded.record.channel == humata2_sessions::DESK_CHANNEL {
            "Desk".to_owned()
        } else {
            self.find_channel(&loaded.record.channel)
                .await
                .map(|channel| channel.name)
                .unwrap_or_default()
        };
        let agents = self.starter.agents.clone();
        let (store, session, here) = (self.starter.store.clone(), id.clone(), from.clone());
        let (terminals, forks_staying) = blocking(move || -> Result<_, StoreError> {
            let mut terminals = Vec::new();
            for tab in store.tabs()?.into_iter().filter(|tab| tab.session.as_deref() == Some(session.as_str())) {
                let words = match tab.kind {
                    humata2_store::TabKind::Agent => store
                        .session(&session)?
                        .map(|record| agents.person_name(&record.agent))
                        .unwrap_or_default(),
                    humata2_store::TabKind::Shell => tab.title.clone(),
                };
                terminals.push(if words.is_empty() { tab.title } else { words });
            }
            let mut forks = Vec::new();
            for origin in store.fork_origins()?.into_iter().filter(|origin| origin.parent == session) {
                if let Some(fork) = store.session(&origin.session_id)?
                    && (fork.machine == here || fork.machine.is_empty())
                {
                    forks.push(fork.title);
                }
            }
            Ok((terminals, forks))
        })
        .await
        .ok()?
        .ok()?;
        let repos = loaded
            .detail
            .as_ref()
            .map(|detail| detail.repos.iter().map(|repo| repo.name.clone()).collect())
            .unwrap_or_default();
        Some(crate::inbox::BoardAsk {
            preview: pb::request_preview::Preview::Move(pb::MovePreview {
                session: Some(pb::SessionId { value: id }),
                from: Some(pb::MachineId { value: from }),
                to: Some(to),
                bytes: 0,
                session_title: loaded.record.title.clone(),
                channel_name,
                repos,
                uncommitted_files: 0,
                terminals,
                forks_staying,
                clones_needed: Vec::new(),
            }),
            reason: request.reason.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use humata2_store::ChannelRepo;

    use super::*;

    #[test]
    fn a_start_refused_for_containment_is_a_failed_precondition() {
        let refused = StartError::Revive {
            source: humata2_revive::ReviveError::Contain {
                session: "s1".to_owned(),
                source: humata2_revive::ContainRefused::NoSession {
                    session: "s1".to_owned(),
                },
            },
        };
        let (code, error) = start_error(&refused);
        assert_eq!(code, Code::FailedPrecondition);
        assert!(!error.message.is_empty());
        assert!(!error.fix.is_empty());
    }

    fn running() -> AgentState {
        AgentState::Running { holder_pid: 7 }
    }

    #[test]
    fn status_follows_the_agent_and_the_start() {
        use pb::SessionStatus as S;
        assert_eq!(
            session_status(Some(&running()), Some(StartStep::Done)),
            S::Working
        );
        assert_eq!(session_status(Some(&running()), None), S::Working);
        assert_eq!(
            session_status(Some(&AgentState::Launching), None),
            S::Starting
        );
        assert_eq!(
            session_status(Some(&AgentState::Resuming), None),
            S::Starting
        );
        assert_eq!(
            session_status(Some(&AgentState::Exited { code: 2 }), None),
            S::Exited
        );
        assert_eq!(session_status(Some(&AgentState::Stopped), None), S::Stopped);
        assert_eq!(session_status(None, None), S::Idle);
        let failed = AgentState::ResumeFailed {
            reason: ResumeFailure::AgentExited,
            exit_code: Some(1),
        };
        assert_eq!(session_status(Some(&failed), None), S::ResumeFailed);
        // A start that hasn't finished is starting, whatever the agent says.
        assert_eq!(
            session_status(Some(&running()), Some(StartStep::Cloned)),
            S::Starting
        );
        assert_eq!(session_status(None, Some(StartStep::Recorded)), S::Starting);
    }

    /// D784 (Fable B2): Stop is offered only when the conversation is
    /// known; a shell of any kind, or an agent with its conversation, stops.
    #[test]
    fn stop_is_refused_only_for_an_agent_whose_conversation_is_unknown() {
        let agents = Agents::builtin().with_shell(std::path::Path::new("/bin/bash"), &[]);
        let launch = |agent: &str, conversation: &str| humata2_store::AgentLaunch {
            agent: agent.to_owned(),
            conversation_id: conversation.to_owned(),
            ..humata2_store::AgentLaunch::default()
        };
        assert!(conversation_unknown(&launch("copilot", ""), &agents));
        assert!(!conversation_unknown(&launch("copilot", "c_1"), &agents));
        assert!(!conversation_unknown(&launch("zsh", ""), &agents));
    }

    #[test]
    fn exit_code_comes_from_exit_or_failed_resume() {
        assert_eq!(exit_code(Some(&AgentState::Exited { code: 3 })), 3);
        let failed = AgentState::ResumeFailed {
            reason: ResumeFailure::AgentExited,
            exit_code: Some(9),
        };
        assert_eq!(exit_code(Some(&failed)), 9);
        assert_eq!(exit_code(Some(&running())), 0);
        assert_eq!(exit_code(None), 0);
    }

    fn channel(repos: &[(&str, &str, MainClone)]) -> ChannelRecord {
        ChannelRecord {
            id: "ch_1".to_owned(),
            name: "Payments".to_owned(),
            folder: "payments".to_owned(),
            branch_prefix: None,
            default_agent: "claude".to_owned(),
            repos: repos
                .iter()
                .map(|(owner, name, main)| ChannelRepo {
                    owner: (*owner).to_owned(),
                    name: (*name).to_owned(),
                    main: main.clone(),
                })
                .collect(),
            machines: vec!["m1".to_owned()],
            revision: 1,
            created_unix_ms: 0,
        }
    }

    fn choice(owner: &str, name: &str, include: bool) -> pb::RepoChoice {
        pb::RepoChoice {
            repo: Some(pb::RepoRef {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            include,
        }
    }

    fn pair(owner: &str, name: &str) -> (String, String) {
        (owner.to_owned(), name.to_owned())
    }

    #[test]
    fn no_choices_takes_every_ready_repo() {
        let channel = channel(&[
            ("a", "api", MainClone::Ready),
            ("a", "web", MainClone::Cloning),
            ("a", "db", MainClone::Ready),
        ]);
        assert_eq!(
            choose_repos(&channel, &[]).unwrap(),
            [pair("a", "api"), pair("a", "db")]
        );
    }

    #[test]
    fn leaving_a_repo_out_drops_it_and_naming_some_takes_only_those() {
        let channel = channel(&[
            ("a", "api", MainClone::Ready),
            ("a", "db", MainClone::Ready),
        ]);
        assert_eq!(
            choose_repos(&channel, &[choice("a", "api", false)]).unwrap(),
            [pair("a", "db")]
        );
        assert_eq!(
            choose_repos(&channel, &[choice("a", "db", true)]).unwrap(),
            [pair("a", "db")]
        );
    }

    #[test]
    fn a_start_with_nothing_ready_or_an_unknown_repo_is_refused() {
        let channel = channel(&[("a", "api", MainClone::Cloning)]);
        assert!(choose_repos(&channel, &[]).is_err());
        assert!(choose_repos(&channel, &[choice("a", "api", true)]).is_err());
        assert!(choose_repos(&channel, &[choice("a", "nope", true)]).is_err());
    }

    #[test]
    fn a_channel_with_no_repos_starts_with_none() {
        let channel = channel(&[]);
        assert_eq!(choose_repos(&channel, &[]).unwrap(), Vec::<(String, String)>::new());
    }

    #[test]
    fn a_title_is_trimmed_and_bounded() {
        assert_eq!(clean_title("  fix flaky test ").unwrap(), "fix flaky test");
        assert_eq!(clean_title("").unwrap(), "");
        assert!(clean_title(&"x".repeat(TITLE_MAX + 1)).is_err());
    }

    /// D395: read marks that outran the listener make every subscriber
    /// reload, rather than leaving clients with stale marks.
    #[tokio::test]
    async fn lost_read_marks_make_every_subscriber_resync() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("a temp folder: {e}"));
        let store = humata2_store::Store::open(dir.path())
            .unwrap_or_else(|e| panic!("a store: {e}"));
        let feed = Arc::new(Feed::default());
        let (_, mut live) = feed.subscribe(None);
        // Capacity 1 with three sends: the listener lags before it runs.
        let (changes_tx, changes) = broadcast::channel(1);
        for session in ["s1", "s2", "s3"] {
            let _ = changes_tx.send(session.to_owned());
        }
        let tell = Tell::new(feed, Arc::new(store), "this-mac".to_owned());
        tokio::spawn(tell_read_marks(tell, changes));
        let resynced = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let event = live
                    .recv()
                    .await
                    .unwrap_or_else(|error| panic!("the feed closed: {error}"));
                if matches!(event.payload, Some(pb::event::Payload::Resync(_))) {
                    return;
                }
            }
        })
        .await;
        assert!(resynced.is_ok(), "no subscriber was told to resync");
    }

    #[test]
    fn stamp_status_since_tracks_the_final_merged_status() {
        let mut shown = pb::Session {
            id: Some(pb::SessionId {
                value: "se_stamp_merged_test".to_owned(),
            }),
            status: pb::SessionStatus::Working as i32,
            ..pb::Session::default()
        };
        let ms = |shown: &pb::Session| {
            shown
                .status_since
                .as_ref()
                .map(|at| at.seconds * 1_000 + i64::from(at.nanos) / 1_000_000)
        };
        stamp_status_since_at(&mut shown, 10_000);
        assert_eq!(ms(&shown), Some(10_000));
        // Repeating the same status keeps its start time.
        stamp_status_since_at(&mut shown, 25_000);
        assert_eq!(ms(&shown), Some(10_000));
        // A status transition (including after a remote merge) moves it.
        shown.status = pb::SessionStatus::Idle as i32;
        stamp_status_since_at(&mut shown, 40_000);
        assert_eq!(ms(&shown), Some(40_000));
        shown.status = pb::SessionStatus::Working as i32;
        stamp_status_since_at(&mut shown, 55_000);
        assert_eq!(ms(&shown), Some(55_000));
    }

    #[test]
    fn resuming_now_guard_tracks_active_resume_and_releases_on_drop() {
        // SMB-046: ResumingNow marks a session active only while a live
        // resume or start-fresh is running, so a stale Resuming row left by a
        // failed rollback is no longer treated as busy.
        let id = "se_smb046_guard_test";
        assert!(!resuming_now(id));
        {
            let _guard = ResumingNow::enter(id);
            assert!(resuming_now(id));
        }
        assert!(!resuming_now(id));
    }
}
