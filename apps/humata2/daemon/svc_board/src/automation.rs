//! To-do automation (TODO2WIRE gap 3): Ask/Auto on PR merge and on close,
//! and the prompt choices that act on sessions.
//!
//! The writes of `todo.rs` are synchronous and run under the write lock,
//! but the session actions are async. So a write only records what is to be
//! done: a prompt for the person (Ask), or a hidden pending row (Auto, or a
//! chosen "Archive sessions" / "Undo"). The async handler then runs the
//! session actions with no lock held, and a second short write swaps the
//! pending row for its result prompt. The response the caller gets is the
//! item after that second write, and a pending row left by a crash is
//! picked up by the next write that settles the item.

use humata2_daemon_ports::Caller;
use humata2_rpc_kit::{blocking, lock};
use humata2_store::{BoardItemChange, BoardItemRecord, BoardKey, BoardWrite};
use humata2_todo::State;
use tonic::Status;

use super::scope::by_of;
use super::todo::{Entry, Planned};
use super::todo_fields::{self as saved, Fields, LifecyclePromptRow, TodoHistoryRow};
use super::{ArchiveOutcome, BoardSvc, SessionControl, SyncMode, item_id, merged};
use crate::pb;

const CLOSE_ARCHIVE: &str = "close_archive_sessions";
const SESSIONS_ARCHIVED: &str = "sessions_archived";
const ARCHIVE_PENDING: &str = "archive_pending";
const RESTORE_PENDING: &str = "restore_pending";

/// Whether `prompt` is bookkeeping for a session action that is yet to run,
/// not something to show.
pub(super) fn is_pending(prompt: &LifecyclePromptRow) -> bool {
    matches!(prompt.kind.as_str(), ARCHIVE_PENDING | RESTORE_PENDING)
}

fn channel_opt(channel: &str) -> Option<&str> {
    (!channel.is_empty()).then_some(channel)
}

/// The sessions linked to an item, with the session whose board it was on.
pub(crate) fn linked_sessions(record: &BoardItemRecord) -> Vec<String> {
    let mut sessions = record.sessions.clone();
    if let BoardKey::Session(session) = &record.board
        && !sessions.contains(session)
    {
        sessions.push(session.clone());
    }
    sessions
}

fn note(
    fields: &mut Fields,
    now: i64,
    by: &str,
    kind: pb::HistoryKind,
    field: &str,
    change: String,
    revision: u64,
) {
    fields.extra_history.push(TodoHistoryRow {
        at_unix_ms: now,
        by: by.to_owned(),
        kind: kind as i32,
        field: field.to_owned(),
        before: String::new(),
        after: String::new(),
        change,
        revision,
    });
}

fn sessions_word(count: usize) -> String {
    if count == 1 {
        "1 session".to_owned()
    } else {
        format!("{count} sessions")
    }
}

/// Puts `prompt` in place of any prompt with its id, remembering the
/// sessions it acts on.
fn put_prompt(fields: &mut Fields, prompt: LifecyclePromptRow, sessions: Vec<String>) {
    fields.lifecycle_prompts.retain(|p| p.id != prompt.id);
    if sessions.is_empty() {
        fields.prompt_sessions.remove(&prompt.id);
    } else {
        fields.prompt_sessions.insert(prompt.id.clone(), sessions);
    }
    fields.lifecycle_prompts.push(prompt);
}

fn pending(kind: &str, key: &str, sessions: Vec<String>) -> (LifecyclePromptRow, Vec<String>) {
    let prefix = if kind == ARCHIVE_PENDING {
        "archive-pending"
    } else {
        "restore-pending"
    };
    (
        LifecyclePromptRow {
            id: format!("{prefix}:{key}"),
            kind: kind.to_owned(),
            ..LifecyclePromptRow::default()
        },
        sessions,
    )
}

/// What a watched pull request's evaluation came to for the item.
pub(crate) struct PrOutcome {
    /// Set when the item was closed because its pull request merged: the
    /// column the item belongs in now.
    pub(crate) close_column: Option<i32>,
}

