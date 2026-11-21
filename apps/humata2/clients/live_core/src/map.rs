//! The daemon's messages as the app's model, and back. Pure: no calls.

use std::collections::BTreeMap;

use humata2_clients::comments::{
    Anchor, Author, Comment, CommentId, Destination, Side, Status as CommentStatus, ThreadReply,
};
use humata2_clients::machines::OperationId;
use humata2_clients::model::{
    Agent, AwayLeftOut, Channel, ChannelId, ForkKind, Machine, MachineId, MachineKind, MachineStop,
    MoveProgress, MoveStep, MoveStepState, NotBroughtBack, Recovery, Restart, ResumeFailure,
    Session, SessionClone, SessionId, Status, StoppedBy, SummarySource, Turn,
};
use humata2_clients::progress::{OpKind, OpProgress, OpUpdate};
use crate::humata2_grpc::humata::v1 as pb;

use crate::local_time::LocalNow;

/// Which daemon agent name each app agent runs under. By default the
/// agent's command name ("claude", "zsh", ...).
#[derive(Clone, Debug, Default)]
pub struct AgentNames {
    overrides: BTreeMap<String, Agent>,
    names: BTreeMap<String, String>,
}

impl AgentNames {
    pub fn set(&mut self, agent: Agent, name: String) {
        self.overrides.insert(name.clone(), agent);
        self.names.insert(agent.command().to_owned(), name);
    }

    /// The daemon's name for `agent`.
    #[must_use]
    pub fn name(&self, agent: Agent) -> String {
        self.names
            .get(agent.command())
            .cloned()
            .unwrap_or_else(|| agent.command().to_owned())
    }

    /// The app's agent for a daemon name; a name nobody knows is a shell.
    #[must_use]
    pub fn agent(&self, name: &str) -> Agent {
        if let Some(agent) = self.overrides.get(name) {
            return *agent;
        }
        Agent::ALL
            .into_iter()
            .find(|agent| agent.command() == name)
            .unwrap_or(Agent::Shell)
    }
}

