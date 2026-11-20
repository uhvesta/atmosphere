//! [`TodoClient`] for the real daemon, over its TodoService (BOARDSIMPLE).
//! The host keeps the list and its revisions; this client only asks, and
//! maps the wire items to the app's [`Entry`]s.

use humata2_clients::todo::{
    BlockerGoneResolution, BulkUpdateFailure, BulkUpdateItem, BulkUpdateResult, Entry, FoldResult,
    MigrationNotice, NewTodo, SplitResult, TodoClient, TodoFilter, TodoHistoryEntry, TodoLists,
    TodoPatch, TodoRunsResult,
};
use humata2_clients::{DaemonError, Reply};
use humata2_todo::{
    Ask, AskDetail, AskKind, BlockerGone, ChannelFilter, CheckpointState, CloseReadiness, Day,
    Effort, Evidence, ExternalBlock, FuzzyDue, Impact, LifecyclePrompt, Need, OldColumn, OwnerKind,
    Review, RunEntry, RunLink, SessionSignal, SourceAnchor, Start, State, Step, StuckAgent,
    Summary, Todo, TodoComment, TodoOwner, WatchedPr, WordDiffKind, WordDiffSpan,
};
use tokio::sync::mpsc;
use tonic::Streaming;

use crate::connection::{LiveConnection, MESSAGE_MAX_BYTES, Wire, fresh_key};
use crate::humata2_grpc::humata::v1 as pb;
use crate::humata2_grpc::humata::v1::event_service_client::EventServiceClient;
use crate::humata2_grpc::humata::v1::inbox_service_client::InboxServiceClient;
use crate::humata2_grpc::humata::v1::todo_service_client::TodoServiceClient;

const PAGE_SIZE: u32 = 200;

/// Lists the watcher may have waiting before the feed waits for it.
const QUEUE: usize = 16;

const DAY_SECS: i64 = 86_400;

/// The To-do list on the real daemon. Cheap to clone.
#[derive(Clone)]
pub struct LiveTodo {
    conn: LiveConnection,
}

impl LiveTodo {
    /// The list over `conn`'s connection.
    #[must_use]
    pub fn new(conn: impl AsRef<LiveConnection>) -> Self {
        Self {
            conn: conn.as_ref().clone(),
        }
    }
}

fn client(conn: &LiveConnection) -> TodoServiceClient<Wire> {
    TodoServiceClient::new(conn.wire())
        .max_decoding_message_size(MESSAGE_MAX_BYTES)
        .max_encoding_message_size(MESSAGE_MAX_BYTES)
}

fn inbox(conn: &LiveConnection) -> InboxServiceClient<Wire> {
    InboxServiceClient::new(conn.wire())
        .max_decoding_message_size(MESSAGE_MAX_BYTES)
        .max_encoding_message_size(MESSAGE_MAX_BYTES)
}

fn event_stream(conn: &LiveConnection) -> EventServiceClient<Wire> {
    EventServiceClient::new(conn.wire())
        .max_decoding_message_size(MESSAGE_MAX_BYTES)
        .max_encoding_message_size(MESSAGE_MAX_BYTES)
}

// ---------------------------------------------------------------------------
// Pure mapping.

fn day_pb(day: Day) -> pb::CivilDay {
    let (year, month, day) = day.ymd();
    pb::CivilDay { year, month, day }
}

/// The day the wire names; unset or a date that doesn't exist is none.
fn day_of(day: Option<&pb::CivilDay>) -> Option<Day> {
    let day = day?;
    Day::from_ymd(day.year, day.month, day.day)
}

/// The civil day (UTC) `secs` since the epoch falls on.
fn day_of_secs(secs: i64) -> Option<Day> {
    let z = secs.div_euclid(DAY_SECS) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Day::from_ymd(i32::try_from(y).ok()?, u32::try_from(m).ok()?, u32::try_from(d).ok()?)
}

fn effort(value: i32) -> Option<Effort> {
    match pb::TodoEffort::try_from(value) {
        Ok(pb::TodoEffort::S) => Some(Effort::S),
        Ok(pb::TodoEffort::M) => Some(Effort::M),
        Ok(pb::TodoEffort::L) => Some(Effort::L),
        Ok(pb::TodoEffort::Unspecified) | Err(_) => None,
    }
}

fn effort_pb(effort: Option<Effort>) -> pb::TodoEffort {
    match effort {
        None => pb::TodoEffort::Unspecified,
        Some(Effort::S) => pb::TodoEffort::S,
        Some(Effort::M) => pb::TodoEffort::M,
        Some(Effort::L) => pb::TodoEffort::L,
    }
}

fn impact(value: i32) -> Option<Impact> {
    match pb::TodoImpact::try_from(value) {
        Ok(pb::TodoImpact::Low) => Some(Impact::Low),
        Ok(pb::TodoImpact::Medium) => Some(Impact::Medium),
        Ok(pb::TodoImpact::High) => Some(Impact::High),
        Ok(pb::TodoImpact::Unspecified) | Err(_) => None,
    }
}

fn impact_pb(impact: Option<Impact>) -> pb::TodoImpact {
    match impact {
        None => pb::TodoImpact::Unspecified,
        Some(Impact::Low) => pb::TodoImpact::Low,
        Some(Impact::Medium) => pb::TodoImpact::Medium,
        Some(Impact::High) => pb::TodoImpact::High,
    }
}

fn need(value: i32) -> Option<Need> {
    match pb::TodoNeed::try_from(value) {
        Ok(pb::TodoNeed::Idea) => Some(Need::Idea),
        Ok(pb::TodoNeed::NiceToHave) => Some(Need::NiceToHave),
        Ok(pb::TodoNeed::Expected) => Some(Need::Expected),
        Ok(pb::TodoNeed::Critical) => Some(Need::Critical),
        Ok(pb::TodoNeed::Unspecified) | Err(_) => None,
    }
}

fn need_pb(need: Option<Need>) -> pb::TodoNeed {
    match need {
        None => pb::TodoNeed::Unspecified,
        Some(Need::Idea) => pb::TodoNeed::Idea,
        Some(Need::NiceToHave) => pb::TodoNeed::NiceToHave,
        Some(Need::Expected) => pb::TodoNeed::Expected,
        Some(Need::Critical) => pb::TodoNeed::Critical,
    }
}

fn fuzzy_due(value: i32) -> Option<FuzzyDue> {
    match pb::TodoFuzzyDue::try_from(value) {
        Ok(pb::TodoFuzzyDue::InAFewDays) => Some(FuzzyDue::InAFewDays),
        Ok(pb::TodoFuzzyDue::NextWeek) => Some(FuzzyDue::NextWeek),
        Ok(pb::TodoFuzzyDue::NextMonth) => Some(FuzzyDue::NextMonth),
        Ok(pb::TodoFuzzyDue::Unspecified) | Err(_) => None,
    }
}

fn fuzzy_due_pb(fuzzy: Option<FuzzyDue>) -> pb::TodoFuzzyDue {
    match fuzzy {
        None => pb::TodoFuzzyDue::Unspecified,
        Some(FuzzyDue::InAFewDays) => pb::TodoFuzzyDue::InAFewDays,
        Some(FuzzyDue::NextWeek) => pb::TodoFuzzyDue::NextWeek,
        Some(FuzzyDue::NextMonth) => pb::TodoFuzzyDue::NextMonth,
    }
}

fn checkpoint(value: i32) -> Option<CheckpointState> {
    match pb::TodoCheckpoint::try_from(value) {
        Ok(pb::TodoCheckpoint::WithContext) => Some(CheckpointState::WithContext),
        Ok(pb::TodoCheckpoint::Defined) => Some(CheckpointState::Defined),
        Ok(pb::TodoCheckpoint::Done) => Some(CheckpointState::Done),
        Ok(pb::TodoCheckpoint::Unspecified) | Err(_) => None,
    }
}

