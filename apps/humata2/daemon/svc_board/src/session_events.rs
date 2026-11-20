//! Live session events on To-do items (TODO2WIRE gaps 1-3): runs, the
//! stuck-agent card, the last-session-archived and idle prompts.

use std::collections::{BTreeMap, HashMap};

use humata2_daemon_ports::{Caller, UserVia};
use humata2_rpc_kit::lock;
use humata2_store::TodoRunRecord;
use humata2_todo::{OwnerKind, State};
use tonic::Status;

use crate::pb;
use crate::ports::{IDLE_NUDGE_DAYS, SeenStatus, SessionSeen};
use crate::pr_watch::{evaluate_last_session_archived, evaluate_session_idle};
use crate::todo::Entry;
use crate::todo_fields::{
    self as saved, Fields, LifecyclePromptRow, SessionSignalRow, StuckAgentRow,
};
use crate::BoardSvc;

const DAY_MS: i64 = 86_400_000;

/// How long an agent-owned item's session may sit idle, with nothing asked,
/// before the item shows as stuck.
const STUCK_IDLE_MS: i64 = 2 * 60 * 60 * 1000;

/// What the To-do list last saw of each session, and since when.
#[derive(Default)]
pub(crate) struct Seen {
    pub(crate) sessions: HashMap<String, SeenEntry>,
}

/// One session's last view.
#[derive(Clone, Debug, Default)]
pub(crate) struct SeenEntry {
    pub(crate) session: SessionSeen,
    /// When its current turn began, while it works.
    pub(crate) working_since_ms: Option<i64>,
    /// When it went idle, while it is.
    pub(crate) idle_since_ms: Option<i64>,
}

fn works(status: SeenStatus) -> bool {
    matches!(status, SeenStatus::Working | SeenStatus::Starting)
}

fn is_idle(status: SeenStatus) -> bool {
    matches!(status, SeenStatus::Idle | SeenStatus::ProbablyIdle)
}

/// Whether leaving work for `status` ends a turn worth a run.
fn ends_turn(status: SeenStatus) -> bool {
    matches!(
        status,
        SeenStatus::Idle
            | SeenStatus::ProbablyIdle
            | SeenStatus::NeedsInput
            | SeenStatus::Exited
            | SeenStatus::Stopped
            | SeenStatus::ResumeFailed
    )
}

fn status_word(status: SeenStatus) -> &'static str {
    match status {
        SeenStatus::Unknown => "unknown",
        SeenStatus::Starting => "starting",
        SeenStatus::Working => "working",
        SeenStatus::NeedsInput => "needs input",
        SeenStatus::Idle => "idle",
        SeenStatus::ProbablyIdle => "idle",
        SeenStatus::Exited => "exited",
        SeenStatus::Stopped => "stopped",
        SeenStatus::ResumeFailed => "resume failed",
        SeenStatus::Archived => "archived",
        SeenStatus::Offline => "offline",
    }
}

fn outcome_of(seen: &SessionSeen) -> String {
    match seen.status {
        SeenStatus::NeedsInput => "needs input".to_owned(),
        SeenStatus::Exited if seen.exit_code != 0 => format!("exited {}", seen.exit_code),
        SeenStatus::Exited => "exited".to_owned(),
        SeenStatus::Stopped => "stopped".to_owned(),
        SeenStatus::ResumeFailed => "resume failed".to_owned(),
        _ => "turn ended".to_owned(),
    }
}

