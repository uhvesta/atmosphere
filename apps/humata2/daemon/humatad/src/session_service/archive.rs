//! PlanArchive, ArchiveSession and RestoreSession (MV-7..MV-9, MV-15,
//! LD-2, LD-4, D110, D172): the unpushed-work plan, archive with its undo
//! window, the delayed free of the session folder, restore, and finishing
//! pending archives after a restart.
//!
//! What this removes, plainly: after the undo window, only the session's
//! own folder, `<root>/<channel>/sessions/<session>`. Not the channel, not
//! `shared/`, not the main clones. The free step canonicalizes the path and
//! refuses any path that is not a direct child of the channel's `sessions/`
//! folder under the layout root.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use humata2_access::{Capability, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{GitError, RepoWork, handover_file, repo_work};
use humata2_store::{
    AgentState, ArchiveRow, ArchiveState, SessionDetail, Store, StoreError,
};
use pb::invalid_field::Problem;
use prost::Message;
use tonic::{Code, Status};

use super::fork::{not_built_part, stop_point};
use super::lineage::{Lineage, publish_family};
use super::push::{PullRequestText, RepoPush, run_pushes, step_rows};
use super::{Loaded, SessionSvc, busy, now_ms, run_resume, start_error, turns, watch};
use crate::channel_service::{Gated, blocking, error_of, failed, timestamp_ms};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;

const RESTORE_SESSION: &str = "humata.v1.SessionService.RestoreSession";
/// How long the clones wait after an archive (MV-15, D110).
const UNDO_WINDOW_MS: i64 = 10_000;
/// How long a plan's token can be used.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, thiserror::Error)]
enum ArchiveError {
    #[error("session {session} is already archived")]
    AlreadyArchived { session: String },
    #[error("session {session} is not archived")]
    NotArchived { session: String },
    #[error("session {session} has work that isn't pushed: {work}")]
    HasWork { session: String, work: String },
    #[error(
        "the plan given is not one made for session {session}, or it was already used or has expired"
    )]
    StalePlan { session: String },
    #[error("couldn't check {owner}/{name} for work that isn't pushed")]
    Work {
        owner: String,
        name: String,
        #[source]
        source: GitError,
    },
    #[error("couldn't read or save the archive record of session {session}")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
    #[error("{path} is not a folder directly under {sessions}, so Humata left it alone")]
    NotASessionFolder { path: PathBuf, sessions: PathBuf },
    #[error("couldn't look at {path}")]
    Look {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("couldn't delete {path}")]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(
        "session {session}'s channel {channel} is gone, so its folder can't be checked before it is deleted"
    )]
    NoChannel { session: String, channel: String },
    #[error("couldn't stop session {session}'s agent")]
    Stop {
        session: String,
        #[source]
        source: Box<Status>,
    },
}

/// What a plan token was made for: a token from one plan never passes for
/// another.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PlanKind {
    Archive,
    Push,
}

/// The session lifecycle's state in memory, one per daemon: plans made and
/// not yet used (token to kind, session and when), and one lock per session
/// so a free, a restore, a push and the end of an archive don't interleave.
#[derive(Default)]
pub(super) struct Lifecycle {
    plans: Mutex<HashMap<String, (PlanKind, String, Instant)>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl Lifecycle {
    pub(super) fn lock_of(&self, session: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(session.to_owned())
            .or_default()
            .clone()
    }

    pub(super) fn make_plan(&self, kind: PlanKind, session: &str) -> String {
        let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
        let mut plans = self.plans.lock().unwrap_or_else(PoisonError::into_inner);
        plans.retain(|_, (_, _, made)| made.elapsed() < PLAN_LIFETIME);
        plans.insert(token.clone(), (kind, session.to_owned(), Instant::now()));
        token
    }

    /// Whether `token` is a live `kind` plan for `session`.
    pub(super) fn plan_is_live(&self, kind: PlanKind, token: &str, session: &str) -> bool {
        self.plans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(token)
            .is_some_and(|(made_for, planned, made)| {
                *made_for == kind && planned == session && made.elapsed() < PLAN_LIFETIME
            })
    }

    /// Uses `token` up: true when it was a live `kind` plan for `session`.
    pub(super) fn use_plan(&self, kind: PlanKind, token: &str, session: &str) -> bool {
        let live = self.plan_is_live(kind, token, session);
        if live {
            self.plans
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(token);
        }
        live
    }
}

/// How long the clones wait after an archive. Test daemons read
/// `HUMATA2_ARCHIVE_UNDO_MS`; a release build always waits the 10 seconds.
fn undo_window_ms() -> i64 {
    #[cfg(feature = "admin")]
    if let Some(ms) = std::env::var("HUMATA2_ARCHIVE_UNDO_MS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
    {
        return ms.max(0);
    }
    UNDO_WINDOW_MS
}

pub(super) fn session_id(id: Option<pb::SessionId>) -> Result<String, Status> {
    id.map(|id| id.value)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| status::missing("id"))
}

/// A work-in-progress repository: owner, name, branch, where its clone is
/// and what it holds.
struct Work {
    owner: String,
    name: String,
    branch: String,
    path: PathBuf,
    /// None when the clone is not on disk.
    work: Option<RepoWork>,
}

/// Reads the work in each clone of `detail`. Blocking: it runs git.
/// Who else works in a session's folder, as a context-only fork works in
/// its parent's (D187, D214).
#[derive(Default)]
struct Sharers {
    /// Another session there isn't archived: the work in the folder is its.
    live: bool,
    /// Another session there isn't freed: the folder stays.
    unfreed: bool,
}

/// The other sessions of this machine whose folder is `session`'s.
fn sharers_of(store: &Store, session: &str) -> Result<Sharers, StoreError> {
    let mut sharers = Sharers::default();
    let Some(record) = store.session(session)? else {
        return Ok(sharers);
    };
    for other in store.sessions()? {
        if other.id == record.id || other.machine != record.machine || other.folder != record.folder {
            continue;
        }
        match store.archive(&other.id)? {
            None => {
                sharers.live = true;
                sharers.unfreed = true;
            }
            Some(row) if row.state != ArchiveState::Freed => sharers.unfreed = true,
            Some(_) => {}
        }
    }
    Ok(sharers)
}

/// The work in the session's clones. A session whose folder another live
/// session works in has none of its own: that work is the other's (D214).
fn works_of(
    store: &Store,
    session: &str,
    detail: Option<&SessionDetail>,
) -> Result<Vec<Work>, ArchiveError> {
    let shared = sharers_of(store, session)
        .map_err(|source| ArchiveError::Store {
            session: session.to_owned(),
            source,
        })?
        .live;
    let mut works = Vec::new();
    for repo in detail.iter().flat_map(|detail| detail.repos.iter()) {
        let work = if !shared && repo.path.join(".git").exists() {
            Some(repo_work(&repo.path).map_err(|source| ArchiveError::Work {
                owner: repo.owner.clone(),
                name: repo.name.clone(),
                source,
            })?)
        } else {
            None
        };
        works.push(Work {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
            branch: repo.branch.clone(),
            path: repo.path.clone(),
            work,
        });
    }
    Ok(works)
}

impl Work {
    fn has_work(&self) -> bool {
        self.work.as_ref().is_some_and(RepoWork::has_work)
    }

