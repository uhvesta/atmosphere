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
use humata2_revive::{Revived, resume_agent};
use humata2_sessions::{Agents, StartError, StartRequest, Starter, branch_slug};
use humata2_store::{
    AgentRecord, AgentState, ChannelRecord, MainClone, ResumeFailure, SessionDetail, SessionRecord,
    StartStep, StoreError, TabRecord,
};
use pb::invalid_field::Problem;
use tokio::sync::oneshot;
use tonic::{Code, Request, Response, Status};

use crate::desk_service;
use crate::events::{Feed, Scope};
use crate::channel_service::{Gated, answer_gated, answer_gated_showing, blocking, error_of, failed, gate, timestamp_ms};
use crate::guard::{Checked, Guard};
use crate::identity::{self, Caller, Identities};
use crate::inbox::Inbox;
use crate::operations::{Operations, Progress, Started};
use crate::remote_work::RemoteWork;
use crate::restart::Tokens;
use crate::status;

mod archive;
mod archive_remote;
mod forget;
mod fork;
mod fork_card;
mod fork_remote;
mod hooks;
mod lineage;
mod move_recover;
mod move_session;
mod own_copy;
mod pointers;
mod push;
mod rebuild;
mod rename_branch;
mod remote;
mod start_fresh;
mod turns;
mod turns_list;
mod watch;

pub(crate) use archive_remote::Removal;

pub(crate) use remote::TellSink;
pub(crate) use turns::{
    HooksError, changes as turn_changes, install_hooks, running as running_turn, set_sources,
};
pub(crate) use hooks::install_agent_hooks;
pub(crate) use pointers_start::after_restart;
pub(crate) use move_session::{is_moving, moving_changes};

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
const COLS: u16 = 80;
const ROWS: u16 = 24;

#[derive(Clone)]
pub(crate) struct SessionSvc {
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
    /// The host's mirror's scrubbed lines, for rebuilding a lost session (SL-4).
    raw_lines: rebuild::Lines,
}

/// Puts a session as the store holds it now on the event feed.
#[derive(Clone)]
pub(crate) struct Tell {
    feed: Arc<Feed>,
    store: Arc<humata2_store::Store>,
    machine: String,
    /// Set once the host's remote work exists: what a remote session shows is
    /// merged with what its machine reported (SL-3).
    remote: Arc<std::sync::OnceLock<RemoteWork>>,
    /// The last pointers of this machine's sessions (SL-3).
    pointers: Arc<pointers::Cache>,
}

impl Tell {
    pub(crate) fn new(feed: Arc<Feed>, store: Arc<humata2_store::Store>, machine: String) -> Self {
        Self {
            feed,
            store,
            machine,
            remote: Arc::default(),
            pointers: pointers::shared(),
        }
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

    /// Reads `session` back and publishes it as GetSession would show it.
    /// A session that can't be read is not told; publishing never fails
    /// the call that changed it.
    pub(crate) async fn session(&self, session: &str, change: pb::Change, by: Option<pb::Actor>) {
        let (store, key) = (self.store.clone(), session.to_owned());
        let read = blocking(move || Loaded::read(&store, &key))
        .await;
        let Ok(Ok(Some(loaded))) = read else {
            return;
        };
        let mut shown = session_pb(
            &loaded.record,
            loaded.detail.as_ref(),
            loaded.agent.as_ref(),
            loaded.summary_source,
            &self.machine,
        );
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
            agent,
            record,
        }))
    }
}

