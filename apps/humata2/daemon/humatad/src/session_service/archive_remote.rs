//! Archive and restore of a session that runs on another machine (MV-7 to
//! MV-9, LD-2 to LD-5, D195). The machine archives it as it archives its own
//! (stops the agent, frees its clones once the undo window closes) and the
//! host writes an archive row of its own, so the session leaves the plain
//! list and shows `archive`. Restoring either asks the machine to undo its
//! archive in place, or, on this Mac, brings the session here: its clones
//! are cloned again from the branch, the conversation is read from the
//! machine while it is online, the agent is resumed in it, and the record
//! names this Mac.
//!
//! Nothing here waits on a timer: the host follows the machine's own
//! operation until it ends. A link lost after the machine was asked leaves
//! the host's row to the next time the link comes up: the host asks the
//! machine then whether it archived the session. A restore on this Mac
//! while the machine is away takes the conversation from the host's mirror
//! and lets the machine's copy go the next time its link comes up. Both
//! follow-ups are kept on disk before they wait (`pending`), so a restart
//! of the host goes on with them (D222). The machine's copy goes only once
//! the host has compared it with what it restored and found nothing more
//! there (`compare`, D224).

mod compare;
mod pending;

use std::collections::HashMap;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{ArchiveRow, ArchiveState, SessionRecord, StoreError};
use prost::Message;
use tonic::{Code, Status};

use super::fork::hold_point;
use super::lineage::publish_family;
use super::{Gated, Loaded, SessionSvc, now_ms, run_resume, start_error, watch};
use crate::channel_service::{blocking, error_of, failed, timestamp_ms};
use crate::carry::{Carry, Place};
use crate::host_relay::{LinkState, Links, RelayError, Relayed, assigned};
use crate::identity::{self, Caller};
use crate::operations::{Progress, Started};
use crate::status;
use compare::{More, Restored, Theirs};
use pending::Pending;