    /// What COMMIT_PUSH_PR_THEN_ARCHIVE does for this repository: commit
    /// what's there, push, and open a pull request titled with the session.
    fn push(&self, title: &str) -> RepoPush {
        RepoPush {
            owner: self.owner.clone(),
            name: self.name.clone(),
            branch: self.branch.clone(),
            path: self.path.clone(),
            commit_message: Some(format!("Archive: {title}")),
            pull_request: Some(PullRequestText {
                title: title.to_owned(),
                body: format!("{title}\n\nBranch: {}", self.branch),
            }),
        }
    }

    fn unpushed(&self) -> Option<pb::UnpushedRepo> {
        let work = self.work.as_ref().filter(|work| work.has_work())?;
        Some(pb::UnpushedRepo {
            repo: Some(pb::RepoRef {
                owner: self.owner.clone(),
                name: self.name.clone(),
            }),
            branch: self.branch.clone(),
            dirty_files: work.dirty_files,
            unpushed_commits: work.unpushed_commits,
            never_pushed: work.never_pushed,
        })
    }
}

/// What archiving kept of the session's clones (MV-7).
fn summary_of(works: &[Work], archived_unix_ms: i64) -> pb::ArchiveSummary {
    pb::ArchiveSummary {
        archived_at: Some(timestamp_ms(archived_unix_ms)),
        repos: works
            .iter()
            .map(|work| pb::archive_summary::Repo {
                repo: Some(pb::RepoRef {
                    owner: work.owner.clone(),
                    name: work.name.clone(),
                }),
                branch: work.branch.clone(),
                last_pushed_sha: work
                    .work
                    .as_ref()
                    .and_then(|work| work.last_pushed_sha.clone())
                    .unwrap_or_default(),
                files: work
                    .work
                    .iter()
                    .flat_map(|work| work.files.iter())
                    .map(|file| pb::FileChange {
                        path: file.path.clone(),
                        added: file.added,
                        removed: file.removed,
                    })
                    .collect(),
                unpushed_subjects: Vec::new(),
            })
            .collect(),
        machine_lost: false,
        lost: Vec::new(),
    }
}

/// What a session's unpushed work says, for the refusal.
fn describe(works: &[Work]) -> String {
    let parts: Vec<String> = works
        .iter()
        .filter_map(|work| {
            let found = work.work.as_ref().filter(|found| found.has_work())?;
            Some(format!(
                "{}/{} on {} has {} uncommitted file(s) and {} commit(s) not pushed",
                work.owner, work.name, work.branch, found.dirty_files, found.unpushed_commits
            ))
        })
        .collect();
    parts.join("; ")
}

/// Removes the session folder `folder` of a channel kept in
/// `<root>/<channel_folder>`, and nothing else. The path is canonicalized
/// and must be a direct child of the channel's `sessions/` folder, which
/// must itself be under the canonical `root`; any other path is refused and
/// nothing is removed. A folder that isn't there is already freed.
fn remove_session_folder(
    root: &Path,
    channel_folder: &str,
    folder: &Path,
) -> Result<(), ArchiveError> {
    let look = |path: &Path| {
        let path = path.to_owned();
        move |source| ArchiveError::Look { path, source }
    };
    let sessions = root.join(channel_folder).join("sessions");
    let sessions = match sessions.canonicalize() {
        Ok(sessions) => sessions,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(look(&sessions)(source)),
    };
    let root = root.canonicalize().map_err(look(root))?;
    let canonical = match folder.canonicalize() {
        Ok(canonical) => canonical,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(look(folder)(source)),
    };
    let refused = || ArchiveError::NotASessionFolder {
        path: folder.to_owned(),
        sessions: sessions.clone(),
    };
    if !sessions.starts_with(&root)
        || sessions == root
        || canonical.parent() != Some(sessions.as_path())
        || !canonical.is_dir()
    {
        return Err(refused());
    }
    std::fs::remove_dir_all(&canonical).map_err(|source| ArchiveError::Remove {
        path: canonical,
        source,
    })
}

/// Removes a file of the session's own in a folder it shares; one that
/// isn't there is already gone.
fn remove_own_file(path: &Path) -> Result<(), ArchiveError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ArchiveError::Remove {
            path: path.to_owned(),
            source,
        }),
    }
}