impl BoardSvc {
    /// Adds the prompts a watched pull request calls for to `fields`. With
    /// "on PR merged" set to Auto, a merged pull request closes the item in
    /// place of its prompt; the caller moves the item to `close_column`.
    /// A pull request closed without merge always prompts.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn apply_pr_prompts(
        &self,
        fields: &mut Fields,
        current: State,
        column: i32,
        channel: &str,
        prompts: Vec<LifecyclePromptRow>,
        now: i64,
        by: &str,
        revision: u64,
    ) -> PrOutcome {
        let auto = self.automation_for(channel_opt(channel)).on_pr_merged == SyncMode::Auto;
        let mut state = current;
        let mut close_column = None;
        for prompt in prompts {
            if auto && prompt.kind == "pr_merged" {
                if !state.is_terminal() {
                    let target = saved::column_for(State::Closed, column);
                    fields.state = Some(State::Closed);
                    fields.state_column = Some(target);
                    fields.closed_at_ms = Some(now);
                    state = State::Closed;
                    close_column = Some(target);
                    note(
                        fields,
                        now,
                        by,
                        pb::HistoryKind::PrSynced,
                        "state",
                        format!(
                            "closed automatically because PR {} merged",
                            prompt.pr_ref
                        ),
                        revision,
                    );
                }
                continue;
            }
            if !fields.lifecycle_prompts.iter().any(|p| p.id == prompt.id) {
                fields.lifecycle_prompts.push(prompt);
            }
        }
        PrOutcome { close_column }
    }

    /// Moves an item to `column`, at the top of it, for the watcher's
    /// automatic close.
    pub(crate) fn move_to_column(
        &self,
        record: &BoardItemRecord,
        column: i32,
        by: &str,
        now: i64,
    ) -> Result<BoardItemRecord, Status> {
        let rank = self.rank_in(&record.board, column, &record.key, None)?;
        let write = self
            .store
            .change_board_item(&BoardItemChange {
                key: record.key.clone(),
                expected_revision: None,
                by: by.to_owned(),
                at_unix_ms: now,
                title: None,
                description: None,
                column: Some(column),
                rank: Some(rank),
                sessions: None,
                depends_on: None,
                delete: false,
                cause: String::new(),
            })
            .map_err(|error| self.failed(error, "move the closed item"))?;
        Ok(match write {
            BoardWrite::Applied(item) => item,
            _ => record.clone(),
        })
    }

    /// The hooks of one `update_todo_in`: what the watched pull requests
    /// now say (Auto may close the item), then what closing does to the
    /// item's sessions.
    pub(super) fn automate_update(
        &self,
        from: &Entry,
        planned: &mut Planned,
        by: &str,
        now: i64,
    ) -> Result<(), Status> {
        let key = from.record.key.clone();
        let channel = from.channel();
        let before = from.state();
        let revision = from.record.revision + planned.fields.extra_revisions + 1;
        let column = planned.column.unwrap_or(from.record.column);
        let state = planned.fields.state(column);
        let prompts = std::mem::take(&mut planned.new_prompts);
        let outcome = self.apply_pr_prompts(
            &mut planned.fields,
            state,
            column,
            &channel,
            prompts,
            now,
            by,
            revision,
        );
        if let Some(target) = outcome.close_column {
            planned.column = (target != from.record.column).then_some(target);
        }
        let after = planned
            .fields
            .state(planned.column.unwrap_or(from.record.column));
        let linked = merged(
            &from.sessions(),
            planned.sessions.as_deref().unwrap_or(&[]),
            &[],
        );
        self.after_close(
            &mut planned.fields,
            &key,
            before,
            after,
            &linked,
            &channel,
            now,
            by,
            revision,
        )
    }

    /// When an item went from open to Closed and has sessions that aren't
    /// archived: Ask leaves a prompt, Auto leaves a pending archive. Does
    /// nothing without a session control (a machine, or a build that has
    /// none).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn after_close(
        &self,
        fields: &mut Fields,
        key: &str,
        before: State,
        after: State,
        linked: &[String],
        channel: &str,
        now: i64,
        by: &str,
        revision: u64,
    ) -> Result<(), Status> {
        if self.sessions_control.is_none() || before.is_terminal() || after != State::Closed {
            return Ok(());
        }
        let archived = self.archived()?;
        let sessions: Vec<String> = linked
            .iter()
            .filter(|session| !archived.contains(*session))
            .cloned()
            .collect();
        if sessions.is_empty() {
            return Ok(());
        }
        match self
            .automation_for(channel_opt(channel))
            .on_todo_closed_archive_sessions
        {
            SyncMode::Ask => {
                let prompt = LifecyclePromptRow {
                    id: format!("close-archive:{key}"),
                    kind: CLOSE_ARCHIVE.to_owned(),
                    title: format!("Archive {}?", sessions_word(sessions.len())),
                    detail: format!("Linked: {}", sessions.join(", ")),
                    choices: vec!["Archive sessions".to_owned(), "Keep".to_owned()],
                    pr_ref: String::new(),
                    session: None,
                };
                put_prompt(fields, prompt, sessions);
            }
            SyncMode::Auto => {
                let (prompt, sessions) = pending(ARCHIVE_PENDING, key, sessions);
                put_prompt(fields, prompt, sessions);
                note(
                    fields,
                    now,
                    by,
                    pb::HistoryKind::Sessions,
                    "sessions",
                    "closing: archiving its clean sessions".to_owned(),
                    revision,
                );
            }
        }
        Ok(())
    }

    /// What a chosen prompt button does inside the write (the prompt is
    /// already off the list): queue the archive or restore, or check that a
    /// note has a session to go to.
    pub(super) fn act_on_prompt(
        &self,
        fields: &mut Fields,
        record: &BoardItemRecord,
        prompt: &LifecyclePromptRow,
        choice: &str,
        _by: &str,
        _now: i64,
    ) -> Result<(), Status> {
        let choice = choice.to_lowercase();
        let sessions = fields
            .prompt_sessions
            .get(&prompt.id)
            .cloned()
            .unwrap_or_default();
        match (prompt.kind.as_str(), choice.as_str()) {
            (CLOSE_ARCHIVE, "archive sessions") if !sessions.is_empty() => {
                let (row, sessions) = pending(ARCHIVE_PENDING, &record.key, sessions);
                put_prompt(fields, row, sessions);
            }
            (SESSIONS_ARCHIVED, "undo") if !sessions.is_empty() => {
                let (row, sessions) = pending(RESTORE_PENDING, &record.key, sessions);
                put_prompt(fields, row, sessions);
            }
            _ if sends_note(prompt, &choice) => {
                note_for(fields, record, prompt)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Resolves a prompt, sending its note first when its button sends one
    /// (a failed send keeps the prompt), then runs the session actions the
    /// choice queued.
    pub(super) async fn resolve_prompt_acting(
        &self,
        caller: &Caller,
        request: pb::ResolveLifecyclePromptRequest,
        now: i64,
    ) -> Result<pb::WriteTodoResponse, Status> {
        let (me, who, asked) = (self.clone(), caller.clone(), request.clone());
        let send = blocking(move || me.note_to_send(&who, &asked)).await??;
        if let Some((session, text)) = send {
            let control = self.sessions_control.clone().ok_or_else(|| {
                Status::failed_precondition(
                    "This machine can't send notes to sessions. Open Humata on the machine that runs the session and use the button there",
                )
            })?;
            control.send_note(&session, &text).await?;
        }
        let (me, who, asked) = (self.clone(), caller.clone(), request.clone());
        let mut response =
            blocking(move || me.resolve_lifecycle_prompt_in(&who, &asked, now)).await??;
        if let Some(id) = request.id.as_ref()
            && let Some(item) = self.settle_key(caller, &id.value, now).await?
        {
            response.item = Some(item);
        }
        Ok(response)
    }

    /// The session and text a prompt's send button delivers; none for other
    /// buttons.
    fn note_to_send(
        &self,
        caller: &Caller,
        request: &pb::ResolveLifecyclePromptRequest,
    ) -> Result<Option<(String, String)>, Status> {
        Self::refuse_peer(caller)?;
        let id = item_id(request.id.as_ref())?;
        let record = self.load(caller, &id, false)?;
        let map = saved::load(&self.store)?;
        let Some(fields) = map.get(&id) else {
            return Ok(None);
        };
        let Some(prompt) = fields
            .lifecycle_prompts
            .iter()
            .find(|p| p.id == request.prompt_id.trim())
        else {
            return Ok(None);
        };
        if !sends_note(prompt, &request.choice.trim().to_lowercase()) {
            return Ok(None);
        }
        note_for(fields, &record, prompt).map(Some)
    }

    /// Runs the session actions queued on the item and replaces each queued
    /// row with its result. None when nothing was queued.
    pub(super) async fn settle_key(
        &self,
        caller: &Caller,
        key: &str,
        now: i64,
    ) -> Result<Option<pb::TodoItem>, Status> {
        let Some(control) = self.sessions_control.clone() else {
            return Ok(None);
        };
        let (me, item) = (self.clone(), key.to_owned());
        let work = blocking(move || me.queued_work(&item)).await??;
        if work.is_empty() {
            return Ok(None);
        }
        let mut done = Vec::new();
        for job in work {
            done.push(run_job(control.as_ref(), job).await);
        }
        let (me, who, item) = (self.clone(), caller.clone(), key.to_owned());
        blocking(move || me.finish_jobs(&who, &item, done, now))
            .await?
            .map(Some)
    }

    /// `settle_key` for an item in a response, when it is Closed.
    pub(super) async fn settle_item(
        &self,
        caller: &Caller,
        item: &pb::TodoItem,
        now: i64,
    ) -> Result<Option<pb::TodoItem>, Status> {
        if self.sessions_control.is_none() {
            return Ok(None);
        }
        let closed = item
            .fields
            .as_ref()
            .is_some_and(|fields| fields.state == pb::TodoState::Closed as i32);
        let key = item
            .item
            .as_ref()
            .and_then(|item| item.id.as_ref())
            .map(|id| id.value.clone());
        match key {
            Some(key) if closed => self.settle_key(caller, &key, now).await,
            _ => Ok(None),
        }
    }

    /// `settle_item` on a write's response.
    pub(super) async fn settle_response(
        &self,
        caller: &Caller,
        mut response: pb::WriteTodoResponse,
        now: i64,
    ) -> Result<pb::WriteTodoResponse, Status> {
        if let Some(item) = &response.item
            && let Some(settled) = self.settle_item(caller, item, now).await?
        {
            response.item = Some(settled);
        }
        Ok(response)
    }

    fn queued_work(&self, key: &str) -> Result<Vec<Job>, Status> {
        let map = saved::load(&self.store)?;
        let Some(fields) = map.get(key) else {
            return Ok(Vec::new());
        };
        Ok(fields
            .lifecycle_prompts
            .iter()
            .filter(|prompt| is_pending(prompt))
            .map(|prompt| Job {
                prompt_id: prompt.id.clone(),
                restore: prompt.kind == RESTORE_PENDING,
                sessions: fields
                    .prompt_sessions
                    .get(&prompt.id)
                    .cloned()
                    .unwrap_or_default(),
            })
            .collect())
    }

    fn finish_jobs(
        &self,
        caller: &Caller,
        key: &str,
        done: Vec<Done>,
        now: i64,
    ) -> Result<pb::TodoItem, Status> {
        let _write = lock(&self.writes);
        let record = self.load(caller, key, false)?;
        let mut map = saved::load(&self.store)?;
        let mut fields = map.get(key).cloned().unwrap_or_default();
        let by = by_of(caller);
        fields.extra_revisions += 1;
        let revision = record.revision + fields.extra_revisions;
        for job in done {
            fields.lifecycle_prompts.retain(|p| p.id != job.prompt_id);
            fields.prompt_sessions.remove(&job.prompt_id);
            match job.result {
                Outcome::Archived { archived, kept } => {
                    let lines: Vec<String> = kept
                        .iter()
                        .map(|(session, reason)| format!("kept: {session} {reason}"))
                        .collect();
                    if archived.is_empty() && lines.is_empty() {
                        continue;
                    }
                    let (title, choices) = if archived.is_empty() {
                        ("No sessions archived".to_owned(), vec!["Dismiss".to_owned()])
                    } else {
                        (
                            format!("Archived {}", sessions_word(archived.len())),
                            vec!["Undo".to_owned(), "Dismiss".to_owned()],
                        )
                    };
                    let id = format!("sessions-archived:{key}");
                    put_prompt(
                        &mut fields,
                        LifecyclePromptRow {
                            id,
                            kind: SESSIONS_ARCHIVED.to_owned(),
                            title,
                            detail: lines.join("\n"),
                            choices,
                            pr_ref: String::new(),
                            session: None,
                        },
                        archived.clone(),
                    );
                    if !archived.is_empty() {
                        note(
                            &mut fields,
                            now,
                            &by,
                            pb::HistoryKind::Sessions,
                            "sessions",
                            format!("archived {}", archived.join(", ")),
                            revision,
                        );
                    }
                }
                Outcome::Restored { restored, failed } => {
                    if !restored.is_empty() {
                        note(
                            &mut fields,
                            now,
                            &by,
                            pb::HistoryKind::Sessions,
                            "sessions",
                            format!("restored {}", restored.join(", ")),
                            revision,
                        );
                    }
                    if !failed.is_empty() {
                        let lines: Vec<String> = failed
                            .iter()
                            .map(|(session, why)| format!("not restored: {session} ({why})"))
                            .collect();
                        let sessions: Vec<String> =
                            failed.iter().map(|(session, _)| session.clone()).collect();
                        put_prompt(
                            &mut fields,
                            LifecyclePromptRow {
                                id: format!("sessions-archived:{key}"),
                                kind: SESSIONS_ARCHIVED.to_owned(),
                                title: format!("Couldn't restore {}", sessions_word(sessions.len())),
                                detail: lines.join("\n"),
                                choices: vec!["Undo".to_owned(), "Dismiss".to_owned()],
                                pr_ref: String::new(),
                                session: None,
                            },
                            sessions,
                        );
                    }
                }
            }
        }
        map.insert(key.to_owned(), fields.clone());
        saved::save(&self.store, &map)?;
        let entry = Entry { record, fields };
        let archived = self.archived()?;
        self.tell(caller, pb::Change::Updated, &entry.shown_record(), &archived);
        Ok(self.todo_item(caller, &entry, &[], &archived))
    }
}

/// One queued session action.
struct Job {
    prompt_id: String,
    restore: bool,
    sessions: Vec<String>,
}

struct Done {
    prompt_id: String,
    result: Outcome,
}

enum Outcome {
    Archived {
        archived: Vec<String>,
        kept: Vec<(String, String)>,
    },
    Restored {
        restored: Vec<String>,
        failed: Vec<(String, String)>,
    },
}

async fn run_job(control: &dyn SessionControl, job: Job) -> Done {
    let result = if job.restore {
        let (mut restored, mut failed) = (Vec::new(), Vec::new());
        for session in job.sessions {
            match control.restore(&session).await {
                Ok(()) => restored.push(session),
                Err(status) => failed.push((session, status.message().to_owned())),
            }
        }
        Outcome::Restored { restored, failed }
    } else {
        let (mut archived, mut kept) = (Vec::new(), Vec::new());
        for session in job.sessions {
            match control.archive_if_clean(&session).await {
                Ok(ArchiveOutcome::Archived) => archived.push(session),
                Ok(ArchiveOutcome::Kept { reason }) => kept.push((session, reason)),
                Err(status) => kept.push((session, status.message().to_owned())),
            }
        }
        Outcome::Archived { archived, kept }
    };
    Done {
        prompt_id: job.prompt_id,
        result,
    }
}

fn sends_note(prompt: &LifecyclePromptRow, choice_lower: &str) -> bool {
    matches!(
        prompt.kind.as_str(),
        "pr_ci_failed" | "pr_comments" | "pr_conflicts" | "pr_stack_rebase"
    ) && choice_lower.starts_with("send")
}

/// The session a prompt's note goes to (the prompt's own, else the item's
/// owner, else its first linked session) and the note.
fn note_for(
    fields: &Fields,
    record: &BoardItemRecord,
    prompt: &LifecyclePromptRow,
) -> Result<(String, String), Status> {
    let session = prompt
        .session
        .clone()
        .or_else(|| fields.owner.as_ref().and_then(|owner| owner.session.clone()))
        .or_else(|| linked_sessions(record).into_iter().next())
        .ok_or_else(|| {
            Status::failed_precondition(
                "This to-do has no session to send that to. Link a session to it first (open the to-do and add one), then press the button again",
            )
        })?;
    let pr = &prompt.pr_ref;
    let text = match prompt.kind.as_str() {
        "pr_ci_failed" => {
            format!("CI failed on {pr}. Look at the failing checks, fix them and push.")
        }
        "pr_comments" => {
            let count = fields
                .watched_prs
                .iter()
                .find(|row| row.repo_pr() == *pr)
                .map_or(0, |row| row.unresolved_comments);
            let what = if count == 1 {
                "1 unresolved review comment".to_owned()
            } else {
                format!("{count} unresolved review comments")
            };
            format!("{pr} has {what}. Read each one, address it, reply and push.")
        }
        "pr_conflicts" => {
            format!("{pr} has merge conflicts with its base. Resolve the conflicts and push.")
        }
        _ => format!(
            "The pull request below {pr} in its stack was merged. Rebase {pr} onto the merged base and push."
        ),
    };
    Ok((session, text))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use humata2_daemon_ports::UserVia;
    use tonic::Code;

    use super::*;
    use crate::test_support::test_svc;
    use crate::todo::TodoApplied;
    use crate::todo_fields::{TodoOwnerRow, WatchedPrRow};
    use crate::ports::{TodoAutomation, TodoAutomationSource};

    struct Settings(TodoAutomation);

    impl TodoAutomationSource for Settings {
        fn settings(&self, _channel: Option<&str>) -> TodoAutomation {
            self.0
        }
    }

    /// Records every call; `dirty` sessions are kept with their reason.
    #[derive(Default)]
    struct FakeControl {
        calls: Mutex<Vec<String>>,
        dirty: HashMap<String, String>,
    }

    impl FakeControl {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[tonic::async_trait]
    impl SessionControl for FakeControl {
        async fn send_note(&self, session: &str, text: &str) -> Result<(), Status> {
            self.calls.lock().unwrap().push(format!("note {session}: {text}"));
            Ok(())
        }
        async fn archive_if_clean(&self, session: &str) -> Result<ArchiveOutcome, Status> {
            self.calls.lock().unwrap().push(format!("archive {session}"));
            Ok(match self.dirty.get(session) {
                Some(reason) => ArchiveOutcome::Kept {
                    reason: reason.clone(),
                },
                None => ArchiveOutcome::Archived,
            })
        }
        async fn restore(&self, session: &str) -> Result<(), Status> {
            self.calls.lock().unwrap().push(format!("restore {session}"));
            Ok(())
        }
    }

    fn user() -> Caller {
        Caller::User(UserVia::App)
    }

    fn now() -> i64 {
        1_800_000_000_000
    }

    struct Rig {
        svc: BoardSvc,
        control: Arc<FakeControl>,
        _dir: tempfile::TempDir,
    }

    fn rig(settings: TodoAutomation, dirty: &[(&str, &str)]) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let control = Arc::new(FakeControl {
            calls: Mutex::new(Vec::new()),
            dirty: dirty
                .iter()
                .map(|(s, r)| ((*s).to_owned(), (*r).to_owned()))
                .collect(),
        });
        let svc = test_svc(&dir)
            .with_automation(Arc::new(Settings(settings)))
            .with_session_control(control.clone());
        svc.store
            .put_channel(&humata2_store::ChannelRecord {
                id: "ch_pay".to_owned(),
                name: "Payments".to_owned(),
                folder: "payments".to_owned(),
                branch_prefix: None,
                default_agent: String::new(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 1,
            })
            .unwrap();
        for id in ["se_1", "se_2"] {
            svc.store
                .put_session(&humata2_store::SessionRecord {
                    id: id.to_owned(),
                    channel: "ch_pay".to_owned(),
                    machine: "local".to_owned(),
                    title: id.to_owned(),
                    agent: "claude".to_owned(),
                    folder: std::path::PathBuf::from("/tmp"),
                    created_unix_ms: 1,
                })
                .unwrap();
        }
        Rig {
            svc,
            control,
            _dir: dir,
        }
    }

    fn item_with(svc: &BoardSvc, sessions: &[&str]) -> String {
        let applied = svc
            .create_todo_in(
                &user(),
                &pb::CreateTodoRequest {
                    title: "Ship it".to_owned(),
                    channel: Some(pb::ChannelId {
                        value: "ch_pay".to_owned(),
                    }),
                    sessions: sessions
                        .iter()
                        .map(|s| pb::SessionId {
                            value: (*s).to_owned(),
                        })
                        .collect(),
                    ..Default::default()
                },
                false,
                now(),
            )
            .unwrap();
        match applied {
            TodoApplied::Done(done) => done.item.unwrap().item.unwrap().id.unwrap().value,
            TodoApplied::Ask(..) => panic!("asked"),
        }
    }

    async fn patch(svc: &BoardSvc, key: &str, patch: pb::TodoPatch) -> pb::WriteTodoResponse {
        let request = pb::UpdateTodoRequest {
            id: Some(pb::BoardItemId {
                value: key.to_owned(),
            }),
            patch: Some(patch),
            ..Default::default()
        };
        let applied = svc.update_todo_in(&user(), &request, false, now()).unwrap();
        match applied {
            TodoApplied::Done(done) => svc.settle_response(&user(), *done, now()).await.unwrap(),
            TodoApplied::Ask(..) => panic!("asked"),
        }
    }

    fn close_patch() -> pb::TodoPatch {
        pb::TodoPatch {
            state: Some(pb::TodoState::Closed as i32),
            ..Default::default()
        }
    }

    fn fields(svc: &BoardSvc, key: &str) -> Fields {
        saved::load(&svc.store)
            .unwrap()
            .get(key)
            .cloned()
            .unwrap_or_default()
    }

    fn kinds(svc: &BoardSvc, key: &str) -> Vec<String> {
        fields(svc, key)
            .lifecycle_prompts
            .iter()
            .map(|p| p.kind.clone())
            .collect()
    }

    fn state_of(svc: &BoardSvc, key: &str) -> State {
        let record = svc.store.board_item(key).unwrap().unwrap();
        fields(svc, key).state(record.column)
    }

    async fn resolve(
        svc: &BoardSvc,
        key: &str,
        prompt: &str,
        choice: &str,
    ) -> Result<pb::WriteTodoResponse, Status> {
        svc.resolve_prompt_acting(
            &user(),
            pb::ResolveLifecyclePromptRequest {
                idempotency_key: None,
                id: Some(pb::BoardItemId {
                    value: key.to_owned(),
                }),
                prompt_id: prompt.to_owned(),
                choice: choice.to_owned(),
            },
            now(),
        )
        .await
    }

    fn merged_pr_patch() -> pb::TodoPatch {
        pb::TodoPatch {
            watched_prs_add: vec![pb::WatchedPr {
                pr: Some(pb::PullRequestRef {
                    repo: Some(pb::RepoRef {
                        owner: "acme".into(),
                        name: "api".into(),
                    }),
                    number: 7,
                    note: String::new(),
                }),
                title: "Add it".into(),
                merged: true,
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn auto_all() -> TodoAutomation {
        TodoAutomation {
            on_pr_merged: SyncMode::Auto,
            on_todo_closed_archive_sessions: SyncMode::Auto,
            ..TodoAutomation::default()
        }
    }

    #[tokio::test]
    async fn merged_pr_under_ask_prompts_and_leaves_the_item_open() {
        let rig = rig(TodoAutomation::default(), &[]);
        let key = item_with(&rig.svc, &[]);
        patch(&rig.svc, &key, merged_pr_patch()).await;
        assert_eq!(kinds(&rig.svc, &key), ["pr_merged"]);
        assert!(!state_of(&rig.svc, &key).is_terminal());
    }

    #[tokio::test]
    async fn merged_pr_under_auto_closes_the_item_with_no_prompt() {
        let rig = rig(
            TodoAutomation {
                on_pr_merged: SyncMode::Auto,
                ..TodoAutomation::default()
            },
            &[],
        );
        let key = item_with(&rig.svc, &[]);
        patch(&rig.svc, &key, merged_pr_patch()).await;
        assert!(kinds(&rig.svc, &key).is_empty());
        assert_eq!(state_of(&rig.svc, &key), State::Closed);
        let record = rig.svc.store.board_item(&key).unwrap().unwrap();
        assert_eq!(record.column, saved::DONE_COLUMN);
        assert!(
            fields(&rig.svc, &key)
                .extra_history
                .iter()
                .any(|row| row.change.contains("automatically"))
        );
    }

    #[tokio::test]
    async fn closing_under_ask_offers_the_archive_and_archives_nothing() {
        let rig = rig(TodoAutomation::default(), &[]);
        let key = item_with(&rig.svc, &["se_1", "se_2"]);
        patch(&rig.svc, &key, close_patch()).await;
        assert_eq!(kinds(&rig.svc, &key), ["close_archive_sessions"]);
        assert!(rig.control.calls().is_empty());
    }

    #[tokio::test]
    async fn choosing_archive_archives_the_clean_one_reports_the_kept_one_and_offers_undo() {
        let rig = rig(TodoAutomation::default(), &[("se_2", "has unpushed commits")]);
        let key = item_with(&rig.svc, &["se_1", "se_2"]);
        patch(&rig.svc, &key, close_patch()).await;
        let prompt = fields(&rig.svc, &key).lifecycle_prompts[0].id.clone();

        let response = resolve(&rig.svc, &key, &prompt, "Archive sessions").await.unwrap();
        let shown = response.item.unwrap().fields.unwrap().lifecycle_prompts;
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].kind, "sessions_archived");
        assert!(shown[0].choices.contains(&"Undo".to_owned()));
        assert!(shown[0].detail.contains("kept: se_2 has unpushed commits"));
        let calls = rig.control.calls();
        assert!(calls.contains(&"archive se_1".to_owned()));
        assert!(calls.contains(&"archive se_2".to_owned()));
        assert_eq!(
            fields(&rig.svc, &key).prompt_sessions[&shown[0].id],
            ["se_1"]
        );
    }

    #[tokio::test]
    async fn undo_restores_only_the_archived_sessions() {
        let rig = rig(TodoAutomation::default(), &[("se_2", "has unpushed commits")]);
        let key = item_with(&rig.svc, &["se_1", "se_2"]);
        patch(&rig.svc, &key, close_patch()).await;
        let ask = fields(&rig.svc, &key).lifecycle_prompts[0].id.clone();
        resolve(&rig.svc, &key, &ask, "Archive sessions").await.unwrap();
        let undo = fields(&rig.svc, &key).lifecycle_prompts[0].id.clone();

        let response = resolve(&rig.svc, &key, &undo, "Undo").await.unwrap();
        assert!(response.item.unwrap().fields.unwrap().lifecycle_prompts.is_empty());
        let restores: Vec<String> = rig
            .control
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("restore"))
            .collect();
        assert_eq!(restores, ["restore se_1"]);
    }

    #[tokio::test]
    async fn closing_under_auto_archives_and_offers_undo() {
        let rig = rig(auto_all(), &[]);
        let key = item_with(&rig.svc, &["se_1", "se_2"]);
        let response = patch(&rig.svc, &key, close_patch()).await;
        let shown = response.item.unwrap().fields.unwrap().lifecycle_prompts;
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].kind, "sessions_archived");
        assert_eq!(
            rig.control.calls(),
            ["archive se_1".to_owned(), "archive se_2".to_owned()]
        );
    }

    #[tokio::test]
    async fn closing_without_a_session_control_does_nothing() {
        let rig = rig(TodoAutomation::default(), &[]);
        let key = item_with(&rig.svc, &["se_1"]);
        // The same store, but a service built with no session control.
        let svc = BoardSvc {
            sessions_control: None,
            ..rig.svc.clone()
        };
        patch(&svc, &key, close_patch()).await;
        assert!(kinds(&svc, &key).is_empty());
    }

    fn comments_prompt(session: Option<&str>) -> LifecyclePromptRow {
        LifecyclePromptRow {
            id: "pr-comments:acme/api#7".to_owned(),
            kind: "pr_comments".to_owned(),
            title: "2 unresolved review comments".to_owned(),
            detail: String::new(),
            choices: vec!["Send comments to session".to_owned(), "Dismiss".to_owned()],
            pr_ref: "acme/api#7".to_owned(),
            session: session.map(str::to_owned),
        }
    }

    fn put_comments_prompt(svc: &BoardSvc, key: &str, owner: Option<&str>) {
        let mut map = saved::load(&svc.store).unwrap();
        let fields = map.entry(key.to_owned()).or_default();
        fields.lifecycle_prompts.push(comments_prompt(None));
        fields.watched_prs.push(WatchedPrRow {
            owner: "acme".into(),
            name: "api".into(),
            number: 7,
            unresolved_comments: 2,
            ..WatchedPrRow::default()
        });
        fields.owner = owner.map(|session| TodoOwnerRow {
            kind: humata2_todo::OwnerKind::Agent,
            label: "claude".to_owned(),
            session: Some(session.to_owned()),
        });
        saved::save(&svc.store, &map).unwrap();
    }

    #[tokio::test]
    async fn send_comments_goes_once_to_the_owner_session() {
        let rig = rig(TodoAutomation::default(), &[]);
        let key = item_with(&rig.svc, &["se_1", "se_2"]);
        put_comments_prompt(&rig.svc, &key, Some("se_2"));
        resolve(&rig.svc, &key, "pr-comments:acme/api#7", "Send comments to session")
            .await
            .unwrap();
        let calls = rig.control.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].starts_with("note se_2: "));
        assert!(kinds(&rig.svc, &key).is_empty());
    }

    #[tokio::test]
    async fn send_with_no_session_fails_and_keeps_the_prompt() {
        let rig = rig(TodoAutomation::default(), &[]);
        let key = item_with(&rig.svc, &[]);
        put_comments_prompt(&rig.svc, &key, None);
        let error = resolve(&rig.svc, &key, "pr-comments:acme/api#7", "Send comments to session")
            .await
            .unwrap_err();
        assert_eq!(error.code(), Code::FailedPrecondition);
        assert_eq!(kinds(&rig.svc, &key), ["pr_comments"]);
        assert!(rig.control.calls().is_empty());
    }
}