fn checkpoint_pb(cp: Option<CheckpointState>) -> pb::TodoCheckpoint {
    match cp {
        None => pb::TodoCheckpoint::Unspecified,
        Some(CheckpointState::WithContext) => pb::TodoCheckpoint::WithContext,
        Some(CheckpointState::Defined) => pb::TodoCheckpoint::Defined,
        Some(CheckpointState::Done) => pb::TodoCheckpoint::Done,
    }
}

/// A state the app doesn't know reads as open, the list's start.
fn state(value: i32) -> State {
    match pb::TodoState::try_from(value) {
        Ok(pb::TodoState::InProgress) => State::InProgress,
        Ok(pb::TodoState::Closed) => State::Closed,
        Ok(pb::TodoState::WontDo) => State::WontDo,
        Ok(pb::TodoState::Became) => State::Became,
        Ok(pb::TodoState::WatchingPr) => State::WatchingPr,
        Ok(pb::TodoState::Open | pb::TodoState::Unspecified) | Err(_) => State::Open,
    }
}

fn state_pb(state: State) -> pb::TodoState {
    match state {
        State::Open => pb::TodoState::Open,
        State::InProgress => pb::TodoState::InProgress,
        State::Closed => pb::TodoState::Closed,
        State::WontDo => pb::TodoState::WontDo,
        State::Became => pb::TodoState::Became,
        State::WatchingPr => pb::TodoState::WatchingPr,
    }
}

fn old_column(value: i32) -> OldColumn {
    match pb::BoardColumn::try_from(value) {
        Ok(pb::BoardColumn::Backlog) => OldColumn::Backlog,
        Ok(pb::BoardColumn::InProgress) => OldColumn::InProgress,
        Ok(pb::BoardColumn::InReview) => OldColumn::InReview,
        Ok(pb::BoardColumn::Done) => OldColumn::Done,
        Ok(pb::BoardColumn::ToDo | pb::BoardColumn::Unspecified) | Err(_) => OldColumn::ToDo,
    }
}

fn source_anchor(wire: &pb::SourceAnchor) -> SourceAnchor {
    SourceAnchor {
        repo: wire.repo.clone(),
        path: wire.path.clone(),
        line: wire.line,
        end_line: (wire.end_line > 0).then_some(wire.end_line),
        comment_id: (!wire.comment_id.is_empty()).then(|| wire.comment_id.clone()),
    }
}

fn source_anchor_pb(anchor: &SourceAnchor) -> pb::SourceAnchor {
    pb::SourceAnchor {
        repo: anchor.repo.clone(),
        path: anchor.path.clone(),
        line: anchor.line,
        end_line: anchor.end_line.unwrap_or(0),
        comment_id: anchor.comment_id.clone().unwrap_or_default(),
    }
}

fn external_block(wire: &pb::ExternalBlock) -> ExternalBlock {
    ExternalBlock {
        reason: wire.reason.clone(),
        since: wire.since.as_ref().and_then(|at| day_of_secs(at.seconds)),
    }
}

fn external_block_pb(block: &ExternalBlock) -> pb::ExternalBlock {
    pb::ExternalBlock {
        reason: block.reason.clone(),
        since: None,
    }
}

fn comment(wire: &pb::TodoComment) -> TodoComment {
    TodoComment {
        id: wire.id.clone(),
        author: wire.author.clone(),
        body: wire.body.clone(),
        created: wire.created_at.as_ref().and_then(|at| day_of_secs(at.seconds)),
        step_id: (!wire.step_id.is_empty()).then(|| wire.step_id.clone()),
    }
}

fn owner(wire: &pb::TodoOwner) -> Option<TodoOwner> {
    let kind = match pb::TodoOwnerKind::try_from(wire.kind) {
        Ok(pb::TodoOwnerKind::User) => OwnerKind::User,
        Ok(pb::TodoOwnerKind::Agent) => OwnerKind::Agent,
        Ok(pb::TodoOwnerKind::Unspecified) | Err(_) => return None,
    };
    Some(TodoOwner {
        kind,
        label: wire.label.clone(),
        session: wire
            .session
            .as_ref()
            .map(|s| s.value.clone())
            .filter(|s| !s.is_empty()),
    })
}

fn owner_pb(o: &TodoOwner) -> pb::TodoOwner {
    pb::TodoOwner {
        kind: match o.kind {
            OwnerKind::User => pb::TodoOwnerKind::User as i32,
            OwnerKind::Agent => pb::TodoOwnerKind::Agent as i32,
        },
        label: o.label.clone(),
        session: o.session.as_ref().map(|s| pb::SessionId { value: s.clone() }),
    }
}

fn watched_pr(wire: &pb::WatchedPr) -> WatchedPr {
    let repo_pr = wire.pr.as_ref().map_or_else(String::new, |pr| {
        let repo = pr
            .repo
            .as_ref()
            .map_or_else(String::new, |r| format!("{}/{}", r.owner, r.name));
        format!("{repo}#{}", pr.number)
    });
    WatchedPr {
        repo_pr,
        title: wire.title.clone(),
        ci_state: wire.ci_state.clone(),
        unresolved_comments: wire.unresolved_comments,
        has_conflicts: wire.has_conflicts,
        upstream_merged: wire.upstream_merged,
        merged: wire.merged,
        closed_without_merge: wire.closed_without_merge,
        stack_key: (!wire.stack_key.is_empty()).then(|| wire.stack_key.clone()),
    }
}

fn watched_pr_pb(w: &WatchedPr) -> pb::WatchedPr {
    let pr = w.repo_pr.split_once('#').and_then(|(repo_part, num_part)| {
        let (owner, name) = repo_part.split_once('/')?;
        let number = num_part.parse::<u32>().ok()?;
        Some(pb::PullRequestRef {
            repo: Some(pb::RepoRef {
                owner: owner.to_owned(),
                name: name.to_owned(),
            }),
            number,
            note: String::new(),
        })
    });
    pb::WatchedPr {
        pr,
        title: w.title.clone(),
        ci_state: w.ci_state.clone(),
        unresolved_comments: w.unresolved_comments,
        has_conflicts: w.has_conflicts,
        upstream_merged: w.upstream_merged,
        merged: w.merged,
        closed_without_merge: w.closed_without_merge,
        stack_key: w.stack_key.clone().unwrap_or_default(),
    }
}

fn step_pb(step: &Step) -> pb::TodoStep {
    pb::TodoStep {
        text: step.text.clone(),
        done: step.done,
        ticked_by: None,
        id: step.id.clone(),
        markdown_context: step.markdown_context.clone().unwrap_or_default(),
        dropped: step.dropped,
        from_key: step.from_key.clone().unwrap_or_default(),
        source_anchor: step.source_anchor.as_ref().map(source_anchor_pb),
    }
}

fn word_diff_span(wire: &pb::WordDiffSpan) -> Option<WordDiffSpan> {
    let kind = match pb::WordDiffKind::try_from(wire.kind) {
        Ok(pb::WordDiffKind::Equal) => WordDiffKind::Equal,
        Ok(pb::WordDiffKind::Added) => WordDiffKind::Added,
        Ok(pb::WordDiffKind::Removed) => WordDiffKind::Removed,
        Ok(pb::WordDiffKind::Unspecified) | Err(_) => return None,
    };
    Some(WordDiffSpan {
        kind,
        text: wire.text.clone(),
    })
}