impl SessionSvc {
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
    ) -> Self {
        let tokens = Arc::new(Tokens::new(starter.store.clone(), identities, socket));
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
            raw_lines: rebuild::Lines::default(),
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
            },
            approved,
        )? {
            return Ok(Gated::Ask(asked));
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
        // A start naming a session this machine already has (the host asking again,
        // D150) answers with it and starts nothing.
        if let Some(id) = &assigned
            && let Some(known) = self.known_start(id).await?
        {
            return Ok(Gated::Done(known));
        }
        let title = clean_title(&request.title)?;
        let repos = choose_repos(&channel, &request.repos)?;
        turns::refresh_hooks().await;

        let mut keys = vec!["write_record".to_owned()];
        let mut steps = vec![("write_record".to_owned(), "Saving the session".to_owned())];
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
        tokio::spawn(run_start(
            self.starter.clone(),
            self.tell.clone(),
            self.tokens.clone(),
            start,
            progress,
            keys,
            ready,
            identity::actor(caller),
        ));
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
        let loaded = self.load(&id).await?;
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
        match agent.state {
            AgentState::Launching => return Err(busy(&id, "still starting")),
            AgentState::Resuming => return Err(busy(&id, "being resumed")),
            AgentState::Exited { .. } | AgentState::Stopped | AgentState::ResumeFailed { .. } => {
                return Ok(Gated::Done(self.show(&loaded)));
            }
            AgentState::Running { .. } => {}
        }
        // A holder already gone is as stopped as one that was asked to.
        if let Err(error) = humata2_holder::shutdown(&agent.launch.holder_socket).await {
            crate::log::say!(
                "humatad2: stopping session {id}'s terminal: {}",
                status::chain(&error)
            );
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
        desk_service::refuse_if_closed(&self.starter.store, &self.machine, &id).await?;
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
            tokio::spawn(run_resume(
                self.starter.clone(),
                self.tell.clone(),
                identity::actor(caller),
                self.tokens.clone(),
                loaded,
                progress,
            ));
        }
        Ok(Gated::Done(operation))
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
        self.merge_remote(&loaded.record, &mut shown);
        if !remote::elsewhere(&loaded.record, &self.machine) {
            self.tell.pointers.fill(&loaded.record.id, &mut shown);
        }
        self.apply_lineage(&mut shown);
        move_session::show_moving(&mut shown);
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

    /// The agent a start would run: the one asked for, else the channel's.
    fn agent_for(
        &self,
        wanted: Option<&pb::AgentId>,
        channel: &ChannelRecord,
    ) -> Result<String, Status> {
        let name = wanted
            .map(|agent| agent.value.clone())
            .filter(|agent| !agent.is_empty())
            .unwrap_or_else(|| channel.default_agent.clone());
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
                running: None,
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
        Some(AgentState::Exited { .. }) => pb::SessionStatus::Exited,
        Some(AgentState::ResumeFailed { .. }) => pb::SessionStatus::ResumeFailed,
        Some(AgentState::Stopped) | None => pb::SessionStatus::Idle,
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
    match (session_status(agent, start), agent) {
        (pb::SessionStatus::Working, Some(AgentState::Running { holder_pid })) => {
            // An agent with no turns shows none (SE-21).
            if turns::source_of(kind) == pb::TurnEndSource::None {
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

fn session_pb(
    record: &SessionRecord,
    detail: Option<&SessionDetail>,
    agent: Option<&AgentRecord>,
    summary_source: pb::session::SummarySource,
    here: &str,
) -> pb::Session {
    let state = agent.map(|agent| &agent.state);
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
        status: shown_status(&record.id, &record.agent, state, detail.map(|detail| detail.start)) as i32,
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
        ..pb::Session::default()
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
        if chosen.is_empty() {
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
        StartError::NoRepos { .. } => (
            Code::FailedPrecondition,
            error_of(
                error,
                "add a repository to the channel first",
                field("repos", Problem::Missing),
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
        StartError::Store { .. } => (
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
        StartError::NoConversation { .. } | StartError::Copy { .. } => (
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
        starter.start(request, &env, &mut note).await
    };
    match started {
        Ok(started) => {
            progress.set_step("verify", pb::StepState::Done, None);
            let session = started.session.id;
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: session.clone(),
            })));
            tell.session(&session, pb::Change::Updated, Some(by)).await;
            watch::agent(tell, session);
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

/// The reason a failed resume gives, as the API names it.
fn resume_reason(reason: ResumeFailure) -> pb::resume_failed::Reason {
    use pb::resume_failed::Reason;
    match reason {
        ResumeFailure::ConversationFileMissing => Reason::ConversationFileMissing,
        ResumeFailure::AgentStartedAnotherConversation => Reason::AgentStartedAnotherConversation,
        ResumeFailure::AgentExited => Reason::AgentExited,
        ResumeFailure::AgentNotInstalled => Reason::AgentNotInstalled,
        ResumeFailure::FolderMissing => Reason::FolderMissing,
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
    let Loaded { record, agent, .. } = loaded;
    let session = record.id.clone();
    progress.set_step("launch", pb::StepState::Running, None);
    let Some(mut agent) = agent else {
        progress.fail(error_of(&NoAgent { session }, "start a new session", None));
        return;
    };
    agent.state = AgentState::Resuming;
    agent.updated_unix_ms = now_ms();
    let (store, saving) = (starter.store.clone(), agent.clone());
    let saved = tokio::task::spawn_blocking(move || store.put_agent(&saving)).await;
    match saved {
        Ok(Ok(())) => {}
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
    let env = |tab: &TabRecord| tokens.env(tab);
    let outcome = resume_agent(&starter.store, &starter.revive, agent, &env).await;
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

#[derive(Debug, thiserror::Error)]
#[error("session {session} has no agent to resume")]
struct NoAgent {
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
        let assigned = identity::host_assigned_id(&request, &caller)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self
            .begin_session(&caller, request.clone(), assigned.clone(), false)
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
        let (sessions, details, agents, sources, lineage) = blocking(move || {
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
                self.merge_remote(session, &mut shown);
                lineage.apply(&mut shown);
                move_session::show_moving(&mut shown);
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

    /// Starts the agent again with its pinned conversation (SE-15, PER-13):
    /// an agent that is running is left as it is. Starting fresh with a
    /// summary isn't built.
    async fn resume_session(
        &self,
        request: Request<pb::ResumeSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
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
        self.refuse_desk(request.get_ref().parent.as_ref(), "Fork").await?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.fork_session_of(&caller, request.clone(), false).await;
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
        self.refuse_desk(request.get_ref().id.as_ref(), "Move").await?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.move_session_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.Operation",
            first,
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
        let key = None;
        let first = self.plan_archive_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            key,
            "humata.v1.ArchivePlan",
            first,
            move || async move { me.plan_archive_of(&who, request, true).await },
        )
        .await
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
        _request: Request<pb::PlanDeleteRequest>,
    ) -> Result<Response<pb::DeletePlan>, Status> {
        Err(status::not_built("humata.v1.SessionService.PlanDelete"))
    }

    async fn delete_session(
        &self,
        _request: Request<pb::DeleteSessionRequest>,
    ) -> Result<Response<pb::DeleteSessionResponse>, Status> {
        Err(status::not_built("humata.v1.SessionService.DeleteSession"))
    }

    /// A session lost with its machine, rebuilt on another (SL-4).
    async fn rebuild_session(
        &self,
        request: Request<pb::RebuildSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
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

    async fn add_repo_to_session(
        &self,
        _request: Request<pb::AddRepoToSessionRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        Err(status::not_built(
            "humata.v1.SessionService.AddRepoToSession",
        ))
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
        let request = request.into_inner();
        let first = self.plan_push_of(&caller, request.clone(), false).await;
        let (me, who) = (self.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            None,
            "humata.v1.PushPlan",
            first,
            move || async move { me.plan_push_of(&who, request, true).await },
        )
        .await
    }

    async fn push_and_open_pull_requests(
        &self,
        request: Request<pb::PushAndOpenPullRequestsRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = identity::caller(&request)?;
        self.refuse_desk(request.get_ref().id.as_ref(), "Push").await?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self
            .push_and_open_pull_requests_of(&caller, request.clone(), false)
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

    async fn read_conversation(
        &self,
        _request: Request<pb::ReadConversationRequest>,
    ) -> Result<Response<pb::ReadConversationResponse>, Status> {
        Err(status::not_built(
            "humata.v1.SessionService.ReadConversation",
        ))
    }

    async fn send_note(
        &self,
        _request: Request<pb::SendNoteRequest>,
    ) -> Result<Response<pb::SendNoteResponse>, Status> {
        Err(status::not_built("humata.v1.SessionService.SendNote"))
    }

    async fn ask_session(
        &self,
        _request: Request<pb::AskSessionRequest>,
    ) -> Result<Response<pb::AskSessionResponse>, Status> {
        Err(status::not_built("humata.v1.SessionService.AskSession"))
    }

    async fn answer_question(
        &self,
        _request: Request<pb::AnswerQuestionRequest>,
    ) -> Result<Response<pb::AnswerQuestionResponse>, Status> {
        Err(status::not_built("humata.v1.SessionService.AnswerQuestion"))
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
        assert_eq!(session_status(Some(&AgentState::Stopped), None), S::Idle);
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
    fn a_title_is_trimmed_and_bounded() {
        assert_eq!(clean_title("  fix flaky test ").unwrap(), "fix flaky test");
        assert_eq!(clean_title("").unwrap(), "");
        assert!(clean_title(&"x".repeat(TITLE_MAX + 1)).is_err());
    }
}