pub(super) fn failure_of(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: "try again".to_owned(),
        retry: None,
        kind: None,
    })
}

fn store_failure(status: &Status, session: &str) -> pb::Error {
    let mut failure = failure_of(status);
    failure.message = format!("session {session}: {}", failure.message);
    failure
}

impl SessionSvc {
    /// What archiving the session would lose, and the token ArchiveSession
    /// takes to delete that work. Changes nothing.
    pub(super) async fn plan_archive_of(
        &self,
        caller: &Caller,
        request: pb::PlanArchiveRequest,
        approved: bool,
    ) -> Result<Gated<pb::ArchivePlan>, Status> {
        Self::refuse_peer(caller)?;
        let id = session_id(request.id)?;
        let loaded = self.load(&id).await?;
        if let Some(asked) = self.gate_end(caller, &loaded, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some((machine, _)) = self.lives_elsewhere(&loaded.record) {
            return self.plan_archive_remote(&id, &machine).await.map(Gated::Done);
        }
        let lineage = self.read_lineage().await?;
        if lineage.is_archived(&id) {
            return Err(failed(
                Code::FailedPrecondition,
                &ArchiveError::AlreadyArchived { session: id },
                "restore it to bring it back; there is nothing more to archive",
                None,
            ));
        }
        let (store, key, detail) = (self.starter.store.clone(), id.clone(), loaded.detail.clone());
        let works = blocking(move || works_of(&store, &key, detail.as_ref())).await?.map_err(|error| {
            failed(
                Code::FailedPrecondition,
                &error,
                "fix the cause the message names, then plan the archive again",
                None,
            )
        })?;
        let unpushed: Vec<pb::UnpushedRepo> = works.iter().filter_map(Work::unpushed).collect();
        let choices = if unpushed.is_empty() {
            vec![pb::ArchiveChoice::Archive]
        } else {
            vec![
                pb::ArchiveChoice::CommitPushPrThenArchive,
                pb::ArchiveChoice::ArchiveAndDeleteWork,
            ]
        };
        Ok(Gated::Done(pb::ArchivePlan {
            id: Some(pb::SessionId { value: id.clone() }),
            forks_keep_running: u32::try_from(lineage.fork_count(&id)).unwrap_or(u32::MAX),
            unpushed,
            choices: choices.into_iter().map(|choice| choice as i32).collect(),
            unsubmitted_drafts: 0,
            token: Some(pb::PlanToken {
                value: self.lifecycle.make_plan(PlanKind::Archive, &id),
            }),
        }))
    }

    /// Records the archive and answers its operation. The agent stops, the
    /// summary is written and the clones go once the undo window closes;
    /// the operation does not wait for that.
    pub(super) async fn archive_session_of(
        &self,
        caller: &Caller,
        request: pb::ArchiveSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        Self::refuse_peer(caller)?;
        let id = session_id(request.id.clone())?;
        let loaded = self.load(&id).await?;
        let choice = match pb::ArchiveChoice::try_from(request.choice) {
            Ok(
                choice @ (pb::ArchiveChoice::Archive
                | pb::ArchiveChoice::ArchiveAndDeleteWork
                | pb::ArchiveChoice::CommitPushPrThenArchive),
            ) => choice,
            _ => {
                return Err(status::invalid_field(
                    "choice",
                    Problem::Missing,
                    "the request doesn't say how to archive",
                    "set choice to ARCHIVE, ARCHIVE_AND_DELETE_WORK or COMMIT_PUSH_PR_THEN_ARCHIVE, and send it again",
                ));
            }
        };
        if let Some(asked) = self.gate_end(caller, &loaded, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        if self.read_lineage().await?.is_archived(&id) {
            return Err(failed(
                Code::FailedPrecondition,
                &ArchiveError::AlreadyArchived { session: id },
                "restore it to bring it back; there is nothing more to archive",
                None,
            ));
        }
        if let Some((machine, _)) = self.lives_elsewhere(&loaded.record) {
            return self.archive_remote(caller, request, loaded, choice, machine).await;
        }
        match loaded.agent.as_ref().map(|agent| agent.state) {
            Some(AgentState::Launching) => return Err(busy(&id, "still starting")),
            Some(AgentState::Resuming) => return Err(busy(&id, "being resumed")),
            _ => {}
        }
        let token = request
            .plan
            .as_ref()
            .map(|plan| plan.value.clone())
            .unwrap_or_default();
        let stale = || {
            failed(
                Code::FailedPrecondition,
                &ArchiveError::StalePlan {
                    session: id.clone(),
                },
                "plan the archive again, read what it would lose, and send the new token",
                None,
            )
        };
        let deleting = choice == pb::ArchiveChoice::ArchiveAndDeleteWork;
        let pushing = choice == pb::ArchiveChoice::CommitPushPrThenArchive;
        // Both take the work the plan listed, so both need that plan.
        let spends_plan = deleting || pushing;
        if spends_plan && !self.lifecycle.plan_is_live(PlanKind::Archive, &token, &id) {
            return Err(stale());
        }
        let (store, key, detail) = (self.starter.store.clone(), id.clone(), loaded.detail.clone());
        let works = blocking(move || works_of(&store, &key, detail.as_ref()))
            .await?
            .map_err(|error| {
                failed(
                    Code::FailedPrecondition,
                    &error,
                    "fix the cause the message names, then archive again",
                    None,
                )
            })?;
        if !spends_plan && works.iter().any(Work::has_work) {
            return Err(failed(
                Code::FailedPrecondition,
                &ArchiveError::HasWork {
                    session: id.clone(),
                    work: describe(&works),
                },
                "commit and push the work yourself and archive again, or plan the archive and choose to archive and delete the work",
                None,
            ));
        }
        // Pushing sends the work, so it is asked for like a push, before the
        // plan is used up.
        let pushes: Vec<RepoPush> = if pushing {
            works
                .iter()
                .filter(|work| work.has_work())
                .map(|work| work.push(&loaded.record.title))
                .collect()
        } else {
            Vec::new()
        };
        if pushing {
            let targets = pushes
                .iter()
                .map(|push| {
                    humata2_access::RepoBranch {
                        repo: humata2_access::RepoRef::new(format!("{}/{}", push.owner, push.name)),
                        branch: push.branch.clone(),
                    }
                })
                .collect();
            if let Some(asked) = self.ask_for_pushes(caller, targets, approved)? {
                return Ok(Gated::Ask(asked));
            }
        }
        if spends_plan && !self.lifecycle.use_plan(PlanKind::Archive, &token, &id) {
            return Err(stale());
        }

        let mut steps = if pushing { step_rows(&pushes) } else { Vec::new() };
        steps.extend([
            ("stop".to_owned(), "Stopping the agent".to_owned()),
            ("summarize".to_owned(), "Writing what the session had".to_owned()),
            (
                "free_clones".to_owned(),
                "Deleting the clones when the undo window closes".to_owned(),
            ),
        ]);
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::ArchiveSession,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        if pushing {
            // The archive is recorded only once every push has gone through.
            tokio::spawn(self.clone().push_then_archive(
                progress,
                id,
                pushes,
                identity::actor(caller),
            ));
            return Ok(Gated::Done(operation));
        }
        let archived = now_ms();
        let row = ArchiveRow {
            session_id: id.clone(),
            archived_unix_ms: archived,
            state: ArchiveState::Archiving,
            summary: Vec::new(),
            free_after_unix_ms: archived.saturating_add(undo_window_ms()),
        };
        if let Err(refused) = self.save_archive(&row).await {
            progress.fail(store_failure(&refused, &id));
            return Err(refused);
        }
        // A kill here leaves the intent recorded and the agent running: the
        // restart finishes the archive.
        stop_point("archive.recorded");
        let by = identity::actor(caller);
        self.tell
            .session(&id, pb::Change::Updated, Some(by.clone()))
            .await;
        publish_family(&self.tell, &id, Some(by.clone())).await;
        tokio::spawn(self.clone().run_archive(progress, row, by));
        Ok(Gated::Done(operation))
    }

    /// Brings an archived session back: the Undo before the clones are
    /// freed, a restore from what was pushed after.
    pub(super) async fn restore_session_of(
        &self,
        caller: &Caller,
        request: pb::RestoreSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        Self::refuse_peer(caller)?;
        let id = session_id(request.id.clone())?;
        let loaded = self.load(&id).await?;
        let (store, key) = (self.starter.store.clone(), id.clone());
        let row = blocking(move || store.archive(&key))
            .await?
            .map_err(|error| status::store(error, &self.machine, "look up the archive"))?;
        if row.is_none() {
            return Err(failed(
                Code::FailedPrecondition,
                &ArchiveError::NotArchived { session: id },
                "only an archived session can be restored",
                None,
            ));
        }
        if self.lives_elsewhere(&loaded.record).is_none()
            && request
                .machine
                .as_ref()
                .is_some_and(|machine| !machine.value.is_empty() && machine.value != self.machine)
        {
            return Err(not_built_part(
                RESTORE_SESSION,
                "a session on another machine",
            ));
        }
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
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        if let Some((machine, _)) = self.lives_elsewhere(&loaded.record) {
            return self.restore_remote(caller, request, loaded, machine).await;
        }
        turns::refresh_hooks().await;

        let mut steps = Vec::new();
        for repo in loaded.detail.iter().flat_map(|detail| detail.repos.iter()) {
            steps.push((format!("clone:{}", repo.name), format!("Cloning {}", repo.name)));
        }
        steps.push((
            "import_conversation".to_owned(),
            "Bringing the conversation back".to_owned(),
        ));
        steps.push(("launch".to_owned(), "Starting the agent again".to_owned()));
        steps.push((
            "verify".to_owned(),
            "Checking it took up its conversation".to_owned(),
        ));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::RestoreSession,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        tokio::spawn(self.clone().run_restore(progress, id, identity::actor(caller)));
        Ok(Gated::Done(operation))
    }

    /// Finishes the archives a cut run left (`recover_archives`) once the
    /// start's reconcile and revive are done (`started` is set then), so an
    /// agent revive brought back is stopped. Never blocks the start.
    pub(crate) fn finish_archives_after(&self, started: Arc<crate::restart::Started>) {
        let me = self.clone();
        tokio::spawn(async move {
            started.wait().await;
            me.recover_archives().await;
        });
    }

    /// After a restart: an archive cut short is finished (the agent stopped,
    /// the summary written) and every archived session's clones are freed
    /// now (MV-15). Takes as long as the git reads do; the caller spawns it.
    pub(super) async fn recover_archives(&self) {
        let store = self.starter.store.clone();
        let rows = match blocking(move || store.archives()).await {
            Ok(Ok(rows)) => rows,
            Ok(Err(error)) => {
                crate::log::say!(
                    "humatad2: couldn't read the archives to finish them: {}",
                    status::chain(&error)
                );
                return;
            }
            Err(error) => {
                crate::log::say!("humatad2: couldn't finish the archives: {}", error.message());
                return;
            }
        };
        for row in rows
            .iter()
            .filter(|row| row.state == ArchiveState::Archiving)
        {
            let guard = self.lifecycle.lock_of(&row.session_id).lock_owned().await;
            if let Err(error) = self.finish_archive(row.clone()).await {
                crate::log::say!(
                    "humatad2: couldn't finish archiving session {}: {}",
                    row.session_id,
                    status::chain(&error)
                );
            }
            drop(guard);
        }
        let store = self.starter.store.clone();
        let Ok(Ok(rows)) = blocking(move || store.archives()).await else {
            return;
        };
        for row in rows
            .iter()
            .filter(|row| row.state == ArchiveState::Archived)
        {
            self.free_now(&row.session_id).await;
        }
    }

    /// The access rule for ending a session: its own agent may end it.
    fn gate_end(
        &self,
        caller: &Caller,
        loaded: &Loaded,
        approved: bool,
    ) -> Result<Option<Box<humata2_access::AccessRequest>>, Status> {
        let own =
            matches!(caller, Caller::Agent(agent) if agent.session.id.as_str() == loaded.record.id);
        self.gate(
            caller,
            if own {
                Capability::OwnSessionEnd
            } else {
                Capability::SessionStopOther
            },
            Target::Session(self.session_ref(&loaded.record)),
            approved,
        )
    }

    async fn read_lineage(&self) -> Result<Lineage, Status> {
        let store = self.starter.store.clone();
        blocking(move || Lineage::read(&store))
            .await?
            .map_err(|error| status::store(error, &self.machine, "read the archived sessions"))
    }

    async fn save_archive(&self, row: &ArchiveRow) -> Result<(), Status> {
        let (store, row) = (self.starter.store.clone(), row.clone());
        blocking(move || store.put_archive(&row))
            .await?
            .map_err(|error| status::store(error, &self.machine, "save the archive"))
    }

    /// Stops the session's agent as StopSession does.
    async fn stop_agent(&self, session: &str) -> Result<(), ArchiveError> {
        let stopped = |source: Status| ArchiveError::Stop {
            session: session.to_owned(),
            source: Box::new(source),
        };
        let (store, key) = (self.starter.store.clone(), session.to_owned());
        let agent = blocking(move || store.agent(&key))
            .await
            .map_err(stopped)?
            .map_err(|error| stopped(status::store(error, &self.machine, "read the agent")))?;
        let Some(mut agent) = agent else {
            return Ok(());
        };
        if !matches!(agent.state, AgentState::Running { .. }) {
            return Ok(());
        }
        // A holder already gone is as stopped as one that was asked to.
        if let Err(error) = humata2_holder::shutdown(&agent.launch.holder_socket).await {
            crate::log::say!(
                "humatad2: stopping session {session}'s terminal: {}",
                status::chain(&error)
            );
        }
        agent.state = AgentState::Stopped;
        agent.updated_unix_ms = now_ms();
        let store = self.starter.store.clone();
        // A stopped agent keeps no tab: resuming makes its tab again.
        let tab = agent.tab.clone();
        blocking(move || store.put_agent(&agent).and_then(|()| store.remove_tab(&tab)))
            .await
            .map_err(stopped)?
            .map_err(|error| stopped(status::store(error, &self.machine, "save that the agent stopped")))
    }

    /// COMMIT_PUSH_PR_THEN_ARCHIVE's work (MV-10): commit, push and open the
    /// pull request of each repository with work. Only when all of them go
    /// through is the archive recorded and carried on as ARCHIVE does. Any
    /// failure archives nothing: the agent keeps running, and the error says
    /// what to fix.
    async fn push_then_archive(
        self,
        progress: Progress,
        id: String,
        pushes: Vec<RepoPush>,
        by: pb::Actor,
    ) {
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        if let Err(stopped) = run_pushes(&progress, &pushes, "archive again").await {
            for step in ["stop", "summarize", "free_clones"] {
                progress.set_step(step, pb::StepState::Skipped, None);
            }
            progress.fail(stopped.into_error("Nothing was archived; the session keeps running."));
            drop(guard);
            // The branches that did get pushed changed the session.
            self.tell
                .session(&id, pb::Change::Updated, Some(by))
                .await;
            return;
        }
        let archived = now_ms();
        let row = ArchiveRow {
            session_id: id.clone(),
            archived_unix_ms: archived,
            state: ArchiveState::Archiving,
            summary: Vec::new(),
            free_after_unix_ms: archived.saturating_add(undo_window_ms()),
        };
        if let Err(refused) = self.save_archive(&row).await {
            progress.fail(store_failure(&refused, &id));
            return;
        }
        // A kill here leaves the intent recorded and the agent running: the
        // restart finishes the archive.
        stop_point("archive.recorded");
        drop(guard);
        self.tell
            .session(&id, pb::Change::Updated, Some(by.clone()))
            .await;
        publish_family(&self.tell, &id, Some(by.clone())).await;
        self.run_archive(progress, row, by).await;
    }

    /// The rest of an archive once it is recorded: stop, summarize, mark it
    /// archived, end the session's grants and requests, and set the free.
    async fn run_archive(self, progress: Progress, row: ArchiveRow, by: pb::Actor) {
        let id = row.session_id.clone();
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        progress.set_step("stop", pb::StepState::Running, None);
        progress.set_step("summarize", pb::StepState::Waiting, None);
        let finished = self.finish_archive(row.clone()).await;
        drop(guard);
        match finished {
            Ok(()) => {
                progress.set_step("stop", pb::StepState::Done, None);
                progress.set_step("summarize", pb::StepState::Done, None);
                progress.set_step("free_clones", pb::StepState::Done, None);
                progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                    value: id.clone(),
                })));
                self.schedule_free(&id, row.free_after_unix_ms);
            }
            Err(error) => {
                progress.set_step("summarize", pb::StepState::Failed, Some(status::chain(&error)));
                progress.fail(error_of(
                    &error,
                    "fix the cause the message names, then restart Humata on this machine to finish archiving, or restore the session to undo it",
                    None,
                ));
            }
        }
        self.tell
            .session(&id, pb::Change::Updated, Some(by.clone()))
            .await;
        publish_family(&self.tell, &id, Some(by)).await;
    }

    /// Stops the agent, writes the summary, marks the row archived and ends
    /// the session's grants. The caller holds the session's lock.
    async fn finish_archive(&self, row: ArchiveRow) -> Result<(), ArchiveError> {
        let id = row.session_id.clone();
        self.stop_agent(&id).await?;
        let summary = if row.summary.is_empty() {
            let (store, key) = (self.starter.store.clone(), id.clone());
            let archived = row.archived_unix_ms;
            let detail = blocking(move || store.session_detail(&key))
                .await
                .map_err(|source| ArchiveError::Stop {
                    session: id.clone(),
                    source: Box::new(source),
                })?
                .map_err(|source| ArchiveError::Store {
                    session: id.clone(),
                    source,
                })?;
            let (store, key) = (self.starter.store.clone(), id.clone());
            let works = blocking(move || works_of(&store, &key, detail.as_ref()))
                .await
                .map_err(|source| ArchiveError::Stop {
                    session: id.clone(),
                    source: Box::new(source),
                })??;
            summary_of(&works, archived).encode_to_vec()
        } else {
            row.summary.clone()
        };
        let archived = ArchiveRow {
            state: ArchiveState::Archived,
            summary,
            ..row
        };
        let (store, saving) = (self.starter.store.clone(), archived);
        blocking(move || store.put_archive(&saving))
            .await
            .map_err(|source| ArchiveError::Stop {
                session: id.clone(),
                source: Box::new(source),
            })?
            .map_err(|source| ArchiveError::Store {
                session: id.clone(),
                source,
            })?;
        // The session no longer waits on anyone (IN-22, TP-15).
        if let Err(error) = self.end_grants_of(&id).await {
            crate::log::say!(
                "humatad2: ending archived session {id}'s grants: {}",
                error.message()
            );
        }
        if let Err(error) = self
            .withdraw_requests_of(&id, pb::WithdrawReason::SessionArchived)
            .await
        {
            crate::log::say!(
                "humatad2: withdrawing archived session {id}'s requests: {}",
                error.message()
            );
        }
        Ok(())
    }

    /// Frees the session's folder when the undo window closes.
    fn schedule_free(&self, session: &str, free_after_unix_ms: i64) {
        let (me, session) = (self.clone(), session.to_owned());
        tokio::spawn(async move {
            let wait = free_after_unix_ms.saturating_sub(now_ms());
            if wait > 0 {
                tokio::time::sleep(Duration::from_millis(wait.unsigned_abs())).await;
            }
            me.free_now(&session).await;
        });
    }

    /// Frees an archived session's folder now, unless it was restored.
    async fn free_now(&self, session: &str) {
        if let Err(error) = self.free_folder(session).await {
            crate::log::say!(
                "humatad2: couldn't free archived session {session}'s folder: {}",
                status::chain(&error)
            );
        }
    }

    /// Removes the session's own folder and marks the archive freed, if the
    /// row is still archived: a restore in the window left no row.
    async fn free_folder(&self, session: &str) -> Result<(), ArchiveError> {
        let guard = self.lifecycle.lock_of(session).lock_owned().await;
        // Sessions that share a folder free it one at a time, so the last
        // one sees the others freed and removes it (D214).
        let (store, key) = (self.starter.store.clone(), session.to_owned());
        let folder = blocking(move || store.session(&key))
            .await
            .map_err(|source| ArchiveError::Stop {
                session: session.to_owned(),
                source: Box::new(source),
            })?
            .map_err(|source| ArchiveError::Store {
                session: session.to_owned(),
                source,
            })?
            .map(|record| record.folder);
        let folder_guard = match &folder {
            Some(folder) => Some(
                self.lifecycle
                    .lock_of(&format!("folder:{}", folder.display()))
                    .lock_owned()
                    .await,
            ),
            None => None,
        };
        let (store, key) = (self.starter.store.clone(), session.to_owned());
        let layout_root = self.starter.layout.root.clone();
        let here = self.machine.clone();
        let session_key = session.to_owned();
        let freed = blocking(move || -> Result<bool, ArchiveError> {
            let store_error = |source| ArchiveError::Store {
                session: key.clone(),
                source,
            };
            let Some(row) = store.archive(&key).map_err(store_error)? else {
                return Ok(false);
            };
            if row.state != ArchiveState::Archived {
                return Ok(false);
            }
            if let Some(record) = store.session(&key).map_err(store_error)? {
                // A session archived on another machine has its clones there; that machine frees them.
                if super::remote::elsewhere(&record, &here) {
                    return Ok(false);
                }
                if sharers_of(&store, &key).map_err(store_error)?.unfreed {
                    // Another session still works here: only what is this
                    // session's own goes.
                    remove_own_file(&record.folder.join(handover_file(&key)))?;
                } else {
                    let channel = store
                        .channel(&record.channel)
                        .map_err(store_error)?
                        .ok_or_else(|| ArchiveError::NoChannel {
                            session: key.clone(),
                            channel: record.channel.clone(),
                        })?;
                    remove_session_folder(&layout_root, &channel.folder, &record.folder)?;
                }
            }
            store
                .put_archive(&ArchiveRow {
                    state: ArchiveState::Freed,
                    ..row
                })
                .map_err(store_error)?;
            Ok(true)
        })
        .await
        .map_err(|source| ArchiveError::Stop {
            session: session_key.clone(),
            source: Box::new(source),
        })??;
        drop(folder_guard);
        drop(guard);
        if freed {
            self.tell
                .session(&session_key, pb::Change::Updated, None)
                .await;
        }
        Ok(())
    }

    /// Restore's work: bring the clones back when they were freed, drop the
    /// archive row, resume the agent on its conversation.
    async fn run_restore(self, progress: Progress, id: String, by: pb::Actor) {
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        let loaded = match self.load(&id).await {
            Ok(loaded) => loaded,
            Err(refused) => {
                progress.fail(store_failure(&refused, &id));
                return;
            }
        };
        // The row may have changed while this waited for the lock.
        let (store, key) = (self.starter.store.clone(), id.clone());
        let row = match blocking(move || store.archive(&key)).await {
            Ok(Ok(Some(row))) => row,
            Ok(Ok(None)) => {
                progress.fail(error_of(
                    &ArchiveError::NotArchived { session: id.clone() },
                    "it was restored by someone else; open it from the list",
                    None,
                ));
                return;
            }
            Ok(Err(error)) => {
                progress.fail(error_of(
                    &error,
                    "fix the cause the message names, then restore again",
                    None,
                ));
                return;
            }
            Err(refused) => {
                progress.fail(store_failure(&refused, &id));
                return;
            }
        };
        let repos: Vec<String> = loaded
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .map(|repo| repo.name.clone())
            .collect();
        if row.state == ArchiveState::Freed {
            for name in &repos {
                progress.set_step(&format!("clone:{name}"), pb::StepState::Running, None);
            }
            let cloned = match &loaded.detail {
                Some(detail) => self.starter.reclone(&loaded.record, detail).await,
                None => Ok(Vec::new()),
            };
            if let Err(error) = cloned {
                let (_, failure) = start_error(&error);
                for name in &repos {
                    progress.set_step(
                        &format!("clone:{name}"),
                        pb::StepState::Failed,
                        Some(status::chain(&error)),
                    );
                }
                progress.fail(failure);
                return;
            }
            for name in &repos {
                progress.set_step(&format!("clone:{name}"), pb::StepState::Done, None);
            }
        } else {
            for name in &repos {
                progress.set_step(&format!("clone:{name}"), pb::StepState::Skipped, None);
            }
        }
        // The conversation never left this machine's agent folders.
        progress.set_step("import_conversation", pb::StepState::Skipped, None);
        let (store, key) = (self.starter.store.clone(), id.clone());
        let removed = blocking(move || store.remove_archive(&key)).await;
        match removed {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                progress.fail(error_of(
                    &error,
                    "fix the cause the message names, then restore again",
                    None,
                ));
                return;
            }
            Err(refused) => {
                progress.fail(store_failure(&refused, &id));
                return;
            }
        }
        drop(guard);
        self.tell
            .session(&id, pb::Change::Updated, Some(by.clone()))
            .await;
        publish_family(&self.tell, &id, Some(by.clone())).await;

        let Ok(loaded) = self.load(&id).await else {
            progress.fail(error_of(
                &ArchiveError::NotArchived { session: id },
                "open the session from the list",
                None,
            ));
            return;
        };
        let running = match loaded.agent.as_ref() {
            Some(agent) if matches!(agent.state, AgentState::Running { .. }) => {
                humata2_holder::attach(&agent.launch.holder_socket, u64::MAX)
                    .await
                    .ok()
                    .is_some_and(|attached| attached.welcome.exited.is_none())
            }
            _ => false,
        };
        if running {
            // Undone before the agent stopped: it never went away.
            progress.set_step("launch", pb::StepState::Skipped, None);
            progress.set_step("verify", pb::StepState::Skipped, None);
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: id.clone(),
            })));
            watch::agent(self.tell.clone(), id);
            return;
        }
        run_resume(
            self.starter.clone(),
            self.tell.clone(),
            by,
            self.tokens.clone(),
            loaded,
            progress,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// `<root>/ch/{main,sessions/<one>,shared}` and a file outside the root.
    fn layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        for part in ["main/api", "sessions/one/api", "shared"] {
            fs::create_dir_all(root.join("ch").join(part)).unwrap();
        }
        fs::write(root.join("ch/sessions/one/api/file"), "x").unwrap();
        fs::write(root.join("ch/shared/keep"), "x").unwrap();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep"), "x").unwrap();
        (dir, root, outside)
    }

    fn refused(root: &Path, folder: &Path) -> bool {
        matches!(
            remove_session_folder(root, "ch", folder),
            Err(ArchiveError::NotASessionFolder { .. })
        )
    }

    #[test]
    fn the_session_folder_goes_and_nothing_beside_it() {
        let (_dir, root, _) = layout();
        remove_session_folder(&root, "ch", &root.join("ch/sessions/one")).unwrap();
        assert!(!root.join("ch/sessions/one").exists());
        assert!(root.join("ch/sessions").is_dir());
        assert!(root.join("ch/main/api").is_dir());
        assert!(root.join("ch/shared/keep").is_file());
    }

    #[test]
    fn a_folder_already_gone_is_freed() {
        let (_dir, root, _) = layout();
        remove_session_folder(&root, "ch", &root.join("ch/sessions/gone")).unwrap();
    }

    #[test]
    fn the_sessions_folder_itself_is_refused() {
        let (_dir, root, _) = layout();
        assert!(refused(&root, &root.join("ch/sessions")));
        assert!(root.join("ch/sessions/one/api/file").is_file());
    }

    #[test]
    fn a_path_that_climbs_out_with_dots_is_refused() {
        let (_dir, root, _) = layout();
        assert!(refused(&root, &root.join("ch/sessions/../main")));
        assert!(refused(&root, &root.join("ch/sessions/one/..")));
        assert!(root.join("ch/main/api").is_dir());
        assert!(root.join("ch/sessions/one/api/file").is_file());
    }

    #[test]
    fn a_symlink_out_of_the_sessions_folder_is_refused() {
        let (_dir, root, outside) = layout();
        std::os::unix::fs::symlink(&outside, root.join("ch/sessions/link")).unwrap();
        assert!(refused(&root, &root.join("ch/sessions/link")));
        assert!(outside.join("keep").is_file());
    }

    #[test]
    fn a_path_outside_the_root_is_refused() {
        let (_dir, root, outside) = layout();
        assert!(refused(&root, &outside));
        assert!(refused(&root, &root.join("ch/main")));
        assert!(outside.join("keep").is_file());
    }

    #[test]
    fn a_session_folder_of_another_channel_is_refused() {
        let (_dir, root, _) = layout();
        fs::create_dir_all(root.join("other/sessions/two")).unwrap();
        assert!(refused(&root, &root.join("other/sessions/two")));
        assert!(root.join("other/sessions/two").is_dir());
    }

    #[test]
    fn a_plan_is_for_one_session_and_used_once() {
        let lifecycle = Lifecycle::default();
        let token = lifecycle.make_plan(PlanKind::Archive, "se_a");
        assert!(lifecycle.plan_is_live(PlanKind::Archive, &token, "se_a"));
        assert!(!lifecycle.plan_is_live(PlanKind::Archive, &token, "se_b"));
        assert!(!lifecycle.plan_is_live(PlanKind::Push, &token, "se_a"));
        assert!(!lifecycle.use_plan(PlanKind::Archive, &token, "se_b"));
        assert!(lifecycle.use_plan(PlanKind::Archive, &token, "se_a"));
        assert!(!lifecycle.use_plan(PlanKind::Archive, &token, "se_a"));
        assert!(!lifecycle.plan_is_live(PlanKind::Archive, "pl_unknown", "se_a"));
    }

    #[test]
    fn a_summary_lists_each_repo_with_its_files_and_pushed_sha() {
        let works = [Work {
            owner: "acme".to_owned(),
            name: "api".to_owned(),
            branch: "me/x".to_owned(),
            path: PathBuf::from("/nonexistent/api"),
            work: Some(RepoWork {
                branch: "me/x".to_owned(),
                dirty_files: 1,
                unpushed_commits: 0,
                never_pushed: false,
                last_pushed_sha: Some("abc".to_owned()),
                files: vec![humata2_sessions::FileChange {
                    path: "wip.txt".to_owned(),
                    added: 2,
                    removed: 0,
                }],
            }),
        }];
        let summary = summary_of(&works, 7);
        assert_eq!(summary.repos.len(), 1);
        assert_eq!(summary.repos[0].last_pushed_sha, "abc");
        assert_eq!(summary.repos[0].files[0].path, "wip.txt");
        assert_eq!(summary.repos[0].files[0].added, 2);
    }
}
