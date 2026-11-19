//! [`TodoClient`] for the real daemon, over its TodoService (BOARDSIMPLE).
//! The host keeps the list and its revisions; this client only asks, and
//! maps the wire items to the app's [`Entry`]s.

use humata2_clients::todo::{
    Entry, MigrationNotice, NewTodo, TodoClient, TodoFilter, TodoLists, TodoPatch,
};
use humata2_clients::{DaemonError, Reply};
use humata2_todo::{
    Ask, AskDetail, AskKind, ChannelFilter, Day, Effort, Evidence, Impact, OldColumn, Review, Start, State,
    Step, Summary, Todo,
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

/// A state the app doesn't know reads as open, the list's start.
fn state(value: i32) -> State {
    match pb::TodoState::try_from(value) {
        Ok(pb::TodoState::InProgress) => State::InProgress,
        Ok(pb::TodoState::Closed) => State::Closed,
        Ok(pb::TodoState::Open | pb::TodoState::Unspecified) | Err(_) => State::Open,
    }
}

fn state_pb(state: State) -> pb::TodoState {
    match state {
        State::Open => pb::TodoState::Open,
        State::InProgress => pb::TodoState::InProgress,
        State::Closed => pb::TodoState::Closed,
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

fn step_pb(step: &Step) -> pb::TodoStep {
    pb::TodoStep {
        text: step.text.clone(),
        done: step.done,
        ticked_by: None,
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
            text: step.text.clone(),
            done: step.done,
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
        ..pb::CreateTodoRequest::default()
    }
}

fn patch_pb(patch: TodoPatch) -> pb::TodoPatch {
    let (due, clear_due) = optional_day(patch.due);
    let (hide_until, clear_hide_until) = optional_day(patch.hide_until);
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
        accept_suggestion: patch.accept_suggestion,
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
                steps: vec![pb::TodoStep { text: "a".to_owned(), done: true, ticked_by: None }],
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
        assert_eq!(todo.steps, [Step { text: "a".to_owned(), done: true }]);
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
}
