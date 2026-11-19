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
use humata2_todo::{Day, Effort, Impact, OldColumn, State};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Request, Response, Status};

use super::asks::{change_ask, field_change, new_item_ask, pending_for};
use super::ports::{answer_key, caller_of};
use super::scope::{board_for_policy, board_pb, by_of, made_by_session};
use super::todo_fields::{self as saved, Fields, FieldsError, MigrationRow, StartRow, StepRow};
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
        Ok(pb::TodoState::Unspecified) | Err(_) => {
            Err(unknown(field, "state", "Open, In progress or Closed"))
        }
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
    }
}

fn state_label(state: State) -> &'static str {
    match state {
        State::Open => "Open",
        State::InProgress => "In progress",
        State::Closed => "Closed",
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
        .map(|step| StepRow {
            text: step.text.trim().to_owned(),
            done: step.done,
            ticked_by: step.ticked_by.as_ref().map(|id| id.value.clone()),
        })
        .collect()
}

// -------------------------------------------------------------- the view

/// An item as the list sees it: the board item and what the list keeps.
#[derive(Clone)]
struct Entry {
    record: BoardItemRecord,
    fields: Fields,
}

impl Entry {
    /// The channel the item shows under; empty is none. Items of a session's
    /// board show under that session's channel, which the store keeps.
    fn channel(&self) -> String {
        self.fields
            .channel
            .clone()
            .unwrap_or_else(|| self.record.channel.clone())
    }

    /// The sessions on it, with the session whose board it was on first.
    fn sessions(&self) -> Vec<String> {
        let mut sessions = self.record.sessions.clone();
        if let BoardKey::Session(session) = &self.record.board
            && !sessions.contains(session)
        {
            sessions.push(session.clone());
        }
        sessions
    }

    fn state(&self) -> State {
        self.fields.state(self.record.column)
    }

    fn revision(&self) -> u64 {
        self.record.revision + self.fields.extra_revisions
    }

    fn closed_ms(&self) -> Option<i64> {
        (self.state() == State::Closed).then(|| {
            self.fields
                .closed_at_ms
                .or(self.record.done_unix_ms)
                .unwrap_or(self.record.updated_unix_ms)
        })
    }

    /// The record with the list's view of it filled in, for the board item
    /// the API shows.
    fn shown_record(&self) -> BoardItemRecord {
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
    fn entries(&self, caller: &Caller) -> Result<Vec<Entry>, Status> {
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

    /// The domain's item for `entry`; `ask` is an agent's request waiting on it.
    fn domain_item(
        entry: &Entry,
        ask: Option<&pb::PendingAsk>,
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
                text: step.text.clone(),
                done: step.done,
            })
            .collect();
        todo.hide_until = fields.hide_until;
        todo.pin = fields.pin_order.checked_sub(1);
        todo.depends_on = record.depends_on.clone();
        todo.suggested = fields.suggested;
        todo.ask = ask.map(|ask| domain::Ask {
            kind: domain::AskKind::Close,
            by: String::new(),
            reason: ask.reason.clone(),
        });
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
        let today = day_of(now).ok_or_else(|| internal("the clock is outside the calendar", ""))?;
        let items: Vec<domain::Todo> = entries
            .iter()
            .map(|entry| {
                let pending = pending_for(&entry.record, asks);
                Self::domain_item(entry, pending.first(), today, now)
            })
            .collect();
        let ranked = domain::rank(&items, today);
        Ok((entries, ranked))
    }

    fn fields_pb(&self, entry: &Entry) -> pb::TodoFields {
        let fields = &entry.fields;
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
                    text: step.text.clone(),
                    done: step.done,
                    ticked_by: step
                        .ticked_by
                        .as_ref()
                        .map(|id| pb::SessionId { value: id.clone() }),
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
        }
    }

    /// `entry` for the API: the board item with the list's channel, the
    /// sessions it has and its revision, and the to-do fields.
    fn todo_item(
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
    pub fn migrate_boards(&self, vip: &HashSet<String>, now: i64) -> Result<MigrationRow, Status> {
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

    fn create_todo_in(
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

    fn update_todo_in(
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
        let planned = apply_patch(&fresh, &patch, &parsed, channel, &add, &remove, &deps_add, &deps_remove, now);
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
        if fields != fresh.fields {
            map.insert(id, fields.clone());
            saved::save(&self.store, &map)?;
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
                if let Some(key) = key {
                    self.finished_todo.put(key, (*done).clone());
                }
                Ok(Response::new(*done))
            }
            TodoApplied::Ask(asked, board) => {
                let (me, who) = (self.clone(), caller.clone());
                let run: Run = Box::new(move |_| {
                    Box::pin(async move {
                        let (inner, who) = (me.clone(), who);
                        let applied =
                            blocking(move || inner.apply_todo(&who, &op, true, now_ms()))
                                .await??;
                        match applied {
                            TodoApplied::Done(done) => Ok(Deferred {
                                result: done.encode_to_vec(),
                                result_type: "humata.v1.WriteTodoResponse".to_owned(),
                            }),
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
        })
    }
}

/// What a patch makes of an item.
struct Planned {
    fields: Fields,
    title: Option<String>,
    description: Option<String>,
    sessions: Option<Vec<String>>,
    depends_on: Option<Vec<String>>,
    column: Option<i32>,
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
    let mut column = None;
    if let Some(state) = parsed.state
        && state != from.state()
    {
        let target = saved::column_for(state, record.column);
        column = (target != record.column).then_some(target);
        fields.state = Some(state);
        fields.state_column = Some(target);
        fields.closed_at_ms = (state == State::Closed).then_some(now);
    }
    Planned {
        fields,
        title,
        description,
        sessions: (sessions != record.sessions).then_some(sessions),
        depends_on: (deps != record.depends_on).then_some(deps),
        column,
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
        let (svc, _guard, agent) = ruled_svc(&dir);
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
}
