//! TodoService (boards.proto, BOARDSIMPLE, D830-D834): the one ranked
//! to-do list, served from the same items as BoardService.
//!
//! An item is a board item plus the to-do fields of [`todo_fields`]. The
//! list's rules (rank, groups, Do next, the move from boards) are
//! `humata2_todo`'s; this module only reads and writes the items and
//! hands them to it. An agent's change goes through the same rules as a
//! board write: an allowed one applies, one the rules ask about waits in
//! the Inbox as a board change and applies when the user says yes, and a new
//! item by an agent is suggested until the user accepts it.

use std::collections::{BTreeMap, HashSet};

use humata2_access::{BoardAct, Call, Capability, Target};
use humata2_daemon_ports::{BoardAsk, Caller, Deferred, Run};
use humata2_rpc_kit::{
    blocking, internal, invalid_field, lock, missing, not_built_status, now_ms, timestamp_ms,
};
use humata2_store::{BoardItemChange, BoardItemRecord, BoardKey, BoardWrite, NewBoardItem};
use humata2_todo as domain;
use humata2_todo::{CheckpointState, Day, Effort, FuzzyDue, Impact, Need, OldColumn, OwnerKind, State};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Request, Response, Status};

use super::automation;
use super::asks::{change_ask, field_change, new_item_ask, pending_for};
use super::history::{entry_pb, word_diff_pb};
use super::ownership;
use super::ports::{answer_key, caller_of};
use super::pr_watch;
use super::scope::{board_for_policy, board_pb, by_of, made_by_session};
use super::todo_fields::{
    self as saved, BlockerGoneRow, ExternalBlockRow, Fields, FieldsError, MigrationRow,
    LifecyclePromptRow, SourceAnchorRow, StartRow, StepRow, TodoCommentRow, TodoHistoryRow, TodoOwnerRow, WatchedPrRow,
};
use super::{
    BoardSvc, DEFAULT_COLUMN, ids, item_id, item_ids, keys, merged, no_item, page_of,
    revision_conflict,
};
use crate::pb;

const DAY_MS: i64 = 86_400_000;

/// Closed items stay in the list this long, with Reopen.
const CLOSED_WINDOW_MS: i64 = 7 * DAY_MS;

impl From<FieldsError> for Status {
    fn from(error: FieldsError) -> Self {
        internal(humata2_rpc_kit::chain(&error), "")
    }
}

// ---------------------------------------------------------------- days

/// The civil day (UTC) `ms` falls on. None only for a time outside the
/// calendar the list can hold.
fn day_of(ms: i64) -> Option<Day> {
    let days = ms.div_euclid(DAY_MS);
    // Days since 1970-01-01 to a civil date.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    Day::from_ymd(
        i32::try_from(y).ok()?,
        u32::try_from(m).ok()?,
        u32::try_from(d).ok()?,
    )
}

fn civil_pb(day: Day) -> pb::CivilDay {
    let (year, month, day) = day.ymd();
    pb::CivilDay { year, month, day }
}

/// The day a request names; unset is none, a date that doesn't exist is
/// refused.
fn civil(day: Option<&pb::CivilDay>, field: &str) -> Result<Option<Day>, Status> {
    let Some(day) = day else {
        return Ok(None);
    };
    Day::from_ymd(day.year, day.month, day.day)
        .map(Some)
        .ok_or_else(|| {
            invalid_field(
                field,
                Problem::BadFormat,
                format!(
                    "{}-{}-{} isn't a date in the calendar",
                    day.year, day.month, day.day
                ),
                "send a real year, month (1 to 12) and day of that month",
            )
        })
}

fn day_text(day: Option<Day>) -> String {
    day.map_or_else(String::new, |day| {
        let (y, m, d) = day.ymd();
        format!("{y:04}-{m:02}-{d:02}")
    })
}

// ---------------------------------------------------- enums and columns

fn effort_of(value: i32, field: &str) -> Result<Option<Effort>, Status> {
    match pb::TodoEffort::try_from(value) {
        Ok(pb::TodoEffort::Unspecified) => Ok(None),
        Ok(pb::TodoEffort::S) => Ok(Some(Effort::S)),
        Ok(pb::TodoEffort::M) => Ok(Some(Effort::M)),
        Ok(pb::TodoEffort::L) => Ok(Some(Effort::L)),
        Err(_) => Err(unknown(field, "effort", "S, M or L")),
    }
}

fn impact_of(value: i32, field: &str) -> Result<Option<Impact>, Status> {
    match pb::TodoImpact::try_from(value) {
        Ok(pb::TodoImpact::Unspecified) => Ok(None),
        Ok(pb::TodoImpact::Low) => Ok(Some(Impact::Low)),
        Ok(pb::TodoImpact::Medium) => Ok(Some(Impact::Medium)),
        Ok(pb::TodoImpact::High) => Ok(Some(Impact::High)),
        Err(_) => Err(unknown(field, "impact", "Low, Medium or High")),
    }
}

fn state_of(value: i32, field: &str) -> Result<State, Status> {
    match pb::TodoState::try_from(value) {
        Ok(pb::TodoState::Open) => Ok(State::Open),
        Ok(pb::TodoState::InProgress) => Ok(State::InProgress),
        Ok(pb::TodoState::Closed) => Ok(State::Closed),
        Ok(pb::TodoState::WontDo) => Ok(State::WontDo),
        Ok(pb::TodoState::Became) => Ok(State::Became),
        Ok(pb::TodoState::WatchingPr) => Ok(State::WatchingPr),
        Ok(pb::TodoState::Unspecified) | Err(_) => Err(unknown(
            field,
            "state",
            "Open, In progress, Closed, Won't do, Became or Watching PR",
        )),
    }
}

fn need_of(value: i32, field: &str) -> Result<Option<Need>, Status> {
    match pb::TodoNeed::try_from(value) {
        Ok(pb::TodoNeed::Unspecified) => Ok(None),
        Ok(pb::TodoNeed::Idea) => Ok(Some(Need::Idea)),
        Ok(pb::TodoNeed::NiceToHave) => Ok(Some(Need::NiceToHave)),
        Ok(pb::TodoNeed::Expected) => Ok(Some(Need::Expected)),
        Ok(pb::TodoNeed::Critical) => Ok(Some(Need::Critical)),
        Err(_) => Err(unknown(
            field,
            "need",
            "Idea, Nice to have, Expected or Critical",
        )),
    }
}

fn fuzzy_due_of(value: i32, field: &str) -> Result<Option<FuzzyDue>, Status> {
    match pb::TodoFuzzyDue::try_from(value) {
        Ok(pb::TodoFuzzyDue::Unspecified) => Ok(None),
        Ok(pb::TodoFuzzyDue::InAFewDays) => Ok(Some(FuzzyDue::InAFewDays)),
        Ok(pb::TodoFuzzyDue::NextWeek) => Ok(Some(FuzzyDue::NextWeek)),
        Ok(pb::TodoFuzzyDue::NextMonth) => Ok(Some(FuzzyDue::NextMonth)),
        Err(_) => Err(unknown(
            field,
            "fuzzy due",
            "In a few days, Next week or Next month",
        )),
    }
}

fn checkpoint_of(value: i32, field: &str) -> Result<Option<CheckpointState>, Status> {
    match pb::TodoCheckpoint::try_from(value) {
        Ok(pb::TodoCheckpoint::Unspecified) => Ok(None),
        Ok(pb::TodoCheckpoint::WithContext) => Ok(Some(CheckpointState::WithContext)),
        Ok(pb::TodoCheckpoint::Defined) => Ok(Some(CheckpointState::Defined)),
        Ok(pb::TodoCheckpoint::Done) => Ok(Some(CheckpointState::Done)),
        Err(_) => Err(unknown(
            field,
            "checkpoint",
            "With context, Defined or Done",
        )),
    }
}

fn owner_of(owner: &pb::TodoOwner, field: &str) -> Result<TodoOwnerRow, Status> {
    let kind = match pb::TodoOwnerKind::try_from(owner.kind) {
        Ok(pb::TodoOwnerKind::User) => OwnerKind::User,
        Ok(pb::TodoOwnerKind::Agent) => OwnerKind::Agent,
        _ => return Err(unknown(field, "owner kind", "User or Agent")),
    };
    Ok(TodoOwnerRow {
        kind,
        label: owner.label.clone(),
        session: owner
            .session
            .as_ref()
            .map(|s| s.value.clone())
            .filter(|s| !s.is_empty()),
    })
}

fn source_anchor_of(anchor: &pb::SourceAnchor) -> SourceAnchorRow {
    SourceAnchorRow {
        repo: anchor.repo.clone(),
        path: anchor.path.clone(),
        line: anchor.line,
        end_line: anchor.end_line,
        comment_id: anchor.comment_id.clone(),
    }
}

fn source_anchor_pb(anchor: &SourceAnchorRow) -> pb::SourceAnchor {
    pb::SourceAnchor {
        repo: anchor.repo.clone(),
        path: anchor.path.clone(),
        line: anchor.line,
        end_line: anchor.end_line,
        comment_id: anchor.comment_id.clone(),
    }
}

fn external_block_of(block: &pb::ExternalBlock, now: i64) -> ExternalBlockRow {
    let since_ms = block.since.as_ref().map_or(now, |t| {
        t.seconds
            .saturating_mul(1000)
            .saturating_add(i64::from(t.nanos / 1_000_000))
    });
    ExternalBlockRow {
        reason: block.reason.trim().to_owned(),
        since_ms: Some(since_ms),
    }
}

fn watched_pr_of(pr: &pb::WatchedPr) -> WatchedPrRow {
    let (owner, name, number, note) = match pr.pr.as_ref() {
        Some(p) => (
            p.repo.as_ref().map_or_else(String::new, |r| r.owner.clone()),
            p.repo.as_ref().map_or_else(String::new, |r| r.name.clone()),
            p.number,
            p.note.clone(),
        ),
        None => (String::new(), String::new(), 0, String::new()),
    };
    WatchedPrRow {
        owner,
        name,
        number,
        note,
        title: pr.title.clone(),
        ci_state: pr.ci_state.clone(),
        unresolved_comments: pr.unresolved_comments,
        has_conflicts: pr.has_conflicts,
        upstream_merged: pr.upstream_merged,
        merged: pr.merged,
        closed_without_merge: pr.closed_without_merge,
        stack_key: pr.stack_key.clone(),
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

fn fuzzy_due_pb(fd: Option<FuzzyDue>) -> pb::TodoFuzzyDue {
    match fd {
        None => pb::TodoFuzzyDue::Unspecified,
        Some(FuzzyDue::InAFewDays) => pb::TodoFuzzyDue::InAFewDays,
        Some(FuzzyDue::NextWeek) => pb::TodoFuzzyDue::NextWeek,
        Some(FuzzyDue::NextMonth) => pb::TodoFuzzyDue::NextMonth,
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

fn unknown(field: &str, what: &str, use_one_of: &str) -> Status {
    invalid_field(
        field,
        Problem::UnknownValue,
        format!("{field} names no {what}"),
        format!("use {use_one_of}"),
    )
}

fn effort_pb(effort: Option<Effort>) -> pb::TodoEffort {
    match effort {
        None => pb::TodoEffort::Unspecified,
        Some(Effort::S) => pb::TodoEffort::S,
        Some(Effort::M) => pb::TodoEffort::M,
        Some(Effort::L) => pb::TodoEffort::L,
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

fn state_label(state: State) -> &'static str {
    match state {
        State::Open => "Open",
        State::InProgress => "In progress",
        State::Closed => "Closed",
        State::WontDo => "Won't do",
        State::Became => "Became",
        State::WatchingPr => "Watching PR",
    }
}

fn old_column(column: i32) -> OldColumn {
    match column {
        1 => OldColumn::Backlog,
        3 => OldColumn::InProgress,
        4 => OldColumn::InReview,
        5 => OldColumn::Done,
        _ => OldColumn::ToDo,
    }
}

/// Tags as kept: trimmed, none empty, each once, in the order given.
fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !out.iter().any(|have| have == tag) {
            out.push(tag.to_owned());
        }
    }
    out
}

fn steps_of(steps: &[pb::TodoStep]) -> Vec<StepRow> {
    steps
        .iter()
        .enumerate()
        .map(|(idx, step)| StepRow {
            id: if step.id.is_empty() {
                format!("step-{}", idx + 1)
            } else {
                step.id.clone()
            },
            text: step.text.trim().to_owned(),
            done: step.done,
            ticked_by: step.ticked_by.as_ref().map(|id| id.value.clone()),
            markdown_context: (!step.markdown_context.is_empty())
                .then(|| step.markdown_context.clone()),
            dropped: step.dropped,
            from_key: (!step.from_key.is_empty()).then(|| step.from_key.clone()),
            source_anchor: step.source_anchor.as_ref().map(source_anchor_of),
        })
        .collect()
}

// -------------------------------------------------------------- the view

/// An item as the list sees it: the board item and what the list keeps.
#[derive(Clone)]
pub(super) struct Entry {
    pub(super) record: BoardItemRecord,
    pub(super) fields: Fields,
}

impl Entry {
    /// The channel the item shows under; empty is none. Items of a session's
    /// board show under that session's channel, which the store keeps.
    pub(super) fn channel(&self) -> String {
        self.fields
            .channel
            .clone()
            .unwrap_or_else(|| self.record.channel.clone())
    }

    /// The sessions on it, with the session whose board it was on first.
    pub(super) fn sessions(&self) -> Vec<String> {
        let mut sessions = self.record.sessions.clone();
        if let BoardKey::Session(session) = &self.record.board
            && !sessions.contains(session)
        {
            sessions.push(session.clone());
        }
        sessions
    }

    pub(super) fn state(&self) -> State {
        self.fields.state(self.record.column)
    }

    pub(super) fn revision(&self) -> u64 {
        self.record.revision + self.fields.extra_revisions
    }

    fn closed_ms(&self) -> Option<i64> {
        self.state().is_terminal().then(|| {
            self.fields
                .closed_at_ms
                .or(self.record.done_unix_ms)
                .unwrap_or(self.record.updated_unix_ms)
        })
    }

    /// The record with the list's view of it filled in, for the board item
    /// the API shows.
    pub(super) fn shown_record(&self) -> BoardItemRecord {
        let mut record = self.record.clone();
        record.revision = self.revision();
        record.channel = self.channel();
        record.sessions = self.sessions();
        record
    }
}

/// What the filter of a list or Do next asks for.
struct Wanted {
    channel: Option<String>,
    no_channel: bool,
    session: Option<String>,
    show_closed: bool,
}

impl Wanted {
    fn of(filter: Option<&pb::TodoFilter>) -> Self {
        let filter = filter.cloned().unwrap_or_default();
        Self {
            channel: filter.channel.map(|id| id.value).filter(|id| !id.is_empty()),
            no_channel: filter.no_channel,
            session: filter.session.map(|id| id.value).filter(|id| !id.is_empty()),
            show_closed: filter.show_closed,
        }
    }

    fn shows(&self, entry: &Entry, now: i64) -> bool {
        let channel = entry.channel();
        let in_channel = match (&self.channel, self.no_channel) {
            (Some(wanted), _) => channel == *wanted,
            (None, true) => channel.is_empty(),
            (None, false) => true,
        };
        let in_session = self
            .session
            .as_ref()
            .is_none_or(|wanted| entry.sessions().contains(wanted));
        let open_enough = match entry.closed_ms() {
            None => true,
            Some(at) => self.show_closed && at >= now - CLOSED_WINDOW_MS,
        };
        in_channel && in_session && open_enough
    }
}

/// One line of a card fact or effect: its bold lead and the rest, with a space
/// between them when neither side supplies whitespace or attaching punctuation.
fn card_text(lead: &str, rest: &str) -> String {
    let lead = lead.trim_start();
    let rest = rest.trim_end();
    if lead.is_empty() {
        return rest.trim().to_owned();
    }
    if rest.is_empty() {
        return lead.trim().to_owned();
    }
    let needs_space = !lead.ends_with(char::is_whitespace)
        && !rest.starts_with(|c: char| {
            c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | ')')
        });
    if needs_space {
        format!("{lead} {rest}")
    } else {
        format!("{lead}{rest}")
    }
}

fn ask_kind_of(change: Option<&pb::BoardChangePreview>) -> domain::AskKind {
    let Some(change) = change else {
        return domain::AskKind::Close;
    };
    if change.deletes_item {
        return domain::AskKind::Close;
    }
    let Some(first) = change.changes.first() else {
        return domain::AskKind::Close;
    };
    let field = first.field.to_lowercase();
    let closes = matches!(field.as_str(), "column" | "status" | "state")
        && matches!(first.after.to_lowercase().as_str(), "done" | "closed");
    if closes {
        domain::AskKind::Close
    } else {
        domain::AskKind::Edit {
            field: first.field.clone(),
            before: first.before.clone(),
            after: first.after.clone(),
        }
    }
}

fn ask_by_of(requester: Option<&pb::AgentActor>) -> String {
    let Some(requester) = requester else {
        return String::new();
    };
    let agent = requester
        .agent
        .as_ref()
        .map_or("", |id| id.value.as_str());
    let session = requester.session_title.as_str();
    match (agent.is_empty(), session.is_empty()) {
        (false, false) => format!("{agent} in {session}"),
        (false, true) => agent.to_owned(),
        (true, false) => session.to_owned(),
        (true, true) => String::new(),
    }
}

fn ask_requester<'a>(
    ask: &'a pb::PendingAsk,
    request: Option<&'a pb::InboxRequest>,
) -> Option<&'a pb::AgentActor> {
    request
        .and_then(|req| req.requester.as_ref())
        .or_else(|| match ask.asked_by.as_ref().and_then(|a| a.actor.as_ref()) {
            Some(pb::actor::Actor::Agent(agent)) => Some(agent),
            _ => None,
        })
}