/// Why an archive or restore of a session on another machine couldn't go on.
#[derive(Debug, thiserror::Error)]
enum RemoteError {
    #[error("this daemon has no way to reach other machines, so session {session} on {machine} can't be archived or restored from here")]
    NoLinks { session: String, machine: String },
    #[error("the link to {machine} was lost while it was {doing}, so it isn't known whether it finished")]
    LinkLost { machine: String, doing: &'static str },
    #[error("{machine} ended the {doing} without saying how it went")]
    NoAnswer { machine: String, doing: &'static str },
    #[error("couldn't save session {session}'s records on this machine")]
    Store {
        session: String,
        #[source]
        source: StoreError,
    },
    #[error("session {session} can be restored on {machine} or on this Mac; restoring it on another machine isn't built yet")]
    ElsewhereNotBuilt { session: String, machine: String },
    #[error("session {session} runs on {machine} and has no clone folder here to restore into")]
    NoDetail { session: String, machine: String },
    #[error("{machine} is offline and this Mac's mirror holds no conversation of session {session} to restore it from")]
    NoMirror { session: String, machine: String },
    #[error("this Mac's mirror of session {session}'s conversation couldn't be read")]
    Mirror {
        session: String,
        #[source]
        source: humata2_mirror::MirrorError,
    },
}

/// Waits until `links` show `machine` up on a link other than `stale`, and
/// returns that link's generation; `None` once the machine is forgotten.
async fn next_link_up(links: &Links, machine: &str, stale: Option<u64>) -> Option<u64> {
    let mut state = links.watch(machine);
    let now = *state
        .wait_for(|now| matches!(now, LinkState::Up { generation } if Some(*generation) != stale))
        .await
        .ok()?;
    match now {
        LinkState::Up { generation } => Some(generation),
        LinkState::Down => None,
    }
}

/// Whether `repo` of a pointer is `owner`/`name`.
fn names(pointer: &pb::Pointer, owner: &str, name: &str) -> bool {
    pointer
        .repo
        .as_ref()
        .is_some_and(|named| named.owner == owner && named.name == name)
}

/// The host's summary of a session its machine archived: what the machine's
/// own archive says (the files the clones changed), with what the machine
/// last reported filling what that leaves out (the unpushed commits'
/// subjects, the last pushed sha). Without the machine's summary, the
/// repositories and reports alone.
fn host_summary(
    loaded: &Loaded,
    machines: Option<pb::ArchiveSummary>,
    reported: &[pb::Pointer],
    archived: i64,
) -> pb::ArchiveSummary {
    let theirs = machines.unwrap_or_default();
    let repos = loaded
        .detail
        .iter()
        .flat_map(|detail| detail.repos.iter())
        .map(|repo| {
            let mut kept = theirs
                .repos
                .iter()
                .find(|kept| {
                    kept.repo
                        .as_ref()
                        .is_some_and(|named| named.owner == repo.owner && named.name == repo.name)
                })
                .cloned()
                .unwrap_or_else(|| pb::archive_summary::Repo {
                    repo: Some(pb::RepoRef {
                        owner: repo.owner.clone(),
                        name: repo.name.clone(),
                    }),
                    branch: repo.branch.clone(),
                    ..Default::default()
                });
            let mine = reported.iter().filter(|pointer| names(pointer, &repo.owner, &repo.name));
            if kept.last_pushed_sha.is_empty()
                && let Some(pushed) = mine.clone().find(|pointer| pointer.pushed)
            {
                kept.last_pushed_sha.clone_from(&pushed.head_sha);
            }
            if kept.unpushed_subjects.is_empty()
                && let Some(subjects) = mine.clone().find(|pointer| !pointer.unpushed_subjects.is_empty())
            {
                kept.unpushed_subjects.clone_from(&subjects.unpushed_subjects);
            }
            if kept.files.is_empty() {
                kept.files = mine
                    .flat_map(|pointer| pointer.dirty_paths.iter())
                    .map(|path| pb::FileChange {
                        path: path.clone(),
                        added: 0,
                        removed: 0,
                    })
                    .collect();
            }
            kept
        })
        .collect();
    pb::ArchiveSummary {
        archived_at: Some(timestamp_ms(archived)),
        repos,
        machine_lost: false,
        lost: Vec::new(),
    }
}

/// A machine's own archive of a session: its summary and the pointers it
/// shows for the session.
struct Archived {
    summary: pb::ArchiveSummary,
    pointers: Vec<pb::Pointer>,
}

/// The pointers an archive row takes: the machine's own, where its answer
/// carries any, over what it last reported (D218).
fn pointers_of<'a>(theirs: &'a [pb::Pointer], reported: &'a [pb::Pointer]) -> &'a [pb::Pointer] {
    if theirs.is_empty() { reported } else { theirs }
}

/// A call to `machine` that failed: its own answer stays as it is, a link
/// that broke under the call is "offline".
pub(super) fn machine_failed(machine: &str, source: Status) -> Status {
    if status::error_of(&source).is_none()
        && matches!(source.code(), Code::Unavailable | Code::Unknown | Code::Cancelled)
    {
        return Status::from(RelayError::Offline {
            machine: machine.to_owned(),
        });
    }
    source
}

pub(super) fn finished(operation: &pb::Operation) -> bool {
    operation.state != pb::OperationState::Running as i32
}

/// Shows the steps `operation` has on the host's `progress`.
pub(super) fn mirror_steps(
    operation: &pb::Operation,
    progress: &Progress,
    seen: &mut HashMap<String, (i32, String)>,
) {
    for step in &operation.steps {
        let now = (step.state, step.output_tail.clone());
        if seen.get(&step.key) == Some(&now) {
            continue;
        }
        let state = pb::StepState::try_from(step.state).unwrap_or(pb::StepState::Waiting);
        let tail = Some(step.output_tail.clone()).filter(|tail| !tail.is_empty());
        progress.set_step(&step.key, state, tail);
        seen.insert(step.key.clone(), now);
    }
}

/// The error of a link to `machine` lost while it was `doing` something it
/// finishes on its own. An archive is checked again once the machine
/// reconnects (D218).
fn lost_link(machine: &str, doing: &'static str) -> Status {
    let fix = if doing == "archive" {
        format!(
            "{machine} may still have archived the session: Humata checks when {machine} reconnects and shows it archived then. Reconnect {machine} rather than archiving again"
        )
    } else {
        "reconnect the machine and look at the session: the machine finishes what it started on its own".to_owned()
    };
    failed(
        Code::Unavailable,
        &RemoteError::LinkLost {
            machine: machine.to_owned(),
            doing,
        },
        &fix,
        None,
    )
}

/// Follows the machine's operation `first` until it ends, showing its steps
/// on `progress`. A link that breaks first is the error `lost` makes: the
/// machine goes on with the work on its own.
pub(super) async fn follow_to_end(
    relayed: &Relayed,
    machine: &str,
    first: pb::Operation,
    progress: &Progress,
    lost: impl FnOnce() -> Status,
) -> Result<pb::Operation, Status> {
    let mut seen = HashMap::new();
    mirror_steps(&first, progress, &mut seen);
    if finished(&first) {
        return Ok(first);
    }
    let id = first.id.map(|id| id.value).unwrap_or_default();
    let mut stream = relayed
        .operation()
        .watch_operation(pb::WatchOperationRequest {
            id: Some(pb::OperationId { value: id }),
            from_version: 0,
        })
        .await
        .map_err(|source| machine_failed(machine, source))?
        .into_inner();
    loop {
        match stream.message().await {
            Ok(Some(operation)) => {
                mirror_steps(&operation, progress, &mut seen);
                if finished(&operation) {
                    return Ok(operation);
                }
            }
            Ok(None) | Err(_) => return Err(lost()),
        }
    }
}

/// What the machine's ended operation says went wrong.
fn operation_error(machine: &str, doing: &'static str, ended: &pb::Operation) -> pb::Error {
    ended.error.clone().unwrap_or_else(|| {
        error_of(
            &RemoteError::NoAnswer {
                machine: machine.to_owned(),
                doing,
            },
            "look at the session on the machine, then try again",
            None,
        )
    })
}

impl SessionSvc {
    /// The host's links to other machines, when this daemon has them.
    fn links_to(&self, session: &str, machine: &str) -> Result<Links, Status> {
        self.carry.get().map(|carry| carry.links().clone()).ok_or_else(|| {
            failed(
                Code::FailedPrecondition,
                &RemoteError::NoLinks {
                    session: session.to_owned(),
                    machine: machine.to_owned(),
                },
                "archive and restore it from the host",
                None,
            )
        })
    }

    fn relayed_to(&self, session: &str, machine: &str) -> Result<Relayed, Status> {
        self.links_to(session, machine)?
            .relayed(machine)
            .map_err(Status::from)
    }

    /// PlanArchive for a session on `machine`: the machine reads its own
    /// clones and answers with its plan and the token ArchiveSession takes.
    pub(super) async fn plan_archive_remote(
        &self,
        id: &str,
        machine: &str,
    ) -> Result<pb::ArchivePlan, Status> {
        let relayed = self.relayed_to(id, machine)?;
        let request = pb::PlanArchiveRequest {
            id: Some(pb::SessionId { value: id.to_owned() }),
        };
        relayed
            .session()
            .plan_archive(assigned(request, None).map_err(Status::from)?)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| machine_failed(machine, source))
    }

    /// ArchiveSession for a session on `machine`: the host's operation shows
    /// the machine's, and once the machine has archived the session the host
    /// writes its own archive row.
    pub(super) async fn archive_remote(
        &self,
        caller: &Caller,
        request: pb::ArchiveSessionRequest,
        loaded: Loaded,
        choice: pb::ArchiveChoice,
        machine: String,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = loaded.record.id.clone();
        // An offline machine answers at once and changes nothing (CH-4).
        self.relayed_to(&id, &machine)?;
        let steps = [
            ("stop", "Stopping the agent"),
            ("summarize", "Writing what the session had"),
            ("free_clones", "Deleting the clones when the undo window closes"),
        ]
        .map(|(key, title)| (key.to_owned(), title.to_owned()));
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::ArchiveSession,
            target,
            &steps,
            false,
            request.idempotency_key.clone(),
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        let asked = pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(pb::SessionId { value: id }),
            choice: choice as i32,
            plan: request.plan,
        };
        tokio::spawn(self.clone().run_archive_remote(
            progress,
            asked,
            loaded,
            machine,
            identity::actor(caller),
        ));
        Ok(Gated::Done(operation))
    }

