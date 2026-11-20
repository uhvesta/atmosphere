//! The watched-PR watcher (TODO2WIRE gap 4): reads each watched pull request
//! from GitHub gently, keeps the item's rows current, and raises a quiet
//! prompt (or wakes the item's session, when Settings say so) once per change.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use humata2_daemon_ports::{Caller, UserVia};
use humata2_rpc_kit::{blocking, lock, now_ms, say};
use tokio::sync::Notify;
use tonic::Status;

use crate::pb;
use crate::pr_watch::{compute_close_readiness, evaluate_watched_pr};
use crate::ports::{PrStatus, PrStatusError};
use crate::todo_fields::{self as saved, TodoHistoryRow};
use crate::BoardSvc;

/// How long to wait after a rate limit that doesn't say how long.
const UNKNOWN_LIMIT_WAIT: Duration = Duration::from_secs(15 * 60);

/// What was last signalled for each watched pull request, keyed by its
/// `owner/name#number`, and how to wake the watcher.
#[derive(Default)]
pub(crate) struct WatchState {
    /// The prompt ids each pull request last called for. A signal that goes
    /// away and comes back is new again; one that stays is not repeated.
    pub(crate) signalled: HashMap<String, Vec<String>>,
    wake: Option<Arc<Notify>>,
}

/// How one refresh ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refresh {
    Done,
    /// GitHub is limiting requests; ask again after this many seconds, when
    /// it said how many.
    RateLimited { wait_seconds: Option<u64> },
}

/// An item with pull requests to check.
struct Watched {
    key: String,
    prs: Vec<(String, String, u32)>,
}

/// A note to type into a session.
struct Wake {
    session: String,
    text: String,
}

impl BoardSvc {
    /// Whether any open item watches a pull request.
    pub fn has_watched_prs(&self) -> Result<bool, Status> {
        Ok(!self.watched_items()?.is_empty())
    }

    /// Wakes the watcher, so a pull request just added is read at once.
    pub(crate) fn poke_pr_watcher(&self) {
        if let Some(wake) = &lock(&self.pr_watch_state).wake {
            wake.notify_one();
        }
    }

    /// Checks the watcher every `every`, when a pull request is watched, and
    /// whenever `wake` is notified. After a rate limit it waits as long as
    /// GitHub asked.
    pub fn spawn_pr_watcher(
        self,
        every: Duration,
        wake: Arc<Notify>,
    ) -> tokio::task::JoinHandle<()> {
        lock(&self.pr_watch_state).wake = Some(wake.clone());
        tokio::spawn(async move {
            let mut pause = Duration::ZERO;
            loop {
                tokio::select! {
                    () = tokio::time::sleep(pause) => {}
                    () = wake.notified() => {}
                }
                pause = every;
                let me = self.clone();
                if !matches!(blocking(move || me.has_watched_prs()).await, Ok(Ok(true))) {
                    continue;
                }
                match self.refresh_watched_prs(now_ms()).await {
                    Ok(Refresh::Done) => {}
                    Ok(Refresh::RateLimited { wait_seconds }) => {
                        tokio::time::sleep(
                            wait_seconds.map_or(UNKNOWN_LIMIT_WAIT, Duration::from_secs),
                        )
                        .await;
                    }
                    Err(status) => say!(
                        "humatad2: watched pull requests weren't checked: {}",
                        status.message()
                    ),
                }
            }
        })
    }

    fn watched_items(&self) -> Result<Vec<Watched>, Status> {
        let entries = self.entries(&Caller::User(UserVia::App))?;
        Ok(entries
            .into_iter()
            .filter(|entry| {
                !entry.fields.watched_prs.is_empty() && !entry.state().is_terminal()
            })
            .map(|entry| Watched {
                key: entry.record.key.clone(),
                prs: entry
                    .fields
                    .watched_prs
                    .iter()
                    .map(|pr| (pr.owner.clone(), pr.name.clone(), pr.number))
                    .collect(),
            })
            .collect())
    }