fn ask_detail_of(
    ask: &pb::PendingAsk,
    request: Option<&pb::InboxRequest>,
    archived: &HashSet<String>,
    now_ms: i64,
) -> domain::AskDetail {
    let change = ask
        .change
        .as_ref()
        .and_then(|c| c.changes.first())
        .map(|fc| {
            let left = if fc.before.is_empty() {
                format!("{}: none", fc.field)
            } else {
                format!("{}: {}", fc.field, fc.before)
            };
            (left, fc.after.clone())
        });
    let Some(request) = request else {
        let session_ended = ask
            .session
            .as_ref()
            .is_some_and(|id| archived.contains(&id.value));
        return domain::AskDetail {
            change,
            session_ended,
            ..domain::AskDetail::default()
        };
    };
    let evidence = request
        .facts
        .iter()
        .map(|fact| (fact, card_text(&fact.lead, &fact.rest)))
        .filter(|(_, text)| !text.is_empty())
        .map(|(fact, text)| domain::Evidence {
            text: if fact.label.is_empty() {
                text
            } else {
                format!("{}: {text}", fact.label)
            },
            link: None,
        })
        .collect();
    let affects = request
        .affects
        .iter()
        .map(|effect| card_text(&effect.lead, &effect.rest))
        .filter(|text| !text.is_empty())
        .collect();
    let asked_minutes_ago = request.created_at.as_ref().map_or(0, |at| {
        let created_ms = at
            .seconds
            .saturating_mul(1000)
            .saturating_add(i64::from(at.nanos / 1_000_000));
        u32::try_from(now_ms.saturating_sub(created_ms).max(0) / 60_000).unwrap_or(u32::MAX)
    });
    let session_id = request
        .requester
        .as_ref()
        .and_then(|r| r.session.as_ref())
        .or(ask.session.as_ref())
        .map(|id| id.value.as_str());
    let withdrawn_session_ended = matches!(
        pb::WithdrawReason::try_from(request.withdraw_reason),
        Ok(
            pb::WithdrawReason::SessionStopped
                | pb::WithdrawReason::SessionArchived
                | pb::WithdrawReason::SessionDeleted
        )
    );
    let session_ended = withdrawn_session_ended
        || session_id.is_some_and(|id| archived.contains(id));
    domain::AskDetail {
        evidence,
        affects,
        change,
        asked_minutes_ago,
        session_ended,
    }
}

fn first_request_for<'a>(
    item: &BoardItemRecord,
    asks: &'a [pb::InboxRequest],
) -> Option<&'a pb::InboxRequest> {
    asks.iter().find(|ask| {
        matches!(
            ask.preview.as_ref().and_then(|p| p.preview.as_ref()),
            Some(pb::request_preview::Preview::BoardChange(change))
                if change.item.as_ref().map(|id| id.value.as_str()) == Some(item.key.as_str())
        )
    })
}

/// What a write came to: done, or stopped at a rule that asks.
pub(super) enum TodoApplied {
    Done(Box<pb::WriteTodoResponse>),
    Ask(Box<humata2_access::AccessRequest>, BoardAsk),
}

/// A to-do write, as asked.
#[derive(Clone)]
pub(super) enum TodoOp {
    Create(pb::CreateTodoRequest),
    Update(pb::UpdateTodoRequest),
}

impl TodoOp {
    fn identity(&self) -> (&'static str, String, Option<String>) {
        match self {
            Self::Create(r) => (
                "create_todo",
                String::new(),
                r.idempotency_key.as_ref().map(|key| key.value.clone()),
            ),
            Self::Update(r) => (
                "update_todo",
                r.id.as_ref().map(|id| id.value.clone()).unwrap_or_default(),
                r.idempotency_key.as_ref().map(|key| key.value.clone()),
            ),
        }
    }
}

impl BoardSvc {
    // ------------------------------------------------------------ reading

    /// Every live item the caller may read, with its to-do fields.
    pub(super) fn entries(&self, caller: &Caller) -> Result<Vec<Entry>, Status> {
        let fields = saved::load(&self.store)?;
        let mut records = self
            .store
            .all_board_items_but_sessions()
            .map_err(|error| self.failed(error, "list board items"))?;
        let sessions = self
            .store
            .sessions()
            .map_err(|error| self.failed(error, "list the sessions"))?;
        let boards: Vec<BoardKey> = sessions
            .into_iter()
            .map(|session| BoardKey::Session(session.id))
            .collect();
        records.extend(
            self.store
                .board_items(&boards)
                .map_err(|error| self.failed(error, "list the sessions' board items"))?,
        );
        let mut readable: Vec<(BoardKey, bool)> = Vec::new();
        let mut out = Vec::new();
        for record in records {
            let known = readable
                .iter()
                .find(|(board, _)| *board == record.board)
                .map(|(_, ok)| *ok);
            let ok = known.unwrap_or_else(|| {
                let ok = self.can_read(caller, self.policy_of(&record));
                readable.push((record.board.clone(), ok));
                ok
            });
            if ok {
                let fields = fields.get(&record.key).cloned().unwrap_or_default();
                out.push(Entry { record, fields });
            }
        }
        out.sort_by(|a, b| {
            (a.record.created_unix_ms, &a.record.key).cmp(&(b.record.created_unix_ms, &b.record.key))
        });
        Ok(out)
    }

    /// The domain's item for `entry`; `ask` is an agent's pending request
    /// waiting on it, with `request` the Inbox request it came from so
    /// evidence, affects, change, `asked_minutes_ago` and `session_ended` are
    /// filled in.
    fn domain_item(
        entry: &Entry,
        ask: Option<&pb::PendingAsk>,
        request: Option<&pb::InboxRequest>,
        archived: &HashSet<String>,
        today: Day,
        now: i64,
    ) -> domain::Todo {
        let record = &entry.record;
        let fields = &entry.fields;
        // A time the calendar can't hold ranks as added today.
        let created = day_of(record.created_unix_ms).unwrap_or(today);
        let mut todo = domain::Todo::new(&record.key, &record.title, created);
        let channel = entry.channel();
        todo.channel = (!channel.is_empty()).then_some(channel);
        todo.sessions = entry.sessions();
        todo.state = entry.state();
        todo.due = fields.due;
        todo.effort = fields.effort;
        todo.impact = fields.impact;
        todo.tags = fields.tags.clone();
        todo.steps = fields
            .steps
            .iter()
            .map(|step| domain::Step {
                id: step.id.clone(),
                text: step.text.clone(),
                done: step.done,
                markdown_context: step.markdown_context.clone(),
                dropped: step.dropped,
                from_key: step.from_key.clone(),
                source_anchor: step.source_anchor.as_ref().map(|a| domain::SourceAnchor {
                    repo: a.repo.clone(),
                    path: a.path.clone(),
                    line: a.line,
                    end_line: (a.end_line > 0).then_some(a.end_line),
                    comment_id: (!a.comment_id.is_empty()).then(|| a.comment_id.clone()),
                }),
            })
            .collect();
        todo.hide_until = fields.hide_until;
        todo.pin = fields.pin_order.checked_sub(1);
        todo.depends_on = record.depends_on.clone();
        todo.suggested = fields.suggested;
        todo.need = fields.need;
        todo.fuzzy_due = fields.fuzzy_due;
        todo.external_block = fields.external_block.as_ref().map(|b| domain::ExternalBlock {
            reason: b.reason.clone(),
            since: b.since_ms.and_then(day_of),
        });
        todo.from_keys = fields.from_keys.clone();
        todo.became_keys = fields.became_keys.clone();
        todo.source_anchor = fields.source_anchor.as_ref().map(|a| domain::SourceAnchor {
            repo: a.repo.clone(),
            path: a.path.clone(),
            line: a.line,
            end_line: (a.end_line > 0).then_some(a.end_line),
            comment_id: (!a.comment_id.is_empty()).then(|| a.comment_id.clone()),
        });
        todo.comments = fields
            .comments
            .iter()
            .map(|c| domain::TodoComment {
                id: c.id.clone(),
                author: c.author.clone(),
                body: c.body.clone(),
                created: day_of(c.created_at_ms),
                step_id: c.step_id.clone(),
            })
            .collect();
        todo.owner = fields.owner.as_ref().map(|o| domain::TodoOwner {
            kind: o.kind,
            label: o.label.clone(),
            session: o.session.clone(),
        });
        todo.checkpoint = fields.checkpoint;
        todo.stuck_agent = fields.stuck_agent.as_ref().map(|s| domain::StuckAgent {
            session: s.session.clone(),
            agent_label: s.agent_label.clone(),
            reason: s.reason.clone(),
            since_minutes: u32::try_from(now.saturating_sub(s.since_ms).max(0) / 60_000)
                .unwrap_or(u32::MAX),
        });
        todo.blocker_gone = fields
            .blocker_gone
            .iter()
            .map(|b| domain::BlockerGone {
                blocker_key: b.blocker_key.clone(),
                blocker_state: b.blocker_state,
                replacement_keys: b.replacement_keys.clone(),
            })
            .collect();
        todo.watched_prs = fields
            .watched_prs
            .iter()
            .map(|pr| domain::WatchedPr {
                repo_pr: pr.repo_pr(),
                title: pr.title.clone(),
                ci_state: pr.ci_state.clone(),
                unresolved_comments: pr.unresolved_comments,
                has_conflicts: pr.has_conflicts,
                upstream_merged: pr.upstream_merged,
                merged: pr.merged,
                closed_without_merge: pr.closed_without_merge,
                stack_key: (!pr.stack_key.is_empty()).then(|| pr.stack_key.clone()),
            })
            .collect();
        todo.close_readiness = fields
            .close_readiness
            .as_ref()
            .map(|cr| domain::CloseReadiness {
                steps_done: cr.steps_done,
                steps_total: cr.steps_total,
                unresolved_comments: cr.unresolved_comments,
                ci_status: cr.ci_status.clone(),
                commits: cr.commits,
            });
        todo.session_signals = fields
            .session_signals
            .iter()
            .map(|s| domain::SessionSignal {
                session: s.session.clone(),
                label: s.label.clone(),
                status: s.status.clone(),
                idle_days: s.idle_days,
                active_turn: s.active_turn,
            })
            .collect();
        todo.lifecycle_prompts = fields
            .lifecycle_prompts
            .iter()
            .filter(|p| !automation::is_pending(p))
            .map(|p| domain::LifecyclePrompt {
                id: p.id.clone(),
                kind: p.kind.clone(),
                title: p.title.clone(),
                detail: p.detail.clone(),
                choices: p.choices.clone(),
                pr_ref: (!p.pr_ref.is_empty()).then(|| p.pr_ref.clone()),
                session: p.session.clone(),
            })
            .collect();
        todo.wont_do_reason = fields.wont_do_reason.clone();
        todo.ask = ask.map(|ask| domain::Ask {
            kind: ask_kind_of(ask.change.as_ref()),
            by: ask_by_of(ask_requester(ask, request)),
            reason: ask.reason.clone(),
        });
        if let Some(ask) = ask {
            todo.ask_detail = ask_detail_of(ask, request, archived, now);
        }
        todo.review = fields.review.as_ref().map(|review| domain::Review {
            repo_prs: review
                .pull_requests
                .iter()
                .map(|pr| format!("{}/{}#{}", pr.owner, pr.name, pr.number))
                .collect(),
            author: review.author.clone(),
            waiting_days: u32::try_from((now - review.requested_unix_ms).max(0) / DAY_MS)
                .unwrap_or(u32::MAX),
        });
        todo.start = fields.start.as_ref().map(|start| domain::Start {
            column: old_column(start.column),
            position: start.position,
            board: start.board_channel.clone(),
            vip: start.was_vip,
        });
        todo.closed = entry.closed_ms().and_then(day_of);
        todo
    }

    /// The items the caller may read, and how the list ranks them today.
    fn ranked(
        &self,
        caller: &Caller,
        asks: &[pb::InboxRequest],
        now: i64,
    ) -> Result<(Vec<Entry>, domain::Ranked), Status> {
        let entries = self.entries(caller)?;
        let archived = self.archived()?;
        let today = day_of(now).ok_or_else(|| internal("the clock is outside the calendar", ""))?;
        let items: Vec<domain::Todo> = entries
            .iter()
            .map(|entry| {
                let pending = pending_for(&entry.record, asks);
                let first = pending.first();
                let request = first
                    .and_then(|ask| ask.request.as_ref())
                    .and_then(|id| asks.iter().find(|req| req.id.as_ref() == Some(id)))
                    .or_else(|| first_request_for(&entry.record, asks));
                Self::domain_item(entry, first, request, &archived, today, now)
            })
            .collect();
        let ranked = domain::rank(&items, today);
        Ok((entries, ranked))
    }

    fn fields_pb(&self, entry: &Entry) -> pb::TodoFields {
        let fields = &entry.fields;
        let runs = self
            .store
            .list_todo_runs(&entry.record.key)
            .unwrap_or_default();
        let run_count = u32::try_from(runs.len()).unwrap_or(u32::MAX);
        let total_cost_micros = runs.iter().fold(None::<u64>, |acc, r| match r.cost_micros {
            Some(c) => Some(acc.unwrap_or(0).saturating_add(c)),
            None => acc,
        });
        let close_readiness = fields
            .close_readiness
            .as_ref()
            .cloned()
            .or_else(|| {
                (!fields.steps.is_empty() || !fields.watched_prs.is_empty())
                    .then(|| pr_watch::compute_close_readiness(&fields.steps, &fields.watched_prs, 0))
            });
        pb::TodoFields {
            key: entry.record.key.clone(),
            state: state_pb(entry.state()) as i32,
            due: fields.due.map(civil_pb),
            effort: effort_pb(fields.effort) as i32,
            impact: impact_pb(fields.impact) as i32,
            tags: fields.tags.clone(),
            steps: fields
                .steps
                .iter()
                .map(|step| pb::TodoStep {
                    id: step.id.clone(),
                    text: step.text.clone(),
                    done: step.done,
                    ticked_by: step
                        .ticked_by
                        .as_ref()
                        .map(|id| pb::SessionId { value: id.clone() }),
                    markdown_context: step.markdown_context.clone().unwrap_or_default(),
                    dropped: step.dropped,
                    from_key: step.from_key.clone().unwrap_or_default(),
                    source_anchor: step.source_anchor.as_ref().map(source_anchor_pb),
                })
                .collect(),
            hide_until: fields.hide_until.map(civil_pb),
            pin_order: fields.pin_order,
            suggested: fields.suggested,
            start: fields.start.as_ref().map(|start| pb::TodoStart {
                column: super::scope::column_pb(start.column) as i32,
                position: start.position,
                board_channel: start
                    .board_channel
                    .as_ref()
                    .map(|id| pb::ChannelId { value: id.clone() }),
                was_vip: start.was_vip,
            }),
            review: fields.review.as_ref().map(|review| pb::TodoReview {
                pull_requests: review
                    .pull_requests
                    .iter()
                    .map(|pr| pb::PullRequestRef {
                        repo: Some(pb::RepoRef {
                            owner: pr.owner.clone(),
                            name: pr.name.clone(),
                        }),
                        number: pr.number,
                        note: pr.note.clone(),
                    })
                    .collect(),
                author: review.author.clone(),
                requested_at: Some(timestamp_ms(review.requested_unix_ms)),
            }),
            closed_at: entry.closed_ms().map(timestamp_ms),
            need: need_pb(fields.need) as i32,
            fuzzy_due: fuzzy_due_pb(fields.fuzzy_due) as i32,
            external_block: fields.external_block.as_ref().map(|b| pb::ExternalBlock {
                reason: b.reason.clone(),
                since: b.since_ms.map(timestamp_ms),
            }),
            from_keys: fields.from_keys.clone(),
            became_keys: fields.became_keys.clone(),
            source_anchor: fields.source_anchor.as_ref().map(source_anchor_pb),
            comments: fields
                .comments
                .iter()
                .map(|c| pb::TodoComment {
                    id: c.id.clone(),
                    author: c.author.clone(),
                    body: c.body.clone(),
                    created_at: Some(timestamp_ms(c.created_at_ms)),
                    step_id: c.step_id.clone().unwrap_or_default(),
                })
                .collect(),
            owner: fields.owner.as_ref().map(|o| pb::TodoOwner {
                kind: match o.kind {
                    OwnerKind::User => pb::TodoOwnerKind::User as i32,
                    OwnerKind::Agent => pb::TodoOwnerKind::Agent as i32,
                },
                label: o.label.clone(),
                session: o
                    .session
                    .as_ref()
                    .map(|id| pb::SessionId { value: id.clone() }),
            }),
            checkpoint: checkpoint_pb(fields.checkpoint) as i32,
            stuck_agent: fields.stuck_agent.as_ref().map(|s| pb::StuckAgentNotice {
                session: Some(pb::SessionId {
                    value: s.session.clone(),
                }),
                agent_label: s.agent_label.clone(),
                reason: s.reason.clone(),
                since: Some(timestamp_ms(s.since_ms)),
            }),
            blocker_gone: fields
                .blocker_gone
                .iter()
                .map(|b| pb::BlockerGoneNotice {
                    blocker_key: b.blocker_key.clone(),
                    blocker_state: state_pb(b.blocker_state) as i32,
                    replacement_keys: b.replacement_keys.clone(),
                })
                .collect(),
            watched_prs: fields
                .watched_prs
                .iter()
                .map(|pr| pb::WatchedPr {
                    pr: Some(pb::PullRequestRef {
                        repo: Some(pb::RepoRef {
                            owner: pr.owner.clone(),
                            name: pr.name.clone(),
                        }),
                        number: pr.number,
                        note: pr.note.clone(),
                    }),
                    title: pr.title.clone(),
                    ci_state: pr.ci_state.clone(),
                    unresolved_comments: pr.unresolved_comments,
                    has_conflicts: pr.has_conflicts,
                    upstream_merged: pr.upstream_merged,
                    merged: pr.merged,
                    closed_without_merge: pr.closed_without_merge,
                    stack_key: pr.stack_key.clone(),
                })
                .collect(),
            close_readiness: close_readiness.map(|cr| pb::CloseReadiness {
                steps_done: cr.steps_done,
                steps_total: cr.steps_total,
                unresolved_comments: cr.unresolved_comments,
                ci_status: cr.ci_status,
                commits: cr.commits,
            }),
            session_signals: fields
                .session_signals
                .iter()
                .map(|s| pb::SessionSignal {
                    session: Some(pb::SessionId {
                        value: s.session.clone(),
                    }),
                    label: s.label.clone(),
                    status: s.status.clone(),
                    idle_days: s.idle_days,
                    active_turn: s.active_turn,
                })
                .collect(),
            lifecycle_prompts: fields
                .lifecycle_prompts
                .iter()
                .filter(|p| !automation::is_pending(p))
                .map(|p| pb::LifecyclePrompt {
                    id: p.id.clone(),
                    kind: p.kind.clone(),
                    title: p.title.clone(),
                    detail: p.detail.clone(),
                    choices: p.choices.clone(),
                    pr_ref: p.pr_ref.clone(),
                    session: p
                        .session
                        .as_ref()
                        .map(|id| pb::SessionId { value: id.clone() }),
                })
                .collect(),
            run_count,
            total_cost_micros,
            wont_do_reason: fields.wont_do_reason.clone().unwrap_or_default(),
        }
    }