    async fn run_archive_remote(
        self,
        progress: Progress,
        asked: pb::ArchiveSessionRequest,
        loaded: Loaded,
        machine: String,
        by: pb::Actor,
    ) {
        let id = loaded.record.id.clone();
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        progress.set_step("stop", pb::StepState::Running, None);
        // What the machine last reported, before its archive may clear it.
        let reported = self.reported_pointers(&machine, &id);
        // Kept before the machine is asked: a restart that cuts the wait
        // short still learns whether the machine archived it (D222).
        let mut journal = Pending::Reconcile {
            session: id.clone(),
            machine: machine.clone(),
            reported: reported.clone(),
            by: by.clone(),
            asked: None,
        };
        let mut asked_machine = false;
        let ended = async {
            let relayed = self.relayed_to(&id, &machine)?;
            let request = assigned(asked, None).map_err(Status::from)?;
            self.keep(&journal).await;
            asked_machine = true;
            let first = relayed
                .session()
                .archive_session(request)
                .await
                .map_err(|source| machine_failed(&machine, source));
            // The machine's operation, so a later check can wait for it to
            // end rather than find the archive still under way.
            if let (Ok(answer), Pending::Reconcile { asked, .. }) = (&first, &mut journal) {
                *asked = answer.get_ref().id.as_ref().map(|id| id.value.clone());
                self.keep(&journal).await;
            }
            hold_point("archive-remote-asked").await;
            follow_to_end(&relayed, &machine, first?.into_inner(), &progress, || lost_link(&machine, "archive")).await
        }
        .await;
        let ended = match ended {
            Ok(ended) => ended,
            Err(refused) => {
                progress.fail(status::error_of(&refused).unwrap_or_else(|| pb::Error {
                    message: refused.message().to_owned(),
                    fix: "try the archive again".to_owned(),
                    retry: None,
                    kind: None,
                }));
                drop(guard);
                // The machine may finish the archive on its own; the host
                // learns of it once the link is back.
                if asked_machine && refused.code() == Code::Unavailable {
                    tokio::spawn(self.clone().reconcile_when_linked(journal));
                } else if asked_machine {
                    self.settled(&journal).await;
                }
                return;
            }
        };
        if ended.state != pb::OperationState::Succeeded as i32 {
            self.settled(&journal).await;
            progress.fail(operation_error(&machine, "archive", &ended));
            return;
        }
        let theirs = self.archived_on(&machine, &id).await.ok().flatten();
        if let Err(refused) = self.write_remote_row(&loaded, theirs, &reported).await {
            progress.fail(status::error_of(&refused).unwrap_or_default());
            return;
        }
        self.settled(&journal).await;
        drop(guard);
        progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
            value: id.clone(),
        })));
        self.after_remote_archive(&id, by).await;
    }

    /// The pointers `machine` last reported for `session`.
    fn reported_pointers(&self, machine: &str, session: &str) -> Vec<pb::Pointer> {
        self.remote
            .as_ref()
            .and_then(|remote| remote.view(machine, session))
            .map(|view| view.pointers)
            .unwrap_or_default()
    }

    /// What `machine`'s own archive of `session` says, when it has one, with
    /// the pointers it shows for the session.
    async fn archived_on(&self, machine: &str, session: &str) -> Result<Option<Archived>, Status> {
        let relayed = self.relayed_to(session, machine)?;
        let request = pb::GetSessionRequest {
            id: Some(pb::SessionId { value: session.to_owned() }),
        };
        match relayed.session().get_session(tonic::Request::new(request)).await {
            Ok(shown) => {
                let shown = shown.into_inner();
                Ok(shown.archive.map(|summary| Archived {
                    summary,
                    pointers: shown.pointers,
                }))
            }
            Err(refused) if refused.code() == Code::NotFound => Ok(None),
            Err(refused) => Err(machine_failed(machine, refused)),
        }
    }

    /// After a link lost while `machine` archived `session`: each time the
    /// link comes up, asks the machine whether it archived the session, and
    /// writes the host's row when it did. Ends once the machine answers, or
    /// once it is forgotten; `journal` goes then. A failure other than the
    /// link's leaves it for the next start.
    async fn reconcile_when_linked(self, journal: Pending) {
        let Pending::Reconcile {
            session,
            machine,
            reported,
            by,
            asked,
        } = &journal
        else {
            return;
        };
        let Ok(links) = self.links_to(session, machine) else {
            return;
        };
        let mut stale = None;
        while let Some(generation) = next_link_up(&links, machine, stale).await {
            match self.reconcile_archive(session, machine, reported, by, asked.as_deref()).await {
                Ok(()) => break,
                Err(refused) if refused.code() == Code::Unavailable => stale = Some(generation),
                Err(refused) => {
                    crate::log::say!(
                        "humatad2: couldn't learn whether {machine} archived session {session}: {}; Humata asks again when {machine} is next linked or Humata next starts. Forgetting {machine} drops the question",
                        refused.message()
                    );
                    stale = Some(generation);
                }
            }
        }
        // Answered, or the machine was forgotten (which archives its
        // sessions on its own): nothing is left to learn.
        self.settled(&journal).await;
    }

    /// Keeps `journal` on disk, so a restart goes on with it. Best effort: a
    /// follow-up that can't be kept still runs, and the log says so.
    async fn keep(&self, journal: &Pending) {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return;
        };
        let kept = journal.clone();
        if let Ok(Err(error)) = blocking(move || pending::write(&root, &kept)).await {
            crate::log::say!(
                "humatad2: {}; if Humata restarts before the machine is back, look at session {} on it",
                status::chain(&error),
                journal.session()
            );
        }
    }

    /// `journal` is done or moot; its file goes.
    async fn settled(&self, journal: &Pending) {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return;
        };
        let done = journal.clone();
        let _ = blocking(move || pending::remove(&root, &done)).await;
    }

    /// Goes on with the follow-ups the last run left waiting on a machine's
    /// link, once the start is done (D222). Call it once the carry is set.
    pub(super) fn recover_remote_archives_after(&self, started: std::sync::Arc<crate::restart::Started>) {
        let Some(root) = self.carry.get().map(Carry::remote_archive_folder) else {
            return;
        };
        for journal in pending::read_all(&root) {
            let (svc, started) = (self.clone(), started.clone());
            tokio::spawn(async move {
                started.wait().await;
                match journal {
                    Pending::Reconcile { .. } => svc.reconcile_when_linked(journal).await,
                    Pending::Release { .. } => svc.release_when_linked(journal).await,
                }
            });
        }
    }

    /// Writes the host's row for `session` when `machine` archived it and
    /// the host has none yet, and the session still runs there.
    async fn reconcile_archive(
        &self,
        session: &str,
        machine: &str,
        reported: &[pb::Pointer],
        by: &pb::Actor,
        asked: Option<&str>,
    ) -> Result<(), Status> {
        // An archive still under way when the link came back is waited for,
        // outside the session's lock.
        if let Some(operation) = asked {
            self.machine_operation_ended(session, machine, operation).await?;
        }
        let guard = self.lifecycle.lock_of(session).lock_owned().await;
        let loaded = match self.load(session).await {
            Ok(loaded) => loaded,
            Err(refused) if refused.code() == Code::NotFound => return Ok(()),
            Err(refused) => return Err(refused),
        };
        let (store, key) = (self.starter.store.clone(), session.to_owned());
        let row = blocking(move || store.archive(&key)).await?.map_err(|source| {
            failed(
                Code::Internal,
                &RemoteError::Store {
                    session: session.to_owned(),
                    source,
                },
                "restart Humata on this machine",
                None,
            )
        })?;
        if loaded.record.machine != machine || row.is_some() {
            return Ok(());
        }
        let Some(theirs) = self.archived_on(machine, session).await? else {
            return Ok(());
        };
        self.write_remote_row(&loaded, Some(theirs), reported).await?;
        drop(guard);
        self.after_remote_archive(session, by.clone()).await;
        Ok(())
    }

    /// Waits until `machine`'s operation `id` ends. One the machine no longer
    /// knows ended with a restart of it, which finished or undid its work.
    async fn machine_operation_ended(&self, session: &str, machine: &str, id: &str) -> Result<(), Status> {
        let relayed = self.relayed_to(session, machine)?;
        let watched = relayed
            .operation()
            .watch_operation(pb::WatchOperationRequest {
                id: Some(pb::OperationId { value: id.to_owned() }),
                from_version: 0,
            })
            .await;
        let mut stream = match watched {
            Ok(stream) => stream.into_inner(),
            Err(refused) if refused.code() == Code::NotFound => return Ok(()),
            Err(refused) => return Err(machine_failed(machine, refused)),
        };
        loop {
            match stream.message().await {
                Ok(Some(operation)) if finished(&operation) => return Ok(()),
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => return Err(lost_link(machine, "archive")),
            }
        }
    }

    /// What follows the host's row for a session its machine archived: its
    /// grants and requests end, and the feeds show it archived.
    async fn after_remote_archive(&self, id: &str, by: pb::Actor) {
        // The session no longer waits on anyone (IN-22, TP-15).
        if let Err(error) = self.end_grants_of(id).await {
            crate::log::say!(
                "humatad2: ending archived session {id}'s grants: {}",
                error.message()
            );
        }
        if let Err(error) = self
            .withdraw_requests_of(id, pb::WithdrawReason::SessionArchived)
            .await
        {
            crate::log::say!(
                "humatad2: withdrawing archived session {id}'s requests: {}",
                error.message()
            );
        }
        self.tell.session(id, pb::Change::Updated, Some(by.clone())).await;
        publish_family(&self.tell, id, Some(by)).await;
    }

    /// The host's own row for a session its machine archived. It is written
    /// archived at once: the host has no clones of the session to free.
    async fn write_remote_row(
        &self,
        loaded: &Loaded,
        theirs: Option<Archived>,
        reported: &[pb::Pointer],
    ) -> Result<(), Status> {
        let id = loaded.record.id.clone();
        let archived = now_ms();
        let (summary, pointers) = match theirs {
            Some(Archived { summary, pointers }) => (Some(summary), pointers),
            None => (None, Vec::new()),
        };
        let summary = host_summary(loaded, summary, pointers_of(&pointers, reported), archived);
        let row = ArchiveRow {
            session_id: id.clone(),
            archived_unix_ms: archived,
            state: ArchiveState::Archived,
            summary: summary.encode_to_vec(),
            free_after_unix_ms: archived,
        };
        let store = self.starter.store.clone();
        blocking(move || store.put_archive(&row))
            .await?
            .map_err(|source| {
                failed(
                    Code::Internal,
                    &RemoteError::Store { session: id, source },
                    "restart Humata on this machine and archive the session again",
                    None,
                )
            })
    }

    /// RestoreSession for an archived session of another machine: with no
    /// machine named, or the session's own, the machine undoes its archive
    /// where it is; naming this Mac brings the session here.
    pub(super) async fn restore_remote(
        &self,
        caller: &Caller,
        request: pb::RestoreSessionRequest,
        loaded: Loaded,
        machine: String,
    ) -> Result<Gated<pb::Operation>, Status> {
        let id = loaded.record.id.clone();
        let wanted = request
            .machine
            .as_ref()
            .map(|named| named.value.clone())
            .filter(|named| !named.is_empty());
        let here = wanted.as_deref() == Some(self.machine.as_str());
        match wanted {
            Some(named) if named != self.machine && named != machine => {
                return Err(failed(
                    Code::FailedPrecondition,
                    &RemoteError::ElsewhereNotBuilt {
                        session: id,
                        machine: machine.clone(),
                    },
                    "restore it on its own machine or on this Mac",
                    None,
                ));
            }
            _ => {}
        }
        if here {
            // With the machine away, the host's mirror has the conversation.
            self.links_to(&id, &machine)?;
        } else {
            // An offline machine answers at once and changes nothing (CH-4).
            self.relayed_to(&id, &machine)?;
        }
        let mut steps = Vec::new();
        if here {
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
        } else {
            steps.push(("restore".to_owned(), format!("Restoring it on {machine}")));
        }
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId { value: id.clone() })),
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
        let by = identity::actor(caller);
        if here {
            tokio::spawn(self.clone().run_restore_here(progress, loaded, machine, by));
        } else {
            tokio::spawn(self.clone().run_restore_there(progress, id, machine, by));
        }
        Ok(Gated::Done(operation))
    }

    /// Undo in place: the machine restores the session where it is, and the
    /// host drops its own archive row.
    async fn run_restore_there(self, progress: Progress, id: String, machine: String, by: pb::Actor) {
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        progress.set_step("restore", pb::StepState::Running, None);
        let ended = async {
            let relayed = self.relayed_to(&id, &machine)?;
            let request = pb::RestoreSessionRequest {
                idempotency_key: None,
                id: Some(pb::SessionId { value: id.clone() }),
                machine: None,
            };
            let first = relayed
                .session()
                .restore_session(assigned(request, None).map_err(Status::from)?)
                .await
                .map_err(|source| machine_failed(&machine, source))?
                .into_inner();
            follow_to_end(&relayed, &machine, first, &progress, || lost_link(&machine, "restore")).await
        }
        .await;
        let ended = match ended {
            Ok(ended) => ended,
            Err(refused) => {
                progress.set_step("restore", pb::StepState::Failed, Some(refused.message().to_owned()));
                progress.fail(status::error_of(&refused).unwrap_or_else(|| pb::Error {
                    message: refused.message().to_owned(),
                    fix: "try the restore again".to_owned(),
                    retry: None,
                    kind: None,
                }));
                return;
            }
        };
        if ended.state != pb::OperationState::Succeeded as i32 {
            progress.set_step("restore", pb::StepState::Failed, None);
            progress.fail(operation_error(&machine, "restore", &ended));
            return;
        }
        let (store, key) = (self.starter.store.clone(), id.clone());
        match blocking(move || store.remove_archive(&key)).await {
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
                progress.fail(status::error_of(&refused).unwrap_or_default());
                return;
            }
        }
        drop(guard);
        progress.set_step("restore", pb::StepState::Done, None);
        progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
            value: id.clone(),
        })));
        self.tell.session(&id, pb::Change::Updated, Some(by.clone())).await;
        publish_family(&self.tell, &id, Some(by)).await;
    }

    /// Restore on this Mac: the machine stages the session's records and
    /// conversation and they are copied here and taken in (the same steps a
    /// move uses, without the folder, which was freed), the branch is cloned
    /// again, the record names this Mac and the agent is resumed in its
    /// conversation.
    async fn run_restore_here(self, progress: Progress, loaded: Loaded, machine: String, by: pb::Actor) {
        let id = loaded.record.id.clone();
        let guard = self.lifecycle.lock_of(&id).lock_owned().await;
        let operation = progress.operation().id.map(|id| id.value).unwrap_or_default();
        // The machine away: the conversation comes from the host's mirror.
        if self.relayed_to(&id, &machine).is_err() {
            return self.restore_here_from_mirror(progress, loaded, machine, by, guard).await;
        }
        let repos: Vec<String> = loaded
            .detail
            .iter()
            .flat_map(|detail| detail.repos.iter())
            .map(|repo| repo.name.clone())
            .collect();
        let Some(detail) = loaded.detail.clone() else {
            progress.fail(error_of(
                &RemoteError::NoDetail {
                    session: id,
                    machine,
                },
                "restore it on its own machine",
                None,
            ));
            return;
        };

        progress.set_step("import_conversation", pb::StepState::Running, None);
        if let Err(status) = self.take_conversation(&loaded.record, &machine, &operation).await {
            if status.code() == Code::Unavailable {
                self.undo_conversation(&loaded.record, &operation).await;
                return self.restore_here_from_mirror(progress, loaded, machine, by, guard).await;
            }
            progress.set_step(
                "import_conversation",
                pb::StepState::Failed,
                Some(status.message().to_owned()),
            );
            progress.fail(error_from(&status));
            return;
        }
        progress.set_step("import_conversation", pb::StepState::Done, None);

        for name in &repos {
            progress.set_step(&format!("clone:{name}"), pb::StepState::Running, None);
        }
        if let Err(error) = self.starter.reclone(&loaded.record, &detail).await {
            let (_, failure) = start_error(&error);
            for name in &repos {
                progress.set_step(
                    &format!("clone:{name}"),
                    pb::StepState::Failed,
                    Some(status::chain(&error)),
                );
            }
            self.undo_conversation(&loaded.record, &operation).await;
            progress.fail(failure);
            return;
        }
        for name in &repos {
            progress.set_step(&format!("clone:{name}"), pb::StepState::Done, None);
        }

        // Switch: the record names this Mac, and the archive row goes.
        // The mirror first reads the conversation taken here, which repeats
        // what it held from the machine, so nothing shows twice (D211).
        let here = self.machine.clone();
        self.rebase_mirror(&id, &here, None).await;
        let mut record = loaded.record.clone();
        record.machine.clone_from(&self.machine);
        let (store, saved, key) = (self.starter.store.clone(), record.clone(), id.clone());
        let switched = blocking(move || -> Result<(), StoreError> {
            store.put_session(&saved)?;
            store.remove_archive(&key)
        })
        .await;
        match switched {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                self.rebase_mirror(&id, &machine, None).await;
                progress.fail(error_of(
                    &error,
                    "fix the cause the message names, then restore again",
                    None,
                ));
                return;
            }
            Err(refused) => {
                self.rebase_mirror(&id, &machine, None).await;
                progress.fail(error_from(&refused));
                return;
            }
        }
        drop(guard);
        if let Some(remote) = &self.remote {
            remote.forget_session(&machine, &id).await;
        }
        self.release_old_copy(&machine, &id, &record, &operation).await;
        self.tell.session(&id, pb::Change::Updated, Some(by.clone())).await;
        publish_family(&self.tell, &id, Some(by.clone())).await;

        let Ok(loaded) = self.load(&id).await else {
            progress.fail(error_of(
                &RemoteError::NoDetail {
                    session: id,
                    machine,
                },
                "open the session from the list",
                None,
            ));
            return;
        };
        run_resume(
            self.starter.clone(),
            self.tell.clone(),
            by,
            self.tokens.clone(),
            loaded,
            progress,
        )
        .await;
        watch::agent(self.tell.clone(), id);
    }

    /// Restore on this Mac while the session's machine is away: its branch
    /// is cloned again here, the conversation the host's mirror kept is
    /// written as the agent's, the record names this Mac and the agent
    /// resumes in it. The machine lets its copy go the next time its link
    /// comes up.
    async fn restore_here_from_mirror(
        self,
        progress: Progress,
        loaded: Loaded,
        machine: String,
        by: pb::Actor,
        guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let id = loaded.record.id.clone();
        let operation = progress.operation().id.map(|id| id.value).unwrap_or_default();
        let Some(detail) = loaded.detail.clone() else {
            progress.fail(error_of(
                &RemoteError::NoDetail { session: id, machine },
                "restore it on its own machine once it is back",
                None,
            ));
            return;
        };

        progress.set_step("import_conversation", pb::StepState::Running, None);
        let (svc, key) = (self.clone(), id.clone());
        let lines = match blocking(move || svc.mirrored_lines(&key)).await {
            Ok(Ok(Some(lines))) if !lines.is_empty() => lines,
            Ok(Ok(_)) => {
                let error = RemoteError::NoMirror { session: id, machine };
                progress.set_step("import_conversation", pb::StepState::Failed, Some(error.to_string()));
                progress.fail(error_of(&error, "bring the machine back online, then restore again", None));
                return;
            }
            Ok(Err(source)) => {
                let error = RemoteError::Mirror { session: id, source };
                progress.set_step("import_conversation", pb::StepState::Failed, Some(status::chain(&error)));
                progress.fail(error_of(&error, "bring the machine back online, then restore again", None));
                return;
            }
            Err(refused) => {
                progress.fail(error_from(&refused));
                return;
            }
        };
        let channel = match self.find_channel(&loaded.record.channel).await {
            Ok(channel) => channel,
            Err(refused) => {
                progress.fail(error_from(&refused));
                return;
            }
        };
        // The session's folder on this Mac, by the same name it had there.
        let name = loaded
            .record
            .folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| id.clone());
        let folder = self.starter.layout.session(&channel.folder, &name);
        let old_folder = detail
            .repos
            .first()
            .and_then(|repo| repo.path.parent())
            .unwrap_or(loaded.record.folder.as_path())
            .display()
            .to_string();
        let mut record = loaded.record.clone();
        record.machine.clone_from(&self.machine);
        record.folder.clone_from(&folder);
        let mut detail = detail;
        for repo in &mut detail.repos {
            repo.path = folder.join(&repo.name);
        }
        progress.set_step("import_conversation", pb::StepState::Done, None);

        let repos: Vec<String> = detail.repos.iter().map(|repo| repo.name.clone()).collect();
        for name in &repos {
            progress.set_step(&format!("clone:{name}"), pb::StepState::Running, None);
        }
        if let Err(error) = self.starter.reclone(&record, &detail).await {
            let (_, failure) = start_error(&error);
            for name in &repos {
                progress.set_step(&format!("clone:{name}"), pb::StepState::Failed, Some(status::chain(&error)));
            }
            progress.fail(failure);
            return;
        }
        for name in &repos {
            progress.set_step(&format!("clone:{name}"), pb::StepState::Done, None);
        }
        // What the machine's copy is compared with before it goes (D224).
        let cloned = detail.repos.clone();
        let (lines, restored) = match blocking(move || {
            let restored = Restored::of(&lines, &cloned);
            (lines, restored)
        })
        .await
        {
            Ok(read) => read,
            Err(refused) => {
                progress.fail(error_from(&refused));
                return;
            }
        };

        // Kept before the switch: a restart after it still lets the
        // machine's copy go (D222). The release checks the record first.
        let journal = Pending::Release {
            session: id.clone(),
            machine: machine.clone(),
            channel: record.channel.clone(),
            operation: operation.clone(),
            restored: Some(restored),
            kept: None,
        };
        self.keep(&journal).await;
        // An archive this restore undoes has nothing left to learn; a check
        // still running finds the record here and leaves it (D222).
        self.settled(&Pending::Reconcile {
            session: id.clone(),
            machine: machine.clone(),
            reported: Vec::new(),
            by: pb::Actor::default(),
            asked: None,
        })
        .await;
        // Switch: the new file repeats exactly the entries of the lines
        // written, so the mirror adds only what follows (D211).
        let here = self.machine.clone();
        self.rebase_mirror(&id, &here, Some(humata2_mirror::entry_count(&lines))).await;
        let (store, saved, saved_detail, key) = (self.starter.store.clone(), record.clone(), detail.clone(), id.clone());
        let switched = blocking(move || -> Result<(), StoreError> {
            store.put_session(&saved)?;
            store.put_session_detail(&saved_detail)?;
            store.remove_archive(&key)
        })
        .await;
        let failure = match switched {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error_of(&error, "fix the cause the message names, then restore again", None)),
            Err(refused) => Some(error_from(&refused)),
        };
        if let Some(failure) = failure {
            self.settled(&journal).await;
            self.rebase_mirror(&id, &machine, None).await;
            progress.fail(failure);
            return;
        }
        drop(guard);
        if let Some(remote) = &self.remote {
            remote.forget_session(&machine, &id).await;
        }
        tokio::spawn(self.clone().release_when_linked(journal));
        self.tell.session(&id, pb::Change::Updated, Some(by.clone())).await;
        publish_family(&self.tell, &id, Some(by.clone())).await;

        progress.set_step("launch", pb::StepState::Running, None);
        if let Err(failure) = self.resume_here_from(&record, lines, &old_folder).await {
            progress.set_step("launch", pb::StepState::Failed, Some(failure.message.clone()));
            progress.fail(failure);
            self.tell.session(&id, pb::Change::Updated, Some(by)).await;
            return;
        }
        progress.set_step("launch", pb::StepState::Done, None);
        progress.set_step("verify", pb::StepState::Done, None);
        progress.succeed(Some(pb::operation::Result::Session(pb::SessionId { value: id.clone() })));
        self.tell.session(&id, pb::Change::Updated, Some(by)).await;
        watch::agent(self.tell.clone(), id);
    }

    /// After a restore here while `machine` was away: the next time its link
    /// comes up, the host compares the machine's copy with what it restored
    /// (D224). With nothing more there, the copy goes to its trash, its
    /// records are dropped, and what it reported of the session meanwhile is
    /// forgotten. A copy with more, or one that couldn't be compared, is
    /// kept, and the next link compares again. Tries again on each new link
    /// while the call fails for the link; ends once the machine is forgotten
    /// or has no copy, and `journal` goes then. Nothing is released unless
    /// the host's record names this Mac: a restore cut short before its
    /// switch left the session on the machine.
    async fn release_when_linked(self, mut journal: Pending) {
        let Pending::Release {
            session,
            machine,
            channel,
            operation,
            restored,
            ..
        } = journal.clone()
        else {
            return;
        };
        let Some(carry) = self.carry.get().cloned() else {
            return;
        };
        let links = carry.links().clone();
        let mut stale = None;
        while let Some(generation) = next_link_up(&links, &machine, stale).await {
            match self.load(&session).await {
                Ok(loaded) if loaded.record.machine == self.machine => {}
                Ok(_) => break,
                Err(refused) if refused.code() == Code::NotFound => break,
                Err(refused) => {
                    crate::log::say!(
                        "humatad2: session {session}'s old copy on {machine} is still there, because its record here couldn't be read: {}; Humata tries again when {machine} is next linked or Humata next starts",
                        refused.message()
                    );
                    stale = Some(generation);
                    continue;
                }
            }
            match self.compare_copy(&session, &machine, restored.as_ref()).await {
                Compared::Gone => break,
                Compared::NothingMore => {}
                Compared::More(more) => {
                    crate::log::say!(
                        "humatad2: session {session} runs here now, but its old copy on {machine} has more than was restored here: {more}. The copy on {machine} is kept until that can be brought over; Humata compares again when {machine} is next linked or Humata next starts"
                    );
                    if let Pending::Release { kept, .. } = &mut journal {
                        *kept = Some(more.to_string());
                    }
                    self.keep(&journal).await;
                    stale = Some(generation);
                    continue;
                }
            }
            match carry.release(&machine, &operation, &session, &channel).await {
                Ok(()) => {
                    // A report it sent before letting go isn't the session's.
                    if let Some(remote) = &self.remote {
                        remote.forget_session(&machine, &session).await;
                    }
                    break;
                }
                Err(refused) if matches!(refused.code(), Code::Unavailable | Code::Unknown | Code::Cancelled) => {
                    stale = Some(generation);
                }
                Err(refused) => {
                    crate::log::say!(
                        "humatad2: session {session} runs here now, but its old copy on {machine} is still there: {}; Humata tries again when {machine} is next linked or Humata next starts",
                        refused.message()
                    );
                    stale = Some(generation);
                }
            }
        }
        self.settled(&journal).await;
    }

    /// Compares `machine`'s copy of `session` with what a restore here
    /// brought (D224). Whatever can't be learned counts as more.
    async fn compare_copy(&self, session: &str, machine: &str, restored: Option<&Restored>) -> Compared {
        let summary = match self.relayed_to(session, machine) {
            Ok(relayed) => {
                let request = pb::GetSessionRequest {
                    id: Some(pb::SessionId { value: session.to_owned() }),
                };
                match relayed.session().get_session(tonic::Request::new(request)).await {
                    Ok(shown) => shown.into_inner().archive,
                    // The machine holds no copy: nothing is there to keep.
                    Err(refused) if refused.code() == Code::NotFound => return Compared::Gone,
                    Err(_) => None,
                }
            }
            Err(_) => None,
        };
        let theirs = Theirs {
            turns: self.machine_turns(session, machine).await,
            summary,
        };
        let restored = restored.cloned();
        match blocking(move || compare::compare(restored.as_ref(), &theirs, compare::holds)).await {
            Ok(None) => Compared::NothingMore,
            Ok(Some(more)) => Compared::More(more),
            Err(_) => Compared::More(More(vec![compare::Extra::TurnsUnknown, compare::Extra::NoSummary])),
        }
    }

    /// The turns of `machine`'s conversation of `session`, read as the
    /// host's mirror reads it (ReadMirror) and by the rule the restore's are
    /// counted with; `None` when it can't be read whole.
    async fn machine_turns(&self, session: &str, machine: &str) -> Option<Vec<humata2_mirror::TurnMark>> {
        let links = self.links_to(session, machine).ok()?;
        let mut decoder = humata2_chunk::Decoder::new();
        let (mut lines, mut offset, mut source) = (Vec::new(), 0_u64, String::new());
        let mut rewrites = 0_u32;
        loop {
            let channel = links.channel(machine).ok()?;
            let request = pb::ReadMirrorRequest {
                session: Some(pb::SessionId { value: session.to_owned() }),
                offset,
                max_bytes: 0,
                source: source.clone(),
                // Raw bytes only.
                accept_codecs: Vec::new(),
            };
            let answer = pb::history_service_client::HistoryServiceClient::new(channel)
                .read_mirror(request)
                .await
                .ok()?
                .into_inner();
            let header = crate::compression::header(answer.codec, answer.raw_size, answer.crc32, &answer.data)?;
            let (more, rewritten, next_offset) = (answer.more, answer.rewritten, answer.next_offset);
            let data = decoder.decode(header, answer.data).ok()?;
            if rewritten {
                // Rewritten twice while it was read: it isn't settled, so
                // it can't be compared now.
                rewrites += 1;
                if rewrites > 1 {
                    return None;
                }
                lines.clear();
            } else if more && next_offset <= offset {
                return None;
            }
            lines.extend_from_slice(&data);
            source = answer.source;
            if !more {
                break;
            }
            offset = next_offset;
        }
        blocking(move || humata2_mirror::turns_in(&lines)).await.ok()
    }

    /// The machine's copy of the session goes to its trash and its records
    /// are dropped, now that the session runs here; the staged files here go
    /// too. A failure leaves the old copy where it is and is logged.
    async fn release_old_copy(&self, machine: &str, id: &str, record: &SessionRecord, operation: &str) {
        let Some(carry) = self.carry.get() else {
            return;
        };
        if let Err(error) = carry.release(machine, operation, id, &record.channel).await {
            crate::log::say!(
                "humatad2: session {id} runs here now, but its old copy on {machine} is still there: {}",
                error.message()
            );
        }
        self.unstage_here(record, operation).await;
    }

    /// Removes the files a restore staged on this machine.
    async fn unstage_here(&self, record: &SessionRecord, operation: &str) {
        let Some(carry) = self.carry.get() else {
            return;
        };
        let place = Place {
            machine: self.machine.clone(),
            channel: record.channel.clone(),
            folder: String::new(),
        };
        let name = record
            .folder
            .file_name()
            .map(|name| format!("sessions/{}", name.to_string_lossy()))
            .unwrap_or_default();
        if let Err(error) = carry.abort(&self.machine, operation, &record.id, &place, &name, false).await {
            crate::log::say!(
                "humatad2: couldn't remove the files staged here for restoring session {}: {}",
                record.id,
                error.message()
            );
        }
    }

    /// Takes back what taking the conversation in put on this machine.
    async fn undo_conversation(&self, record: &SessionRecord, operation: &str) {
        let Some(carry) = self.carry.get() else {
            return;
        };
        let name = record
            .folder
            .file_name()
            .map(|name| format!("sessions/{}", name.to_string_lossy()))
            .unwrap_or_default();
        let place = Place {
            machine: self.machine.clone(),
            channel: record.channel.clone(),
            folder: String::new(),
        };
        let _ = carry
            .abort(&self.machine, operation, &record.id, &place, &name, true)
            .await;
        let _ = carry
            .abort(&record.machine, operation, &record.id, &place, &name, false)
            .await;
    }

    /// Brings session `record`'s records and conversation from `machine`
    /// here as a move does: the machine stages them (its folder was freed, so
    /// only the records and the agent's files), they are copied across and
    /// taken in, with the agent saved as stopped and ready to resume.
    async fn take_conversation(&self, record: &SessionRecord, machine: &str, operation: &str) -> Result<(), Status> {
        let Some(carry) = self.carry.get().cloned() else {
            return Err(self.links_to(&record.id, machine).err().unwrap_or_else(|| {
                status::internal("the carry driver is missing", "")
            }));
        };
        let channel = self.find_channel(&record.channel).await?;
        let at = |machine: &str| Place {
            machine: machine.to_owned(),
            channel: channel.id.clone(),
            folder: channel.folder.clone(),
        };
        let (from, here) = (at(machine), at(&self.machine));
        let packed = carry.pack(machine, operation, &record.id).await?;
        carry
            .copy(
                &format!("{operation}_stage"),
                (&from, &packed.stage),
                (&here, &format!("{}/{operation}/in", humata2_sessions::CARRY_DIR)),
            )
            .await?;
        carry
            .unpack(&self.machine, operation, &record.id, &channel.id)
            .await
            .map(drop)
    }
}