    /// Reads every watched pull request once and applies what changed. Stops
    /// at a rate limit and returns the wait; any other failure leaves that
    /// pull request as it was.
    pub(crate) async fn refresh_watched_prs(&self, now: i64) -> Result<Refresh, Status> {
        let Some(port) = self.pr_status.clone() else {
            return Ok(Refresh::Done);
        };
        let me = self.clone();
        let items = blocking(move || me.watched_items()).await??;
        let mut first_error: Option<PrStatusError> = None;
        for item in items {
            let mut reads = Vec::new();
            for (owner, name, number) in &item.prs {
                match port.fetch(owner, name, *number).await {
                    Ok(status) => reads.push((owner.clone(), name.clone(), *number, status)),
                    Err(PrStatusError::RateLimited { wait_seconds }) => {
                        return Ok(Refresh::RateLimited { wait_seconds });
                    }
                    Err(error) => {
                        first_error.get_or_insert(error);
                    }
                }
            }
            if reads.is_empty() {
                continue;
            }
            let me = self.clone();
            let key = item.key.clone();
            let wakes = blocking(move || me.apply_pr_reads(&key, reads, now)).await??;
            self.settle_key(&Caller::User(UserVia::App), &item.key, now).await?;
            let Some(control) = &self.sessions_control else {
                continue;
            };
            for wake in wakes {
                if let Err(status) = control.send_note(&wake.session, &wake.text).await {
                    say!(
                        "humatad2: couldn't tell session {} about its pull request: {}",
                        wake.session,
                        status.message()
                    );
                }
            }
        }
        if let Some(error) = first_error {
            say!("humatad2: {error}");
        }
        Ok(Refresh::Done)
    }