fn history_entry(wire: &pb::ItemHistoryEntry) -> TodoHistoryEntry {
    let kind = match pb::HistoryKind::try_from(wire.kind) {
        Ok(pb::HistoryKind::Created) => "created",
        Ok(pb::HistoryKind::Changed) => "changed",
        Ok(pb::HistoryKind::Moved) => "moved",
        Ok(pb::HistoryKind::Deleted) => "deleted",
        Ok(pb::HistoryKind::Sessions) => "sessions",
        Ok(pb::HistoryKind::Dependencies) => "dependencies",
        Ok(pb::HistoryKind::Commented) => "commented",
        Ok(pb::HistoryKind::Split) => "split",
        Ok(pb::HistoryKind::Folded) => "folded",
        Ok(pb::HistoryKind::StepChanged) => "step_changed",
        Ok(pb::HistoryKind::OwnerChanged) => "owner_changed",
        Ok(pb::HistoryKind::CheckpointChanged) => "checkpoint_changed",
        Ok(pb::HistoryKind::PrSynced) => "pr_synced",
        Ok(pb::HistoryKind::AskAnswered) => "ask_answered",
        Ok(pb::HistoryKind::Unspecified) | Err(_) => "changed",
    };
    TodoHistoryEntry {
        at_day: wire
            .at
            .as_ref()
            .and_then(|t| t.at.as_ref())
            .and_then(|ts| day_of_secs(ts.seconds)),
        actor: wire
            .at
            .as_ref()
            .and_then(|t| t.by.as_ref())
            .map(|a| format!("{a:?}"))
            .unwrap_or_default(),
        kind: kind.to_owned(),
        field: wire.field.clone(),
        before: wire.before.clone(),
        after: wire.after.clone(),
        change: wire.change.clone(),
        word_diff: wire.word_diff.iter().filter_map(word_diff_span).collect(),
    }
}

fn run_entry(wire: &pb::TodoRun) -> RunEntry {
    RunEntry {
        id: wire.id.clone(),
        tool: wire.tool.clone(),
        model: wire.model.clone(),
        outcome: wire.outcome.clone(),
        cost_micros: wire.cost_micros,
        started: wire.started_at.as_ref().and_then(|ts| day_of_secs(ts.seconds)),
        links: wire
            .links
            .iter()
            .map(|l| RunLink {
                label: l.label.clone(),
                target: l.target.clone(),
            })
            .collect(),
        session: wire
            .session
            .as_ref()
            .map(|s| s.value.clone())
            .filter(|s| !s.is_empty()),
    }
}

fn id_pb(value: &str) -> pb::BoardItemId {
    pb::BoardItemId {
        value: value.to_owned(),
    }
}

/// What an agent's pending ask is about, as the row's tag names it.
fn ask_kind(change: Option<&pb::BoardChangePreview>) -> AskKind {
    let Some(change) = change else {
        return AskKind::Close;
    };
    if change.deletes_item {
        return AskKind::Close;
    }
    let Some(first) = change.changes.first() else {
        return AskKind::Close;
    };
    let field = first.field.to_lowercase();
    let closes = matches!(field.as_str(), "column" | "status" | "state")
        && matches!(first.after.to_lowercase().as_str(), "done" | "closed");
    if closes {
        AskKind::Close
    } else {
        AskKind::Edit {
            field: first.field.clone(),
            before: first.before.clone(),
            after: first.after.clone(),
        }
    }
}

fn review(wire: &pb::TodoReview, now: i64) -> Review {
    Review {
        repo_prs: wire
            .pull_requests
            .iter()
            .map(|pr| {
                let repo = pr.repo.as_ref().map_or_else(String::new, |repo| format!("{}/{}", repo.owner, repo.name));
                format!("{repo}#{}", pr.number)
            })
            .collect(),
        author: wire.author.clone(),
        waiting_days: wire.requested_at.as_ref().map_or(0, |at| {
            u32::try_from((now - at.seconds).max(0) / DAY_SECS).unwrap_or(u32::MAX)
        }),
    }
}

/// An item as the app holds it. `today` dates an item the daemon sent no
/// creation time for; `now` is seconds since the epoch.
fn entry(wire: &pb::TodoItem, today: Day, now: i64) -> Entry {
    let item = wire.item.clone().unwrap_or_default();
    let fields = wire.fields.clone().unwrap_or_default();
    let mut todo = Todo::new(
        &fields.key,
        &item.title,
        item.created_at
            .as_ref()
            .and_then(|at| day_of_secs(at.seconds))
            .unwrap_or(today),
    );
    todo.channel = item
        .channel
        .as_ref()
        .map(|channel| channel.value.clone())
        .filter(|channel| !channel.is_empty());
    todo.sessions = item.sessions.iter().map(|session| session.value.clone()).collect();
    todo.state = state(fields.state);
    todo.due = day_of(fields.due.as_ref());
    todo.effort = effort(fields.effort);
    todo.impact = impact(fields.impact);
    todo.tags = fields.tags.clone();
    todo.steps = fields
        .steps
        .iter()
        .map(|step| Step {
            id: step.id.clone(),
            text: step.text.clone(),
            done: step.done,
            markdown_context: (!step.markdown_context.is_empty())
                .then(|| step.markdown_context.clone()),
            dropped: step.dropped,
            from_key: (!step.from_key.is_empty()).then(|| step.from_key.clone()),
            source_anchor: step.source_anchor.as_ref().map(source_anchor),
        })
        .collect();
    todo.hide_until = day_of(fields.hide_until.as_ref());
    // The wire counts pins from 1 and keeps 0 for unpinned.
    todo.pin = fields.pin_order.checked_sub(1);
    todo.depends_on = item
        .depends_on
        .iter()
        .filter_map(|edge| edge.id.as_ref().map(|id| id.value.clone()))
        .collect();
    todo.suggested = fields.suggested;
    todo.description = item.description.clone();
    todo.review = fields.review.as_ref().map(|wire| review(wire, now));
    todo.start = fields.start.as_ref().map(|start| Start {
        column: old_column(start.column),
        position: start.position,
        board: start
            .board_channel
            .as_ref()
            .map(|channel| channel.value.clone())
            .filter(|channel| !channel.is_empty()),
        vip: start.was_vip,
    });
    todo.closed = fields
        .closed_at
        .as_ref()
        .and_then(|at| day_of_secs(at.seconds));
    todo.need = need(fields.need);
    todo.fuzzy_due = fuzzy_due(fields.fuzzy_due);
    todo.external_block = fields.external_block.as_ref().map(external_block);
    todo.from_keys = fields.from_keys.clone();
    todo.became_keys = fields.became_keys.clone();
    todo.source_anchor = fields.source_anchor.as_ref().map(source_anchor);
    todo.comments = fields.comments.iter().map(comment).collect();
    todo.owner = fields.owner.as_ref().and_then(owner);
    todo.checkpoint = checkpoint(fields.checkpoint);
    todo.stuck_agent = fields.stuck_agent.as_ref().map(|s| StuckAgent {
        session: s.session.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
        agent_label: s.agent_label.clone(),
        reason: s.reason.clone(),
        since_minutes: s.since.as_ref().map_or(0, |at| {
            u32::try_from((now - at.seconds).max(0) / 60).unwrap_or(u32::MAX)
        }),
    });
    todo.blocker_gone = fields
        .blocker_gone
        .iter()
        .map(|b| BlockerGone {
            blocker_key: b.blocker_key.clone(),
            blocker_state: state(b.blocker_state),
            replacement_keys: b.replacement_keys.clone(),
        })
        .collect();
    todo.watched_prs = fields.watched_prs.iter().map(watched_pr).collect();
    todo.close_readiness = fields.close_readiness.as_ref().map(|cr| CloseReadiness {
        steps_done: cr.steps_done,
        steps_total: cr.steps_total,
        unresolved_comments: cr.unresolved_comments,
        ci_status: cr.ci_status.clone(),
        commits: cr.commits,
    });
    todo.session_signals = fields
        .session_signals
        .iter()
        .map(|s| SessionSignal {
            session: s.session.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
            label: s.label.clone(),
            status: s.status.clone(),
            idle_days: s.idle_days,
            active_turn: s.active_turn,
        })
        .collect();
    todo.lifecycle_prompts = fields
        .lifecycle_prompts
        .iter()
        .map(|p| LifecyclePrompt {
            id: p.id.clone(),
            kind: p.kind.clone(),
            title: p.title.clone(),
            detail: p.detail.clone(),
            choices: p.choices.clone(),
            pr_ref: (!p.pr_ref.is_empty()).then(|| p.pr_ref.clone()),
            session: p
                .session
                .as_ref()
                .map(|id| id.value.clone())
                .filter(|s| !s.is_empty()),
        })
        .collect();
    todo.run_count = fields.run_count;
    todo.total_cost_micros = fields.total_cost_micros;
    todo.wont_do_reason = (!fields.wont_do_reason.is_empty()).then(|| fields.wont_do_reason.clone());
    let pending = item.pending.first();
    todo.ask = pending.map(|ask| Ask {
        kind: ask_kind(ask.change.as_ref()),
        by: String::new(),
        reason: ask.reason.clone(),
    });
    Entry {
        todo,
        revision: item.revision.as_ref().map_or(0, |revision| revision.value),
        ask_id: pending
            .and_then(|ask| ask.request.as_ref())
            .map(|request| request.value.clone())
            .filter(|id| !id.is_empty()),
    }
}