/// What the comparison of a machine's copy with a restore here found.
enum Compared {
    /// The machine holds no copy of the session.
    Gone,
    /// Nothing there that the restore didn't bring: the copy may go.
    NothingMore,
    /// More there, or something couldn't be learned: the copy stays.
    More(More),
}

/// What a machine's removal does with one of its sessions (CH-8, LD-10).
#[derive(Clone)]
pub(crate) enum Removal {
    /// Move it to this machine of the channel.
    MoveTo(String),
    /// Archive it as the user chose in the plan.
    Archive(pb::ArchiveChoice),
}

/// The error a failed call or operation says, whole.
fn error_from(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: "try again".to_owned(),
        retry: None,
        kind: None,
    })
}

impl SessionSvc {
    /// Moves or archives one session of a machine that is being removed, as
    /// a user who chose it in the removal's dialog would, and waits for the
    /// operation to end. The access rules of the call that asked for the
    /// removal already ran, so the moves and archives here are approved.
    pub(crate) async fn dispose_for_removal(
        &self,
        caller: &Caller,
        session: &str,
        how: Removal,
    ) -> Result<(), pb::Error> {
        let id = Some(pb::SessionId {
            value: session.to_owned(),
        });
        let asked = async {
            match how {
                Removal::MoveTo(to) => {
                    let request = pb::MoveSessionRequest {
                        idempotency_key: None,
                        id,
                        to: Some(pb::MachineId { value: to }),
                    };
                    self.move_session_of(caller, request, true).await
                }
                Removal::Archive(choice) => {
                    let plan = match self
                        .plan_archive_of(caller, pb::PlanArchiveRequest { id: id.clone() }, true)
                        .await?
                    {
                        Gated::Done(plan) => plan,
                        Gated::Ask(_) => return Err(asked_twice()),
                    };
                    let request = pb::ArchiveSessionRequest {
                        idempotency_key: None,
                        id,
                        choice: choice as i32,
                        plan: plan.token,
                    };
                    self.archive_session_of(caller, request, true).await
                }
            }
        }
        .await
        .map_err(|status| error_from(&status))?;
        let started = match asked {
            Gated::Done(operation) => operation,
            Gated::Ask(_) => return Err(error_from(&asked_twice())),
        };
        let operation = started.id.map(|id| id.value).unwrap_or_default();
        let mut watching = self
            .operations
            .progress_of(caller, &operation)
            .map_err(|status| error_from(&status))?
            .watch();
        let ended = loop {
            let now = watching.borrow_and_update().clone();
            if now.state != pb::OperationState::Running as i32 {
                break now;
            }
            if watching.changed().await.is_err() {
                break now;
            }
        };
        if ended.state == pb::OperationState::Succeeded as i32 {
            return Ok(());
        }
        Err(ended.error.unwrap_or_else(|| pb::Error {
            message: format!("session {session}'s work ended without saying how it went"),
            fix: "look at the session, then try the removal again".to_owned(),
            retry: None,
            kind: None,
        }))
    }
}