/// `owner/name` as a repository reference; `None` unless it has exactly
/// one `/` with text on both sides.
#[must_use]
pub fn parse_repo(text: &str) -> Option<pb::RepoRef> {
    let (owner, name) = text.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(pb::RepoRef {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// A repository reference written `owner/name`, or just `name` when the
/// owner is empty.
#[must_use]
pub fn repo_text(repo: &pb::RepoRef) -> String {
    if repo.owner.is_empty() {
        repo.name.clone()
    } else {
        format!("{}/{}", repo.owner, repo.name)
    }
}

#[must_use]
pub fn machine_id(id: Option<&pb::MachineId>) -> MachineId {
    MachineId::new(id.map(|id| id.value.clone()).unwrap_or_default())
}

#[must_use]
pub fn channel_id(id: Option<&pb::ChannelId>) -> ChannelId {
    ChannelId::new(id.map(|id| id.value.clone()).unwrap_or_default())
}

#[must_use]
pub fn session_id(id: Option<&pb::SessionId>) -> SessionId {
    SessionId::new(id.map(|id| id.value.clone()).unwrap_or_default())
}

#[must_use]
pub fn channel(channel: &pb::Channel, agents: &AgentNames) -> Channel {
    Channel {
        id: ChannelId::new(
            channel
                .id
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default(),
        ),
        name: channel.name.clone(),
        repos: channel.repos.iter().map(repo_text).collect(),
        machines: channel
            .machines
            .iter()
            .map(|machine| MachineId::new(machine.value.clone()))
            .collect(),
        purpose: String::new(),
        default_agent: channel
            .default_agent
            .as_ref()
            .filter(|agent| !agent.value.is_empty())
            .map(|agent| agents.agent(&agent.value)),
    }
}

/// What the app calls the machine it runs on.
pub const THIS_MAC: &str = "This Mac";

/// This Mac, as the app shows it; its errors name it so from now on.
#[must_use]
pub fn this_machine(id: MachineId) -> Machine {
    humata2_clients::machine_names::remember(&id, THIS_MAC);
    Machine {
        id,
        name: THIS_MAC.to_owned(),
        kind: MachineKind::ThisMac,
        online: true,
        detail: String::new(),
        last_seen: String::new(),
        lost_at: None,
        stop: None,
        root: humata2_clients::desk::DEFAULT_ROOT.to_owned(),
    }
}

/// A stop of a machine's sessions as the host keeps it (D261, D262);
/// `delivered` is whether the machine took it. A stop whose maker the
/// daemon doesn't name was the person's: only users stop without a rule
/// saying who (D263). Its time reads in this Mac's local time as of `now`
/// (D503).
#[must_use]
pub fn machine_stop(
    stopped: &pb::SessionsStopped,
    delivered: bool,
    now: &LocalNow,
) -> MachineStop {
    use pb::StoppedByKind as K;
    let by = match stopped.by.as_ref() {
        Some(maker) => match K::try_from(maker.kind).unwrap_or(K::Unspecified) {
            K::Terminal => StoppedBy::Terminal { machine: MachineId::new(maker.machine.clone()) },
            K::Agent => StoppedBy::Agent { session: SessionId::new(maker.session.clone()) },
            K::Script => StoppedBy::Script,
            K::User | K::Unspecified => StoppedBy::You,
        },
        None => StoppedBy::You,
    };
    MachineStop {
        stop_id: stopped.stop_id.clone(),
        at: now.clock(stopped.stopped_at_unix_ms),
        by,
        delivered,
    }
}

/// `machine` as the app shows it; a stop's time reads as of `now`.
#[must_use]
pub fn machine(machine: &pb::Machine, now: &LocalNow) -> Machine {
    let kind = if machine.is_this_mac {
        MachineKind::ThisMac
    } else if let Some(provider) = &machine.provider {
        MachineKind::Provider {
            provider: provider.value.clone(),
        }
    } else {
        MachineKind::Ssh {
            address: machine.address.clone(),
        }
    };
    let id = machine_id(machine.id.as_ref());
    humata2_clients::machine_names::remember(&id, if machine.is_this_mac { THIS_MAC } else { &machine.name });
    Machine {
        id,
        name: machine.name.clone(),
        kind,
        online: machine.reachability == pb::Reachability::Online as i32,
        detail: [machine.os.as_str(), machine.arch.as_str()]
            .iter()
            .filter(|part| !part.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
        last_seen: String::new(),
        lost_at: None,
        stop: machine
            .sessions_stopped
            .as_ref()
            .map(|stopped| machine_stop(stopped, machine.stop_delivered, now)),
        root: if machine.root.is_empty() {
            humata2_clients::desk::DEFAULT_ROOT.to_owned()
        } else {
            machine.root.clone()
        },
    }
}

/// The tabs in the app's order: the agent's first, then the shells as the
/// daemon lists them.
#[must_use]
pub fn order_tabs(tabs: Vec<pb::Tab>) -> Vec<pb::Tab> {
    let (agents, shells): (Vec<_>, Vec<_>) = tabs
        .into_iter()
        .partition(|tab| tab.kind == pb::tab::Kind::Agent as i32);
    agents.into_iter().take(1).chain(shells).collect()
}

/// A session's status and, when it needs one, the recovery note.
fn status(session: &pb::Session) -> (Status, Option<Recovery>) {
    use pb::SessionStatus as S;
    let exited = Status::Exited {
        code: session.exit_code,
    };
    match S::try_from(session.status).unwrap_or(S::Unspecified) {
        S::Starting | S::Working => (Status::Working, None),
        S::NeedsInput => (Status::NeedsYou, None),
        S::ProbablyIdle => (Status::ProbablyIdle, None),
        S::Exited => (exited, None),
        S::Offline => (Status::Offline, None),
        S::MachineStopped => (Status::Stopped, None),
        S::Stopped => (Status::AgentStopped, None),
        S::Moving => (Status::Moving, None),
        S::ResumeFailed => (
            exited,
            Some(Recovery::Recoverable {
                needs: "Humata couldn't resume this session's agent. Try Resume again".to_owned(),
            }),
        ),
        _ => (Status::Idle, None),
    }
}

/// A timestamp's seconds and nanoseconds as Unix milliseconds.
#[must_use]
pub fn unix_ms(seconds: i64, nanos: i32) -> i64 {
    seconds
        .saturating_mul(1000)
        .saturating_add(i64::from(nanos) / 1_000_000)
}

/// Why a resume failed; `None` when the daemon gave no reason it knows.
fn resume_failure(failed: &pb::ResumeFailed) -> Option<ResumeFailure> {
    use pb::resume_failed::Reason as R;
    match R::try_from(failed.reason).unwrap_or(R::Unspecified) {
        R::ConversationFileMissing => Some(ResumeFailure::ConversationMissing),
        R::AgentStartedAnotherConversation => Some(ResumeFailure::OtherConversation),
        R::AgentExited => Some(ResumeFailure::AgentExited(failed.exit_code)),
        R::AgentNotInstalled => Some(ResumeFailure::AgentNotInstalled),
        R::FolderMissing => Some(ResumeFailure::FolderMissing),
        R::ConversationUnknown => Some(ResumeFailure::ConversationUnknown),
        R::Unspecified => None,
    }
}

/// What Start fresh can summarize; `None` unless the daemon says, which it
/// does only while the resume has failed.
fn summary_source(session: &pb::Session) -> Option<SummarySource> {
    use pb::session::SummarySource as S;
    match S::try_from(session.summary_source).unwrap_or(S::Unspecified) {
        S::Transcript => Some(SummarySource::Transcript),
        S::Scrollback => Some(SummarySource::Scrollback),
        S::None => Some(SummarySource::Nothing),
        S::Unspecified => None,
    }
}

/// What a host found when it last started, from its server info. `None`
/// while the daemon is still bringing sessions back (no report yet).
#[must_use]
pub fn restart(machine: &MachineId, last: Option<&pb::LastStart>) -> Option<Restart> {
    let last = last?;
    Some(Restart {
        machine: machine.clone(),
        at_unix_ms: last
            .started_at
            .as_ref()
            .map(|at| unix_ms(at.seconds, at.nanos))
            .unwrap_or_default(),
        report: (!last.report.is_empty()).then(|| last.report.clone()),
        resumed: last.resumed.iter().map(|id| SessionId::new(id.value.clone())).collect(),
        not_brought_back: last
            .not_brought_back
            .iter()
            .map(|entry| NotBroughtBack {
                session: (!entry.session.is_empty()).then(|| SessionId::new(entry.session.clone())),
                tab: (!entry.tab.is_empty()).then(|| entry.tab.clone()),
                problem: entry.problem.clone(),
            })
            .collect(),
    })
}

/// A session; `tabs` are the session's tabs in [`order_tabs`] order.
#[must_use]
pub fn session(session: &pb::Session, tabs: &[pb::Tab], agents: &AgentNames) -> Session {
    let (status, recovery) = status(session);
    let agent = agents.agent(
        session
            .agent
            .as_ref()
            .map(|agent| agent.value.as_str())
            .unwrap_or_default(),
    );
    // A plain shell has no turns: while it starts, or runs a command, it is
    // not an agent at work, so it never reads Working.
    let status = match status {
        Status::Working if agent == Agent::Shell => Status::Idle,
        status => status,
    };
    let first = session.repos.first();
    let fork = session.fork_origin.as_ref();
    let (shells, terminal_agents) = if !session.shell_tabs.is_empty() {
        let mut shells = Vec::with_capacity(session.shell_tabs.len());
        let mut terminal_agents = BTreeMap::new();
        for (pos, entry) in session.shell_tabs.iter().enumerate() {
            shells.push(entry.label.clone());
            if let Some(agent_id) = entry.agent.as_ref().filter(|id| !id.value.is_empty()) {
                let kind = agents.agent(&agent_id.value);
                if kind != Agent::Shell {
                    terminal_agents.insert(pos + 1, kind);
                }
            }
        }
        (shells, terminal_agents)
    } else {
        let mut shells = Vec::new();
        let mut terminal_agents = BTreeMap::new();
        for (pos, tab) in tabs
            .iter()
            .filter(|tab| tab.kind == pb::tab::Kind::Shell as i32)
            .enumerate()
        {
            shells.push(tab.title.clone());
            if let Some(agent_id) = tab.agent.as_ref().filter(|id| !id.value.is_empty()) {
                let kind = agents.agent(&agent_id.value);
                if kind != Agent::Shell {
                    terminal_agents.insert(pos + 1, kind);
                }
            }
        }
        (shells, terminal_agents)
    };
    Session {
        id: session_id(session.id.as_ref()),
        channel: ChannelId::new(
            session
                .channel
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default(),
        ),
        parent: fork.and_then(|origin| origin.parent.as_ref().map(|id| SessionId::new(id.value.clone()))),
        fork: fork.and_then(|origin| match pb::ForkKind::try_from(origin.kind) {
            Ok(pb::ForkKind::CodeAndContext) => Some(ForkKind::Worktree),
            Ok(pb::ForkKind::CodeOnly) => Some(ForkKind::Shared),
            Ok(pb::ForkKind::ContextOnly) => Some(ForkKind::Conversation),
            _ => None,
        }),
        title: session.title.clone(),
        agent,
        machine: machine_id(session.machine.as_ref()),
        // Needs you is still inside the turn: the agent waits on an answer, and a
        // queued message typed now would land in its permission prompt. Probably
        // idle comes from the output fallback, which is never a sure turn end
        // (SE-21, SE-23).
        turn: match status {
            Status::Working | Status::NeedsYou => Turn::Running,
            Status::ProbablyIdle => Turn::Uncertain,
            _ => Turn::Ended,
        },
        status,
        pinned: session.pinned,
        vip: session.vip,
        unread: u32::from(session.unread),
        repo: first
            .and_then(|repo| repo.repo.as_ref())
            .map(repo_text)
            .unwrap_or_default(),
        branch: first.map(|repo| repo.branch.clone()).unwrap_or_default(),
        started: String::new(),
        active: String::new(),
        last_used: String::new(),
        activity: String::new(),
        transcript: Vec::new(),
        shells,
        terminal_agents,
        archived: session.status == pb::SessionStatus::Archived as i32,
        recovery,
        resume_failed: session.resume_failed.as_ref().and_then(resume_failure),
        interrupted_unix_ms: session
            .interrupted_at
            .as_ref()
            .map(|at| unix_ms(at.seconds, at.nanos)),
        away_left_out: session.away_left_out.as_ref().map(away_left_out),
        summary_source: summary_source(session),
        model: (!session.model.is_empty()).then(|| session.model.clone()),
        restart_detail: None,
        permissions: Vec::new(),
        pull_request_review: session
            .pull_request
            .as_ref()
            .is_some_and(|binding| !binding.pull_requests.is_empty()),
        push_allowed: session
            .pull_request
            .as_ref()
            .is_some_and(|binding| binding.push_allowed),
        folder: session.folder.clone(),
        status_since_ms: session
            .status_since
            .as_ref()
            .map(|at| unix_ms(at.seconds, at.nanos)),
        clones: session
            .repos
            .iter()
            .map(|each| SessionClone {
                repo: each.repo.as_ref().map(repo_text).unwrap_or_default(),
                branch: each.branch.clone(),
                path: each.path.clone(),
            })
            .collect(),
        // The step and progress come from the move's operation
        // ([`with_operation`]); until it is read they are empty.
        moving: (status == Status::Moving).then(|| MoveProgress {
            to: machine_id(session.moving_to.as_ref()),
            operation: OperationId::new(
                session
                    .running_operation
                    .as_ref()
                    .map(|id| id.value.clone())
                    .unwrap_or_default(),
            ),
            step: String::new(),
            done_units: 0,
            total_units: 0,
            steps: Vec::new(),
        }),
    }
}

/// Fills a moving session's step and progress from its move's operation
/// (MV-4, MV-17): the running step, else the first not yet done, else the
/// last. A session that isn't moving is left as it is.
pub fn with_operation(session: &mut Session, operation: &pb::Operation) {
    let Some(moving) = session.moving.as_mut() else {
        return;
    };
    let state = |step: &pb::OperationStep| {
        pb::StepState::try_from(step.state).unwrap_or(pb::StepState::Unspecified)
    };
    let at = operation
        .steps
        .iter()
        .find(|step| state(step) == pb::StepState::Running)
        .or_else(|| {
            operation
                .steps
                .iter()
                .find(|step| !matches!(state(step), pb::StepState::Done | pb::StepState::Skipped))
        })
        .or_else(|| operation.steps.last());
    if let Some(step) = at {
        moving.step.clone_from(&step.title);
        moving.done_units = step.done_units;
        moving.total_units = step.total_units;
    }
    moving.steps = operation
        .steps
        .iter()
        .map(|step| MoveStep {
            title: step.title.clone(),
            state: match state(step) {
                pb::StepState::Running => MoveStepState::Running,
                pb::StepState::Done => MoveStepState::Done,
                pb::StepState::Failed => MoveStepState::Failed,
                pb::StepState::Skipped => MoveStepState::Skipped,
                pb::StepState::Undone => MoveStepState::Undone,
                pb::StepState::Waiting | pb::StepState::Unspecified => MoveStepState::Pending,
            },
            done_units: step.done_units,
            total_units: step.total_units,
        })
        .collect();
}

/// The step that copies a move's session, whose units are bytes.
const COPY: &str = "copy";
/// Bytes per MB as the progress label counts them (decimal, as the move card).
const MB: u64 = 1_000_000;

/// The running step of an operation as the progress line draws it
/// (PROGRESS). `None` unless the operation is running and one of its steps
/// is. A step with no total is unmeasured: no fraction, no label.
#[must_use]
pub fn progress(operation: &pb::Operation) -> Option<OpProgress> {
    if operation.state != pb::OperationState::Running as i32 {
        return None;
    }
    let step = operation
        .steps
        .iter()
        .find(|step| step.state == pb::StepState::Running as i32)?;
    let (fraction, label) = if step.total_units == 0 {
        (None, String::new())
    } else {
        let done = step.done_units.min(step.total_units);
        let label = if step.key == COPY {
            format!("{} of {} MB", done / MB, step.total_units / MB)
        } else {
            format!("{}%", percent(done, step.total_units))
        };
        (Some((done as f64 / step.total_units as f64) as f32), label)
    };
    Some(OpProgress {
        title: step.title.clone(),
        fraction,
        label,
        started_ms: operation
            .started_at
            .as_ref()
            .map_or(0, |at| unix_ms(at.seconds, at.nanos)),
    })
}

/// The kind of a daemon operation, as the dialog that started one picks its
/// own out of the feed (PROGRESS). A kind no dialog follows, or one this
/// client doesn't know, is `Other`.
#[must_use]
pub fn op_kind(kind: i32) -> OpKind {
    match pb::OperationKind::try_from(kind) {
        Ok(pb::OperationKind::StartSession) => OpKind::StartSession,
        Ok(pb::OperationKind::ForkSession) => OpKind::Fork,
        Ok(pb::OperationKind::ContinueInChannel) => OpKind::ContinueInChannel,
        Ok(pb::OperationKind::AddRepo) => OpKind::AddRepo,
        Ok(pb::OperationKind::RestoreSession) => OpKind::Restore,
        Ok(pb::OperationKind::RebuildSession) => OpKind::Rebuild,
        Ok(pb::OperationKind::StartPrReview) => OpKind::StartPrReview,
        _ => OpKind::Other,
    }
}

/// The update the progress feed carries for an operation: its id, kind and
/// running step (PROGRESS). `None` when the operation has no id to match it
/// by.
#[must_use]
pub fn op_update(operation: &pb::Operation) -> Option<OpUpdate> {
    let id = operation.id.as_ref()?;
    Some(OpUpdate {
        id: id.value.clone(),
        kind: op_kind(operation.kind),
        progress: progress(operation),
    })
}

/// `done` of `total` as a whole percent, capped at 100; 0 when `total` is 0.
#[must_use]
pub fn percent(done: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    let value = (u128::from(done) * 100 / u128::from(total)).min(100);
    u8::try_from(value).unwrap_or(100)
}

/// What the host left out of a session's agent's writes (AW-9, D267).
fn away_left_out(left: &pb::AwayWritesLeftOut) -> AwayLeftOut {
    AwayLeftOut {
        count: left.count,
        items: left.items.iter().map(|id| id.value.clone()).collect(),
        not_made: left.not_made.clone(),
        refused: left.refused,
    }
}

// ---------------------------------------------------------------------------
// Pure comment mapping (shared by `live_desk::docs` and `live_review::comments`).
// ---------------------------------------------------------------------------

pub fn comment_author(actor: Option<&pb::Actor>) -> Author {
    use pb::actor::Actor;
    match actor.and_then(|actor| actor.actor.as_ref()) {
        Some(Actor::Agent(agent)) => {
            let name = agent.agent.as_ref().map(|id| id.value.clone()).unwrap_or_default();
            Author::Agent(if name.is_empty() { "agent".to_owned() } else { name })
        }
        Some(Actor::Script(script)) => Author::Agent(script.name.clone()),
        Some(Actor::System(_)) => Author::Agent("Humata".to_owned()),
        Some(Actor::User(_)) | None => Author::User,
    }
}

/// `text` as one entry per line; empty text is no lines.
fn lines_of(text: &str) -> Vec<String> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n').map(str::to_owned).collect()
    }
}

/// `owner/name`, or just the name when the daemon gave no owner.
#[must_use]
pub fn repo_name(repo: &pb::RepoRef) -> String {
    if repo.owner.is_empty() {
        repo.name.clone()
    } else {
        repo_text(repo)
    }
}

/// The app's destination for a daemon comment. A comment with none is for
/// the session's terminal, as every comment was before notes and pull
/// request drafts.
#[must_use]
pub fn comment_destination(destination: Option<&pb::CommentDestination>) -> Destination {
    use pb::comment_destination::Destination as Wire;
    match destination.and_then(|destination| destination.destination.as_ref()) {
        None | Some(Wire::Terminal(_)) => Destination::Terminal,
        Some(Wire::Note(_)) => Destination::Note,
        Some(Wire::PullRequest(pull_request)) => Destination::PullRequest {
            repo: pull_request.repo.as_ref().map(repo_name).unwrap_or_default(),
            number: pull_request.number,
        },
    }
}

#[must_use]
pub fn comment_anchor(anchor: &pb::CommentAnchor) -> Anchor {
    let (repo, path) = match &anchor.file {
        Some(file) => (
            file.repo.as_ref().map(repo_name).unwrap_or_default(),
            file.path.clone(),
        ),
        None => (String::new(), String::new()),
    };
    let lines = anchor.lines.clone().unwrap_or_default();
    Anchor {
        repo,
        path,
        side: if pb::DiffSide::try_from(anchor.side) == Ok(pb::DiffSide::Old) {
            Side::Old
        } else {
            Side::New
        },
        first: lines.first,
        last: lines.last,
        quoted: lines_of(&anchor.quoted),
        base: (!anchor.base.is_empty()).then(|| anchor.base.clone()),
        detached: anchor.detached,
        outdated: anchor.outdated,
    }
}

/// The daemon's comment as the app's.
#[must_use]
pub fn comment(comment: &pb::Comment) -> Comment {
    let status = match pb::CommentStatus::try_from(comment.status) {
        Ok(pb::CommentStatus::Queued) => CommentStatus::Waiting,
        Ok(pb::CommentStatus::Sent) => CommentStatus::Sent,
        Ok(pb::CommentStatus::Posted) => CommentStatus::Posted,
        Ok(pb::CommentStatus::Addressed) => CommentStatus::Addressed,
        Ok(pb::CommentStatus::Resolved) => CommentStatus::Resolved,
        Ok(pb::CommentStatus::Draft | pb::CommentStatus::Unspecified) | Err(_) => CommentStatus::Draft,
    };
    Comment {
        id: CommentId::new(comment.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()),
        session: session_id(comment.session.as_ref()),
        anchor: comment.anchor.as_ref().map(comment_anchor).unwrap_or_else(|| Anchor {
            repo: String::new(),
            path: String::new(),
            side: Side::New,
            first: 0,
            last: 0,
            quoted: Vec::new(),
            base: None,
            detached: false,
            outdated: false,
        }),
        text: comment.text.clone(),
        status,
        revision: comment.revision.as_ref().map_or(0, |revision| revision.value),
        unconfirmed: comment.delivery_unconfirmed,
        archived: comment.archived_at.is_some(),
        replies: comment
            .replies
            .iter()
            .map(|reply| ThreadReply {
                id: reply.id.clone(),
                author: comment_author(reply.author.as_ref()),
                text: reply.text.clone(),
            })
            .collect(),
        resolved_by: (status == CommentStatus::Resolved)
            .then(|| comment.resolved.as_ref().map(|resolved| comment_author(resolved.by.as_ref())))
            .flatten(),
        addressed: (status == CommentStatus::Addressed).then(|| lines_of(&comment.addressed_text)),
        destination: comment_destination(comment.destination.as_ref()),
        author: comment_author(comment.author.as_ref()),
        posted_url: (!comment.posted_url.is_empty()).then(|| comment.posted_url.clone()),
        post_unconfirmed: comment.post_unconfirmed,
    }
}

#[must_use]
pub fn comments(comments: &[pb::Comment]) -> Vec<Comment> {
    comments.iter().map(comment).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb_session(status: pb::SessionStatus) -> pb::Session {
        pb::Session {
            id: Some(pb::SessionId {
                value: "se_1".to_owned(),
            }),
            status: status as i32,
            exit_code: 3,
            agent: Some(pb::AgentId {
                value: "claude".to_owned(),
            }),
            repos: vec![pb::SessionRepo {
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "app".to_owned(),
                }),
                branch: "feature".to_owned(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn tab(kind: pb::tab::Kind, id: &str) -> pb::Tab {
        pb::Tab {
            id: Some(pb::TabId {
                value: id.to_owned(),
            }),
            kind: kind as i32,
            title: id.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn repo_text_omits_leading_slash_when_owner_is_empty() {
        assert_eq!(
            repo_text(&pb::RepoRef {
                owner: String::new(),
                name: "api".to_owned(),
            }),
            "api"
        );
        assert_eq!(
            repo_text(&pb::RepoRef {
                owner: "acme".to_owned(),
                name: "api".to_owned(),
            }),
            "acme/api"
        );
    }

    #[test]
    fn statuses_map_to_the_apps_statuses() {
        let agents = AgentNames::default();
        let status_of = |status| session(&pb_session(status), &[], &agents).status;
        assert_eq!(status_of(pb::SessionStatus::Starting), Status::Working);
        assert_eq!(status_of(pb::SessionStatus::Working), Status::Working);
        assert_eq!(status_of(pb::SessionStatus::NeedsInput), Status::NeedsYou);
        assert_eq!(status_of(pb::SessionStatus::Idle), Status::Idle);
        assert_eq!(status_of(pb::SessionStatus::Offline), Status::Offline);
        // MV-4: a moving session reads Moving (to its target, with the
        // move's step and progress), never Idle, and takes no input.
        assert_ne!(status_of(pb::SessionStatus::Moving), Status::Idle);
        assert!(!status_of(pb::SessionStatus::Moving).accepts_input());
        assert_eq!(
            status_of(pb::SessionStatus::Exited),
            Status::Exited { code: 3 }
        );
    }

    #[test]
    fn only_a_reported_idle_ends_the_turn() {
        let agents = AgentNames::default();
        let turn_of = |status| session(&pb_session(status), &[], &agents).turn;
        assert_eq!(turn_of(pb::SessionStatus::Working), Turn::Running);
        assert_eq!(turn_of(pb::SessionStatus::NeedsInput), Turn::Running);
        assert_eq!(turn_of(pb::SessionStatus::ProbablyIdle), Turn::Uncertain);
        assert_eq!(turn_of(pb::SessionStatus::Idle), Turn::Ended);
    }

    #[test]
    fn a_moving_session_reads_moving_to_its_target_with_the_running_step() {
        let step = |key: &str, title: &str, state: pb::StepState, done: u64, total: u64| pb::OperationStep {
            key: key.to_owned(),
            title: title.to_owned(),
            state: state as i32,
            done_units: done,
            total_units: total,
            ..Default::default()
        };
        use pb::StepState::{Done, Running, Waiting};
        let operation = pb::Operation {
            id: Some(pb::OperationId {
                value: "op_move".to_owned(),
            }),
            state: pb::OperationState::Running as i32,
            steps: vec![
                step("check", "Checking the target", Done, 0, 0),
                step("pause", "Pausing the agent", Done, 0, 0),
                step("pack", "Packing the session", Done, 0, 0),
                step("copy", "Copying the session", Running, 300, 1000),
                step("verify", "Verifying", Waiting, 0, 0),
                step("resume", "Resuming", Waiting, 0, 0),
            ],
            ..Default::default()
        };
        let mut wire = pb_session(pb::SessionStatus::Moving);
        wire.moving_to = Some(pb::MachineId {
            value: "gpu-box".to_owned(),
        });
        wire.running_operation = operation.id.clone();
        let mut mapped = session(&wire, &[], &AgentNames::default());
        assert_eq!(mapped.status, Status::Moving);
        // Before the operation is read: Moving, empty step, nothing done.
        assert_eq!(mapped.moving.as_ref().map(|m| m.step.as_str()), Some(""));
        with_operation(&mut mapped, &operation);
        let moving = mapped.moving.as_ref().expect("a moving session carries its move");
        assert_eq!(moving.to, MachineId::new("gpu-box"));
        assert_eq!(moving.operation, OperationId::new("op_move"));
        assert_eq!(moving.step, "Copying the session");
        assert_eq!((moving.done_units, moving.total_units), (300, 1000));
        assert_eq!(moving.steps.len(), 6, "every step of the operation is carried");
        assert_eq!(mapped.turn, Turn::Ended);
        let idle = session(&pb_session(pb::SessionStatus::Idle), &[], &AgentNames::default());
        assert_eq!(idle.moving, None);
    }

    #[test]
    fn an_operations_steps_map_in_order_with_their_states_and_progress() {
        use pb::StepState::{Done, Failed, Running, Skipped, Undone, Unspecified, Waiting};
        let wire = [
            ("a", Done, 5, 5),
            ("b", Skipped, 0, 0),
            ("c", Running, 40, 100),
            ("d", Waiting, 0, 0),
            ("e", Unspecified, 0, 0),
            ("f", Failed, 1, 2),
            ("g", Undone, 0, 0),
        ];
        let operation = pb::Operation {
            id: Some(pb::OperationId {
                value: "op_move".to_owned(),
            }),
            steps: wire
                .iter()
                .map(|(key, state, done, total)| pb::OperationStep {
                    key: (*key).to_owned(),
                    title: format!("step {key}"),
                    state: *state as i32,
                    done_units: *done,
                    total_units: *total,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let mut wire_session = pb_session(pb::SessionStatus::Moving);
        wire_session.running_operation = operation.id.clone();
        let mut mapped = session(&wire_session, &[], &AgentNames::default());
        assert_eq!(mapped.moving.as_ref().map(|m| m.steps.len()), Some(0), "none until the operation is read");
        with_operation(&mut mapped, &operation);
        let steps = mapped.moving.map(|m| m.steps).unwrap_or_default();
        let states: Vec<_> = steps.iter().map(|step| step.state).collect();
        assert_eq!(
            states,
            [
                MoveStepState::Done,
                MoveStepState::Skipped,
                MoveStepState::Running,
                MoveStepState::Pending,
                MoveStepState::Pending,
                MoveStepState::Failed,
                MoveStepState::Undone,
            ]
        );
        let titles: Vec<_> = steps.iter().map(|step| step.title.as_str()).collect();
        assert_eq!(titles, ["step a", "step b", "step c", "step d", "step e", "step f", "step g"]);
        assert_eq!((steps[2].done_units, steps[2].total_units), (40, 100));
    }

    #[test]
    fn a_failed_resume_is_exited_and_recoverable() {
        let mapped = session(
            &pb_session(pb::SessionStatus::ResumeFailed),
            &[],
            &AgentNames::default(),
        );
        assert_eq!(mapped.status, Status::Exited { code: 3 });
        assert!(matches!(
            mapped.recovery,
            Some(Recovery::Recoverable { .. })
        ));
    }

    #[test]
    fn a_session_takes_its_branch_repo_and_agent() {
        let mut agents = AgentNames::default();
        let plain = session(&pb_session(pb::SessionStatus::Idle), &[], &agents);
        assert_eq!(plain.branch, "feature");
        assert_eq!(plain.repo, "acme/app");
        assert_eq!(plain.agent, Agent::Claude);
        assert_eq!(plain.recovery, None);

        agents.set(Agent::Codex, "claude".to_owned());
        let renamed = session(&pb_session(pb::SessionStatus::Idle), &[], &agents);
        assert_eq!(renamed.agent, Agent::Codex);
        assert_eq!(agents.name(Agent::Codex), "claude");
        assert_eq!(agents.name(Agent::Shell), "zsh");
        assert_eq!(agents.agent("nobody-knows"), Agent::Shell);
    }

    #[test]
    fn shells_are_the_shell_tabs_after_the_agent() {
        let tabs = order_tabs(vec![
            tab(pb::tab::Kind::Shell, "sh1"),
            tab(pb::tab::Kind::Agent, "ag"),
            tab(pb::tab::Kind::Shell, "sh2"),
        ]);
        let ids: Vec<_> = tabs
            .iter()
            .filter_map(|tab| tab.id.as_ref().map(|id| id.value.as_str()))
            .collect();
        assert_eq!(ids, ["ag", "sh1", "sh2"]);
        let mapped = session(
            &pb_session(pb::SessionStatus::Idle),
            &tabs,
            &AgentNames::default(),
        );
        assert_eq!(mapped.shells, ["sh1", "sh2"]);
    }

    #[test]
    fn a_failed_resume_carries_why_and_the_exit_code() {
        let mut failed = pb_session(pb::SessionStatus::ResumeFailed);
        failed.resume_failed = Some(pb::ResumeFailed {
            reason: pb::resume_failed::Reason::AgentExited as i32,
            exit_code: 7,
            ..Default::default()
        });
        let mapped = session(&failed, &[], &AgentNames::default());
        assert_eq!(mapped.resume_failed, Some(ResumeFailure::AgentExited(7)));

        failed.resume_failed = Some(pb::ResumeFailed {
            reason: pb::resume_failed::Reason::ConversationFileMissing as i32,
            ..Default::default()
        });
        let mapped = session(&failed, &[], &AgentNames::default());
        assert_eq!(mapped.resume_failed, Some(ResumeFailure::ConversationMissing));

        let plain = session(&pb_session(pb::SessionStatus::Idle), &[], &AgentNames::default());
        assert_eq!(plain.resume_failed, None);
        assert_eq!(plain.interrupted_unix_ms, None);
    }

    #[test]
    fn what_start_fresh_can_summarize_comes_across() {
        use pb::session::SummarySource as S;
        let mut failed = pb_session(pb::SessionStatus::ResumeFailed);
        for (said, wanted) in [
            (S::Transcript, Some(SummarySource::Transcript)),
            (S::Scrollback, Some(SummarySource::Scrollback)),
            (S::None, Some(SummarySource::Nothing)),
            (S::Unspecified, None),
        ] {
            failed.summary_source = said as i32;
            let mapped = session(&failed, &[], &AgentNames::default());
            assert_eq!(mapped.summary_source, wanted);
        }
    }

    #[test]
    fn an_interruption_time_becomes_unix_ms() {
        let mut interrupted = pb_session(pb::SessionStatus::Idle);
        interrupted.interrupted_at = Some(Default::default());
        if let Some(at) = interrupted.interrupted_at.as_mut() {
            at.seconds = 1_700_000_000;
            at.nanos = 250_000_000;
        }
        let mapped = session(&interrupted, &[], &AgentNames::default());
        assert_eq!(mapped.interrupted_unix_ms, Some(1_700_000_000_250));
    }

    #[test]
    fn a_status_start_time_becomes_unix_ms_and_none_stays_none() {
        let mut since = pb_session(pb::SessionStatus::Working);
        since.status_since = Some(Default::default());
        if let Some(at) = since.status_since.as_mut() {
            at.seconds = 1_700_000_000;
            at.nanos = 250_000_000;
        }
        let mapped = session(&since, &[], &AgentNames::default());
        assert_eq!(mapped.status_since_ms, Some(1_700_000_000_250));
        let unknown = session(
            &pb_session(pb::SessionStatus::Working),
            &[],
            &AgentNames::default(),
        );
        assert_eq!(unknown.status_since_ms, None);
    }

    #[test]
    fn what_the_host_left_out_comes_across_and_none_stays_none() {
        let plain = session(&pb_session(pb::SessionStatus::Idle), &[], &AgentNames::default());
        assert_eq!(plain.away_left_out, None);
        let mut left = pb_session(pb::SessionStatus::Idle);
        left.away_left_out = Some(pb::AwayWritesLeftOut {
            count: 2,
            items: vec![pb::BoardItemId {
                value: "PAY-12".to_owned(),
            }],
            not_made: vec!["Fix login".to_owned()],
            refused: true,
            at: None,
        });
        let mapped = session(&left, &[], &AgentNames::default());
        assert_eq!(
            mapped.away_left_out,
            Some(AwayLeftOut {
                count: 2,
                items: vec!["PAY-12".to_owned()],
                not_made: vec!["Fix login".to_owned()],
                refused: true,
            })
        );
    }

    #[test]
    fn a_restart_lists_resumed_sessions_and_what_was_not_brought_back() {
        let machine = MachineId::new("ma_1");
        assert_eq!(restart(&machine, None), None);
        let last = pb::LastStart {
            report: "/reports/r1".to_owned(),
            resumed: vec![pb::SessionId {
                value: "se_1".to_owned(),
            }],
            not_brought_back: vec![
                pb::NotBroughtBack {
                    session: "se_2".to_owned(),
                    problem: "its agent is gone".to_owned(),
                    ..Default::default()
                },
                pb::NotBroughtBack {
                    tab: "tab_3".to_owned(),
                    problem: "its folder is gone".to_owned(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mapped = restart(&machine, Some(&last));
        let Some(mapped) = mapped else {
            panic!("a last start maps to a restart");
        };
        assert_eq!(mapped.report.as_deref(), Some("/reports/r1"));
        assert_eq!(mapped.resumed, [SessionId::new("se_1")]);
        assert_eq!(mapped.not_brought_back[0].session, Some(SessionId::new("se_2")));
        assert_eq!(mapped.not_brought_back[0].tab, None);
        assert_eq!(mapped.not_brought_back[1].tab.as_deref(), Some("tab_3"));
        assert_eq!(mapped.not_brought_back[1].session, None);
    }

    #[test]
    fn a_session_on_a_stopped_machine_reads_stopped_and_not_in_a_turn() {
        let mapped = session(
            &pb_session(pb::SessionStatus::MachineStopped),
            &[],
            &AgentNames::default(),
        );
        assert_eq!(mapped.status, Status::Stopped);
        assert_eq!(mapped.turn, Turn::Ended);
        assert_eq!(mapped.recovery, None);
    }

    /// An hour after the stop below, on a Mac whose clock is UTC.
    const NOW: LocalNow = LocalNow {
        unix_ms: 1_699_974_125_000,
        utc_offset_s: 0,
    };

    fn pb_stop(kind: pb::StoppedByKind) -> pb::SessionsStopped {
        pb::SessionsStopped {
            stop_id: "st_1".to_owned(),
            // 2023-11-14 14:02:05 UTC.
            stopped_at_unix_ms: 1_699_970_525_000,
            by: Some(pb::StoppedBy {
                kind: kind as i32,
                via: pb::StopVia::App as i32,
                machine: "ma_1".to_owned(),
                session: "se_9".to_owned(),
            }),
            reason: String::new(),
            ended: None,
        }
    }

    fn pb_machine(stopped: Option<pb::SessionsStopped>, delivered: bool) -> pb::Machine {
        pb::Machine {
            id: Some(pb::MachineId {
                value: "ma_1".to_owned(),
            }),
            name: "devbox-1".to_owned(),
            reachability: pb::Reachability::Online as i32,
            sessions_stopped: stopped,
            stop_delivered: delivered,
            ..Default::default()
        }
    }

    #[test]
    fn a_running_machine_has_no_stop() {
        assert_eq!(machine(&pb_machine(None, false), &NOW).stop, None);
        // Even when the daemon sets delivered without a stop.
        assert_eq!(machine(&pb_machine(None, true), &NOW).stop, None);
    }

    #[test]
    fn a_machine_shows_its_root_or_the_default_when_the_daemon_names_none() {
        let mut named = pb_machine(None, false);
        named.root = "~/Work".to_owned();
        assert_eq!(machine(&named, &NOW).root, "~/Work");
        assert_eq!(machine(&pb_machine(None, false), &NOW).root, humata2_clients::desk::DEFAULT_ROOT);
    }

    #[test]
    fn a_stopped_machine_carries_its_stop_and_whether_it_was_delivered() {
        let delivered = machine(&pb_machine(Some(pb_stop(pb::StoppedByKind::User)), true), &NOW);
        assert_eq!(
            delivered.stop,
            Some(MachineStop {
                stop_id: "st_1".to_owned(),
                at: "14:02".to_owned(),
                by: StoppedBy::You,
                delivered: true,
            })
        );
        // Its sessions being stopped is not the link: it still answers.
        assert!(delivered.online);

        let kept = machine(&pb_machine(Some(pb_stop(pb::StoppedByKind::User)), false), &NOW);
        assert_eq!(kept.stop.map(|stop| stop.delivered), Some(false));
    }

    #[test]
    fn who_stopped_comes_across_as_the_person_a_terminal_or_an_agent() {
        let by = |kind| machine_stop(&pb_stop(kind), true, &NOW).by;
        assert_eq!(by(pb::StoppedByKind::User), StoppedBy::You);
        assert_eq!(
            by(pb::StoppedByKind::Terminal),
            StoppedBy::Terminal {
                machine: MachineId::new("ma_1")
            }
        );
        assert_eq!(
            by(pb::StoppedByKind::Agent),
            StoppedBy::Agent {
                session: SessionId::new("se_9")
            }
        );
        assert_eq!(by(pb::StoppedByKind::Script), StoppedBy::Script);
        // Unnamed is the person's.
        assert_eq!(by(pb::StoppedByKind::Unspecified), StoppedBy::You);
        let mut nobody = pb_stop(pb::StoppedByKind::User);
        nobody.by = None;
        assert_eq!(machine_stop(&nobody, true, &NOW).by, StoppedBy::You);
    }

    #[test]
    fn repos_parse_as_owner_and_name() {
        assert_eq!(
            parse_repo("acme/app"),
            Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "app".to_owned()
            })
        );
        assert_eq!(parse_repo("app"), None);
        assert_eq!(parse_repo("a/b/c"), None);
        assert_eq!(parse_repo("/app"), None);
        assert_eq!(parse_repo("acme/"), None);
    }

    #[test]
    fn a_session_bound_to_pull_requests_is_a_pull_request_review() {
        let agents = AgentNames::default();
        let plain = session(&pb_session(pb::SessionStatus::Idle), &[], &agents);
        assert!(!plain.pull_request_review);

        let mut bound = pb_session(pb::SessionStatus::Idle);
        bound.pull_request = Some(pb::PullRequestBinding {
            pull_requests: vec![pb::PullRequestRef {
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "app".to_owned(),
                }),
                number: 91,
                ..Default::default()
            }],
            push_allowed: false,
        });
        assert!(session(&bound, &[], &agents).pull_request_review);
    }

    fn progress_step(key: &str, state: pb::StepState, done: u64, total: u64) -> pb::OperationStep {
        pb::OperationStep {
            key: key.to_owned(),
            title: format!("title of {key}"),
            state: state as i32,
            done_units: done,
            total_units: total,
            ..Default::default()
        }
    }

    fn running_operation(steps: Vec<pb::OperationStep>) -> pb::Operation {
        pb::Operation {
            state: pb::OperationState::Running as i32,
            steps,
            ..Default::default()
        }
    }

    fn progress_of(operation: &pb::Operation) -> OpProgress {
        match progress(operation) {
            Some(shown) => shown,
            None => panic!("a running operation with a running step shows progress"),
        }
    }

    #[test]
    fn a_measured_clone_step_gives_its_fraction_title_and_percent() {
        let operation = running_operation(vec![
            progress_step("record", pb::StepState::Done, 0, 0),
            progress_step("clone_main:acme/api", pb::StepState::Running, 420, 1000),
        ]);
        let shown = progress_of(&operation);
        assert!(matches!(shown.fraction, Some(f) if (f - 0.42).abs() < 1e-6));
        assert_eq!(shown.title, "title of clone_main:acme/api");
        assert!(shown.label.contains("42"));
    }

    #[test]
    fn a_copy_step_labels_its_bytes_in_megabytes() {
        let operation = running_operation(vec![progress_step(
            "copy",
            pb::StepState::Running,
            120_000_000,
            512_000_000,
        )]);
        let shown = progress_of(&operation);
        assert!(matches!(shown.fraction, Some(f) if (f - 0.234_375).abs() < 1e-6));
        assert!(shown.label.contains("120"));
        assert!(shown.label.contains("512"));
    }

    #[test]
    fn an_unmeasured_step_has_no_fraction_and_no_label() {
        let operation = running_operation(vec![progress_step("clone_main:acme/api", pb::StepState::Running, 5, 0)]);
        let shown = progress_of(&operation);
        assert_eq!(shown.fraction, None);
        assert!(shown.label.is_empty());
    }

    #[test]
    fn done_past_total_is_a_full_fraction() {
        let operation = running_operation(vec![progress_step(
            "clone_main:acme/api",
            pb::StepState::Running,
            1500,
            1000,
        )]);
        let shown = progress_of(&operation);
        assert!(matches!(shown.fraction, Some(f) if (f - 1.0).abs() < 1e-6));
    }

    #[test]
    fn no_running_step_or_a_finished_operation_shows_no_progress() {
        let all_done = running_operation(vec![
            progress_step("record", pb::StepState::Done, 0, 0),
            progress_step("clone_main:acme/api", pb::StepState::Done, 1000, 1000),
        ]);
        assert_eq!(progress(&all_done), None);

        let mut succeeded = running_operation(vec![progress_step("copy", pb::StepState::Running, 1, 2)]);
        succeeded.state = pb::OperationState::Succeeded as i32;
        assert_eq!(progress(&succeeded), None);
    }

    #[test]
    fn the_start_time_is_the_operation_start_in_unix_ms() {
        let mut operation = running_operation(vec![progress_step("record", pb::StepState::Running, 0, 0)]);
        operation.started_at = Some(crate::humata2_grpc::Timestamp {
            seconds: 12,
            nanos: 5_000_000,
        });
        assert_eq!(progress_of(&operation).started_ms, 12_005);
    }

    fn update_of(operation: &pb::Operation) -> OpUpdate {
        match op_update(operation) {
            Some(update) => update,
            None => panic!("an operation with an id has an update"),
        }
    }

    #[test]
    fn a_running_fork_with_a_running_step_is_a_fork_update_with_its_progress() {
        let mut operation = running_operation(vec![progress_step("copy", pb::StepState::Running, 1, 2)]);
        operation.id = Some(pb::OperationId {
            value: "op_fork".to_owned(),
        });
        operation.kind = pb::OperationKind::ForkSession as i32;
        let update = update_of(&operation);
        assert_eq!(update.id, "op_fork");
        assert_eq!(update.kind, OpKind::Fork);
        assert_eq!(
            update.progress.map(|shown| shown.title),
            Some("title of copy".to_owned())
        );
    }

    #[test]
    fn a_succeeded_operation_is_an_update_with_no_progress() {
        let mut operation = running_operation(vec![progress_step("copy", pb::StepState::Running, 1, 2)]);
        operation.id = Some(pb::OperationId {
            value: "op_fork".to_owned(),
        });
        operation.kind = pb::OperationKind::ForkSession as i32;
        operation.state = pb::OperationState::Succeeded as i32;
        let update = update_of(&operation);
        assert_eq!(update.kind, OpKind::Fork);
        assert_eq!(update.progress, None);
    }

    #[test]
    fn an_operation_without_an_id_has_no_update() {
        let operation = running_operation(vec![progress_step("copy", pb::StepState::Running, 1, 2)]);
        assert_eq!(op_update(&operation), None);
    }

    #[test]
    fn a_kind_no_dialog_follows_or_an_unknown_one_is_other() {
        assert_eq!(op_kind(pb::OperationKind::MoveSession as i32), OpKind::Other);
        assert_eq!(op_kind(9_999), OpKind::Other);
    }
}