/// A run id the store accepts, stable for one session turn on one item.
fn run_id(session: &str, turn_ms: i64, item: &str) -> String {
    let clean = |text: &str| -> String {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    format!("run_{}_{turn_ms}_{}", clean(session), clean(item))
}

/// The sessions on an item: its board item's, the session whose board it
/// was on first, and the agent that owns it.
fn sessions_of(entry: &Entry) -> Vec<String> {
    let mut sessions = entry.record.sessions.clone();
    if let humata2_store::BoardKey::Session(session) = &entry.record.board {
        sessions.push(session.clone());
    }
    if let Some(session) = entry.fields.owner.as_ref().and_then(|o| o.session.clone()) {
        sessions.push(session);
    }
    sessions.sort();
    sessions.dedup();
    sessions
}

fn channel_of(entry: &Entry) -> Option<String> {
    let channel = entry
        .fields
        .channel
        .clone()
        .unwrap_or_else(|| entry.record.channel.clone());
    (!channel.is_empty()).then_some(channel)
}

/// The agent-owned item's owner session, when it is in progress.
fn agent_session(entry: &Entry) -> Option<&str> {
    let owner = entry.fields.owner.as_ref()?;
    if owner.kind != OwnerKind::Agent || entry.state() != State::InProgress {
        return None;
    }
    owner.session.as_deref()
}

fn add_prompt(fields: &mut Fields, prompt: LifecyclePromptRow) -> bool {
    if fields.lifecycle_prompts.iter().any(|p| p.id == prompt.id) {
        return false;
    }
    fields.lifecycle_prompts.push(prompt);
    true
}

fn system_caller() -> Caller {
    Caller::User(UserVia::App)
}

impl BoardSvc {
    /// Notes where a session stands after a change on the feed, and acts on
    /// what changed: records a run when a turn ends, marks an agent-owned
    /// item stuck when its agent is gone, and prompts when an item's last
    /// session is archived.
    pub fn observe_session(&self, seen: &SessionSeen, now_ms: i64) -> Result<(), Status> {
        let (previous, started) = {
            let mut book = lock(&self.seen);
            let previous = book.sessions.get(&seen.id).cloned();
            let was = previous.as_ref().map(|p| p.session.status);
            let working_since_ms = if works(seen.status) {
                match (&previous, was) {
                    (Some(p), Some(status)) if works(status) => {
                        p.working_since_ms.or(Some(now_ms))
                    }
                    _ => Some(now_ms),
                }
            } else {
                None
            };
            let idle_since_ms = if is_idle(seen.status) {
                match (&previous, was) {
                    (Some(p), Some(status)) if is_idle(status) => p.idle_since_ms.or(Some(now_ms)),
                    _ => Some(now_ms),
                }
            } else {
                None
            };
            let started = previous.as_ref().and_then(|p| p.working_since_ms);
            book.sessions.insert(
                seen.id.clone(),
                SeenEntry {
                    session: seen.clone(),
                    working_since_ms,
                    idle_since_ms,
                },
            );
            (previous, started)
        };
        let was = previous.as_ref().map(|p| p.session.status);
        let turn_ended = was.is_some_and(works) && ends_turn(seen.status);
        let acts = turn_ended
            || matches!(
                seen.status,
                SeenStatus::Exited
                    | SeenStatus::Stopped
                    | SeenStatus::ResumeFailed
                    | SeenStatus::Working
                    | SeenStatus::Starting
                    | SeenStatus::Archived
            );
        if !acts {
            return Ok(());
        }

        let linked = |entry: &Entry| {
            !entry.record.deleted
                && (sessions_of(entry).contains(&seen.id)
                    || seen.board_item.as_deref() == Some(entry.record.key.as_str()))
        };

        if turn_ended && !seen.agent.is_empty() {
            let turn_start = started.unwrap_or(now_ms);
            let entries: Vec<Entry> = self
                .entries(&system_caller())?
                .into_iter()
                .filter(|entry| linked(entry))
                .collect();
            for entry in &entries {
                let label = if seen.title.is_empty() {
                    &seen.id
                } else {
                    &seen.title
                };
                let links = serde_json::json!([{
                    "label": label,
                    "target": format!("session:{}", seen.id),
                }]);
                self.store
                    .record_todo_run(&TodoRunRecord {
                        id: run_id(&seen.id, turn_start, &entry.record.key),
                        item_key: entry.record.key.clone(),
                        tool: seen.agent.clone(),
                        model: seen.model.clone(),
                        outcome: outcome_of(seen),
                        cost_micros: None,
                        started_unix_ms: turn_start,
                        finished_unix_ms: now_ms,
                        links_json: links.to_string(),
                        session: seen.id.clone(),
                    })
                    .map_err(|error| self.failed(error, "record the agent run"))?;
            }
            self.tell_updated(&entries)?;
        }

        let book = lock(&self.seen);
        let still_active = |session: &str| {
            book.sessions
                .get(session)
                .is_none_or(|entry| entry.session.status != SeenStatus::Archived)
        };
        let remaining: HashMap<String, usize> = self
            .entries(&system_caller())?
            .iter()
            .filter(|entry| linked(entry))
            .map(|entry| {
                let others = sessions_of(entry)
                    .into_iter()
                    .filter(|other| *other != seen.id && still_active(other))
                    .count();
                (entry.record.key.clone(), others)
            })
            .collect();
        drop(book);

        self.edit_items(|entry, fields| {
            if !linked(entry) {
                return false;
            }
            let mut changed = false;
            if agent_session(entry) == Some(seen.id.as_str()) {
                match seen.status {
                    SeenStatus::Exited | SeenStatus::Stopped | SeenStatus::ResumeFailed => {
                        let already = fields
                            .stuck_agent
                            .as_ref()
                            .is_some_and(|stuck| stuck.session == seen.id);
                        if !already {
                            let label = fields
                                .owner
                                .as_ref()
                                .map(|o| o.label.clone())
                                .filter(|label| !label.is_empty())
                                .unwrap_or_else(|| seen.agent.clone());
                            let reason = match seen.status {
                                SeenStatus::Stopped => "the agent was stopped",
                                SeenStatus::ResumeFailed => "the agent couldn't resume",
                                _ => "the agent exited",
                            };
                            fields.stuck_agent = Some(StuckAgentRow {
                                session: seen.id.clone(),
                                agent_label: label,
                                reason: reason.to_owned(),
                                since_ms: now_ms,
                            });
                            changed = true;
                        }
                    }
                    SeenStatus::Working | SeenStatus::Starting => {
                        if fields.stuck_agent.is_some() {
                            fields.stuck_agent = None;
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
            if seen.status == SeenStatus::Archived
                && let Some(count) = remaining.get(&entry.record.key)
                && let Some(prompt) =
                    evaluate_last_session_archived(&entry.record.key, entry.state(), *count)
            {
                changed |= add_prompt(fields, prompt);
            }
            changed
        })
    }

    /// Looks over every in-progress item: refreshes what its sessions are
    /// doing, nudges about one that went quiet for days, and marks an
    /// agent-owned one stuck when its agent sits idle for hours. Called every
    /// few minutes.
    ///
    /// "Nothing asked" means the item holds no lifecycle prompt; an agent
    /// that asks the person is in Needs input, not idle, so a pending Inbox
    /// ask never coincides with the idle status this checks.
    pub fn session_tick(&self, now_ms: i64) -> Result<(), Status> {
        let book: HashMap<String, SeenEntry> = lock(&self.seen).sessions.clone();
        self.edit_items(|entry, fields| {
            if entry.state() != State::InProgress {
                return false;
            }
            let mut changed = false;
            let mut signals = Vec::new();
            for session in sessions_of(entry) {
                let Some(seen) = book.get(&session) else {
                    continue;
                };
                if seen.session.status == SeenStatus::Archived {
                    continue;
                }
                let idle_days = seen
                    .session
                    .last_output_ms
                    .map_or(0, |at| (now_ms - at).max(0) / DAY_MS);
                signals.push(SessionSignalRow {
                    session: session.clone(),
                    label: if seen.session.title.is_empty() {
                        session.clone()
                    } else {
                        seen.session.title.clone()
                    },
                    status: status_word(seen.session.status).to_owned(),
                    idle_days: u32::try_from(idle_days).unwrap_or(u32::MAX),
                    active_turn: works(seen.session.status),
                });
            }
            if fields.session_signals != signals {
                fields.session_signals = signals.clone();
                changed = true;
            }
            let configured = self
                .automation_for(channel_of(entry).as_deref())
                .idle_nudge_days;
            let threshold = if configured == 0 {
                IDLE_NUDGE_DAYS
            } else {
                configured
            };
            for signal in &signals {
                if let Some(prompt) = evaluate_session_idle(signal, threshold) {
                    changed |= add_prompt(fields, prompt);
                }
            }
            if fields.stuck_agent.is_none()
                && fields.lifecycle_prompts.is_empty()
                && let Some(session) = agent_session(entry)
                && let Some(seen) = book.get(session)
                && is_idle(seen.session.status)
                && let Some(since) = seen.idle_since_ms.or(seen.session.last_output_ms)
                && now_ms - since >= STUCK_IDLE_MS
            {
                let label = fields
                    .owner
                    .as_ref()
                    .map(|o| o.label.clone())
                    .filter(|label| !label.is_empty())
                    .unwrap_or_else(|| seen.session.agent.clone());
                fields.stuck_agent = Some(StuckAgentRow {
                    session: session.to_owned(),
                    agent_label: label,
                    reason: "the agent stopped without asking anything".to_owned(),
                    since_ms: now_ms,
                });
                changed = true;
            }
            changed
        })
    }

    /// Fills in what is known of the sessions at startup, acting on none of
    /// it, so the first event after a restart compares against the real
    /// previous status.
    pub fn seed_sessions(&self, sessions: &[SessionSeen]) {
        let mut book = lock(&self.seen);
        for session in sessions {
            book.sessions.insert(
                session.id.clone(),
                SeenEntry {
                    session: session.clone(),
                    working_since_ms: None,
                    idle_since_ms: is_idle(session.status)
                        .then_some(session.last_output_ms)
                        .flatten(),
                },
            );
        }
    }

    /// Lets `edit` change each live item's To-do fields; for every item it
    /// reports changed, bumps the revision, saves once and tells the feed.
    fn edit_items(
        &self,
        mut edit: impl FnMut(&Entry, &mut Fields) -> bool,
    ) -> Result<(), Status> {
        let changed = {
            let _write = lock(&self.writes);
            let entries = self.entries(&system_caller())?;
            let mut map = saved::load(&self.store)?;
            let mut changed = Vec::new();
            for entry in entries {
                if entry.record.deleted {
                    continue;
                }
                let mut fields = entry.fields.clone();
                if !edit(&entry, &mut fields) {
                    continue;
                }
                fields.extra_revisions += 1;
                map.insert(entry.record.key.clone(), fields.clone());
                changed.push(Entry {
                    record: entry.record,
                    fields,
                });
            }
            if changed.is_empty() {
                return Ok(());
            }
            self.save_fields(&map)?;
            changed
        };
        self.tell_updated(&changed)
    }

    fn save_fields(&self, map: &BTreeMap<String, Fields>) -> Result<(), Status> {
        saved::save(&self.store, map)?;
        Ok(())
    }

    /// Tells the feed these items changed, so a view of them refreshes.
    fn tell_updated(&self, entries: &[Entry]) -> Result<(), Status> {
        if entries.is_empty() {
            return Ok(());
        }
        let archived = self.archived()?;
        for entry in entries {
            let mut record = entry.record.clone();
            record.revision = entry.revision();
            record.sessions = sessions_of(entry);
            if let Some(channel) = channel_of(entry) {
                record.channel = channel;
            }
            self.tell(&system_caller(), pb::Change::Updated, &record, &archived);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use humata2_store::{BoardKey, NewBoardItem};

    use super::*;
    use crate::test_support::test_svc;
    use crate::todo_fields::{IN_PROGRESS_COLUMN, TODO_COLUMN, TodoOwnerRow};

    const T0: i64 = 10_000_000_000;

    fn item(svc: &BoardSvc, column: i32, sessions: &[&str]) -> String {
        // The Global board numbers its items once it has a prefix; a repeat
        // claim answers AlreadyYours.
        svc.store.claim_board_prefix("global", "TD").unwrap();
        svc.store
            .create_board_item(&NewBoardItem {
                board: BoardKey::Global,
                channel: String::new(),
                title: "Ship it".to_owned(),
                description: String::new(),
                column,
                rank: "m".to_owned(),
                created_by: String::new(),
                at_unix_ms: 1,
                sessions: sessions.iter().map(|s| (*s).to_owned()).collect(),
                depends_on: Vec::new(),
            })
            .unwrap()
            .key
    }

    fn own_by_agent(svc: &BoardSvc, key: &str, session: &str) {
        let mut map = saved::load(&svc.store).unwrap();
        map.entry(key.to_owned()).or_default().owner = Some(TodoOwnerRow {
            kind: OwnerKind::Agent,
            label: "claude".to_owned(),
            session: Some(session.to_owned()),
        });
        saved::save(&svc.store, &map).unwrap();
    }

    fn fields_of(svc: &BoardSvc, key: &str) -> Fields {
        saved::load(&svc.store)
            .unwrap()
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    fn seen(id: &str, agent: &str, status: SeenStatus) -> SessionSeen {
        SessionSeen {
            id: id.to_owned(),
            title: format!("{id} title"),
            agent: agent.to_owned(),
            model: "sonnet".to_owned(),
            status,
            ..SessionSeen::default()
        }
    }

    #[test]
    fn a_turn_ending_records_one_run_per_linked_item_and_a_repeat_adds_none() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Working), T0)
            .unwrap();
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Idle), T0 + 5_000)
            .unwrap();
        let runs = svc.store.list_todo_runs(&key).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].tool, "claude");
        assert_eq!(runs[0].session, "se_1");
        assert_eq!(runs[0].started_unix_ms, T0);
        assert_eq!(runs[0].finished_unix_ms, T0 + 5_000);
        assert!(runs[0].links_json.contains("session:se_1"));

        svc.observe_session(&seen("se_1", "claude", SeenStatus::Idle), T0 + 9_000)
            .unwrap();
        assert_eq!(svc.store.list_todo_runs(&key).unwrap().len(), 1);
    }

    #[test]
    fn replaying_the_same_turn_end_updates_the_run_instead_of_duplicating_it() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        for _ in 0..2 {
            // The same turn reported twice: same start, so the same run.
            svc.observe_session(&seen("se_1", "claude", SeenStatus::Working), T0)
                .unwrap();
            svc.observe_session(&seen("se_1", "claude", SeenStatus::Idle), T0 + 1)
                .unwrap();
        }
        assert_eq!(svc.store.list_todo_runs(&key).unwrap().len(), 1);
    }

    #[test]
    fn a_plain_shell_records_no_run() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        svc.observe_session(&seen("se_1", "", SeenStatus::Working), T0)
            .unwrap();
        svc.observe_session(&seen("se_1", "", SeenStatus::Idle), T0 + 1)
            .unwrap();
        assert!(svc.store.list_todo_runs(&key).unwrap().is_empty());
    }

    #[test]
    fn a_session_the_item_owns_or_was_started_for_gets_a_run_too() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let owned = item(&svc, IN_PROGRESS_COLUMN, &[]);
        own_by_agent(&svc, &owned, "se_1");
        let started_for = item(&svc, TODO_COLUMN, &[]);
        let mut working = seen("se_1", "claude", SeenStatus::Working);
        working.board_item = Some(started_for.clone());
        let mut done = working.clone();
        done.status = SeenStatus::NeedsInput;
        svc.observe_session(&working, T0).unwrap();
        svc.observe_session(&done, T0 + 1).unwrap();
        assert_eq!(svc.store.list_todo_runs(&owned).unwrap().len(), 1);
        assert_eq!(svc.store.list_todo_runs(&started_for).unwrap().len(), 1);
        assert_eq!(
            svc.store.list_todo_runs(&owned).unwrap()[0].outcome,
            "needs input"
        );
    }

    #[test]
    fn an_exited_agent_sticks_its_in_progress_item_and_working_again_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        own_by_agent(&svc, &key, "se_1");
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Working), T0)
            .unwrap();
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Exited), T0 + 1)
            .unwrap();
        let stuck = fields_of(&svc, &key).stuck_agent.unwrap();
        assert_eq!(stuck.session, "se_1");
        assert_eq!(stuck.since_ms, T0 + 1);
        assert!(!stuck.reason.is_empty());

        // A repeat doesn't rewrite it.
        let revision = fields_of(&svc, &key).extra_revisions;
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Exited), T0 + 2)
            .unwrap();
        let again = fields_of(&svc, &key);
        assert_eq!(again.extra_revisions, revision);
        assert_eq!(again.stuck_agent.unwrap().since_ms, T0 + 1);

        svc.observe_session(&seen("se_1", "claude", SeenStatus::Working), T0 + 3)
            .unwrap();
        assert!(fields_of(&svc, &key).stuck_agent.is_none());
    }

    #[test]
    fn an_item_not_in_progress_or_not_agent_owned_is_never_stuck() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let open = item(&svc, TODO_COLUMN, &["se_1"]);
        own_by_agent(&svc, &open, "se_1");
        let user_owned = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Stopped), T0)
            .unwrap();
        assert!(fields_of(&svc, &open).stuck_agent.is_none());
        assert!(fields_of(&svc, &user_owned).stuck_agent.is_none());
    }

    #[test]
    fn offline_leaves_a_stuck_card_and_a_clean_item_alone() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        own_by_agent(&svc, &key, "se_1");
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Offline), T0)
            .unwrap();
        assert!(fields_of(&svc, &key).stuck_agent.is_none());
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Stopped), T0 + 1)
            .unwrap();
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Offline), T0 + 2)
            .unwrap();
        assert!(fields_of(&svc, &key).stuck_agent.is_some());
    }

    #[test]
    fn archiving_the_only_session_prompts_but_archiving_one_of_two_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let lone = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        let pair = item(&svc, IN_PROGRESS_COLUMN, &["se_1", "se_2"]);
        svc.seed_sessions(&[
            seen("se_1", "claude", SeenStatus::Idle),
            seen("se_2", "claude", SeenStatus::Idle),
        ]);
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Archived), T0)
            .unwrap();
        assert_eq!(fields_of(&svc, &lone).lifecycle_prompts.len(), 1);
        assert_eq!(
            fields_of(&svc, &lone).lifecycle_prompts[0].kind,
            "last_session_archived"
        );
        assert!(fields_of(&svc, &pair).lifecycle_prompts.is_empty());

        svc.observe_session(&seen("se_2", "claude", SeenStatus::Archived), T0 + 1)
            .unwrap();
        assert_eq!(fields_of(&svc, &pair).lifecycle_prompts.len(), 1);

        // The prompt is not added twice.
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Archived), T0 + 2)
            .unwrap();
        assert_eq!(fields_of(&svc, &lone).lifecycle_prompts.len(), 1);
    }

    #[test]
    fn a_session_idle_past_the_threshold_gets_one_idle_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        let mut quiet = seen("se_1", "claude", SeenStatus::Idle);
        quiet.last_output_ms = Some(T0);
        svc.seed_sessions(&[quiet]);

        svc.session_tick(T0 + 2 * DAY_MS).unwrap();
        let fields = fields_of(&svc, &key);
        assert!(fields.lifecycle_prompts.is_empty());
        assert_eq!(fields.session_signals.len(), 1);
        assert_eq!(fields.session_signals[0].idle_days, 2);

        svc.session_tick(T0 + 3 * DAY_MS).unwrap();
        let fields = fields_of(&svc, &key);
        assert_eq!(fields.lifecycle_prompts.len(), 1);
        assert_eq!(fields.lifecycle_prompts[0].kind, "session_idle");
        let revision = fields.extra_revisions;

        svc.session_tick(T0 + 3 * DAY_MS + 1_000).unwrap();
        let fields = fields_of(&svc, &key);
        assert_eq!(fields.lifecycle_prompts.len(), 1);
        assert_eq!(fields.extra_revisions, revision);
    }

    #[test]
    fn a_working_session_is_never_nudged_and_an_open_item_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let busy = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        let open = item(&svc, TODO_COLUMN, &["se_2"]);
        let mut working = seen("se_1", "claude", SeenStatus::Working);
        working.last_output_ms = Some(T0);
        let mut quiet = seen("se_2", "claude", SeenStatus::Idle);
        quiet.last_output_ms = Some(T0);
        svc.seed_sessions(&[working, quiet]);
        svc.session_tick(T0 + 10 * DAY_MS).unwrap();
        assert!(fields_of(&svc, &busy).lifecycle_prompts.is_empty());
        assert!(fields_of(&svc, &open).lifecycle_prompts.is_empty());
    }

    #[test]
    fn an_agent_idle_for_hours_with_no_prompt_pending_is_stuck() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        own_by_agent(&svc, &key, "se_1");
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Working), T0)
            .unwrap();
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Idle), T0 + 1_000)
            .unwrap();

        svc.session_tick(T0 + 1_000 + STUCK_IDLE_MS - 1).unwrap();
        assert!(fields_of(&svc, &key).stuck_agent.is_none());

        svc.session_tick(T0 + 1_000 + STUCK_IDLE_MS).unwrap();
        let stuck = fields_of(&svc, &key).stuck_agent.unwrap();
        assert_eq!(stuck.session, "se_1");

        // Its agent working again clears it.
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Working), T0 + STUCK_IDLE_MS + 5)
            .unwrap();
        assert!(fields_of(&svc, &key).stuck_agent.is_none());
    }

    #[test]
    fn seeding_acts_on_nothing_but_sets_the_previous_status() {
        let dir = tempfile::tempdir().unwrap();
        let svc = test_svc(&dir);
        let key = item(&svc, IN_PROGRESS_COLUMN, &["se_1"]);
        own_by_agent(&svc, &key, "se_1");
        svc.seed_sessions(&[seen("se_1", "claude", SeenStatus::Working)]);
        assert!(svc.store.list_todo_runs(&key).unwrap().is_empty());
        svc.observe_session(&seen("se_1", "claude", SeenStatus::Idle), T0)
            .unwrap();
        assert_eq!(svc.store.list_todo_runs(&key).unwrap().len(), 1);
    }
}