    /// `entry` for the API: the board item with the list's channel, the
    /// sessions it has and its revision, and the to-do fields.
    pub(super) fn todo_item(
        &self,
        caller: &Caller,
        entry: &Entry,
        asks: &[pb::InboxRequest],
        archived: &HashSet<String>,
    ) -> pb::TodoItem {
        let mut item = self.item_pb(caller, &entry.shown_record(), asks, archived);
        let channel = entry.channel();
        // Channel and session are filters on the one list, so the item is no
        // longer on a board of its own: it is on its channel's, or Global.
        item.board = Some(board_pb(&if channel.is_empty() {
            BoardKey::Global
        } else {
            BoardKey::Channel(channel)
        }));
        pb::TodoItem {
            item: Some(item),
            fields: Some(self.fields_pb(entry)),
        }
    }

    fn list_todo_in(
        &self,
        caller: &Caller,
        request: &pb::ListTodoRequest,
        asks: &[pb::InboxRequest],
        now: i64,
    ) -> Result<pb::ListTodoResponse, Status> {
        self.ensure_migrated(now)?;
        let wanted = Wanted::of(request.filter.as_ref());
        let (size, start) = page_of(request.page.as_ref())?;
        let archived = self.archived()?;
        let shown: Vec<Entry> = self
            .entries(caller)?
            .into_iter()
            .filter(|entry| wanted.shows(entry, now))
            .collect();
        let page: Vec<&Entry> = shown.iter().skip(start).take(size).collect();
        let next_page_token = if start + page.len() < shown.len() {
            (start + page.len()).to_string()
        } else {
            String::new()
        };
        Ok(pb::ListTodoResponse {
            items: page
                .iter()
                .map(|entry| self.todo_item(caller, entry, asks, &archived))
                .collect(),
            page: Some(pb::PageResponse { next_page_token }),
        })
    }

    fn next_todo_in(
        &self,
        caller: &Caller,
        request: &pb::NextTodoRequest,
        asks: &[pb::InboxRequest],
        now: i64,
    ) -> Result<pb::NextTodoResponse, Status> {
        self.ensure_migrated(now)?;
        let wanted = Wanted::of(request.filter.as_ref());
        let (entries, ranked) = self.ranked(caller, asks, now)?;
        // The list is ranked whole, so what an item waits on counts even when
        // the filter leaves it out; Do next is the first row the filter shows.
        let first = ranked.next.iter().find_map(|row| {
            entries
                .iter()
                .find(|entry| entry.record.key == row.key && wanted.shows(entry, now))
        });
        let archived = self.archived()?;
        Ok(pb::NextTodoResponse {
            item: first.map(|entry| self.todo_item(caller, entry, asks, &archived)),
        })
    }

    // ---------------------------------------------------------- migration

    /// Moves the boards into the list once. Safe to call every time: it
    /// does nothing when it already ran.
    pub(super) fn ensure_migrated(&self, now: i64) -> Result<MigrationRow, Status> {
        if let Some(done) = saved::load_migration(&self.store)? {
            return Ok(done);
        }
        self.migrate_boards(&HashSet::new(), now)
    }

    /// Moves every board item (global, channel and session boards) into the
    /// one list, keeping where each one stood (D833). `vip` names the items
    /// the owner marked VIP, which become Pin in that order; the daemon
    /// doesn't keep those marks, so the app passes them the first time.
    /// Running it again changes nothing.
    pub(crate) fn migrate_boards(&self, vip: &HashSet<String>, now: i64) -> Result<MigrationRow, Status> {
        let _write = lock(&self.writes);
        if let Some(done) = saved::load_migration(&self.store)? {
            return Ok(done);
        }
        let mut records = self
            .store
            .all_board_items_but_sessions()
            .map_err(|error| self.failed(error, "list board items"))?;
        let boards: Vec<BoardKey> = self
            .store
            .sessions()
            .map_err(|error| self.failed(error, "list the sessions"))?
            .into_iter()
            .map(|session| BoardKey::Session(session.id))
            .collect();
        records.extend(
            self.store
                .board_items(&boards)
                .map_err(|error| self.failed(error, "list the sessions' board items"))?,
        );
        let archived = self.archived()?;
        let (planned, row) = plan_migration(&records, &archived, vip, now);
        let mut map = saved::load(&self.store)?;
        for (key, fields) in planned {
            map.entry(key).or_insert(fields);
        }
        saved::save(&self.store, &map)?;
        saved::save_migration(&self.store, &row)?;
        Ok(row)
    }

    fn migration_pb(row: &MigrationRow) -> pb::TodoMigration {
        pb::TodoMigration {
            migrated_at: Some(timestamp_ms(row.migrated_at_ms)),
            items: row.items,
            open: row.open,
            in_progress: row.in_progress,
            closed: row.closed,
            boards: row.boards,
            banner_dismissed: row.banner_dismissed,
        }
    }

    fn dismiss_migration_in(&self, now: i64) -> Result<pb::TodoMigration, Status> {
        let mut row = self.ensure_migrated(now)?;
        let _write = lock(&self.writes);
        row.banner_dismissed = true;
        saved::save_migration(&self.store, &row)?;
        Ok(Self::migration_pb(&row))
    }

    // ------------------------------------------------------------- writes

    /// The to-do rule for an agent's call: its own session's switch. The
    /// user and scripts have none.
    fn gate_todo(
        &self,
        caller: &Caller,
        capability: Capability,
        approved: bool,
    ) -> Result<Option<Box<humata2_access::AccessRequest>>, Status> {
        let Caller::Agent(agent) = caller else {
            return Ok(None);
        };
        super::gate(
            self.guard.as_ref(),
            caller,
            &Call::Use {
                capability,
                target: Target::Session(agent.session.clone()),
            },
            approved,
        )
    }

    /// Dependency adjacency map across all live board items (key -> depends_on).
    fn all_dependency_edges(&self) -> Result<BTreeMap<String, Vec<String>>, Status> {
        let mut records = self
            .store
            .all_board_items_but_sessions()
            .map_err(|error| self.failed(error, "list board items"))?;
        let boards: Vec<BoardKey> = self
            .store
            .sessions()
            .map_err(|error| self.failed(error, "list the sessions"))?
            .into_iter()
            .map(|session| BoardKey::Session(session.id))
            .collect();
        records.extend(
            self.store
                .board_items(&boards)
                .map_err(|error| self.failed(error, "list the sessions' board items"))?,
        );
        Ok(records
            .into_iter()
            .filter(|r| !r.deleted)
            .map(|r| (r.key, r.depends_on))
            .collect())
    }

    /// Records a `BlockerGoneRow` notice on every active item that depends on
    /// `blocker_key` when `blocker_key` transitions to `WontDo` or `Became`.
    fn notify_blocker_gone(
        &self,
        map: &mut BTreeMap<String, Fields>,
        blocker_key: &str,
        blocker_state: State,
        replacement_keys: &[String],
    ) -> Result<(), Status> {
        if !matches!(blocker_state, State::WontDo | State::Became) {
            return Ok(());
        }
        let edges = self.all_dependency_edges()?;
        for (dep_key, deps) in edges {
            if dep_key == blocker_key || !deps.iter().any(|k| k == blocker_key) {
                continue;
            }
            let entry = map.entry(dep_key).or_default();
            if let Some(existing) = entry
                .blocker_gone
                .iter_mut()
                .find(|bg| bg.blocker_key == blocker_key)
            {
                existing.blocker_state = blocker_state;
                existing.replacement_keys = replacement_keys.to_vec();
            } else {
                entry.blocker_gone.push(BlockerGoneRow {
                    blocker_key: blocker_key.to_owned(),
                    blocker_state,
                    replacement_keys: replacement_keys.to_vec(),
                });
            }
            entry.extra_revisions += 1;
        }
        Ok(())
    }

    pub(super) fn create_todo_in(
        &self,
        caller: &Caller,
        request: &pb::CreateTodoRequest,
        approved: bool,
        now: i64,
    ) -> Result<TodoApplied, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let title = request.title.trim();
        if title.is_empty() {
            return Err(missing("title"));
        }
        let due = civil(request.due.as_ref(), "due")?;
        let effort = effort_of(request.effort, "effort")?;
        let impact = impact_of(request.impact, "impact")?;
        let need = need_of(request.need, "need")?;
        let fuzzy_due = fuzzy_due_of(request.fuzzy_due, "fuzzy_due")?;
        let source_anchor = request.source_anchor.as_ref().map(source_anchor_of);
        let from_keys = clean_tags(&request.from_keys);
        let external_block = request
            .external_block
            .as_ref()
            .map(|b| external_block_of(b, now));
        let (key, channel, channel_name) = match request.channel.as_ref() {
            Some(id) if !id.value.is_empty() => {
                self.resolve_board(&pb::BoardRef {
                    board: Some(pb::board_ref::Board::Channel(id.clone())),
                })?
            }
            _ => (BoardKey::Global, String::new(), String::new()),
        };
        let sessions = self.known_sessions(&request.sessions, "sessions")?;
        let depends_on = self.readable_targets(caller, &request.depends_on)?;
        let policy = board_for_policy(&key, &channel, &self.machine);
        // An agent's suggestion has its own rule on top of the board's.
        let suggest = self.gate_todo(caller, Capability::TodoSuggest, approved)?;
        let asked = match suggest {
            Some(asked) => Some(asked),
            None => self.gate(caller, policy, BoardAct::Add, approved)?,
        };
        if let Some(asked) = asked {
            let ask = new_item_ask(
                pb::NewBoardItemPreview {
                    board: Some(board_pb(&key)),
                    title: title.to_owned(),
                    column: super::scope::column_pb(DEFAULT_COLUMN) as i32,
                    description: request.description.clone(),
                },
                &request.reason,
            );
            return Ok(TodoApplied::Ask(asked, ask));
        }
        let _write = lock(&self.writes);
        if !depends_on.is_empty() {
            let edges = self.all_dependency_edges()?;
            if let Some(cycle) = ownership::find_cycle("", &depends_on, &edges) {
                return Err(super::dependency_loop(&cycle));
            }
        }
        let owner = match &key {
            BoardKey::Global => keys::GLOBAL_OWNER.to_owned(),
            _ => channel.clone(),
        };
        keys::prefix_of(&self.store, &owner, &channel_name)
            .map_err(|error| self.failed(error, "choose the item's key prefix"))?;
        let rank = self.rank_in(&key, DEFAULT_COLUMN, "", None)?;
        let record = self
            .store
            .create_board_item(&NewBoardItem {
                board: key,
                channel,
                title: title.to_owned(),
                description: request.description.clone(),
                column: DEFAULT_COLUMN,
                rank,
                created_by: by_of(caller),
                at_unix_ms: now,
                sessions,
                depends_on,
            })
            .map_err(|error| self.failed(error, "make the board item"))?;
        let fields = Fields {
            due,
            effort,
            impact,
            need,
            fuzzy_due,
            external_block,
            from_keys,
            source_anchor,
            tags: clean_tags(&request.tags),
            steps: steps_of(&request.steps),
            // What an agent adds waits for the user's yes.
            suggested: matches!(caller, Caller::Agent(_)),
            ..Fields::default()
        };
        let mut map = saved::load(&self.store)?;
        map.insert(record.key.clone(), fields.clone());
        saved::save(&self.store, &map)?;
        let entry = Entry { record, fields };
        let archived = self.archived()?;
        self.tell(caller, pb::Change::Created, &entry.shown_record(), &archived);
        Ok(TodoApplied::Done(Box::new(pb::WriteTodoResponse {
            delivery: pb::Delivery::Applied as i32,
            item: Some(self.todo_item(caller, &entry, &[], &archived)),
            journal: None,
            provisional_id: None,
        })))
    }

    pub(super) fn update_todo_in(
        &self,
        caller: &Caller,
        request: &pb::UpdateTodoRequest,
        approved: bool,
        now: i64,
    ) -> Result<TodoApplied, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let id = item_id(request.id.as_ref())?;
        let patch = request.patch.clone().unwrap_or_default();
        if patch
            .title
            .as_deref()
            .is_some_and(|title| title.trim().is_empty())
        {
            return Err(missing("patch.title"));
        }
        if patch.accept_suggestion && !matches!(caller, Caller::User(_)) {
            return Err(invalid_field(
                "patch.accept_suggestion",
                Problem::NotAllowedHere,
                "only the user can say yes to a suggested item",
                "leave accept_suggestion out; the user answers it on the item",
            ));
        }
        let parsed = Parsed::of(&patch)?;
        let record = self.load(caller, &id, false)?;
        let add = self.known_sessions(&patch.sessions_add, "patch.sessions_add")?;
        let remove = ids(&patch.sessions_remove);
        let deps_add = self.readable_targets(caller, &patch.depends_on_add)?;
        let deps_remove = item_ids(&patch.depends_on_remove);
        let channel = match (&patch.channel, patch.clear_channel) {
            (Some(channel), false) if !channel.value.is_empty() => {
                // The channel must exist.
                self.resolve_board(&pb::BoardRef {
                    board: Some(pb::board_ref::Board::Channel(channel.clone())),
                })?;
                Some(channel.value.clone())
            }
            (_, true) => Some(String::new()),
            _ => None,
        };

        let current = Entry {
            fields: saved::load(&self.store)?
                .get(&id)
                .cloned()
                .unwrap_or_default(),
            record: record.clone(),
        };
        let act = BoardAct::Edit {
            created_by: made_by_session(&record.created_by),
        };
        let closes = matches!(parsed.state, Some(State::Closed));
        let close = if closes {
            self.gate_todo(caller, Capability::TodoClose, approved)?
        } else {
            None
        };
        let asked = match close {
            Some(asked) => Some(asked),
            None => self.gate(caller, self.policy_of(&record), act, approved)?,
        };
        if let Some(asked) = asked {
            let planned = apply_patch(&current, &patch, &parsed, channel.clone(), &add, &remove, &deps_add, &deps_remove, now);
            return Ok(TodoApplied::Ask(
                asked,
                change_ask(&record, planned.changes(&current), false, &request.reason),
            ));
        }

        let _write = lock(&self.writes);
        let fresh_record = self.load(caller, &id, false)?;
        let mut map = saved::load(&self.store)?;
        let fresh = Entry {
            fields: map.get(&id).cloned().unwrap_or_default(),
            record: fresh_record,
        };
        let expected = request
            .expected_revision
            .as_ref()
            .map(|revision| revision.value);
        if let Some(expected) = expected
            && expected != fresh.revision()
        {
            return Err(revision_conflict(&fresh.shown_record(), Some(expected)));
        }
        let mut planned = apply_patch(&fresh, &patch, &parsed, channel, &add, &remove, &deps_add, &deps_remove, now);
        self.automate_update(&fresh, &mut planned, &by_of(caller), now)?;
        if let Some(new_deps) = &planned.depends_on {
            let edges = self.all_dependency_edges()?;
            if let Some(cycle) = ownership::find_cycle(&id, new_deps, &edges) {
                return Err(super::dependency_loop(&cycle));
            }
        }
        let mut record = fresh.record.clone();
        let store_changes = planned.board_change(&fresh, by_of(caller), now);
        let mut fields = planned.fields.clone();
        if let Some(mut change) = store_changes {
            if let Some(column) = change.column {
                change.rank = Some(self.rank_in(&fresh.record.board, column, &id, None)?);
            }
            let write = self
                .store
                .change_board_item(&change)
                .map_err(|error| self.failed(error, "change the board item"))?;
            record = match write {
                BoardWrite::Applied(item) => item,
                BoardWrite::Stale { current } => {
                    return Err(revision_conflict(&current, expected));
                }
                BoardWrite::Gone { tombstone } => return Err(no_item(&tombstone.key)),
                BoardWrite::Loop { cycle } => return Err(super::dependency_loop(&cycle)),
                BoardWrite::MissingDependency { key } | BoardWrite::DeletedDependency { key } => {
                    return Err(no_item(&key));
                }
            };
        } else if fields != fresh.fields {
            fields.extra_revisions += 1;
        }
        let new_state = fields.state(record.column);
        let state_became_blocker_gone =
            new_state != fresh.state() && matches!(new_state, State::WontDo | State::Became);
        if fields != fresh.fields || state_became_blocker_gone {
            map.insert(id.clone(), fields.clone());
            if state_became_blocker_gone {
                self.notify_blocker_gone(&mut map, &id, new_state, &fields.became_keys)?;
            }
            saved::save(&self.store, &map)?;
        }
        if !patch.watched_prs_add.is_empty() {
            self.poke_pr_watcher();
        }
        let entry = Entry { record, fields };
        let archived = self.archived()?;
        if entry.revision() != fresh.revision() {
            self.tell(caller, pb::Change::Updated, &entry.shown_record(), &archived);
        }
        Ok(TodoApplied::Done(Box::new(pb::WriteTodoResponse {
            delivery: pb::Delivery::Applied as i32,
            item: Some(self.todo_item(caller, &entry, &[], &archived)),
            journal: None,
            provisional_id: None,
        })))
    }

    fn apply_todo(
        &self,
        caller: &Caller,
        op: &TodoOp,
        approved: bool,
        now: i64,
    ) -> Result<TodoApplied, Status> {
        match op {
            TodoOp::Create(request) => self.create_todo_in(caller, request, approved, now),
            TodoOp::Update(request) => self.update_todo_in(caller, request, approved, now),
        }
    }

    /// Runs a write: applied now, or put in the Inbox when a rule asks, and
    /// run once the user approves it (BD-15, D846).
    async fn todo_write(
        &self,
        caller: Caller,
        op: TodoOp,
    ) -> Result<Response<pb::WriteTodoResponse>, Status> {
        let (name, what, idempotency) = op.identity();
        let key = answer_key(
            name,
            &caller,
            idempotency
                .clone()
                .map(|value| pb::IdempotencyKey { value })
                .as_ref(),
            &what,
        );
        if let Some(done) = key.as_deref().and_then(|key| self.finished_todo.get(key)) {
            return Ok(Response::new(done));
        }
        let (me, who, again) = (self.clone(), caller.clone(), op.clone());
        let applied = blocking(move || me.apply_todo(&who, &again, false, now_ms())).await??;
        match applied {
            TodoApplied::Done(done) => {
                let done = self.settle_response(&caller, *done, now_ms()).await?;
                if let Some(key) = key {
                    self.finished_todo.put(key, done.clone());
                }
                Ok(Response::new(done))
            }
            TodoApplied::Ask(asked, board) => {
                let (me, who) = (self.clone(), caller.clone());
                let run: Run = Box::new(move |_| {
                    Box::pin(async move {
                        let (inner, who) = (me.clone(), who);
                        let settle_as = who.clone();
                        let applied =
                            blocking(move || inner.apply_todo(&who, &op, true, now_ms()))
                                .await??;
                        match applied {
                            TodoApplied::Done(done) => {
                                let done = me.settle_response(&settle_as, *done, now_ms()).await?;
                                Ok(Deferred {
                                    result: done.encode_to_vec(),
                                    result_type: "humata.v1.WriteTodoResponse".to_owned(),
                                })
                            }
                            TodoApplied::Ask(..) => Err(internal(
                                "an approved call was sent to Ask me a second time",
                                "repeat the call",
                            )),
                        }
                    })
                });
                Err(self
                    .inbox
                    .defer_board(&caller, *asked, idempotency, board, run)
                    .await)
            }
        }
    }

    /// The caller of a to-do call, refused when it is a peer, or an agent on
    /// a machine, whose calls aren't forwarded to the host yet.
    fn todo_caller<T>(&self, request: &Request<T>) -> Result<Caller, Status> {
        let caller = caller_of(request)?;
        Self::refuse_peer(&caller)?;
        if self.forwarder_for(&caller).is_some() || self.away_agent(&caller).is_some() {
            return Err(not_built_status("the To-do list for an agent on a machine"));
        }
        Ok(caller)
    }
}