/// One line of a card: its bold lead and the rest, run together.
fn card_text(lead: &str, rest: &str) -> String {
    format!("{lead}{rest}").trim().to_owned()
}

/// What backs an ask, from the Inbox card it waits on. The card doesn't
/// say whether the asking session has ended, so that is left false.
fn ask_detail(request: &pb::InboxRequest, now: i64) -> AskDetail {
    AskDetail {
        evidence: request
            .facts
            .iter()
            .map(|fact| (fact, card_text(&fact.lead, &fact.rest)))
            .filter(|(_, text)| !text.is_empty())
            .map(|(fact, text)| Evidence {
                text: if fact.label.is_empty() { text } else { format!("{}: {text}", fact.label) },
                link: None,
            })
            .collect(),
        affects: request
            .affects
            .iter()
            .map(|effect| card_text(&effect.lead, &effect.rest))
            .filter(|text| !text.is_empty())
            .collect(),
        change: None,
        asked_minutes_ago: request.created_at.as_ref().map_or(0, |at| {
            u32::try_from((now - at.seconds).max(0) / 60).unwrap_or(u32::MAX)
        }),
        session_ended: false,
    }
}

fn filter_pb(filter: &TodoFilter) -> pb::TodoFilter {
    pb::TodoFilter {
        channel: match &filter.channel {
            Some(ChannelFilter::Channel(id)) => Some(pb::ChannelId { value: id.clone() }),
            Some(ChannelFilter::NoChannel) | None => None,
        },
        no_channel: matches!(filter.channel, Some(ChannelFilter::NoChannel)),
        session: filter
            .session
            .as_ref()
            .map(|session| pb::SessionId { value: session.clone() }),
        show_closed: filter.show_closed,
    }
}

fn create_pb(new: NewTodo) -> pb::CreateTodoRequest {
    pb::CreateTodoRequest {
        idempotency_key: fresh_key(),
        title: new.title,
        channel: new.channel.map(|value| pb::ChannelId { value }),
        description: new.description,
        due: new.due.map(day_pb),
        effort: effort_pb(new.effort) as i32,
        impact: impact_pb(new.impact) as i32,
        tags: new.tags,
        need: need_pb(new.need) as i32,
        fuzzy_due: fuzzy_due_pb(new.fuzzy_due) as i32,
        source_anchor: new.source_anchor.as_ref().map(source_anchor_pb),
        from_keys: new.from_keys,
        external_block: new.external_block.as_ref().map(external_block_pb),
        ..pb::CreateTodoRequest::default()
    }
}

fn patch_pb(patch: TodoPatch) -> pb::TodoPatch {
    let (due, clear_due) = optional_day(patch.due);
    let (hide_until, clear_hide_until) = optional_day(patch.hide_until);
    let (channel, clear_channel) = match patch.channel {
        None => (None, false),
        Some(None) => (None, true),
        Some(Some(value)) => (Some(pb::ChannelId { value }), false),
    };
    let (external_block, clear_external_block) = match patch.external_block {
        None => (None, false),
        Some(None) => (None, true),
        Some(Some(block)) => (Some(external_block_pb(&block)), false),
    };
    let (owner, clear_owner) = match patch.owner {
        None => (None, false),
        Some(None) => (None, true),
        Some(Some(o)) => (Some(owner_pb(&o)), false),
    };
    pb::TodoPatch {
        title: patch.title,
        description: patch.description,
        state: patch.state.map(|state| state_pb(state) as i32),
        due,
        clear_due,
        effort: patch.effort.map(|effort| effort_pb(effort) as i32),
        impact: patch.impact.map(|impact| impact_pb(impact) as i32),
        tags_add: patch.tags_add,
        tags_remove: patch.tags_remove,
        steps: patch.steps.map(|steps| pb::TodoSteps {
            steps: steps.iter().map(step_pb).collect(),
        }),
        hide_until,
        clear_hide_until,
        // The app pins from 0, the wire from 1; 0 on the wire unpins.
        pin_order: patch.pin.map(|pin| pin.map_or(0, |at| at.saturating_add(1))),
        channel,
        clear_channel,
        depends_on_add: patch.depends_on_add.iter().map(|k| id_pb(k)).collect(),
        depends_on_remove: patch.depends_on_remove.iter().map(|k| id_pb(k)).collect(),
        accept_suggestion: patch.accept_suggestion,
        need: patch.need.map(|n| need_pb(n) as i32),
        fuzzy_due: patch.fuzzy_due.map(|f| fuzzy_due_pb(f) as i32),
        external_block,
        clear_external_block,
        owner,
        clear_owner,
        checkpoint: patch.checkpoint.map(|c| checkpoint_pb(c) as i32),
        wont_do_reason: patch.wont_do_reason,
        watched_prs_add: patch.watched_prs_add.iter().map(watched_pr_pb).collect(),
        ..pb::TodoPatch::default()
    }
}

/// A day to set, or the flag that takes it off.
fn optional_day(day: Option<Option<Day>>) -> (Option<pb::CivilDay>, bool) {
    match day {
        None => (None, false),
        Some(None) => (None, true),
        Some(Some(day)) => (Some(day_pb(day)), false),
    }
}

fn summary(wire: &pb::TodoMigration) -> Summary {
    let count = |value: u32| usize::try_from(value).unwrap_or(usize::MAX);
    Summary {
        items: count(wire.items),
        open: count(wire.open),
        in_progress: count(wire.in_progress),
        closed: count(wire.closed),
        boards: count(wire.boards),
    }
}

/// The notice the wire holds: none before the move ran or once dismissed.
/// The banner to show, if any: a move that found nothing to move (a fresh
/// install) has nothing to announce.
fn notice(wire: &pb::TodoMigration) -> Option<MigrationNotice> {
    (wire.migrated_at.is_some() && !wire.banner_dismissed && wire.items > 0).then(|| MigrationNotice {
        summary: summary(wire),
    })
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX))
}