fn asked_twice() -> Status {
    status::internal(
        "an approved call was sent to Ask me a second time",
        "repeat the call",
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use humata2_store::{SessionDetail, SessionRepo, StartStep};

    use super::*;

    fn loaded() -> Loaded {
        Loaded {
            record: SessionRecord {
                id: "se_1".to_owned(),
                channel: "ch_1".to_owned(),
                machine: "box-1".to_owned(),
                title: "Cache warmup".to_owned(),
                agent: "claude".to_owned(),
                folder: PathBuf::from("/work/se_1"),
                created_unix_ms: 0,
            },
            detail: Some(SessionDetail {
                session: "se_1".to_owned(),
                repos: vec![SessionRepo {
                    owner: "acme".to_owned(),
                    name: "payments".to_owned(),
                    branch: "cache-warmup".to_owned(),
                    path: PathBuf::from("/work/se_1/payments"),
                }],
                model: None,
                first_message: None,
                start: StartStep::Done,
                operation: None,
                revision: 0,
            }),
            agent: None,
            summary_source: Default::default(),
        }
    }

    fn pointer(subject: &str) -> pb::Pointer {
        pb::Pointer {
            repo: Some(pb::RepoRef {
                owner: "acme".to_owned(),
                name: "payments".to_owned(),
            }),
            unpushed_subjects: vec![subject.to_owned()],
            ..Default::default()
        }
    }

    fn subjects(theirs: &[pb::Pointer], reported: &[pb::Pointer]) -> Vec<String> {
        let summary = host_summary(&loaded(), Some(pb::ArchiveSummary::default()), pointers_of(theirs, reported), 0);
        summary.repos[0].unpushed_subjects.clone()
    }

    #[test]
    fn the_machines_own_pointers_win_over_what_it_last_reported() {
        assert_eq!(subjects(&[pointer("Warm on start")], &[pointer("Older work")]), ["Warm on start"]);
    }

    #[test]
    fn what_the_machine_last_reported_fills_in_when_its_answer_has_no_pointers() {
        assert_eq!(subjects(&[], &[pointer("Older work")]), ["Older work"]);
    }
}