/// A patch's days and enums, checked once.
struct Parsed {
    due: Option<Day>,
    hide_until: Option<Day>,
    effort: Option<Option<Effort>>,
    impact: Option<Option<Impact>>,
    state: Option<State>,
    need: Option<Option<Need>>,
    fuzzy_due: Option<Option<FuzzyDue>>,
    owner: Option<TodoOwnerRow>,
    checkpoint: Option<Option<CheckpointState>>,
}

impl Parsed {
    fn of(patch: &pb::TodoPatch) -> Result<Self, Status> {
        Ok(Self {
            due: civil(patch.due.as_ref(), "patch.due")?,
            hide_until: civil(patch.hide_until.as_ref(), "patch.hide_until")?,
            effort: patch
                .effort
                .map(|value| effort_of(value, "patch.effort"))
                .transpose()?,
            impact: patch
                .impact
                .map(|value| impact_of(value, "patch.impact"))
                .transpose()?,
            state: patch
                .state
                .map(|value| state_of(value, "patch.state"))
                .transpose()?,
            need: patch
                .need
                .map(|value| need_of(value, "patch.need"))
                .transpose()?,
            fuzzy_due: patch
                .fuzzy_due
                .map(|value| fuzzy_due_of(value, "patch.fuzzy_due"))
                .transpose()?,
            owner: patch
                .owner
                .as_ref()
                .map(|owner| owner_of(owner, "patch.owner"))
                .transpose()?,
            checkpoint: patch
                .checkpoint
                .map(|value| checkpoint_of(value, "patch.checkpoint"))
                .transpose()?,
        })
    }
}

/// What a patch makes of an item.
pub(super) struct Planned {
    pub(super) fields: Fields,
    title: Option<String>,
    description: Option<String>,
    pub(super) sessions: Option<Vec<String>>,
    depends_on: Option<Vec<String>>,
    pub(super) column: Option<i32>,
    /// What the watched pull requests now say, for `apply_pr_prompts`.
    pub(super) new_prompts: Vec<LifecyclePromptRow>,
}

impl Planned {
    /// The change to the board item, when the patch touches what the store
    /// keeps.
    fn board_change(&self, from: &Entry, by: String, now: i64) -> Option<BoardItemChange> {
        (self.title.is_some()
            || self.description.is_some()
            || self.sessions.is_some()
            || self.depends_on.is_some()
            || self.column.is_some())
        .then(|| BoardItemChange {
            key: from.record.key.clone(),
            expected_revision: None,
            by,
            at_unix_ms: now,
            title: self.title.clone(),
            description: self.description.clone(),
            column: self.column,
            rank: None,
            sessions: self.sessions.clone(),
            depends_on: self.depends_on.clone(),
            delete: false,
            cause: String::new(),
        })
    }

    /// The card's changes, field by field, as the Inbox shows them.
    fn changes(&self, from: &Entry) -> Vec<pb::FieldChange> {
        let mut out = Vec::new();
        let mut field = |name: &str, before: String, after: String| {
            if before != after {
                out.push(field_change(name, &before, &after));
            }
        };
        let record = &from.record;
        field("title", record.title.clone(), self.title.clone().unwrap_or_else(|| record.title.clone()));
        field(
            "description",
            record.description.clone(),
            self.description.clone().unwrap_or_else(|| record.description.clone()),
        );
        field(
            "sessions",
            record.sessions.join(", "),
            self.sessions.clone().unwrap_or_else(|| record.sessions.clone()).join(", "),
        );
        field(
            "depends_on",
            record.depends_on.join(", "),
            self.depends_on.clone().unwrap_or_else(|| record.depends_on.clone()).join(", "),
        );
        let (old, new) = (&from.fields, &self.fields);
        field("status", state_label(from.state()).to_owned(), state_label(new.state(self.column.unwrap_or(record.column))).to_owned());
        field("due", day_text(old.due), day_text(new.due));
        field("hide_until", day_text(old.hide_until), day_text(new.hide_until));
        field("effort", format!("{:?}", old.effort), format!("{:?}", new.effort));
        field("impact", format!("{:?}", old.impact), format!("{:?}", new.impact));
        field("tags", old.tags.join(", "), new.tags.join(", "));
        field(
            "steps",
            old.steps.len().to_string(),
            new.steps.len().to_string(),
        );
        field("pin_order", old.pin_order.to_string(), new.pin_order.to_string());
        field(
            "channel",
            from.channel(),
            new.channel.clone().unwrap_or_else(|| from.channel()),
        );
        field("suggested", old.suggested.to_string(), new.suggested.to_string());
        out
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_patch(
    from: &Entry,
    patch: &pb::TodoPatch,
    parsed: &Parsed,
    channel: Option<String>,
    sessions_add: &[String],
    sessions_remove: &[String],
    deps_add: &[String],
    deps_remove: &[String],
    now: i64,
) -> Planned {
    let record = &from.record;
    let mut fields = from.fields.clone();
    let title = patch
        .title
        .as_ref()
        .map(|title| title.trim().to_owned())
        .filter(|title| *title != record.title);
    let description = patch
        .description
        .clone()
        .filter(|text| *text != record.description);
    let sessions = merged(&record.sessions, sessions_add, sessions_remove);
    let deps = merged(&record.depends_on, deps_add, deps_remove);

    if patch.clear_due {
        fields.due = None;
    } else if parsed.due.is_some() {
        fields.due = parsed.due;
    }
    if patch.clear_hide_until {
        fields.hide_until = None;
    } else if parsed.hide_until.is_some() {
        fields.hide_until = parsed.hide_until;
    }
    if let Some(effort) = parsed.effort {
        fields.effort = effort;
    }
    if let Some(impact) = parsed.impact {
        fields.impact = impact;
    }
    if !patch.tags_add.is_empty() || !patch.tags_remove.is_empty() {
        let remove = clean_tags(&patch.tags_remove);
        let kept: Vec<String> = fields
            .tags
            .iter()
            .filter(|tag| !remove.contains(tag))
            .cloned()
            .collect();
        let add = clean_tags(&patch.tags_add);
        fields.tags = merged(&kept, &add, &[]);
    }
    if let Some(steps) = &patch.steps {
        fields.steps = steps_of(&steps.steps);
    }
    if let Some(pin) = patch.pin_order {
        fields.pin_order = pin;
    }
    if let Some(channel) = channel {
        fields.channel = Some(channel);
    }
    if patch.accept_suggestion {
        fields.suggested = false;
    }
    if let Some(need) = parsed.need {
        fields.need = need;
    }
    if let Some(fuzzy_due) = parsed.fuzzy_due {
        fields.fuzzy_due = fuzzy_due;
    }
    if patch.clear_external_block {
        fields.external_block = None;
    } else if let Some(block) = &patch.external_block {
        fields.external_block = Some(external_block_of(block, now));
    }
    if patch.clear_owner {
        fields.owner = None;
    } else if let Some(owner) = &parsed.owner {
        fields.owner = Some(owner.clone());
    }
    if let Some(checkpoint) = parsed.checkpoint {
        fields.checkpoint = checkpoint;
    }
    if let Some(reason) = &patch.wont_do_reason {
        let trimmed = reason.trim();
        fields.wont_do_reason = (!trimmed.is_empty()).then(|| trimmed.to_owned());
    }
    let mut new_prompts = Vec::new();
    if !patch.watched_prs_add.is_empty() {
        for pr_pb in &patch.watched_prs_add {
            let pr = watched_pr_of(pr_pb);
            let key = pr.repo_pr();
            if let Some(existing) = fields
                .watched_prs
                .iter_mut()
                .find(|p| p.repo_pr() == key)
            {
                *existing = pr.clone();
            } else {
                fields.watched_prs.push(pr.clone());
            }
            new_prompts.extend(pr_watch::evaluate_watched_pr(&pr));
        }
        let commits = fields.close_readiness.as_ref().map_or(0, |cr| cr.commits);
        fields.close_readiness = Some(pr_watch::compute_close_readiness(
            &fields.steps,
            &fields.watched_prs,
            commits,
        ));
    }
    let mut column = None;
    if let Some(state) = parsed.state
        && state != from.state()
    {
        let target = saved::column_for(state, record.column);
        column = (target != record.column).then_some(target);
        fields.state = Some(state);
        fields.state_column = Some(target);
        fields.closed_at_ms = state.is_terminal().then_some(now);
    }
    Planned {
        fields,
        title,
        description,
        sessions: (sessions != record.sessions).then_some(sessions),
        depends_on: (deps != record.depends_on).then_some(deps),
        column,
        new_prompts,
    }
}

// ------------------------------------------------------------- migration

/// What each board item becomes in the list, and the counts of the move
/// (D833): the domain's mapping, written as the fields this crate keeps.
fn plan_migration(
    records: &[BoardItemRecord],
    archived: &HashSet<String>,
    vip: &HashSet<String>,
    now: i64,
) -> (BTreeMap<String, Fields>, MigrationRow) {
    let fallback = day_of(now);
    let day = |ms: i64| day_of(ms).or(fallback);
    let board_of = |record: &BoardItemRecord| match &record.board {
        BoardKey::Global => None,
        BoardKey::Channel(id) | BoardKey::Session(id) => Some(id.clone()),
    };
    let old: Vec<domain::OldItem> = records
        .iter()
        .filter(|record| !record.deleted)
        .filter_map(|record| {
            let entry = Entry {
                record: record.clone(),
                fields: Fields::default(),
            };
            Some(domain::OldItem {
                key: record.key.clone(),
                title: record.title.clone(),
                column: old_column(record.column),
                board: board_of(record),
                rank: record.rank.clone(),
                vip: vip.contains(&record.key),
                pinned: false,
                channel: (!record.channel.is_empty()).then(|| record.channel.clone()),
                sessions: entry.sessions(),
                session_archived: matches!(&record.board, BoardKey::Session(id) if archived.contains(id)),
                created: day(record.created_unix_ms)?,
                done_at: record.done_unix_ms.and_then(day),
            })
        })
        .collect();
    let (todos, summary) = domain::migrate_with_summary(old);
    let by_key: BTreeMap<&str, &BoardItemRecord> = records
        .iter()
        .map(|record| (record.key.as_str(), record))
        .collect();
    let mut planned = BTreeMap::new();
    for todo in todos {
        let Some(record) = by_key.get(todo.key.as_str()) else {
            continue;
        };
        let closed = todo.state == State::Closed;
        planned.insert(
            todo.key.clone(),
            Fields {
                state: Some(todo.state),
                state_column: Some(record.column),
                pin_order: todo.pin.map_or(0, |pin| pin + 1),
                start: todo.start.as_ref().map(|start| StartRow {
                    column: record.column,
                    position: start.position,
                    board_channel: (!record.channel.is_empty()).then(|| record.channel.clone()),
                    was_vip: start.vip,
                }),
                closed_at_ms: closed.then(|| {
                    record
                        .done_unix_ms
                        .unwrap_or(record.created_unix_ms)
                }),
                ..Fields::default()
            },
        );
    }
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let row = MigrationRow {
        migrated_at_ms: now,
        items: count(summary.items),
        open: count(summary.open),
        in_progress: count(summary.in_progress),
        closed: count(summary.closed),
        boards: count(summary.boards),
        banner_dismissed: false,
    };
    (planned, row)
}

// ----------------------------------------------------------- the service

#[tonic::async_trait]
impl pb::todo_service_server::TodoService for BoardSvc {
    async fn list_todo(
        &self,
        request: Request<pb::ListTodoRequest>,
    ) -> Result<Response<pb::ListTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        self.refresh_review_requests_if_stale(now_ms());
        let asks = self.asks_for(&caller).await;
        let me = self.clone();
        blocking(move || me.list_todo_in(&caller, &request, &asks, now_ms()))
            .await?
            .map(Response::new)
    }

    async fn create_todo(
        &self,
        request: Request<pb::CreateTodoRequest>,
    ) -> Result<Response<pb::WriteTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        self.todo_write(caller, TodoOp::Create(request.into_inner()))
            .await
    }

    async fn update_todo(
        &self,
        request: Request<pb::UpdateTodoRequest>,
    ) -> Result<Response<pb::WriteTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        self.todo_write(caller, TodoOp::Update(request.into_inner()))
            .await
    }

    async fn next_todo(
        &self,
        request: Request<pb::NextTodoRequest>,
    ) -> Result<Response<pb::NextTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        self.refresh_review_requests_if_stale(now_ms());
        // Ranking needs every item's ask, whoever is calling.
        let asks = self.inbox.pending_board_asks().await;
        let me = self.clone();
        blocking(move || me.next_todo_in(&caller, &request, &asks, now_ms()))
            .await?
            .map(Response::new)
    }

    async fn get_todo_migration(
        &self,
        request: Request<pb::GetTodoMigrationRequest>,
    ) -> Result<Response<pb::TodoMigration>, Status> {
        let _caller = self.todo_caller(&request)?;
        let me = self.clone();
        blocking(move || me.ensure_migrated(now_ms()))
            .await?
            .map(|row| Response::new(Self::migration_pb(&row)))
    }

    async fn dismiss_todo_migration(
        &self,
        request: Request<pb::DismissTodoMigrationRequest>,
    ) -> Result<Response<pb::TodoMigration>, Status> {
        let _caller = self.todo_caller(&request)?;
        let me = self.clone();
        blocking(move || me.dismiss_migration_in(now_ms()))
            .await?
            .map(Response::new)
    }

    async fn add_todo_comment(
        &self,
        request: Request<pb::AddTodoCommentRequest>,
    ) -> Result<Response<pb::AddTodoCommentResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        let me = self.clone();
        blocking(move || me.add_todo_comment_in(&caller, &request, now_ms()))
            .await?
            .map(Response::new)
    }

    async fn split_todo(
        &self,
        request: Request<pb::SplitTodoRequest>,
    ) -> Result<Response<pb::SplitTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        let me = self.clone();
        blocking(move || me.split_todo_in(&caller, &request, now_ms()))
            .await?
            .map(Response::new)
    }

    async fn fold_todo_into_step(
        &self,
        request: Request<pb::FoldTodoIntoStepRequest>,
    ) -> Result<Response<pb::FoldTodoIntoStepResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        let me = self.clone();
        blocking(move || me.fold_todo_into_step_in(&caller, &request, now_ms()))
            .await?
            .map(Response::new)
    }

    async fn batch_update_todo(
        &self,
        request: Request<pb::BatchUpdateTodoRequest>,
    ) -> Result<Response<pb::BatchUpdateTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let caller_in = caller.clone();
        let request = request.into_inner();
        let me = self.clone();
        let mut done = blocking(move || me.batch_update_todo_in(&caller_in, &request, now_ms()))
            .await??;
        for item in &mut done.updated {
            if let Some(settled) = self.settle_item(&caller, item, now_ms()).await? {
                *item = settled;
            }
        }
        Ok(Response::new(done))
    }

    async fn list_todo_runs(
        &self,
        request: Request<pb::ListTodoRunsRequest>,
    ) -> Result<Response<pb::ListTodoRunsResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        let me = self.clone();
        blocking(move || me.list_todo_runs_in(&caller, &request))
            .await?
            .map(Response::new)
    }

    async fn get_todo_history(
        &self,
        request: Request<pb::GetTodoHistoryRequest>,
    ) -> Result<Response<pb::GetTodoHistoryResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        let me = self.clone();
        blocking(move || me.get_todo_history_in(&caller, &request))
            .await?
            .map(Response::new)
    }

    async fn resolve_todo_ownership(
        &self,
        request: Request<pb::ResolveTodoOwnershipRequest>,
    ) -> Result<Response<pb::WriteTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        let me = self.clone();
        blocking(move || me.resolve_todo_ownership_in(&caller, &request, now_ms()))
            .await?
            .map(Response::new)
    }

    async fn resolve_blocker_gone(
        &self,
        request: Request<pb::ResolveBlockerGoneRequest>,
    ) -> Result<Response<pb::WriteTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        let me = self.clone();
        let who = caller.clone();
        let done = blocking(move || me.resolve_blocker_gone_in(&who, &request, now_ms()))
            .await??;
        self.settle_response(&caller, done, now_ms())
            .await
            .map(Response::new)
    }

    async fn resolve_lifecycle_prompt(
        &self,
        request: Request<pb::ResolveLifecyclePromptRequest>,
    ) -> Result<Response<pb::WriteTodoResponse>, Status> {
        let caller = self.todo_caller(&request)?;
        let request = request.into_inner();
        self.resolve_prompt_acting(&caller, request, now_ms())
            .await
            .map(Response::new)
    }
}