    /// Writes what `reads` found onto item `key`, under the write lock, and
    /// returns the notes to send its session.
    fn apply_pr_reads(
        &self,
        key: &str,
        reads: Vec<(String, String, u32, PrStatus)>,
        now: i64,
    ) -> Result<Vec<Wake>, Status> {
        let user = Caller::User(UserVia::App);
        let _write = lock(&self.writes);
        let mut record = self.load(&user, key, false)?;
        let mut map = saved::load(&self.store)?;
        let mut fields = map.get(key).cloned().unwrap_or_default();
        if fields.state(record.column).is_terminal() {
            return Ok(Vec::new());
        }
        let channel = fields
            .channel
            .clone()
            .unwrap_or_else(|| record.channel.clone());
        let automation = self.automation_for((!channel.is_empty()).then_some(channel.as_str()));
        let session = fields
            .owner
            .as_ref()
            .and_then(|owner| owner.session.clone())
            .or_else(|| record.sessions.first().cloned())
            .filter(|_| self.sessions_control.is_some());

        let mut changed = false;
        let mut signals: Vec<(String, Vec<String>)> = Vec::new();
        let mut wakes = Vec::new();
        let mut merged_prompts = Vec::new();
        let mut history: Vec<(String, String)> = Vec::new();
        for (owner, name, number, status) in reads {
            let Some(at) = fields
                .watched_prs
                .iter()
                .position(|pr| pr.owner == owner && pr.name == name && pr.number == number)
            else {
                continue;
            };
            let pr_ref = fields.watched_prs[at].repo_pr();
            // With nothing signalled yet (a restart), what the stored row
            // already called for counts as signalled.
            let before = lock(&self.pr_watch_state)
                .signalled
                .get(&pr_ref)
                .cloned()
                .unwrap_or_else(|| {
                    evaluate_watched_pr(&fields.watched_prs[at])
                        .into_iter()
                        .map(|prompt| prompt.id)
                        .collect()
                });
            let row = &mut fields.watched_prs[at];
            let was = row.clone();
            if !status.title.is_empty() {
                row.title = status.title;
            }
            row.merged = status.merged;
            row.closed_without_merge = status.closed_without_merge;
            row.ci_state = status.ci_state;
            row.unresolved_comments = status.unresolved_comments;
            row.has_conflicts = status.has_conflicts;
            row.upstream_merged = status.upstream_merged;
            changed |= *row != was;
            let row = row.clone();

            let prompts = evaluate_watched_pr(&row);
            signals.push((
                pr_ref.clone(),
                prompts.iter().map(|prompt| prompt.id.clone()).collect(),
            ));
            for prompt in prompts {
                if before.contains(&prompt.id) {
                    continue;
                }
                let told = match (prompt.kind.as_str(), &session) {
                    ("pr_ci_failed", Some(_)) if automation.auto_wake_on_ci_fail => {
                        Some((format!("CI failed on {pr_ref}"), format!(
                            "CI failed on {pr_ref}. Look at the failing checks and fix them."
                        )))
                    }
                    ("pr_comments", Some(_)) if automation.auto_wake_on_review_comments => {
                        let count = row.unresolved_comments;
                        let word = if count == 1 { "comment" } else { "comments" };
                        Some((
                            format!("{count} unresolved review {word} on {pr_ref}"),
                            format!(
                                "{count} unresolved review {word} on {pr_ref}. Read them and address each one."
                            ),
                        ))
                    }
                    _ => None,
                };
                if let (Some((what, text)), Some(session)) = (told, &session) {
                    history.push((session.clone(), what));
                    wakes.push(Wake {
                        session: session.clone(),
                        text,
                    });
                } else if prompt.kind == "pr_merged" {
                    merged_prompts.push(prompt);
                } else if !fields
                    .lifecycle_prompts
                    .iter()
                    .any(|existing| existing.id == prompt.id)
                {
                    fields.lifecycle_prompts.push(prompt);
                    changed = true;
                }
            }
        }

        if !merged_prompts.is_empty() {
            let state = fields.state(record.column);
            let revision = record.revision + fields.extra_revisions + 1;
            let outcome = self.apply_pr_prompts(
                &mut fields,
                state,
                record.column,
                &channel,
                merged_prompts,
                now,
                "",
                revision,
            );
            changed = true;
            if let Some(column) = outcome.close_column {
                if column != record.column {
                    record = self.move_to_column(&record, column, "", now)?;
                }
                let linked = crate::automation::linked_sessions(&record);
                self.after_close(
                    &mut fields,
                    key,
                    state,
                    humata2_todo::State::Closed,
                    &linked,
                    &channel,
                    now,
                    "",
                    revision,
                )?;
            }
        }

        if changed || !history.is_empty() {
            let commits = fields.close_readiness.as_ref().map_or(0, |cr| cr.commits);
            fields.close_readiness = Some(compute_close_readiness(
                &fields.steps,
                &fields.watched_prs,
                commits,
            ));
            fields.extra_revisions += 1;
            let revision = record.revision + fields.extra_revisions;
            for (session, what) in history {
                fields.extra_history.push(TodoHistoryRow {
                    at_unix_ms: now,
                    by: String::new(),
                    kind: pb::HistoryKind::PrSynced as i32,
                    field: "watched_prs".to_owned(),
                    before: String::new(),
                    after: what.clone(),
                    change: format!("{what}; woke session {session}"),
                    revision,
                });
            }
            map.insert(key.to_owned(), fields);
            saved::save(&self.store, &map)?;
            let archived = self.archived()?;
            self.tell(&user, pb::Change::Updated, &record, &archived);
        }
        lock(&self.pr_watch_state).signalled.extend(signals);
        Ok(wakes)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use humata2_todo::{OwnerKind, State};

    use super::*;
    use crate::ports::{
        ArchiveOutcome, PrStatusPort, SessionControl, SyncMode, TodoAutomation,
        TodoAutomationSource,
    };
    use crate::test_support::test_svc;
    use crate::todo::TodoApplied;
    use crate::todo_fields::TodoOwnerRow;

    /// Answers each fetch from the front of a queue, repeating the last.
    struct FakeGithub {
        answers: Mutex<VecDeque<Result<PrStatus, PrStatusError>>>,
        last: Mutex<PrStatus>,
    }

    impl FakeGithub {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::default(),
                last: Mutex::default(),
            })
        }

        fn set(&self, status: PrStatus) {
            *lock(&self.last) = status;
        }

        fn fail_next(&self, error: PrStatusError) {
            lock(&self.answers).push_back(Err(error));
        }
    }

    #[tonic::async_trait]
    impl PrStatusPort for FakeGithub {
        async fn fetch(&self, _: &str, _: &str, _: u32) -> Result<PrStatus, PrStatusError> {
            if let Some(answer) = lock(&self.answers).pop_front() {
                return answer;
            }
            Ok(lock(&self.last).clone())
        }
    }

    #[derive(Default)]
    struct FakeSessions {
        notes: Mutex<Vec<(String, String)>>,
    }

    #[tonic::async_trait]
    impl SessionControl for FakeSessions {
        async fn send_note(&self, session: &str, text: &str) -> Result<(), Status> {
            lock(&self.notes).push((session.to_owned(), text.to_owned()));
            Ok(())
        }
        async fn archive_if_clean(&self, _: &str) -> Result<ArchiveOutcome, Status> {
            Ok(ArchiveOutcome::Archived)
        }
        async fn restore(&self, _: &str) -> Result<(), Status> {
            Ok(())
        }
    }

    struct Settings(TodoAutomation);

    impl TodoAutomationSource for Settings {
        fn settings(&self, _: Option<&str>) -> TodoAutomation {
            self.0
        }
    }

    fn failing() -> PrStatus {
        PrStatus {
            title: "Add watcher".into(),
            ci_state: "failing".into(),
            ..PrStatus::default()
        }
    }

    fn passing() -> PrStatus {
        PrStatus {
            ci_state: "passing".into(),
            ..failing()
        }
    }

    struct Rig {
        _dir: tempfile::TempDir,
        svc: BoardSvc,
        github: Arc<FakeGithub>,
        sessions: Arc<FakeSessions>,
        key: String,
    }

    /// An item watching acme/api#7, with the given automation settings and,
    /// when `owned`, an agent session that owns it.
    fn rig(automation: TodoAutomation, owned: bool) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let github = FakeGithub::new();
        let sessions = Arc::new(FakeSessions::default());
        let svc = test_svc(&dir)
            .with_pr_status(github.clone())
            .with_session_control(sessions.clone())
            .with_automation(Arc::new(Settings(automation)));
        let user = Caller::User(UserVia::App);
        let TodoApplied::Done(created) = svc
            .create_todo_in(
                &user,
                &pb::CreateTodoRequest {
                    title: "Ship the watcher".into(),
                    ..Default::default()
                },
                false,
                1,
            )
            .unwrap()
        else {
            panic!("asked where it should have applied");
        };
        let key = created
            .item
            .unwrap()
            .item
            .unwrap()
            .id
            .unwrap()
            .value;
        svc.update_todo_in(
            &user,
            &pb::UpdateTodoRequest {
                id: Some(pb::BoardItemId { value: key.clone() }),
                patch: Some(pb::TodoPatch {
                    watched_prs_add: vec![pb::WatchedPr {
                        pr: Some(pb::PullRequestRef {
                            repo: Some(pb::RepoRef {
                                owner: "acme".into(),
                                name: "api".into(),
                            }),
                            number: 7,
                            note: String::new(),
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            },
            false,
            2,
        )
        .unwrap();
        if owned {
            let _write = lock(&svc.writes);
            let mut map = saved::load(&svc.store).unwrap();
            map.entry(key.clone()).or_default().owner = Some(TodoOwnerRow {
                kind: OwnerKind::Agent,
                label: "claude".into(),
                session: Some("se_1".into()),
            });
            saved::save(&svc.store, &map).unwrap();
        }
        Rig {
            _dir: dir,
            svc,
            github,
            sessions,
            key,
        }
    }

    impl Rig {
        fn fields(&self) -> saved::Fields {
            saved::load(&self.svc.store)
                .unwrap()
                .remove(&self.key)
                .unwrap_or_default()
        }

        fn prompt_kinds(&self) -> Vec<String> {
            self.fields()
                .lifecycle_prompts
                .into_iter()
                .map(|prompt| prompt.kind)
                .collect()
        }

        fn dismiss_prompts(&self) {
            let _write = lock(&self.svc.writes);
            let mut map = saved::load(&self.svc.store).unwrap();
            map.entry(self.key.clone()).or_default().lifecycle_prompts.clear();
            saved::save(&self.svc.store, &map).unwrap();
        }

        async fn refresh(&self) -> Refresh {
            self.svc.refresh_watched_prs(10).await.unwrap()
        }
    }

    #[tokio::test]
    async fn failing_ci_adds_its_prompt_once() {
        let rig = rig(TodoAutomation::default(), false);
        rig.github.set(failing());
        assert_eq!(rig.refresh().await, Refresh::Done);
        assert_eq!(rig.prompt_kinds(), vec!["pr_ci_failed"]);
        assert_eq!(rig.fields().watched_prs[0].ci_state, "failing");
        let revision = rig.fields().extra_revisions;

        rig.refresh().await;
        assert_eq!(rig.prompt_kinds(), vec!["pr_ci_failed"]);
        assert_eq!(rig.fields().extra_revisions, revision, "nothing changed, nothing written");
    }

    #[tokio::test]
    async fn a_dismissed_prompt_stays_gone_until_the_signal_goes_and_comes_back() {
        let rig = rig(TodoAutomation::default(), false);
        rig.github.set(failing());
        rig.refresh().await;
        rig.dismiss_prompts();
        rig.refresh().await;
        assert!(rig.prompt_kinds().is_empty(), "same signal, no new prompt");

        rig.github.set(passing());
        rig.refresh().await;
        assert!(rig.prompt_kinds().is_empty());
        rig.github.set(failing());
        rig.refresh().await;
        assert_eq!(rig.prompt_kinds(), vec!["pr_ci_failed"]);
    }

    #[tokio::test]
    async fn auto_wake_notes_the_owner_session_instead_of_prompting() {
        let automation = TodoAutomation {
            auto_wake_on_ci_fail: true,
            ..TodoAutomation::default()
        };
        let rig = rig(automation, true);
        rig.github.set(failing());
        rig.refresh().await;
        rig.refresh().await;

        let notes = lock(&rig.sessions.notes).clone();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].0, "se_1");
        assert!(notes[0].1.contains("acme/api#7"));
        assert!(rig.prompt_kinds().is_empty());
        let history = rig.fields().extra_history;
        assert!(history.iter().any(|row| row.kind == pb::HistoryKind::PrSynced as i32));
    }

    #[tokio::test]
    async fn auto_wake_off_leaves_the_session_alone() {
        let rig = rig(TodoAutomation::default(), true);
        rig.github.set(failing());
        rig.refresh().await;
        assert!(lock(&rig.sessions.notes).is_empty());
        assert_eq!(rig.prompt_kinds(), vec!["pr_ci_failed"]);
    }

    #[tokio::test]
    async fn review_comments_wake_the_session_only_when_that_switch_is_on() {
        let automation = TodoAutomation {
            auto_wake_on_review_comments: true,
            ..TodoAutomation::default()
        };
        let rig = rig(automation, true);
        rig.github.set(PrStatus {
            unresolved_comments: 3,
            ..passing()
        });
        rig.refresh().await;
        assert_eq!(lock(&rig.sessions.notes).len(), 1);
        assert!(rig.prompt_kinds().is_empty());
    }

    #[tokio::test]
    async fn a_merged_pull_request_prompts_to_close() {
        let rig = rig(TodoAutomation::default(), false);
        rig.github.set(PrStatus {
            merged: true,
            ..passing()
        });
        rig.refresh().await;
        assert_eq!(rig.prompt_kinds(), vec!["pr_merged"]);
        assert!(rig.fields().watched_prs[0].merged);
    }

    #[tokio::test]
    async fn a_merged_pull_request_closes_the_item_under_auto() {
        let automation = TodoAutomation {
            on_pr_merged: SyncMode::Auto,
            ..TodoAutomation::default()
        };
        let rig = rig(automation, false);
        rig.github.set(PrStatus {
            merged: true,
            ..passing()
        });
        rig.refresh().await;
        assert!(rig.prompt_kinds().is_empty());
        let record = rig.svc.store.board_item(&rig.key).unwrap().unwrap();
        assert_eq!(rig.fields().state(record.column), State::Closed);
    }

    #[tokio::test]
    async fn a_rate_limit_returns_the_wait_and_changes_nothing() {
        let rig = rig(TodoAutomation::default(), false);
        rig.github.set(failing());
        rig.github.fail_next(PrStatusError::RateLimited {
            wait_seconds: Some(90),
        });
        let before = rig.fields();
        assert_eq!(
            rig.refresh().await,
            Refresh::RateLimited {
                wait_seconds: Some(90)
            }
        );
        assert_eq!(rig.fields(), before);
    }

    #[tokio::test]
    async fn another_failure_keeps_the_last_state() {
        let rig = rig(TodoAutomation::default(), false);
        rig.github.fail_next(PrStatusError::Unreachable {
            first_line: "offline".into(),
        });
        let before = rig.fields();
        assert_eq!(rig.refresh().await, Refresh::Done);
        assert_eq!(rig.fields(), before);
    }

    #[tokio::test]
    async fn the_watcher_is_woken_by_a_poke() {
        let rig = rig(TodoAutomation::default(), false);
        assert!(rig.svc.has_watched_prs().unwrap());
        let wake = Arc::new(Notify::new());
        lock(&rig.svc.pr_watch_state).wake = Some(wake.clone());
        rig.svc.poke_pr_watcher();
        wake.notified().await;
    }
}