fn expected(revision: u64) -> Option<pb::Revision> {
    (revision > 0).then_some(pb::Revision { value: revision })
}

// ---------------------------------------------------------------------------
// Calls.

const WHAT: &str = "The to-do list";

async fn list_all(conn: &LiveConnection, filter: &TodoFilter) -> Result<Vec<Entry>, DaemonError> {
    let now = now_secs();
    let today = today(now)?;
    let wire_filter = filter_pb(filter);
    let mut entries = Vec::new();
    let mut token = String::new();
    loop {
        let reply = client(conn)
            .list_todo(pb::ListTodoRequest {
                filter: Some(wire_filter.clone()),
                page: Some(pb::PageRequest {
                    page_size: PAGE_SIZE,
                    page_token: token,
                }),
            })
            .await
            .map_err(|status| conn.error(&status, WHAT))?
            .into_inner();
        entries.extend(reply.items.iter().map(|item| entry(item, today, now)));
        token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
        if token.is_empty() {
            break;
        }
    }
    if entries.iter().any(|entry| entry.ask_id.is_some()) {
        let details = ask_details(conn, now).await?;
        for entry in &mut entries {
            if let Some(detail) = entry.ask_id.as_ref().and_then(|id| details.get(id)) {
                entry.todo.ask_detail = detail.clone();
            }
        }
    }
    Ok(entries)
}

/// The detail of every pending ask, by request id, in one pass over the
/// Inbox.
async fn ask_details(
    conn: &LiveConnection,
    now: i64,
) -> Result<std::collections::BTreeMap<String, AskDetail>, DaemonError> {
    let mut details = std::collections::BTreeMap::new();
    let mut token = String::new();
    loop {
        let reply = inbox(conn)
            .list_requests(pb::ListRequestsRequest {
                scope: Some(pb::InboxScope {
                    scope: Some(pb::inbox_scope::Scope::Global(pb::GlobalInbox {})),
                }),
                include_answered: false,
                agent_inbox: false,
                page: Some(pb::PageRequest {
                    page_size: PAGE_SIZE,
                    page_token: token,
                }),
            })
            .await
            .map_err(|status| conn.error(&status, "The asks on the to-do list"))?
            .into_inner();
        for request in &reply.requests {
            if let Some(id) = request.id.as_ref() {
                details.insert(id.value.clone(), ask_detail(request, now));
            }
        }
        token = reply.page.map(|page| page.next_page_token).unwrap_or_default();
        if token.is_empty() {
            return Ok(details);
        }
    }
}

/// The day `now` falls on, for dating an item the daemon sent no
/// creation time for.
fn today(now: i64) -> Result<Day, DaemonError> {
    day_of_secs(now).ok_or_else(|| DaemonError::Refused {
        what: WHAT.to_owned(),
        reason: "this computer's clock is outside the calendar. Set the clock right, then reload".to_owned(),
    })
}

/// The item a write answered with.
fn written(reply: pb::WriteTodoResponse, what: &str) -> Result<Entry, DaemonError> {
    let now = now_secs();
    let today = today(now)?;
    match reply.item {
        Some(item) => Ok(entry(&item, today, now)),
        None => Err(DaemonError::Refused {
            what: what.to_owned(),
            reason: "the daemon took it but didn't send the item back, so it may be waiting to sync. Reload the list to see whether it is there".to_owned(),
        }),
    }
}

/// Opens the feed narrowed to boards and the Inbox. It is open before
/// this answers, so a change made after it is not missed.
async fn watch_todo(conn: &LiveConnection) -> Result<TodoLists, DaemonError> {
    let stream = event_stream(conn)
        .subscribe(pb::SubscribeRequest {
            after: None,
            channels: Vec::new(),
            sessions: Vec::new(),
            topics: vec![pb::Topic::Boards as i32, pb::Topic::Inbox as i32],
        })
        .await
        .map_err(|status| conn.error(&status, "Following the to-do list"))?
        .into_inner();
    let (sender, receiver) = mpsc::channel(QUEUE);
    conn.spawn(forward(conn.clone(), stream, sender));
    Ok(receiver)
}

/// Reads the whole list after each item or ask event until the feed ends
/// or nobody listens.
async fn forward(conn: LiveConnection, mut stream: Streaming<pb::Event>, sender: mpsc::Sender<Vec<Entry>>) {
    let all = TodoFilter {
        show_closed: true,
        ..TodoFilter::default()
    };
    loop {
        let next = tokio::select! {
            () = sender.closed() => return,
            next = stream.message() => next,
        };
        match next {
            Ok(Some(event)) => {
                if !matches!(
                    event.payload,
                    Some(pb::event::Payload::BoardItem(_) | pb::event::Payload::Request(_))
                ) {
                    continue;
                }
                let Ok(current) = list_all(&conn, &all).await else {
                    return;
                };
                if sender.send(current).await.is_err() {
                    return;
                }
            }
            Ok(None) | Err(_) => return,
        }
    }
}

impl TodoClient for LiveTodo {
    fn list(&self, filter: &TodoFilter) -> Reply<Vec<Entry>> {
        let conn = self.conn.clone();
        let filter = filter.clone();
        Box::pin(async move { list_all(&conn, &filter).await })
    }