impl BoardSvc {
    pub(super) fn add_todo_comment_in(
        &self,
        caller: &Caller,
        request: &pb::AddTodoCommentRequest,
        now: i64,
    ) -> Result<pb::AddTodoCommentResponse, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let id = item_id(request.id.as_ref())?;
        let body = request.body.trim();
        if body.is_empty() {
            return Err(missing("body"));
        }
        let _write = lock(&self.writes);
        let record = self.load(caller, &id, false)?;
        let mut map = saved::load(&self.store)?;
        let mut fields = map.get(&id).cloned().unwrap_or_default();
        let author = match caller {
            Caller::User(_) => "You".to_owned(),
            _ => by_of(caller),
        };
        let comment = TodoCommentRow {
            id: format!("cmt-{}-{}", fields.comments.len() + 1, now),
            author,
            body: body.to_owned(),
            created_at_ms: now,
            step_id: (!request.step_id.is_empty()).then(|| request.step_id.clone()),
        };
        fields.comments.push(comment.clone());
        fields.extra_revisions += 1;
        let rev = record.revision + fields.extra_revisions;
        fields.extra_history.push(TodoHistoryRow {
            at_unix_ms: now,
            by: by_of(caller),
            kind: pb::HistoryKind::Commented as i32,
            field: "comment".to_owned(),
            before: String::new(),
            after: comment.body.clone(),
            change: format!("commented: {}", comment.body),
            revision: rev,
        });
        map.insert(id, fields.clone());
        saved::save(&self.store, &map)?;
        let entry = Entry { record, fields };
        let archived = self.archived()?;
        self.tell(caller, pb::Change::Updated, &entry.shown_record(), &archived);
        Ok(pb::AddTodoCommentResponse {
            item: Some(self.todo_item(caller, &entry, &[], &archived)),
            comment: Some(pb::TodoComment {
                id: comment.id,
                author: comment.author,
                body: comment.body,
                created_at: Some(timestamp_ms(comment.created_at_ms)),
                step_id: comment.step_id.unwrap_or_default(),
            }),
        })
    }

    pub(super) fn split_todo_in(
        &self,
        caller: &Caller,
        request: &pb::SplitTodoRequest,
        now: i64,
    ) -> Result<pb::SplitTodoResponse, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let id = item_id(request.id.as_ref())?;
        let titles: Vec<String> = request
            .titles
            .iter()
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .collect();
        if titles.is_empty() {
            return Err(missing("titles"));
        }
        let _write = lock(&self.writes);
        let source_record = self.load(caller, &id, false)?;
        let mut map = saved::load(&self.store)?;
        let mut source_fields = map.get(&id).cloned().unwrap_or_default();
        let created_at = if request.keep_age {
            source_record.created_unix_ms
        } else {
            now
        };
        let channel = source_fields
            .channel
            .clone()
            .unwrap_or_else(|| source_record.channel.clone());
        let board = if channel.is_empty() {
            BoardKey::Global
        } else {
            BoardKey::Channel(channel.clone())
        };
        let owner = match &board {
            BoardKey::Global => keys::GLOBAL_OWNER.to_owned(),
            _ => channel.clone(),
        };
        keys::prefix_of(&self.store, &owner, &channel)
            .map_err(|error| self.failed(error, "choose the item's key prefix"))?;

        let mut created_entries = Vec::with_capacity(titles.len());
        let mut created_keys = Vec::with_capacity(titles.len());
        for title in titles {
            let rank = self.rank_in(&board, DEFAULT_COLUMN, "", None)?;
            let rec = self
                .store
                .create_board_item(&NewBoardItem {
                    board: board.clone(),
                    channel: channel.clone(),
                    title,
                    description: String::new(),
                    column: DEFAULT_COLUMN,
                    rank,
                    created_by: by_of(caller),
                    at_unix_ms: created_at,
                    sessions: Vec::new(),
                    depends_on: Vec::new(),
                })
                .map_err(|error| self.failed(error, "make the split item"))?;
            let child_fields = Fields {
                from_keys: vec![id.clone()],
                need: source_fields.need,
                tags: source_fields.tags.clone(),
                channel: source_fields.channel.clone(),
                ..Fields::default()
            };
            created_keys.push(rec.key.clone());
            map.insert(rec.key.clone(), child_fields.clone());
            created_entries.push(Entry {
                record: rec,
                fields: child_fields,
            });
        }

        let target_col = saved::column_for(State::Became, source_record.column);
        let updated_source = if target_col != source_record.column {
            let rank = self.rank_in(&source_record.board, target_col, &id, None)?;
            match self
                .store
                .change_board_item(&BoardItemChange {
                    key: id.clone(),
                    expected_revision: None,
                    by: by_of(caller),
                    at_unix_ms: now,
                    title: None,
                    description: None,
                    column: Some(target_col),
                    rank: Some(rank),
                    sessions: None,
                    depends_on: None,
                    delete: false,
                    cause: String::new(),
                })
                .map_err(|error| self.failed(error, "update split source item"))?
            {
                BoardWrite::Applied(item) => item,
                _ => source_record.clone(),
            }
        } else {
            source_record.clone()
        };
        source_fields.state = Some(State::Became);
        source_fields.state_column = Some(target_col);
        source_fields.closed_at_ms = Some(now);
        source_fields.became_keys = merged(&source_fields.became_keys, &created_keys, &[]);
        source_fields.extra_revisions += 1;
        let rev = updated_source.revision + source_fields.extra_revisions;
        source_fields.extra_history.push(TodoHistoryRow {
            at_unix_ms: now,
            by: by_of(caller),
            kind: pb::HistoryKind::Split as i32,
            field: "became_keys".to_owned(),
            before: String::new(),
            after: created_keys.join(", "),
            change: format!("split into {}", created_keys.join(", ")),
            revision: rev,
        });
        map.insert(id.clone(), source_fields.clone());
        self.notify_blocker_gone(&mut map, &id, State::Became, &created_keys)?;
        saved::save(&self.store, &map)?;

        let archived = self.archived()?;
        let source_entry = Entry {
            record: updated_source,
            fields: source_fields,
        };
        self.tell(
            caller,
            pb::Change::Updated,
            &source_entry.shown_record(),
            &archived,
        );
        for child in &created_entries {
            self.tell(caller, pb::Change::Created, &child.shown_record(), &archived);
        }
        Ok(pb::SplitTodoResponse {
            source: Some(self.todo_item(caller, &source_entry, &[], &archived)),
            created: created_entries
                .iter()
                .map(|e| self.todo_item(caller, e, &[], &archived))
                .collect(),
        })
    }

    pub(super) fn fold_todo_into_step_in(
        &self,
        caller: &Caller,
        request: &pb::FoldTodoIntoStepRequest,
        now: i64,
    ) -> Result<pb::FoldTodoIntoStepResponse, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let source_id = item_id(request.source_id.as_ref())?;
        let target_id = item_id(request.target_id.as_ref())?;
        if source_id == target_id {
            return Err(invalid_field(
                "target_id",
                Problem::NotAllowedHere,
                "cannot fold an item into itself",
                "choose a different target item",
            ));
        }
        let _write = lock(&self.writes);
        let source_record = self.load(caller, &source_id, false)?;
        let target_record = self.load(caller, &target_id, false)?;
        let mut map = saved::load(&self.store)?;
        let mut source_fields = map.get(&source_id).cloned().unwrap_or_default();
        let mut target_fields = map.get(&target_id).cloned().unwrap_or_default();

        let step = StepRow {
            id: format!("step-fold-{}-{}", target_fields.steps.len() + 1, now),
            text: source_record.title.clone(),
            done: false,
            ticked_by: None,
            markdown_context: (!source_record.description.is_empty())
                .then(|| source_record.description.clone()),
            dropped: false,
            from_key: Some(source_id.clone()),
            source_anchor: source_fields.source_anchor.clone(),
        };
        target_fields.steps.push(step);
        target_fields.from_keys = merged(&target_fields.from_keys, &[source_id.clone()], &[]);
        target_fields.extra_revisions += 1;
        let target_rev = target_record.revision + target_fields.extra_revisions;
        target_fields.extra_history.push(TodoHistoryRow {
            at_unix_ms: now,
            by: by_of(caller),
            kind: pb::HistoryKind::Folded as i32,
            field: "steps".to_owned(),
            before: String::new(),
            after: source_record.title.clone(),
            change: format!("folded {source_id} into step"),
            revision: target_rev,
        });

        let target_col = saved::column_for(State::Became, source_record.column);
        let updated_source = if target_col != source_record.column {
            let rank = self.rank_in(&source_record.board, target_col, &source_id, None)?;
            match self
                .store
                .change_board_item(&BoardItemChange {
                    key: source_id.clone(),
                    expected_revision: None,
                    by: by_of(caller),
                    at_unix_ms: now,
                    title: None,
                    description: None,
                    column: Some(target_col),
                    rank: Some(rank),
                    sessions: None,
                    depends_on: None,
                    delete: false,
                    cause: String::new(),
                })
                .map_err(|error| self.failed(error, "update folded source item"))?
            {
                BoardWrite::Applied(item) => item,
                _ => source_record.clone(),
            }
        } else {
            source_record.clone()
        };
        source_fields.state = Some(State::Became);
        source_fields.state_column = Some(target_col);
        source_fields.closed_at_ms = Some(now);
        source_fields.became_keys = merged(&source_fields.became_keys, &[target_id.clone()], &[]);
        source_fields.extra_revisions += 1;
        let source_rev = updated_source.revision + source_fields.extra_revisions;
        source_fields.extra_history.push(TodoHistoryRow {
            at_unix_ms: now,
            by: by_of(caller),
            kind: pb::HistoryKind::Folded as i32,
            field: "became_keys".to_owned(),
            before: String::new(),
            after: target_id.clone(),
            change: format!("folded into {target_id}"),
            revision: source_rev,
        });

        map.insert(source_id.clone(), source_fields.clone());
        map.insert(target_id.clone(), target_fields.clone());
        self.notify_blocker_gone(&mut map, &source_id, State::Became, &[target_id.clone()])?;
        saved::save(&self.store, &map)?;

        let archived = self.archived()?;
        let source_entry = Entry {
            record: updated_source,
            fields: source_fields,
        };
        let target_entry = Entry {
            record: target_record,
            fields: target_fields,
        };
        self.tell(
            caller,
            pb::Change::Updated,
            &source_entry.shown_record(),
            &archived,
        );
        self.tell(
            caller,
            pb::Change::Updated,
            &target_entry.shown_record(),
            &archived,
        );
        Ok(pb::FoldTodoIntoStepResponse {
            source: Some(self.todo_item(caller, &source_entry, &[], &archived)),
            target: Some(self.todo_item(caller, &target_entry, &[], &archived)),
        })
    }

    pub(super) fn batch_update_todo_in(
        &self,
        caller: &Caller,
        request: &pb::BatchUpdateTodoRequest,
        now: i64,
    ) -> Result<pb::BatchUpdateTodoResponse, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let mut updated = Vec::new();
        let mut failed = Vec::new();
        for entry in &request.entries {
            let key = entry
                .id
                .as_ref()
                .map(|id| id.value.clone())
                .unwrap_or_default();
            let update_req = pb::UpdateTodoRequest {
                idempotency_key: None,
                id: entry.id.clone(),
                patch: entry.patch.clone(),
                expected_revision: entry.expected_revision,
                reason: String::new(),
                evidence: Vec::new(),
            };
            match self.update_todo_in(caller, &update_req, false, now) {
                Ok(TodoApplied::Done(resp)) => {
                    if let Some(item) = resp.item {
                        updated.push(item);
                    }
                }
                Ok(TodoApplied::Ask(..)) => {
                    failed.push(pb::BatchUpdateTodoFailure {
                        id: entry.id.clone(),
                        key,
                        reason: "requires approval".to_owned(),
                    });
                }
                Err(status) => {
                    failed.push(pb::BatchUpdateTodoFailure {
                        id: entry.id.clone(),
                        key,
                        reason: status.message().to_owned(),
                    });
                }
            }
        }
        Ok(pb::BatchUpdateTodoResponse { updated, failed })
    }

    pub(super) fn list_todo_runs_in(
        &self,
        caller: &Caller,
        request: &pb::ListTodoRunsRequest,
    ) -> Result<pb::ListTodoRunsResponse, Status> {
        Self::refuse_peer(caller)?;
        let id = item_id(request.id.as_ref())?;
        let _record = self.load(caller, &id, true)?;
        let records = self
            .store
            .list_todo_runs(&id)
            .map_err(|error| self.failed(error, "list to-do runs"))?;
        let mut total_cost_micros: Option<u64> = None;
        let runs = records
            .into_iter()
            .map(|r| {
                if let Some(c) = r.cost_micros {
                    total_cost_micros = Some(total_cost_micros.unwrap_or(0).saturating_add(c));
                }
                let links: Vec<domain::RunLink> =
                    serde_json::from_str(&r.links_json).unwrap_or_default();
                pb::TodoRun {
                    id: r.id,
                    tool: r.tool,
                    model: r.model,
                    outcome: r.outcome,
                    cost_micros: r.cost_micros,
                    started_at: (r.started_unix_ms > 0).then(|| timestamp_ms(r.started_unix_ms)),
                    finished_at: (r.finished_unix_ms > 0).then(|| timestamp_ms(r.finished_unix_ms)),
                    links: links
                        .into_iter()
                        .map(|l| pb::RunLink {
                            label: l.label,
                            target: l.target,
                        })
                        .collect(),
                    session: (!r.session.is_empty()).then(|| pb::SessionId { value: r.session }),
                }
            })
            .collect();
        Ok(pb::ListTodoRunsResponse {
            runs,
            total_cost_micros,
        })
    }

    pub(super) fn get_todo_history_in(
        &self,
        caller: &Caller,
        request: &pb::GetTodoHistoryRequest,
    ) -> Result<pb::GetTodoHistoryResponse, Status> {
        Self::refuse_peer(caller)?;
        let id = item_id(request.id.as_ref())?;
        let _record = self.load(caller, &id, true)?;
        let (size, start) = page_of(request.page.as_ref())?;
        let store_rows = self
            .store
            .board_item_history(&id, 0, usize::MAX / 2)
            .map_err(|error| self.failed(error, "read item history"))?;
        let fields = saved::load(&self.store)?
            .get(&id)
            .cloned()
            .unwrap_or_default();

        let mut all: Vec<(i64, pb::ItemHistoryEntry)> = store_rows
            .iter()
            .map(|row| (row.at_unix_ms, entry_pb(row, self.actor_of(&row.by))))
            .collect();
        for extra in &fields.extra_history {
            let diff = word_diff_pb(&extra.before, &extra.after);
            all.push((
                extra.at_unix_ms,
                pb::ItemHistoryEntry {
                    at: Some(pb::Timestamped {
                        at: Some(timestamp_ms(extra.at_unix_ms)),
                        by: Some(self.actor_of(&extra.by)),
                    }),
                    change: extra.change.clone(),
                    lost_conflict: false,
                    journal: None,
                    revision: Some(pb::Revision {
                        value: extra.revision,
                    }),
                    kind: extra.kind,
                    field: extra.field.clone(),
                    before: extra.before.clone(),
                    after: extra.after.clone(),
                    cause: None,
                    word_diff: diff,
                },
            ));
        }
        all.sort_by_key(|(at, _)| *at);
        let total = all.len();
        let page_entries: Vec<pb::ItemHistoryEntry> = all
            .into_iter()
            .skip(start)
            .take(size)
            .map(|(_, e)| e)
            .collect();
        let next_page_token = if start + page_entries.len() < total {
            (start + page_entries.len()).to_string()
        } else {
            String::new()
        };
        Ok(pb::GetTodoHistoryResponse {
            entries: page_entries,
            page: Some(pb::PageResponse { next_page_token }),
        })
    }

    pub(super) fn resolve_todo_ownership_in(
        &self,
        caller: &Caller,
        request: &pb::ResolveTodoOwnershipRequest,
        now: i64,
    ) -> Result<pb::WriteTodoResponse, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let id = item_id(request.id.as_ref())?;
        let _write = lock(&self.writes);
        let record = self.load(caller, &id, false)?;
        let mut map = saved::load(&self.store)?;
        let mut fields = map.get(&id).cloned().unwrap_or_default();
        let change_desc = if request.take_back {
            ownership::take_back_ownership(&mut fields, "You");
            "took back ownership".to_owned()
        } else if request.retry {
            ownership::retry_stuck_agent(&mut fields);
            "retried stuck agent".to_owned()
        } else {
            return Err(missing("take_back or retry"));
        };
        fields.extra_revisions += 1;
        let rev = record.revision + fields.extra_revisions;
        fields.extra_history.push(TodoHistoryRow {
            at_unix_ms: now,
            by: by_of(caller),
            kind: pb::HistoryKind::OwnerChanged as i32,
            field: "owner".to_owned(),
            before: String::new(),
            after: change_desc.clone(),
            change: change_desc,
            revision: rev,
        });
        map.insert(id, fields.clone());
        saved::save(&self.store, &map)?;
        let entry = Entry { record, fields };
        let archived = self.archived()?;
        self.tell(caller, pb::Change::Updated, &entry.shown_record(), &archived);
        Ok(pb::WriteTodoResponse {
            delivery: pb::Delivery::Applied as i32,
            item: Some(self.todo_item(caller, &entry, &[], &archived)),
            journal: None,
            provisional_id: None,
        })
    }

    pub(super) fn resolve_blocker_gone_in(
        &self,
        caller: &Caller,
        request: &pb::ResolveBlockerGoneRequest,
        now: i64,
    ) -> Result<pb::WriteTodoResponse, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let id = item_id(request.id.as_ref())?;
        let blocker_key = request.blocker_key.trim();
        if blocker_key.is_empty() {
            return Err(missing("blocker_key"));
        }
        let _write = lock(&self.writes);
        let mut record = self.load(caller, &id, false)?;
        let mut map = saved::load(&self.store)?;
        let mut fields = map.get(&id).cloned().unwrap_or_default();
        fields
            .blocker_gone
            .retain(|bg| bg.blocker_key != blocker_key);

        let action = request.action.trim().to_lowercase();
        let state_before = fields.state(record.column);
        let mut new_deps = record.depends_on.clone();
        new_deps.retain(|k| k != blocker_key);
        let mut new_col = None;
        match action.as_str() {
            "rewire" => {
                let rewire = clean_tags(&request.rewire_to_keys);
                new_deps = merged(&new_deps, &rewire, &[]);
                let edges = self.all_dependency_edges()?;
                if let Some(cycle) = ownership::find_cycle(&id, &new_deps, &edges) {
                    return Err(super::dependency_loop(&cycle));
                }
            }
            "close" => {
                let target_col = saved::column_for(State::Closed, record.column);
                new_col = (target_col != record.column).then_some(target_col);
                fields.state = Some(State::Closed);
                fields.state_column = Some(target_col);
                fields.closed_at_ms = Some(now);
            }
            "wont_do" => {
                let target_col = saved::column_for(State::WontDo, record.column);
                new_col = (target_col != record.column).then_some(target_col);
                fields.state = Some(State::WontDo);
                fields.state_column = Some(target_col);
                fields.closed_at_ms = Some(now);
            }
            _ => {}
        }

        if new_deps != record.depends_on || new_col.is_some() {
            let rank = match new_col {
                Some(col) => Some(self.rank_in(&record.board, col, &id, None)?),
                None => None,
            };
            let write = self
                .store
                .change_board_item(&BoardItemChange {
                    key: id.clone(),
                    expected_revision: None,
                    by: by_of(caller),
                    at_unix_ms: now,
                    title: None,
                    description: None,
                    column: new_col,
                    rank,
                    sessions: None,
                    depends_on: (new_deps != record.depends_on).then_some(new_deps),
                    delete: false,
                    cause: String::new(),
                })
                .map_err(|error| self.failed(error, "resolve blocker gone"))?;
            if let BoardWrite::Applied(item) = write {
                record = item;
            }
        }
        fields.extra_revisions += 1;
        let entry_now = Entry { record, fields };
        let (state_now, linked, channel) = (
            entry_now.state(),
            entry_now.sessions(),
            entry_now.channel(),
        );
        let Entry { record, mut fields } = entry_now;
        let rev = record.revision + fields.extra_revisions;
        self.after_close(
            &mut fields,
            &id,
            state_before,
            state_now,
            &linked,
            &channel,
            now,
            &by_of(caller),
            rev,
        )?;
        map.insert(id, fields.clone());
        saved::save(&self.store, &map)?;
        let entry = Entry { record, fields };
        let archived = self.archived()?;
        self.tell(caller, pb::Change::Updated, &entry.shown_record(), &archived);
        Ok(pb::WriteTodoResponse {
            delivery: pb::Delivery::Applied as i32,
            item: Some(self.todo_item(caller, &entry, &[], &archived)),
            journal: None,
            provisional_id: None,
        })
    }

    pub(super) fn resolve_lifecycle_prompt_in(
        &self,
        caller: &Caller,
        request: &pb::ResolveLifecyclePromptRequest,
        now: i64,
    ) -> Result<pb::WriteTodoResponse, Status> {
        Self::refuse_peer(caller)?;
        self.ensure_migrated(now)?;
        let id = item_id(request.id.as_ref())?;
        let prompt_id = request.prompt_id.trim();
        if prompt_id.is_empty() {
            return Err(missing("prompt_id"));
        }
        let _write = lock(&self.writes);
        let mut record = self.load(caller, &id, false)?;
        let mut map = saved::load(&self.store)?;
        let mut fields = map.get(&id).cloned().unwrap_or_default();
        let choice = request.choice.trim();
        let state_before = fields.state(record.column);
        let prompt = fields
            .lifecycle_prompts
            .iter()
            .find(|p| p.id == prompt_id)
            .cloned();
        let by = by_of(caller);
        fields.lifecycle_prompts.retain(|p| p.id != prompt_id);
        if let Some(prompt) = &prompt {
            self.act_on_prompt(&mut fields, &record, prompt, choice, &by, now)?;
        }
        fields.prompt_sessions.remove(prompt_id);

        let next_state = match choice.to_lowercase().as_str() {
            "close" => Some(State::Closed),
            "won't do" | "wont_do" => Some(State::WontDo),
            "return to open" | "pause" | "open" => Some(State::Open),
            "resume" | "in progress" => Some(State::InProgress),
            _ => None,
        };
        if let Some(state) = next_state {
            let target_col = saved::column_for(state, record.column);
            if target_col != record.column {
                let rank = self.rank_in(&record.board, target_col, &id, None)?;
                if let BoardWrite::Applied(item) = self
                    .store
                    .change_board_item(&BoardItemChange {
                        key: id.clone(),
                        expected_revision: None,
                        by: by_of(caller),
                        at_unix_ms: now,
                        title: None,
                        description: None,
                        column: Some(target_col),
                        rank: Some(rank),
                        sessions: None,
                        depends_on: None,
                        delete: false,
                        cause: String::new(),
                    })
                    .map_err(|error| self.failed(error, "resolve lifecycle prompt"))?
                {
                    record = item;
                }
            }
            fields.state = Some(state);
            fields.state_column = Some(target_col);
            fields.closed_at_ms = state.is_terminal().then_some(now);
            if matches!(state, State::WontDo | State::Became) {
                self.notify_blocker_gone(&mut map, &id, state, &fields.became_keys)?;
            }
            let linked = Entry {
                record: record.clone(),
                fields: Fields::default(),
            }
            .sessions();
            let channel = fields
                .channel
                .clone()
                .unwrap_or_else(|| record.channel.clone());
            let rev = record.revision + fields.extra_revisions + 1;
            self.after_close(
                &mut fields,
                &id,
                state_before,
                state,
                &linked,
                &channel,
                now,
                &by,
                rev,
            )?;
        }
        fields.extra_revisions += 1;
        let rev = record.revision + fields.extra_revisions;
        fields.extra_history.push(TodoHistoryRow {
            at_unix_ms: now,
            by: by_of(caller),
            kind: pb::HistoryKind::PrSynced as i32,
            field: "lifecycle_prompt".to_owned(),
            before: prompt_id.to_owned(),
            after: choice.to_owned(),
            change: format!("resolved prompt {prompt_id}: {choice}"),
            revision: rev,
        });
        map.insert(id, fields.clone());
        saved::save(&self.store, &map)?;
        let entry = Entry { record, fields };
        let archived = self.archived()?;
        self.tell(caller, pb::Change::Updated, &entry.shown_record(), &archived);
        Ok(pb::WriteTodoResponse {
            delivery: pb::Delivery::Applied as i32,
            item: Some(self.todo_item(caller, &entry, &[], &archived)),
            journal: None,
            provisional_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use humata2_daemon_ports::UserVia;
    use pb::board_service_server::BoardService;
    use tonic::Code;

    use super::super::test_support::{test_svc, user_req};
    use super::*;
    use crate::todo_fields::{PrRow, ReviewRow};

    fn user() -> Caller {
        Caller::User(UserVia::App)
    }

    fn ms_of(year: i32, month: u32, day: u32) -> i64 {
        let epoch = Day::from_ymd(1970, 1, 1).unwrap();
        let day = Day::from_ymd(year, month, day).unwrap();
        i64::from(epoch.days_until(day)) * DAY_MS
    }

    /// Thu 19 Nov 2026, noon, the day of the signed-off frames.
    fn now() -> i64 {
        ms_of(2026, 11, 19) + DAY_MS / 2
    }

    fn civil_day(month: u32, day: u32) -> Option<pb::CivilDay> {
        Some(pb::CivilDay {
            year: 2026,
            month,
            day,
        })
    }

    fn put_channel(svc: &BoardSvc, id: &str, name: &str) {
        svc.store
            .put_channel(&humata2_store::ChannelRecord {
                id: id.to_owned(),
                name: name.to_owned(),
                folder: name.to_lowercase(),
                branch_prefix: None,
                default_agent: String::new(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 1,
            })
            .unwrap();
    }

    fn put_session(svc: &BoardSvc, id: &str, channel: &str) {
        svc.store
            .put_session(&humata2_store::SessionRecord {
                id: id.to_owned(),
                channel: channel.to_owned(),
                machine: "local".to_owned(),
                title: id.to_owned(),
                agent: "claude".to_owned(),
                folder: std::path::PathBuf::from("/tmp"),
                created_unix_ms: 1,
            })
            .unwrap();
    }

    fn channel_id(id: &str) -> Option<pb::ChannelId> {
        Some(pb::ChannelId {
            value: id.to_owned(),
        })
    }

    fn done(applied: TodoApplied) -> pb::TodoItem {
        match applied {
            TodoApplied::Done(done) => done.item.unwrap(),
            TodoApplied::Ask(..) => panic!("asked where it should have applied"),
        }
    }

    fn key_of(item: &pb::TodoItem) -> String {
        item.item.as_ref().unwrap().id.as_ref().unwrap().value.clone()
    }

    fn add(
        svc: &BoardSvc,
        title: &str,
        channel: Option<&str>,
        set: impl FnOnce(&mut pb::CreateTodoRequest),
    ) -> pb::TodoItem {
        let mut request = pb::CreateTodoRequest {
            title: title.to_owned(),
            channel: channel.and_then(channel_id),
            ..Default::default()
        };
        set(&mut request);
        done(
            svc.create_todo_in(&user(), &request, false, now())
                .unwrap(),
        )
    }

    fn update(
        svc: &BoardSvc,
        key: &str,
        expected: Option<u64>,
        patch: pb::TodoPatch,
        at: i64,
    ) -> Result<pb::TodoItem, Status> {
        let request = pb::UpdateTodoRequest {
            id: Some(pb::BoardItemId {
                value: key.to_owned(),
            }),
            patch: Some(patch),
            expected_revision: expected.map(|value| pb::Revision { value }),
            ..Default::default()
        };
        svc.update_todo_in(&user(), &request, false, at).map(done)
    }

    fn list(svc: &BoardSvc, filter: pb::TodoFilter, at: i64) -> Vec<pb::TodoItem> {
        svc.list_todo_in(
            &user(),
            &pb::ListTodoRequest {
                filter: Some(filter),
                page: None,
            },
            &[],
            at,
        )
        .unwrap()
        .items
    }

    fn titles(items: &[pb::TodoItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| item.item.as_ref().unwrap().title.clone())
            .collect()
    }

    fn fields_of(item: &pb::TodoItem) -> &pb::TodoFields {
        item.fields.as_ref().unwrap()
    }

    fn set_fields(svc: &BoardSvc, key: &str, change: impl FnOnce(&mut Fields)) {
        let mut map = saved::load(&svc.store).unwrap();
        change(map.entry(key.to_owned()).or_default());
        saved::save(&svc.store, &map).unwrap();
    }

    /// The titles of the Next group, top first, as the list ranks them at `at`.
    fn next_titles(svc: &BoardSvc, asks: &[pb::InboxRequest], at: i64) -> Vec<String> {
        let (entries, ranked) = svc.ranked(&user(), asks, at).unwrap();
        ranked
            .next
            .iter()
            .map(|row| {
                entries
                    .iter()
                    .find(|entry| entry.record.key == row.key)
                    .unwrap()
                    .record
                    .title
                    .clone()
            })
            .collect()
    }

    fn global() -> pb::BoardRef {
        pb::BoardRef {
            board: Some(pb::board_ref::Board::Global(pb::GlobalBoard {})),
        }
    }

    fn board_of(channel: &str) -> pb::BoardRef {
        pb::BoardRef {
            board: Some(pb::board_ref::Board::Channel(pb::ChannelId {
                value: channel.to_owned(),
            })),
        }
    }

    fn session_board(session: &str) -> pb::BoardRef {
        pb::BoardRef {
            board: Some(pb::board_ref::Board::Session(pb::SessionId {
                value: session.to_owned(),
            })),
        }
    }

    /// An item made the way the boards always made it, so it has no to-do
    /// fields saved.
    async fn old_item(svc: &BoardSvc, board: pb::BoardRef, title: &str, column: i32) -> String {
        let made = svc
            .create_item(user_req(pb::CreateItemRequest {
                board: Some(board),
                title: title.to_owned(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner()
            .item
            .unwrap();
        let id = made.id.unwrap();
        if column != DEFAULT_COLUMN {
            BoardService::move_item(
                svc,
                user_req(pb::MoveItemRequest {
                    id: Some(id.clone()),
                    column,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        }
        id.value
    }

    const IN_PROGRESS: i32 = pb::BoardColumn::InProgress as i32;
    const IN_REVIEW: i32 = pb::BoardColumn::InReview as i32;
    const BACKLOG: i32 = pb::BoardColumn::Backlog as i32;
    const DONE_COL: i32 = pb::BoardColumn::Done as i32;

    // ------------------------------------------------------- migration

    #[tokio::test]
    async fn every_board_moves_into_one_list_keeping_where_each_item_stood() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        put_channel(&svc, "ch_pay", "Payments");
        put_session(&svc, "se_1", "ch_pay");
        let first = old_item(&svc, global(), "Global first", DEFAULT_COLUMN).await;
        let second = old_item(&svc, global(), "Global second", DEFAULT_COLUMN).await;
        let working = old_item(&svc, board_of("ch_pay"), "Channel working", IN_PROGRESS).await;
        let finished = old_item(&svc, board_of("ch_pay"), "Channel finished", DONE_COL).await;
        let on_session = old_item(&svc, session_board("se_1"), "Session item", IN_REVIEW).await;

        let vip: HashSet<String> = HashSet::from([second.clone()]);
        let row = svc.migrate_boards(&vip, now()).unwrap();
        assert_eq!(
            (row.items, row.open, row.in_progress, row.closed, row.boards),
            (5, 2, 2, 1, 3)
        );

        let all = list(
            &svc,
            pb::TodoFilter {
                show_closed: true,
                ..Default::default()
            },
            now(),
        );
        let get = |key: &str| all.iter().find(|item| key_of(item) == key).unwrap();
        assert_eq!(all.len(), 5);
        assert_eq!(fields_of(get(&first)).state, pb::TodoState::Open as i32);
        assert_eq!(fields_of(get(&working)).state, pb::TodoState::InProgress as i32);
        assert_eq!(fields_of(get(&finished)).state, pb::TodoState::Closed as i32);

        // VIP became Pin, and the old position is kept as the start order.
        assert_eq!(fields_of(get(&second)).pin_order, 1);
        assert_eq!(fields_of(get(&first)).pin_order, 0);
        let start = fields_of(get(&second)).start.clone().unwrap();
        assert!(start.was_vip);
        assert_eq!(start.position, 1, "a new board item goes on top");
        assert_eq!(start.column, pb::BoardColumn::ToDo as i32);
        assert!(start.board_channel.is_none());
        let channel_start = fields_of(get(&working)).start.clone().unwrap();
        assert_eq!(channel_start.board_channel, channel_id("ch_pay"));

        // A session board's item is on its channel with the session attached.
        let moved = get(&on_session).item.as_ref().unwrap();
        assert_eq!(moved.channel, channel_id("ch_pay"));
        assert_eq!(
            moved.board,
            Some(board_pb(&BoardKey::Channel("ch_pay".to_owned())))
        );
        assert!(moved.sessions.iter().any(|session| session.value == "se_1"));
    }

    #[tokio::test]
    async fn moving_the_boards_twice_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        old_item(&svc, global(), "One", DEFAULT_COLUMN).await;
        old_item(&svc, global(), "Two", IN_PROGRESS).await;
        let first = svc.migrate_boards(&HashSet::new(), now()).unwrap();
        let fields = saved::load(&svc.store).unwrap();

        // A board item made after the move, and a later run with other marks.
        old_item(&svc, global(), "Three", BACKLOG).await;
        let later = now() + DAY_MS;
        let vip = HashSet::from(["GLB-1".to_owned()]);
        let second = svc.migrate_boards(&vip, later).unwrap();

        assert_eq!(first, second);
        assert_eq!(saved::load(&svc.store).unwrap(), fields);
    }

    #[tokio::test]
    async fn dismissing_the_banner_is_kept_and_the_counts_stay() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        old_item(&svc, global(), "One", DEFAULT_COLUMN).await;
        let before = svc.ensure_migrated(now()).unwrap();
        assert!(!before.banner_dismissed);
        let dismissed = svc.dismiss_migration_in(now()).unwrap();
        assert!(dismissed.banner_dismissed);
        assert_eq!(dismissed.items, 1);
        let again = svc.ensure_migrated(now() + DAY_MS).unwrap();
        assert!(again.banner_dismissed);
        assert_eq!(again.migrated_at_ms, before.migrated_at_ms);
    }

    fn record(key: &str, board: BoardKey, column: i32, rank: &str) -> BoardItemRecord {
        BoardItemRecord {
            key: key.to_owned(),
            board,
            channel: "ch_pay".to_owned(),
            title: key.to_owned(),
            description: String::new(),
            column,
            rank: rank.to_owned(),
            created_by: String::new(),
            created_unix_ms: ms_of(2026, 11, 1),
            updated_unix_ms: ms_of(2026, 11, 1),
            revision: 1,
            done_unix_ms: None,
            deleted: false,
            sessions: Vec::new(),
            depends_on: Vec::new(),
        }
    }

    #[test]
    fn items_of_an_archived_sessions_board_move_closed_even_when_not_done() {
        let records = [
            record("PAY-1", BoardKey::Session("se_old".to_owned()), 3, "a"),
            record("PAY-2", BoardKey::Session("se_live".to_owned()), 3, "a"),
        ];
        let archived = HashSet::from(["se_old".to_owned()]);
        let (planned, row) = plan_migration(&records, &archived, &HashSet::new(), now());
        assert_eq!(planned["PAY-1"].state(3), State::Closed);
        assert_eq!(planned["PAY-2"].state(3), State::InProgress);
        assert_eq!(planned["PAY-1"].pin_order, 0);
        assert_eq!((row.closed, row.in_progress), (1, 1));
    }

    #[test]
    fn vip_items_pin_first_in_their_old_order_and_closed_ones_are_not_pinned() {
        let records = [
            record("PAY-1", BoardKey::Channel("ch_pay".to_owned()), 2, "a"),
            record("PAY-2", BoardKey::Channel("ch_pay".to_owned()), 2, "b"),
            record("PAY-3", BoardKey::Channel("ch_pay".to_owned()), 5, "c"),
        ];
        let vip = HashSet::from(["PAY-2".to_owned(), "PAY-1".to_owned(), "PAY-3".to_owned()]);
        let (planned, _) = plan_migration(&records, &HashSet::new(), &vip, now());
        assert_eq!(planned["PAY-1"].pin_order, 1);
        assert_eq!(planned["PAY-2"].pin_order, 2);
        assert_eq!(planned["PAY-3"].pin_order, 0);
    }

    // -------------------------------------------------- old rows replay

    #[tokio::test]
    async fn an_old_board_row_with_no_fields_takes_its_state_from_its_column() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        svc.ensure_migrated(now()).unwrap();
        for (title, column) in [
            ("backlog", BACKLOG),
            ("to do", DEFAULT_COLUMN),
            ("progress", IN_PROGRESS),
            ("review", IN_REVIEW),
            ("done", DONE_COL),
        ] {
            old_item(&svc, global(), title, column).await;
        }
        assert!(saved::load(&svc.store).unwrap().is_empty());
        let all = list(
            &svc,
            pb::TodoFilter {
                show_closed: true,
                ..Default::default()
            },
            now(),
        );
        let state_of_title = |title: &str| {
            let item = all
                .iter()
                .find(|item| titles(std::slice::from_ref(item))[0] == title)
                .unwrap();
            fields_of(item).state
        };
        let (open, progress, closed) = (
            pb::TodoState::Open as i32,
            pb::TodoState::InProgress as i32,
            pb::TodoState::Closed as i32,
        );
        assert_eq!(state_of_title("backlog"), open);
        assert_eq!(state_of_title("to do"), open);
        assert_eq!(state_of_title("progress"), progress);
        assert_eq!(state_of_title("review"), progress);
        assert_eq!(state_of_title("done"), closed);
    }

    // ----------------------------------------------------------- list

    #[test]
    fn the_list_filters_by_channel_no_channel_and_session() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        put_channel(&svc, "ch_pay", "Payments");
        put_channel(&svc, "ch_web", "Web");
        put_session(&svc, "se_1", "ch_pay");
        add(&svc, "pay one", Some("ch_pay"), |r| {
            r.sessions = vec![pb::SessionId {
                value: "se_1".to_owned(),
            }];
        });
        add(&svc, "pay two", Some("ch_pay"), |_| {});
        add(&svc, "web one", Some("ch_web"), |_| {});
        add(&svc, "loose", None, |_| {});

        let mut filter = pb::TodoFilter::default();
        assert_eq!(list(&svc, filter.clone(), now()).len(), 4);
        filter.channel = channel_id("ch_pay");
        assert_eq!(titles(&list(&svc, filter, now())), ["pay one", "pay two"]);
        let no_channel = pb::TodoFilter {
            no_channel: true,
            ..Default::default()
        };
        assert_eq!(titles(&list(&svc, no_channel, now())), ["loose"]);
        let session = pb::TodoFilter {
            session: Some(pb::SessionId {
                value: "se_1".to_owned(),
            }),
            ..Default::default()
        };
        assert_eq!(titles(&list(&svc, session, now())), ["pay one"]);
    }

    #[test]
    fn closed_items_show_only_when_asked_and_only_for_a_week() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let item = add(&svc, "finish me", None, |_| {});
        add(&svc, "still open", None, |_| {});
        let closed = update(
            &svc,
            &key_of(&item),
            None,
            pb::TodoPatch {
                state: Some(pb::TodoState::Closed as i32),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert_eq!(fields_of(&closed).state, pb::TodoState::Closed as i32);
        assert!(fields_of(&closed).closed_at.is_some());

        let plain = pb::TodoFilter::default();
        let with_closed = pb::TodoFilter {
            show_closed: true,
            ..Default::default()
        };
        assert_eq!(titles(&list(&svc, plain.clone(), now())), ["still open"]);
        assert_eq!(list(&svc, with_closed.clone(), now()).len(), 2);
        assert_eq!(list(&svc, with_closed.clone(), now() + 6 * DAY_MS).len(), 2);
        assert_eq!(list(&svc, with_closed, now() + 8 * DAY_MS).len(), 1);

        // Reopening puts it back in the list.
        update(
            &svc,
            &key_of(&item),
            None,
            pb::TodoPatch {
                state: Some(pb::TodoState::Open as i32),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert_eq!(list(&svc, plain, now()).len(), 2);
    }

    #[test]
    fn a_page_of_the_list_says_where_the_next_one_starts() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        for title in ["a", "b", "c"] {
            add(&svc, title, None, |_| {});
        }
        let first = svc
            .list_todo_in(
                &user(),
                &pb::ListTodoRequest {
                    filter: None,
                    page: Some(pb::PageRequest {
                        page_size: 2,
                        page_token: String::new(),
                    }),
                },
                &[],
                now(),
            )
            .unwrap();
        assert_eq!(first.items.len(), 2);
        let token = first.page.unwrap().next_page_token;
        let rest = svc
            .list_todo_in(
                &user(),
                &pb::ListTodoRequest {
                    filter: None,
                    page: Some(pb::PageRequest {
                        page_size: 2,
                        page_token: token,
                    }),
                },
                &[],
                now(),
            )
            .unwrap();
        assert_eq!(titles(&rest.items), ["c"]);
        assert!(rest.page.unwrap().next_page_token.is_empty());
    }

    // --------------------------------------------------------- writes

    #[test]
    fn a_stale_revision_is_refused_the_way_board_writes_refuse_it() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let item = add(&svc, "edit me", None, |_| {});
        let key = key_of(&item);
        let revision = item.item.as_ref().unwrap().revision.clone().unwrap().value;

        let due = pb::TodoPatch {
            due: civil_day(11, 25),
            ..Default::default()
        };
        let changed = update(&svc, &key, Some(revision), due, now()).unwrap();
        let after = changed.item.as_ref().unwrap().revision.clone().unwrap().value;
        assert!(after > revision, "a change that leaves the board item alone still counts");

        let stale = update(
            &svc,
            &key,
            Some(revision),
            pb::TodoPatch {
                effort: Some(pb::TodoEffort::L as i32),
                ..Default::default()
            },
            now(),
        )
        .unwrap_err();
        assert_eq!(stale.code(), Code::Aborted);
        let kept = list(&svc, pb::TodoFilter::default(), now());
        assert_eq!(fields_of(&kept[0]).effort, pb::TodoEffort::Unspecified as i32);
    }

    #[test]
    fn a_patch_changes_only_the_fields_it_sets() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let item = add(&svc, "plan", None, |r| {
            r.due = civil_day(11, 25);
            r.tags = vec!["a".to_owned(), "b".to_owned(), "a".to_owned()];
            r.impact = pb::TodoImpact::High as i32;
        });
        assert_eq!(fields_of(&item).tags, ["a", "b"]);
        let patched = update(
            &svc,
            &key_of(&item),
            None,
            pb::TodoPatch {
                effort: Some(pb::TodoEffort::S as i32),
                tags_add: vec!["c".to_owned()],
                tags_remove: vec!["a".to_owned()],
                steps: Some(pb::TodoSteps {
                    steps: vec![pb::TodoStep {
                        text: "one".to_owned(),
                        done: true,
                        ticked_by: None,
                        ..Default::default()
                    }],
                }),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        let fields = fields_of(&patched);
        assert_eq!(fields.tags, ["b", "c"]);
        assert_eq!(fields.due, civil_day(11, 25));
        assert_eq!(fields.impact, pb::TodoImpact::High as i32);
        assert_eq!(fields.effort, pb::TodoEffort::S as i32);
        assert_eq!(fields.steps.len(), 1);

        let cleared = update(
            &svc,
            &key_of(&item),
            None,
            pb::TodoPatch {
                clear_due: true,
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert!(fields_of(&cleared).due.is_none());
    }

    #[test]
    fn a_date_that_is_not_in_the_calendar_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let request = pb::CreateTodoRequest {
            title: "bad".to_owned(),
            due: civil_day(2, 31),
            ..Default::default()
        };
        let refused = svc
            .create_todo_in(&user(), &request, false, now())
            .err()
            .unwrap();
        assert_eq!(refused.code(), Code::InvalidArgument);
        assert!(list(&svc, pb::TodoFilter::default(), now()).is_empty());
    }

    #[test]
    fn a_suggested_item_has_no_rank_until_the_user_says_yes() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let item = add(&svc, "agent idea", None, |_| {});
        let key = key_of(&item);
        set_fields(&svc, &key, |fields| fields.suggested = true);
        assert!(next_titles(&svc, &[], now()).is_empty());
        let accepted = update(
            &svc,
            &key,
            None,
            pb::TodoPatch {
                accept_suggestion: true,
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert!(!fields_of(&accepted).suggested);
        assert_eq!(next_titles(&svc, &[], now()), ["agent idea"]);
    }

    // ------------------------------------------------------- ranking

    fn on_day(svc: &BoardSvc, title: &str, month: u32, day: u32, effort: i32, impact: i32) -> String {
        key_of(&add(svc, title, Some("ch_pay"), |r| {
            r.due = civil_day(month, day);
            r.effort = effort;
            r.impact = impact;
        }))
    }

    #[test]
    fn do_next_follows_the_signed_off_order_pinned_then_due_then_review_then_value() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        put_channel(&svc, "ch_pay", "Payments");
        let (s, m) = (pb::TodoEffort::S as i32, pb::TodoEffort::M as i32);
        let (medium, high) = (pb::TodoImpact::Medium as i32, pb::TodoImpact::High as i32);
        on_day(&svc, "TD-3", 12, 14, s, high);
        let pay5 = on_day(&svc, "PAY-5", 11, 23, m, high);
        let review = on_day(&svc, "REV-1", 11, 20, 0, 0);
        let pay7 = on_day(&svc, "PAY-7", 11, 18, s, high);
        let inf6 = on_day(&svc, "INF-6", 11, 19, s, medium);
        // Not in Next at all: in progress, closed, and waiting.
        let working = on_day(&svc, "PAY-3", 11, 20, m, high);
        update(
            &svc,
            &working,
            None,
            pb::TodoPatch {
                state: Some(pb::TodoState::InProgress as i32),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        update(
            &svc,
            &inf6,
            None,
            pb::TodoPatch {
                pin_order: Some(1),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        // A review request with no due day, two days waiting.
        update(
            &svc,
            &review,
            None,
            pb::TodoPatch {
                clear_due: true,
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        set_fields(&svc, &review, |fields| {
            fields.review = Some(ReviewRow {
                pull_requests: vec![PrRow {
                    owner: "acme".to_owned(),
                    name: "api".to_owned(),
                    number: 483,
                    note: String::new(),
                }],
                author: "dana-k".to_owned(),
                requested_unix_ms: now() - 2 * DAY_MS,
            });
        });
        let _ = pay5;

        assert_eq!(
            next_titles(&svc, &[], now()),
            ["INF-6", "PAY-7", "REV-1", "PAY-5", "TD-3"]
        );
        let top = svc
            .next_todo_in(&user(), &pb::NextTodoRequest::default(), &[], now())
            .unwrap()
            .item
            .unwrap();
        assert_eq!(titles(&[top]), ["INF-6"]);
    }

    #[test]
    fn do_next_is_the_first_row_the_filter_shows() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        put_channel(&svc, "ch_pay", "Payments");
        put_channel(&svc, "ch_web", "Web");
        add(&svc, "urgent pay", Some("ch_pay"), |r| r.due = civil_day(11, 18));
        add(&svc, "later web", Some("ch_web"), |r| r.due = civil_day(11, 30));
        let web = pb::NextTodoRequest {
            filter: Some(pb::TodoFilter {
                channel: channel_id("ch_web"),
                ..Default::default()
            }),
        };
        let found = svc.next_todo_in(&user(), &web, &[], now()).unwrap().item.unwrap();
        assert_eq!(titles(&[found]), ["later web"]);
        let all = svc
            .next_todo_in(&user(), &pb::NextTodoRequest::default(), &[], now())
            .unwrap()
            .item
            .unwrap();
        assert_eq!(titles(&[all]), ["urgent pay"]);
        let none = pb::NextTodoRequest {
            filter: Some(pb::TodoFilter {
                channel: channel_id("ch_missing"),
                ..Default::default()
            }),
        };
        assert!(svc.next_todo_in(&user(), &none, &[], now()).unwrap().item.is_none());
    }

    #[test]
    fn an_overdue_item_is_never_waiting_or_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let blocker = add(&svc, "blocker", None, |_| {});
        let blocker_id = pb::BoardItemId {
            value: key_of(&blocker),
        };
        add(&svc, "overdue and blocked", None, |r| {
            r.depends_on = vec![blocker_id.clone()];
            r.due = civil_day(11, 10);
        });
        add(&svc, "blocked", None, |r| r.depends_on = vec![blocker_id.clone()]);
        let hidden = add(&svc, "overdue and hidden", None, |r| r.due = civil_day(11, 12));
        update(
            &svc,
            &key_of(&hidden),
            None,
            pb::TodoPatch {
                hide_until: civil_day(12, 24),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        let later = add(&svc, "hidden for now", None, |_| {});
        update(
            &svc,
            &key_of(&later),
            None,
            pb::TodoPatch {
                hide_until: civil_day(12, 24),
                ..Default::default()
            },
            now(),
        )
        .unwrap();

        let (entries, ranked) = svc.ranked(&user(), &[], now()).unwrap();
        let name = |rows: &[domain::Row]| -> Vec<String> {
            let mut names: Vec<String> = rows
                .iter()
                .map(|row| {
                    entries
                        .iter()
                        .find(|entry| entry.record.key == row.key)
                        .unwrap()
                        .record
                        .title
                        .clone()
                })
                .collect();
            names.sort();
            names
        };
        assert_eq!(name(&ranked.waiting), ["blocked"]);
        assert_eq!(name(&ranked.hidden), ["hidden for now"]);
        assert!(name(&ranked.next).contains(&"overdue and blocked".to_owned()));
        assert!(name(&ranked.next).contains(&"overdue and hidden".to_owned()));
    }

    #[test]
    fn an_item_with_a_pending_ask_stays_in_needs_attention_not_in_next() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let asked = add(&svc, "agent wants to close", None, |_| {});
        add(&svc, "plain", None, |_| {});
        let ask = pb::InboxRequest {
            preview: Some(pb::RequestPreview {
                preview: Some(pb::request_preview::Preview::BoardChange(
                    pb::BoardChangePreview {
                        item: Some(pb::BoardItemId {
                            value: key_of(&asked),
                        }),
                        ..Default::default()
                    },
                )),
            }),
            ..Default::default()
        };
        assert_eq!(next_titles(&svc, &[ask.clone()], now()), ["plain"]);
        let (entries, ranked) = svc.ranked(&user(), &[ask.clone()], now()).unwrap();
        let attention: Vec<&str> = ranked
            .needs_attention
            .iter()
            .map(|row| row.key.as_str())
            .collect();
        assert_eq!(attention, [key_of(&asked).as_str()]);
        assert_eq!(entries.len(), 2);
        // The item carries the ask for the user to answer.
        let shown = svc
            .list_todo_in(&user(), &pb::ListTodoRequest::default(), &[ask], now())
            .unwrap();
        let with_ask = shown
            .items
            .iter()
            .find(|item| key_of(item) == key_of(&asked))
            .unwrap();
        assert_eq!(with_ask.item.as_ref().unwrap().pending.len(), 1);
    }

    #[test]
    fn a_state_change_moves_the_board_item_so_the_boards_agree() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let item = add(&svc, "move me", None, |_| {});
        let moved = update(
            &svc,
            &key_of(&item),
            None,
            pb::TodoPatch {
                state: Some(pb::TodoState::InProgress as i32),
                ..Default::default()
            },
            now(),
        )
        .unwrap();
        assert_eq!(
            moved.item.as_ref().unwrap().column,
            pb::BoardColumn::InProgress as i32
        );
        assert_eq!(fields_of(&moved).state, pb::TodoState::InProgress as i32);
    }

    /// A guard that judges the to-do capabilities by their defaults plus
    /// whatever a test overrides, and lets every board call through.
    struct RuleGuard {
        set: std::sync::Mutex<BTreeMap<humata2_access::Capability, humata2_access::Decision>>,
    }

    impl RuleGuard {
        fn set(&self, capability: humata2_access::Capability, decision: humata2_access::Decision) {
            self.set.lock().unwrap().insert(capability, decision);
        }
    }

    impl crate::ports::BoardGuard for RuleGuard {
        fn check(
            &self,
            caller: &Caller,
            call: &humata2_access::Call,
        ) -> Result<crate::ports::Checked, Status> {
            use humata2_access::Decision;
            let humata2_access::Call::Use { capability, .. } = call else {
                return Ok(crate::ports::Checked::Allowed);
            };
            let Caller::Agent(agent) = caller else {
                return Ok(crate::ports::Checked::Allowed);
            };
            let decision = self
                .set
                .lock()
                .unwrap()
                .get(capability)
                .copied()
                .or_else(|| capability.default_decision());
            match decision {
                Some(Decision::Never) => Err(crate::ports::refused(
                    &humata2_access::Refusal::NeverRule {
                        channel: agent.session.channel.clone(),
                        capability: *capability,
                    },
                )),
                Some(Decision::Ask) => Ok(crate::ports::Checked::Ask(Box::new(
                    humata2_access::AccessRequest {
                        capability: *capability,
                        channel: agent.session.channel.clone(),
                        requester: agent.session.id.clone(),
                        call: call.clone(),
                        always_let: None,
                        ahead: false,
                        answer_at: humata2_access::AnswerAt::Host,
                    },
                ))),
                Some(Decision::Allow) | None => Ok(crate::ports::Checked::Allowed),
            }
        }
        fn check_away(
            &self,
            _caller: &Caller,
            _call: &humata2_access::Call,
        ) -> Result<crate::ports::AwayCall, Status> {
            Ok(crate::ports::AwayCall::Reachable)
        }
        fn lists_low_risk(&self, _asked: &humata2_access::AccessRequest) -> bool {
            false
        }
        fn policy_changed(&self) {}
        fn host_link(&self) -> String {
            String::new()
        }
    }

    fn ruled_svc(dir: &tempfile::TempDir) -> (BoardSvc, std::sync::Arc<RuleGuard>, Caller) {
        let guard = std::sync::Arc::new(RuleGuard {
            set: std::sync::Mutex::new(BTreeMap::new()),
        });
        let store = std::sync::Arc::new(humata2_store::Store::open(dir.path().join("state")).unwrap());
        let svc = BoardSvc::new(
            guard.clone(),
            store,
            std::sync::Arc::new(super::super::test_support::NoopFeed),
            std::sync::Arc::new(super::super::test_support::NoopInbox),
            "local".to_owned(),
        );
        put_channel(&svc, "c1", "Payments");
        put_session(&svc, "s1", "c1");
        let record = svc.store.session("s1").unwrap().unwrap();
        let agent = Caller::Agent(super::super::ports::agent_caller_of(&record, "local"));
        (svc, guard, agent)
    }

    fn agent_create(svc: &BoardSvc, agent: &Caller) -> Result<TodoApplied, Status> {
        let request = pb::CreateTodoRequest {
            title: "look at the retry bug".to_owned(),
            ..Default::default()
        };
        svc.create_todo_in(agent, &request, false, now())
    }

    fn close_request(key: &str) -> pb::UpdateTodoRequest {
        pb::UpdateTodoRequest {
            id: Some(pb::BoardItemId {
                value: key.to_owned(),
            }),
            patch: Some(pb::TodoPatch {
                state: Some(pb::TodoState::Closed as i32),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn an_agent_suggests_by_default_but_its_close_waits_for_approval() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, guard, agent) = ruled_svc(&dir);
        // Under PERMAUDIT, TodoClose defaults to Allow; explicitly gate it to Ask
        // here to verify the approval-waiting path.
        guard.set(
            humata2_access::Capability::TodoClose,
            humata2_access::Decision::Ask,
        );
        let key = key_of(&done(agent_create(&svc, &agent).unwrap()));
        let asked = svc
            .update_todo_in(&agent, &close_request(&key), false, now())
            .unwrap();
        assert!(matches!(asked, TodoApplied::Ask(..)));
        let shown = list(&svc, pb::TodoFilter::default(), now());
        let found = shown.iter().find(|shown| key_of(shown) == key).unwrap();
        assert_ne!(fields_of(found).state, pb::TodoState::Closed as i32);
    }

    #[test]
    fn an_agent_closes_when_closing_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, guard, agent) = ruled_svc(&dir);
        let key = key_of(&done(agent_create(&svc, &agent).unwrap()));
        guard.set(
            humata2_access::Capability::TodoClose,
            humata2_access::Decision::Allow,
        );
        let closed = done(
            svc.update_todo_in(&agent, &close_request(&key), false, now())
                .unwrap(),
        );
        assert_eq!(fields_of(&closed).state, pb::TodoState::Closed as i32);
    }

    #[test]
    fn an_agent_cannot_suggest_when_suggesting_is_never() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, guard, agent) = ruled_svc(&dir);
        guard.set(
            humata2_access::Capability::TodoSuggest,
            humata2_access::Decision::Never,
        );
        assert!(agent_create(&svc, &agent).is_err());
    }

    #[test]
    fn the_user_closes_without_any_to_do_rule() {
        let dir = tempfile::tempdir().unwrap();
        let (svc, _guard, agent) = ruled_svc(&dir);
        let key = key_of(&done(agent_create(&svc, &agent).unwrap()));
        let closed = done(
            svc.update_todo_in(&user(), &close_request(&key), false, now())
                .unwrap(),
        );
        assert_eq!(fields_of(&closed).state, pb::TodoState::Closed as i32);
    }

    #[test]
    fn domain_item_fills_ask_detail_from_the_inbox_request() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let created = add(&svc, "Ledger schema v2", None, |_| {});
        let key = key_of(&created);
        let entries = svc.entries(&user()).unwrap();
        let entry = entries.iter().find(|e| e.record.key == key).unwrap();
        let at = now();
        let ask = pb::InboxRequest {
            id: Some(pb::RequestId { value: "rq_1".to_owned() }),
            requester: Some(pb::AgentActor {
                session: Some(pb::SessionId { value: "se_ended".to_owned() }),
                ..Default::default()
            }),
            reason: "Staging backfill passed.".to_owned(),
            preview: Some(pb::RequestPreview {
                preview: Some(pb::request_preview::Preview::BoardChange(
                    pb::BoardChangePreview {
                        item: Some(pb::BoardItemId { value: key.clone() }),
                        item_title: entry.record.title.clone(),
                        changes: vec![field_change("status", "In progress", "Closed")],
                        deletes_item: false,
                    },
                )),
            }),
            created_at: Some(timestamp_ms(at - 20 * 60_000)),
            facts: vec![
                pb::CardFact {
                    label: "Commits".to_owned(),
                    lead: "3 commits".to_owned(),
                    rest: " on main".to_owned(),
                },
                pb::CardFact {
                    label: String::new(),
                    lead: "212 tests passed".to_owned(),
                    rest: String::new(),
                },
            ],
            affects: vec![pb::CardEffect {
                warn: false,
                lead: "WEB-13".to_owned(),
                rest: " stops waiting".to_owned(),
            }],
            ..Default::default()
        };
        let pending = pending_for(&entry.record, std::slice::from_ref(&ask));
        let today = day_of(at).unwrap();
        let archived = HashSet::from(["se_ended".to_owned()]);
        let todo = BoardSvc::domain_item(entry, pending.first(), Some(&ask), &archived, today, at);
        assert_eq!(todo.ask_detail.evidence.len(), 2);
        assert_eq!(todo.ask_detail.affects.len(), 1);
        assert!(todo.ask_detail.change.is_some());
        assert_eq!(todo.ask_detail.asked_minutes_ago, 20);
        assert!(todo.ask_detail.session_ended);

        let live = BoardSvc::domain_item(
            entry,
            pending.first(),
            Some(&ask),
            &HashSet::new(),
            today,
            at,
        );
        assert!(!live.ask_detail.session_ended);
    }

    #[test]
    fn todo2_extended_fields_watched_pr_and_lifecycle_prompt_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let created = add(&svc, "Ship PR watcher", None, |req| {
            req.need = pb::TodoNeed::Critical as i32;
            req.fuzzy_due = pb::TodoFuzzyDue::NextWeek as i32;
            req.external_block = Some(pb::ExternalBlock {
                reason: "Waiting on design".into(),
                since: None,
            });
            req.from_keys = vec!["OLD-1".into()];
            req.source_anchor = Some(pb::SourceAnchor {
                repo: "acme/api".into(),
                path: "src/lib.rs".into(),
                line: 42,
                end_line: 45,
                comment_id: "c1".into(),
            });
        });
        let fields = created.fields.as_ref().unwrap();
        assert_eq!(fields.need, pb::TodoNeed::Critical as i32);
        assert_eq!(fields.fuzzy_due, pb::TodoFuzzyDue::NextWeek as i32);
        assert_eq!(
            fields.external_block.as_ref().unwrap().reason,
            "Waiting on design"
        );
        assert_eq!(fields.from_keys, vec!["OLD-1"]);
        assert_eq!(fields.source_anchor.as_ref().unwrap().line, 42);

        let key = key_of(&created);
        let updated = done(
            svc.update_todo_in(
                &user(),
                &pb::UpdateTodoRequest {
                    id: Some(pb::BoardItemId { value: key.clone() }),
                    patch: Some(pb::TodoPatch {
                        state: Some(pb::TodoState::WatchingPr as i32),
                        checkpoint: Some(pb::TodoCheckpoint::Defined as i32),
                        clear_external_block: true,
                        watched_prs_add: vec![pb::WatchedPr {
                            pr: Some(pb::PullRequestRef {
                                repo: Some(pb::RepoRef {
                                    owner: "acme".into(),
                                    name: "api".into(),
                                }),
                                number: 99,
                                note: String::new(),
                            }),
                            title: "Add watcher".into(),
                            ci_state: "passing".into(),
                            merged: true,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                false,
                now(),
            )
            .unwrap(),
        );
        let uf = updated.fields.as_ref().unwrap();
        assert_eq!(uf.state, pb::TodoState::WatchingPr as i32);
        assert_eq!(uf.checkpoint, pb::TodoCheckpoint::Defined as i32);
        assert!(uf.external_block.is_none());
        assert_eq!(uf.lifecycle_prompts.len(), 1);
        let prompt_id = uf.lifecycle_prompts[0].id.clone();

        let resolved = svc
            .resolve_lifecycle_prompt_in(
                &user(),
                &pb::ResolveLifecyclePromptRequest {
                    idempotency_key: None,
                    id: Some(pb::BoardItemId { value: key }),
                    prompt_id,
                    choice: "Close".into(),
                },
                now(),
            )
            .unwrap();
        let rf = resolved.item.unwrap().fields.unwrap();
        assert_eq!(rf.state, pb::TodoState::Closed as i32);
        assert!(rf.lifecycle_prompts.is_empty());
    }

    #[test]
    fn todo2_cycle_detection_and_blocker_gone_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let a = add(&svc, "Task A", None, |_| {});
        let key_a = key_of(&a);
        let b = add(&svc, "Task B", None, |req| {
            req.depends_on = vec![pb::BoardItemId {
                value: key_a.clone(),
            }];
        });
        let key_b = key_of(&b);

        // Adding A depends_on B closes a cycle A -> B -> A and must be rejected.
        let err = svc
            .update_todo_in(
                &user(),
                &pb::UpdateTodoRequest {
                    id: Some(pb::BoardItemId {
                        value: key_a.clone(),
                    }),
                    patch: Some(pb::TodoPatch {
                        depends_on_add: vec![pb::BoardItemId {
                            value: key_b.clone(),
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                false,
                now(),
            )
            .err()
            .expect("expected dependency cycle error");
        assert_eq!(err.code(), Code::FailedPrecondition);

        // Marking A as Won't do triggers BlockerGone on B.
        svc.update_todo_in(
            &user(),
            &pb::UpdateTodoRequest {
                id: Some(pb::BoardItemId {
                    value: key_a.clone(),
                }),
                patch: Some(pb::TodoPatch {
                    state: Some(pb::TodoState::WontDo as i32),
                    wont_do_reason: Some("Out of scope".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            false,
            now(),
        )
        .unwrap();

        let list = svc
            .list_todo_in(&user(), &pb::ListTodoRequest::default(), &[], now())
            .unwrap();
        let b_item = list
            .items
            .iter()
            .find(|i| key_of(i) == key_b)
            .unwrap();
        assert_eq!(b_item.fields.as_ref().unwrap().blocker_gone.len(), 1);
        assert_eq!(
            b_item.fields.as_ref().unwrap().blocker_gone[0].blocker_key,
            key_a
        );

        // Resolving BlockerGone with "unblock" removes the notice and dependency.
        let resolved = svc
            .resolve_blocker_gone_in(
                &user(),
                &pb::ResolveBlockerGoneRequest {
                    idempotency_key: None,
                    id: Some(pb::BoardItemId { value: key_b }),
                    blocker_key: key_a,
                    action: "unblock".into(),
                    rewire_to_keys: Vec::new(),
                },
                now(),
            )
            .unwrap();
        let item = resolved.item.unwrap();
        assert!(item.fields.unwrap().blocker_gone.is_empty());
        assert!(item.item.unwrap().depends_on.is_empty());
    }

    #[test]
    fn todo2_split_fold_comments_batch_runs_history_and_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let parent = add(&svc, "Big epic", None, |_| {});
        let parent_key = key_of(&parent);

        // Comment + title edit -> GetTodoHistory includes word_diff and comment.
        let cmt = svc
            .add_todo_comment_in(
                &user(),
                &pb::AddTodoCommentRequest {
                    idempotency_key: None,
                    id: Some(pb::BoardItemId {
                        value: parent_key.clone(),
                    }),
                    body: "Looks good".into(),
                    step_id: String::new(),
                },
                now(),
            )
            .unwrap();
        assert_eq!(cmt.comment.unwrap().body, "Looks good");
        svc.update_todo_in(
            &user(),
            &pb::UpdateTodoRequest {
                id: Some(pb::BoardItemId {
                    value: parent_key.clone(),
                }),
                patch: Some(pb::TodoPatch {
                    title: Some("Big auth epic".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            false,
            now() + 10,
        )
        .unwrap();
        let hist = svc
            .get_todo_history_in(
                &user(),
                &pb::GetTodoHistoryRequest {
                    id: Some(pb::BoardItemId {
                        value: parent_key.clone(),
                    }),
                    page: None,
                },
            )
            .unwrap();
        assert!(hist.entries.iter().any(|e| !e.word_diff.is_empty()));
        assert!(
            hist.entries
                .iter()
                .any(|e| e.kind == pb::HistoryKind::Commented as i32)
        );

        // SplitTodo
        let split = svc
            .split_todo_in(
                &user(),
                &pb::SplitTodoRequest {
                    idempotency_key: None,
                    id: Some(pb::BoardItemId {
                        value: parent_key.clone(),
                    }),
                    titles: vec!["Part 1".into(), "Part 2".into()],
                    keep_age: true,
                },
                now() + 20,
            )
            .unwrap();
        assert_eq!(split.created.len(), 2);
        let k1 = key_of(&split.created[0]);
        let k2 = key_of(&split.created[1]);
        let sf = split.source.unwrap().fields.unwrap();
        assert_eq!(sf.state, pb::TodoState::Became as i32);
        assert_eq!(sf.became_keys, vec![k1.clone(), k2.clone()]);
        assert_eq!(
            split.created[0].fields.as_ref().unwrap().from_keys,
            vec![parent_key]
        );

        // FoldTodoIntoStep
        let folded = svc
            .fold_todo_into_step_in(
                &user(),
                &pb::FoldTodoIntoStepRequest {
                    idempotency_key: None,
                    source_id: Some(pb::BoardItemId { value: k2.clone() }),
                    target_id: Some(pb::BoardItemId { value: k1.clone() }),
                },
                now() + 30,
            )
            .unwrap();
        assert_eq!(
            folded.source.unwrap().fields.unwrap().state,
            pb::TodoState::Became as i32
        );
        let tf = folded.target.unwrap().fields.unwrap();
        assert_eq!(tf.steps.len(), 1);
        assert_eq!(tf.steps[0].from_key, k2);

        // BatchUpdateTodo partial failure isolation
        let batch = svc
            .batch_update_todo_in(
                &user(),
                &pb::BatchUpdateTodoRequest {
                    idempotency_key: None,
                    entries: vec![
                        pb::BatchUpdateTodoEntry {
                            id: Some(pb::BoardItemId { value: k1.clone() }),
                            patch: Some(pb::TodoPatch {
                                need: Some(pb::TodoNeed::Expected as i32),
                                ..Default::default()
                            }),
                            expected_revision: None,
                        },
                        pb::BatchUpdateTodoEntry {
                            id: Some(pb::BoardItemId {
                                value: "MISSING-999".into(),
                            }),
                            patch: Some(pb::TodoPatch {
                                need: Some(pb::TodoNeed::Critical as i32),
                                ..Default::default()
                            }),
                            expected_revision: None,
                        },
                    ],
                },
                now() + 40,
            )
            .unwrap();
        assert_eq!(batch.updated.len(), 1);
        assert_eq!(batch.failed.len(), 1);
        assert_eq!(batch.failed[0].key, "MISSING-999");

        // ListTodoRuns
        svc.store
            .record_todo_run(&humata2_store::TodoRunRecord {
                id: "run_a".into(),
                item_key: k1.clone(),
                tool: "claude".into(),
                model: "sonnet".into(),
                outcome: "done".into(),
                cost_micros: Some(250_000),
                started_unix_ms: 100,
                finished_unix_ms: 200,
                links_json: "[]".into(),
                session: "se_1".into(),
            })
            .unwrap();
        let runs = svc
            .list_todo_runs_in(
                &user(),
                &pb::ListTodoRunsRequest {
                    id: Some(pb::BoardItemId { value: k1.clone() }),
                },
            )
            .unwrap();
        assert_eq!(runs.runs.len(), 1);
        assert_eq!(runs.total_cost_micros, Some(250_000));

        // ResolveTodoOwnership
        let owned = svc
            .resolve_todo_ownership_in(
                &user(),
                &pb::ResolveTodoOwnershipRequest {
                    idempotency_key: None,
                    id: Some(pb::BoardItemId { value: k1 }),
                    take_back: true,
                    retry: false,
                },
                now() + 50,
            )
            .unwrap();
        assert_eq!(
            owned.item.unwrap().fields.unwrap().owner.unwrap().kind,
            pb::TodoOwnerKind::User as i32
        );
    }
}