    fn create(&self, draft: NewTodo) -> Reply<Entry> {
        let conn = self.conn.clone();
        let request = create_pb(draft);
        Box::pin(async move {
            const WHAT: &str = "The new to-do";
            let reply = client(&conn)
                .create_todo(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            written(reply, WHAT)
        })
    }

    fn update(&self, key: &str, patch: TodoPatch, expected_revision: u64) -> Reply<Entry> {
        let conn = self.conn.clone();
        let request = pb::UpdateTodoRequest {
            idempotency_key: fresh_key(),
            id: Some(id_pb(key)),
            patch: Some(patch_pb(patch)),
            expected_revision: expected(expected_revision),
            reason: String::new(),
            evidence: Vec::new(),
        };
        Box::pin(async move {
            const WHAT: &str = "This to-do";
            let reply = client(&conn)
                .update_todo(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            written(reply, WHAT)
        })
    }

    fn answer(&self, ask_id: &str, yes: bool) -> Reply<()> {
        let conn = self.conn.clone();
        let id = pb::RequestId {
            value: ask_id.to_owned(),
        };
        Box::pin(async move {
            use pb::request_answer::Answer as A;
            let answer = if yes {
                A::Approve(pb::Approve {})
            } else {
                A::Deny(pb::Deny { note: String::new() })
            };
            inbox(&conn)
                .answer_request(pb::AnswerRequestRequest {
                    idempotency_key: fresh_key(),
                    id: Some(id),
                    answer: Some(pb::RequestAnswer { answer: Some(answer) }),
                    expected_revision: None,
                    presence: None,
                })
                .await
                .map_err(|status| conn.error(&status, "Answering the ask"))?;
            Ok(())
        })
    }

    fn migration(&self) -> Reply<Option<MigrationNotice>> {
        let conn = self.conn.clone();
        Box::pin(async move {
            let reply = client(&conn)
                .get_todo_migration(pb::GetTodoMigrationRequest {})
                .await
                .map_err(|status| conn.error(&status, "The to-do move"))?
                .into_inner();
            Ok(notice(&reply))
        })
    }

    fn dismiss_migration(&self) -> Reply<()> {
        let conn = self.conn.clone();
        Box::pin(async move {
            client(&conn)
                .dismiss_todo_migration(pb::DismissTodoMigrationRequest {})
                .await
                .map_err(|status| conn.error(&status, "Dismissing the to-do move"))?;
            Ok(())
        })
    }

    fn watch(&self) -> Reply<TodoLists> {
        let conn = self.conn.clone();
        Box::pin(async move { watch_todo(&conn).await })
    }

    fn add_comment(
        &self,
        key: &str,
        body: String,
        step_id: Option<String>,
    ) -> Reply<(Entry, TodoComment)> {
        let conn = self.conn.clone();
        let request = pb::AddTodoCommentRequest {
            idempotency_key: fresh_key(),
            id: Some(id_pb(key)),
            body,
            step_id: step_id.unwrap_or_default(),
        };
        Box::pin(async move {
            const WHAT: &str = "Adding a comment";
            let now = now_secs();
            let today = today(now)?;
            let reply = client(&conn)
                .add_todo_comment(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            let item = reply.item.ok_or_else(|| DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon did not return the updated item".to_owned(),
            })?;
            let cmt = reply.comment.as_ref().map(comment).unwrap_or_default();
            Ok((entry(&item, today, now), cmt))
        })
    }

    fn split(&self, key: &str, titles: Vec<String>, keep_age: bool) -> Reply<SplitResult> {
        let conn = self.conn.clone();
        let request = pb::SplitTodoRequest {
            idempotency_key: fresh_key(),
            id: Some(id_pb(key)),
            titles,
            keep_age,
        };
        Box::pin(async move {
            const WHAT: &str = "Splitting the to-do";
            let now = now_secs();
            let today = today(now)?;
            let reply = client(&conn)
                .split_todo(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            let source = reply.source.ok_or_else(|| DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon did not return the split source item".to_owned(),
            })?;
            Ok(SplitResult {
                source: entry(&source, today, now),
                created: reply.created.iter().map(|i| entry(i, today, now)).collect(),
            })
        })
    }

    fn fold_into_step(&self, source_key: &str, target_key: &str) -> Reply<FoldResult> {
        let conn = self.conn.clone();
        let request = pb::FoldTodoIntoStepRequest {
            idempotency_key: fresh_key(),
            source_id: Some(id_pb(source_key)),
            target_id: Some(id_pb(target_key)),
        };
        Box::pin(async move {
            const WHAT: &str = "Folding the to-do into a step";
            let now = now_secs();
            let today = today(now)?;
            let reply = client(&conn)
                .fold_todo_into_step(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            let source = reply.source.ok_or_else(|| DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon did not return the folded source item".to_owned(),
            })?;
            let target = reply.target.ok_or_else(|| DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon did not return the target item".to_owned(),
            })?;
            Ok(FoldResult {
                source: entry(&source, today, now),
                target: entry(&target, today, now),
            })
        })
    }

    fn bulk_update(&self, items: Vec<BulkUpdateItem>) -> Reply<BulkUpdateResult> {
        let conn = self.conn.clone();
        let entries = items
            .into_iter()
            .map(|item| pb::BatchUpdateTodoEntry {
                id: Some(id_pb(&item.key)),
                patch: Some(patch_pb(item.patch)),
                expected_revision: expected(item.expected_revision),
            })
            .collect();
        let request = pb::BatchUpdateTodoRequest {
            idempotency_key: fresh_key(),
            entries,
        };
        Box::pin(async move {
            const WHAT: &str = "Bulk updating to-dos";
            let now = now_secs();
            let today = today(now)?;
            let reply = client(&conn)
                .batch_update_todo(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            Ok(BulkUpdateResult {
                updated: reply.updated.iter().map(|i| entry(i, today, now)).collect(),
                failed: reply
                    .failed
                    .into_iter()
                    .map(|f| BulkUpdateFailure {
                        key: if f.key.is_empty() {
                            f.id.map(|id| id.value).unwrap_or_default()
                        } else {
                            f.key
                        },
                        reason: f.reason,
                    })
                    .collect(),
            })
        })
    }

    fn history(&self, key: &str) -> Reply<Vec<TodoHistoryEntry>> {
        let conn = self.conn.clone();
        let key = key.to_owned();
        Box::pin(async move {
            let reply = client(&conn)
                .get_todo_history(pb::GetTodoHistoryRequest {
                    id: Some(id_pb(&key)),
                    page: Some(pb::PageRequest {
                        page_size: PAGE_SIZE,
                        page_token: String::new(),
                    }),
                })
                .await
                .map_err(|status| conn.error(&status, "Loading to-do history"))?
                .into_inner();
            Ok(reply.entries.iter().map(history_entry).collect())
        })
    }

    fn runs(&self, key: &str) -> Reply<TodoRunsResult> {
        let conn = self.conn.clone();
        let key = key.to_owned();
        Box::pin(async move {
            let reply = client(&conn)
                .list_todo_runs(pb::ListTodoRunsRequest {
                    id: Some(id_pb(&key)),
                })
                .await
                .map_err(|status| conn.error(&status, "Loading to-do runs"))?
                .into_inner();
            Ok(TodoRunsResult {
                runs: reply.runs.iter().map(run_entry).collect(),
                total_cost_micros: reply.total_cost_micros,
            })
        })
    }

    fn answer_question(
        &self,
        ask_id: &str,
        text: String,
        selected_choices: Vec<String>,
    ) -> Reply<()> {
        let conn = self.conn.clone();
        let id = pb::RequestId { value: ask_id.to_owned() };
        Box::pin(async move {
            inbox(&conn)
                .answer_request(pb::AnswerRequestRequest {
                    idempotency_key: fresh_key(),
                    id: Some(id),
                    answer: Some(pb::RequestAnswer {
                        answer: Some(pb::request_answer::Answer::Reply(pb::Reply {
                            text,
                            selected_choices,
                        })),
                    }),
                    expected_revision: None,
                    presence: None,
                })
                .await
                .map_err(|status| conn.error(&status, "Answering the question"))?;
            Ok(())
        })
    }

    fn answer_action(&self, ask_id: &str, done: bool, note: String) -> Reply<()> {
        let conn = self.conn.clone();
        let id = pb::RequestId { value: ask_id.to_owned() };
        Box::pin(async move {
            inbox(&conn)
                .answer_request(pb::AnswerRequestRequest {
                    idempotency_key: fresh_key(),
                    id: Some(id),
                    answer: Some(pb::RequestAnswer {
                        answer: Some(pb::request_answer::Answer::TodoAction(pb::TodoActionAnswer {
                            done,
                            note,
                        })),
                    }),
                    expected_revision: None,
                    presence: None,
                })
                .await
                .map_err(|status| conn.error(&status, "Answering the action request"))?;
            Ok(())
        })
    }

    fn answer_review(&self, ask_id: &str, approved: bool, note: String) -> Reply<()> {
        let conn = self.conn.clone();
        let id = pb::RequestId { value: ask_id.to_owned() };
        Box::pin(async move {
            inbox(&conn)
                .answer_request(pb::AnswerRequestRequest {
                    idempotency_key: fresh_key(),
                    id: Some(id),
                    answer: Some(pb::RequestAnswer {
                        answer: Some(pb::request_answer::Answer::TodoReview(pb::TodoReviewAnswer {
                            approved,
                            note,
                        })),
                    }),
                    expected_revision: None,
                    presence: None,
                })
                .await
                .map_err(|status| conn.error(&status, "Answering the review request"))?;
            Ok(())
        })
    }

    fn resolve_ownership(&self, key: &str, take_back: bool, retry: bool) -> Reply<Entry> {
        let conn = self.conn.clone();
        let request = pb::ResolveTodoOwnershipRequest {
            idempotency_key: fresh_key(),
            id: Some(id_pb(key)),
            take_back,
            retry,
        };
        Box::pin(async move {
            const WHAT: &str = "Resolving to-do ownership";
            let reply = client(&conn)
                .resolve_todo_ownership(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            written(reply, WHAT)
        })
    }

    fn resolve_blocker_gone(
        &self,
        key: &str,
        blocker_key: &str,
        resolution: BlockerGoneResolution,
    ) -> Reply<Entry> {
        let conn = self.conn.clone();
        let (action, rewire_to_keys) = match resolution {
            BlockerGoneResolution::Unblock => ("unblock".to_owned(), Vec::new()),
            BlockerGoneResolution::CloseWontDo => ("wont_do".to_owned(), Vec::new()),
            BlockerGoneResolution::Rewire(keys) => ("rewire".to_owned(), keys),
        };
        let request = pb::ResolveBlockerGoneRequest {
            idempotency_key: fresh_key(),
            id: Some(id_pb(key)),
            blocker_key: blocker_key.to_owned(),
            action,
            rewire_to_keys,
        };
        Box::pin(async move {
            const WHAT: &str = "Resolving a gone blocker";
            let reply = client(&conn)
                .resolve_blocker_gone(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            written(reply, WHAT)
        })
    }

    fn resolve_lifecycle_prompt(&self, key: &str, prompt_id: &str, choice: &str) -> Reply<Entry> {
        let conn = self.conn.clone();
        let request = pb::ResolveLifecyclePromptRequest {
            idempotency_key: fresh_key(),
            id: Some(id_pb(key)),
            prompt_id: prompt_id.to_owned(),
            choice: choice.to_owned(),
        };
        Box::pin(async move {
            const WHAT: &str = "Resolving a lifecycle prompt";
            let reply = client(&conn)
                .resolve_lifecycle_prompt(request)
                .await
                .map_err(|status| conn.error(&status, WHAT))?
                .into_inner();
            written(reply, WHAT)
        })
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn day(y: i32, m: u32, d: u32) -> Day {
        Day::from_ymd(y, m, d).expect("a real date")
    }

    fn wire_item(fields: pb::TodoFields) -> pb::TodoItem {
        pb::TodoItem {
            item: Some(pb::BoardItem {
                title: "Pay".to_owned(),
                revision: Some(pb::Revision { value: 4 }),
                ..pb::BoardItem::default()
            }),
            fields: Some(fields),
        }
    }

    #[test]
    fn a_day_survives_the_wire() {
        for d in [day(2026, 11, 19), day(2028, 2, 29), day(1970, 1, 1), day(1969, 12, 31)] {
            assert_eq!(day_of(Some(&day_pb(d))), Some(d));
        }
    }

    #[test]
    fn an_unset_or_impossible_day_is_none() {
        assert_eq!(day_of(None), None);
        assert_eq!(
            day_of(Some(&pb::CivilDay { year: 2026, month: 2, day: 30 })),
            None
        );
    }

    #[test]
    fn seconds_land_on_their_utc_day() {
        assert_eq!(day_of_secs(0), Some(day(1970, 1, 1)));
        assert_eq!(day_of_secs(86_399), Some(day(1970, 1, 1)));
        assert_eq!(day_of_secs(86_400), Some(day(1970, 1, 2)));
        assert_eq!(day_of_secs(-1), Some(day(1969, 12, 31)));
    }

    #[test]
    fn pins_count_from_one_on_the_wire() {
        let unpinned = entry(&wire_item(pb::TodoFields::default()), day(2026, 1, 1), 0);
        assert_eq!(unpinned.todo.pin, None);
        let first = entry(
            &wire_item(pb::TodoFields { pin_order: 1, ..pb::TodoFields::default() }),
            day(2026, 1, 1),
            0,
        );
        assert_eq!(first.todo.pin, Some(0));
        let patch = patch_pb(TodoPatch { pin: Some(Some(0)), ..TodoPatch::default() });
        assert_eq!(patch.pin_order, Some(1));
        let unpin = patch_pb(TodoPatch { pin: Some(None), ..TodoPatch::default() });
        assert_eq!(unpin.pin_order, Some(0));
        assert_eq!(patch_pb(TodoPatch::default()).pin_order, None);
    }

    #[test]
    fn an_item_without_optional_fields_reads_as_a_plain_open_one() {
        let made = entry(&wire_item(pb::TodoFields::default()), day(2026, 1, 1), 0);
        assert_eq!(made.todo.state, State::Open);
        assert_eq!(made.todo.due, None);
        assert_eq!(made.todo.effort, None);
        assert_eq!(made.todo.impact, None);
        assert_eq!(made.todo.channel, None);
        assert_eq!(made.todo.created, day(2026, 1, 1));
        assert_eq!(made.revision, 4);
        assert_eq!(made.ask_id, None);
    }

    #[test]
    fn unknown_enum_values_read_as_unset() {
        let made = entry(
            &wire_item(pb::TodoFields {
                state: 99,
                effort: 99,
                impact: 99,
                ..pb::TodoFields::default()
            }),
            day(2026, 1, 1),
            0,
        );
        assert_eq!(made.todo.state, State::Open);
        assert_eq!(made.todo.effort, None);
        assert_eq!(made.todo.impact, None);
    }

    #[test]
    fn the_to_do_fields_come_across() {
        let made = entry(
            &wire_item(pb::TodoFields {
                key: "PAY-3".to_owned(),
                state: pb::TodoState::InProgress as i32,
                due: Some(day_pb(day(2026, 11, 23))),
                effort: pb::TodoEffort::L as i32,
                impact: pb::TodoImpact::High as i32,
                tags: vec!["money".to_owned()],
                steps: vec![pb::TodoStep {
                    text: "a".to_owned(),
                    done: true,
                    ticked_by: None,
                    ..pb::TodoStep::default()
                }],
                hide_until: Some(day_pb(day(2026, 12, 1))),
                suggested: true,
                ..pb::TodoFields::default()
            }),
            day(2026, 1, 1),
            0,
        );
        let todo = made.todo;
        assert_eq!(todo.key, "PAY-3");
        assert_eq!(todo.state, State::InProgress);
        assert_eq!(todo.due, Some(day(2026, 11, 23)));
        assert_eq!(todo.effort, Some(Effort::L));
        assert_eq!(todo.impact, Some(Impact::High));
        assert_eq!(todo.tags, ["money"]);
        assert_eq!(todo.steps, [Step { text: "a".to_owned(), done: true, ..Step::default() }]);
        assert_eq!(todo.hide_until, Some(day(2026, 12, 1)));
        assert!(todo.suggested);
    }

    #[test]
    fn a_pending_ask_carries_its_request_id() {
        let mut item = wire_item(pb::TodoFields::default());
        if let Some(board) = item.item.as_mut() {
            board.pending = vec![pb::PendingAsk {
                request: Some(pb::RequestId { value: "rq_1".to_owned() }),
                reason: "done".to_owned(),
                change: Some(pb::BoardChangePreview {
                    changes: vec![pb::FieldChange {
                        field: "due".to_owned(),
                        before: "a".to_owned(),
                        after: "b".to_owned(),
                    }],
                    ..pb::BoardChangePreview::default()
                }),
                ..pb::PendingAsk::default()
            }];
        }
        let made = entry(&item, day(2026, 1, 1), 0);
        assert_eq!(made.ask_id.as_deref(), Some("rq_1"));
        assert!(matches!(made.todo.ask.map(|ask| ask.kind), Some(AskKind::Edit { .. })));
    }

    #[test]
    fn an_ask_detail_reads_facts_effects_and_age_from_its_card() {
        let request = pb::InboxRequest {
            facts: vec![
                pb::CardFact { label: "Carries".to_owned(), lead: "3 commits".to_owned(), rest: String::new() },
                pb::CardFact { label: String::new(), lead: String::new(), rest: String::new() },
            ],
            affects: vec![pb::CardEffect { warn: true, lead: "Closes ".to_owned(), rest: "PAY-4".to_owned() }],
            created_at: Some(crate::humata2_grpc::Timestamp { seconds: 1_000, nanos: 0 }),
            ..pb::InboxRequest::default()
        };
        let detail = ask_detail(&request, 1_000 + 125);
        assert_eq!(detail.evidence.len(), 1);
        assert!(detail.evidence[0].text.contains("3 commits"));
        assert_eq!(detail.affects, ["Closes PAY-4"]);
        assert_eq!(detail.asked_minutes_ago, 2);
        assert_eq!(ask_detail(&pb::InboxRequest::default(), 5), AskDetail::default());
    }

    #[test]
    fn a_description_travels_both_ways() {
        let mut item = wire_item(pb::TodoFields::default());
        if let Some(board) = item.item.as_mut() {
            board.description = "long text".to_owned();
        }
        assert_eq!(entry(&item, day(2026, 1, 1), 0).todo.description, "long text");
        let patch = patch_pb(TodoPatch { description: Some("x".to_owned()), ..TodoPatch::default() });
        assert_eq!(patch.description.as_deref(), Some("x"));
        let new = create_pb(NewTodo { description: "d".to_owned(), ..NewTodo::default() });
        assert_eq!(new.description, "d");
    }

    #[test]
    fn a_patch_sets_or_clears_each_optional_field() {
        let set = patch_pb(TodoPatch {
            due: Some(Some(day(2026, 11, 23))),
            effort: Some(Some(Effort::S)),
            impact: Some(None),
            hide_until: Some(None),
            ..TodoPatch::default()
        });
        assert_eq!(day_of(set.due.as_ref()), Some(day(2026, 11, 23)));
        assert!(!set.clear_due);
        assert_eq!(set.effort, Some(pb::TodoEffort::S as i32));
        assert_eq!(set.impact, Some(pb::TodoImpact::Unspecified as i32));
        assert!(set.clear_hide_until);
        assert!(set.hide_until.is_none());

        let clear = patch_pb(TodoPatch { due: Some(None), ..TodoPatch::default() });
        assert!(clear.clear_due && clear.due.is_none());
        let untouched = patch_pb(TodoPatch::default());
        assert!(!untouched.clear_due && untouched.due.is_none() && untouched.effort.is_none());
        assert!(untouched.steps.is_none());
    }

    #[test]
    fn a_new_item_without_a_due_date_sends_none() {
        let wire = create_pb(NewTodo {
            title: "t".to_owned(),
            channel: Some("ch_1".to_owned()),
            effort: Some(Effort::M),
            ..NewTodo::default()
        });
        assert!(wire.due.is_none());
        assert_eq!(wire.channel.map(|c| c.value), Some("ch_1".to_owned()));
        assert_eq!(wire.effort, pb::TodoEffort::M as i32);
        assert_eq!(wire.impact, pb::TodoImpact::Unspecified as i32);
        let none = create_pb(NewTodo::default());
        assert!(none.channel.is_none());
    }

    #[test]
    fn the_filter_names_a_channel_or_none() {
        let channel = filter_pb(&TodoFilter {
            channel: Some(ChannelFilter::Channel("c".to_owned())),
            ..TodoFilter::default()
        });
        assert_eq!(channel.channel.map(|c| c.value), Some("c".to_owned()));
        assert!(!channel.no_channel);
        let none = filter_pb(&TodoFilter {
            channel: Some(ChannelFilter::NoChannel),
            show_closed: true,
            ..TodoFilter::default()
        });
        assert!(none.channel.is_none() && none.no_channel && none.show_closed);
    }

    #[test]
    fn the_migration_notice_shows_only_after_the_move_and_before_dismissal() {
        let mut wire = pb::TodoMigration { items: 3, open: 2, closed: 1, boards: 2, ..pb::TodoMigration::default() };
        assert!(notice(&wire).is_none());
        wire.migrated_at = Some(crate::humata2_grpc::Timestamp { seconds: 1, nanos: 0 });
        assert_eq!(notice(&wire).map(|n| n.summary.items), Some(3));
        wire.banner_dismissed = true;
        assert!(notice(&wire).is_none());
    }

    #[test]
    fn todo2_fields_and_patches_round_trip_wire_mapping() {
        let made = entry(
            &wire_item(pb::TodoFields {
                key: "PAY-99".to_owned(),
                state: pb::TodoState::WatchingPr as i32,
                need: pb::TodoNeed::Critical as i32,
                fuzzy_due: pb::TodoFuzzyDue::NextWeek as i32,
                checkpoint: pb::TodoCheckpoint::Defined as i32,
                from_keys: vec!["PAY-1".to_owned()],
                became_keys: vec!["PAY-100".to_owned()],
                run_count: 3,
                total_cost_micros: Some(2_500_000),
                wont_do_reason: "superseded".to_owned(),
                external_block: Some(pb::ExternalBlock {
                    reason: "Waiting on vendor".to_owned(),
                    since: None,
                }),
                owner: Some(pb::TodoOwner {
                    kind: pb::TodoOwnerKind::Agent as i32,
                    label: "claude".to_owned(),
                    session: Some(pb::SessionId { value: "s-1".to_owned() }),
                }),
                close_readiness: Some(pb::CloseReadiness {
                    steps_done: 2,
                    steps_total: 3,
                    unresolved_comments: 1,
                    ci_status: "passing".to_owned(),
                    commits: 4,
                }),
                ..pb::TodoFields::default()
            }),
            day(2026, 11, 19),
            0,
        );
        let t = made.todo;
        assert_eq!(t.state, State::WatchingPr);
        assert_eq!(t.need, Some(Need::Critical));
        assert_eq!(t.fuzzy_due, Some(FuzzyDue::NextWeek));
        assert_eq!(t.checkpoint, Some(CheckpointState::Defined));
        assert_eq!(t.from_keys, ["PAY-1"]);
        assert_eq!(t.became_keys, ["PAY-100"]);
        assert_eq!(t.run_count, 3);
        assert_eq!(t.total_cost_micros, Some(2_500_000));
        assert_eq!(t.wont_do_reason.as_deref(), Some("superseded"));
        assert_eq!(t.external_block.as_ref().map(|b| b.reason.as_str()), Some("Waiting on vendor"));
        assert_eq!(t.owner.as_ref().map(|o| o.kind), Some(OwnerKind::Agent));
        assert_eq!(
            t.close_readiness.as_ref().map(CloseReadiness::summary_line).as_deref(),
            Some("Steps 2/3 · 1 unresolved comment · CI passing · 4 commits")
        );

        let patch = patch_pb(TodoPatch {
            need: Some(Some(Need::Expected)),
            fuzzy_due: Some(Some(FuzzyDue::InAFewDays)),
            checkpoint: Some(Some(CheckpointState::Done)),
            external_block: Some(None),
            owner: Some(None),
            channel: Some(Some("infra".to_owned())),
            depends_on_add: vec!["INF-1".to_owned()],
            ..TodoPatch::default()
        });
        assert_eq!(patch.need, Some(pb::TodoNeed::Expected as i32));
        assert_eq!(patch.fuzzy_due, Some(pb::TodoFuzzyDue::InAFewDays as i32));
        assert_eq!(patch.checkpoint, Some(pb::TodoCheckpoint::Done as i32));
        assert!(patch.clear_external_block);
        assert!(patch.clear_owner);
        assert_eq!(patch.channel.map(|c| c.value), Some("infra".to_owned()));
        assert_eq!(patch.depends_on_add.len(), 1);
    }
}
