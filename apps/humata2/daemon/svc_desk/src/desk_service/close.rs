//! CloseDeskSession and ReopenDeskSession (DK-11, D182.4). Close stops the
//! agent and the session's shells and keeps everything else; Reopen clears
//! the closed mark and resumes the agent with its conversation in the same
//! folder.

use std::time::SystemTime;

use humata2_access::SessionId as AccessSessionId;
use crate::pb;
use humata2_sessions::doc_repo::{self, DocGit};
use humata2_store::{AgentState, DeskFolderKind, TabKind};
use tonic::Status;

use super::view;
use super::{DeskSvc, failed_precondition, now_ms, session_id};
use crate::ports::blocking;
use crate::ports::holder_end;
use humata2_daemon_ports::Caller;
use crate::ports as identity;
use crate::ports::{Started, Progress};
use crate::ports::Loaded;
use crate::ports as status;

impl DeskSvc {
    /// Stops the agent and every shell of the session, then marks it closed
    /// (DK-11). Its record, conversation, scrollback and folder stay, and so
    /// do the agent's tab record and folder; its standing grants end.
    pub(super) async fn close(
        &self,
        caller: &Caller,
        request: pb::CloseDeskSessionRequest,
    ) -> Result<pb::DeskSession, Status> {
        let id = session_id(request.session)?;
        let _held = self.locks.hold(&id).await;
        let (session, mut row) = self.load(&id).await?;
        if row.closed_unix_ms.is_none() {
            if let Some(machine) = self.elsewhere(&session) {
                // The machine stops the agent and its shells; the host only
                // notes that the session is closed.
                let shown = self.close_there(&machine, &id).await?;
                // The machine checks for a version before it closes, as this
                // one does (DV-4): its answer says whether the folder is kept.
                if matches!(row.kind, DeskFolderKind::Scratch { kept: false })
                    && matches!(
                        shown.folder.as_ref().and_then(|folder| folder.kind.as_ref()),
                        Some(pb::desk_folder::Kind::Scratch(pb::desk_folder::Scratch { kept: true }))
                    )
                {
                    row.kind = DeskFolderKind::Scratch { kept: true };
                }
                let now = now_ms();
                row.closed_unix_ms = Some(now);
                row.last_used_unix_ms = now;
                self.save_row(&row).await?;
                self.end_grants(&id).await?;
                self.tell
                    .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
                    .await;
                self.wake.notify_one();
                return Ok(shown);
            }
            self.stop_agent(&id).await?;
            self.stop_shells(&id).await?;
            // A folder that holds a version is kept for good (DV-4, D451).
            if matches!(row.kind, DeskFolderKind::Scratch { kept: false })
                && self.holds_version(&session.folder).await
            {
                row.kind = DeskFolderKind::Scratch { kept: true };
            }
            let now = now_ms();
            row.closed_unix_ms = Some(now);
            row.last_used_unix_ms = now;
            self.save_row(&row).await?;
            self.end_grants(&id).await?;
            self.tell
                .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
                .await;
            // A closed scratch folder may be due at once: let the sweep look.
            self.wake.notify_one();
        }
        Ok(view::desk_pb(&session, &row))
    }

    /// Whether the folder is a versions repository holding a version (DV-4).
    /// A folder that isn't one, or that git can't be asked about, answers
    /// as the check can: an error is logged and counts as a version, so a
    /// folder that can't be checked is never lost.
    pub(super) async fn holds_version(&self, folder: &std::path::Path) -> bool {
        let folder = folder.to_owned();
        let asked = blocking(move || {
            if !doc_repo::is_doc_repo(&folder) {
                return Ok(false);
            }
            doc_repo::has_version(&DocGit::inherit(), &folder).map_err(|error| (folder, error))
        })
        .await;
        match asked {
            Ok(Ok(has)) => has,
            Ok(Err((folder, error))) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: couldn't check whether {} holds a version, so it is kept: {error}",
                    folder.display()
                );
                true
            }
            Err(error) => {
                humata2_rpc_kit::log::say!(
                    "humatad2: couldn't check a Desk folder for a version, so it is kept: {}",
                    error.message()
                );
                true
            }
        }
    }

    /// Reopens a Desk session: its folder must still be there, then its
    /// agent resumes with its conversation (DK-11). The operation follows
    /// the resume, as a channel session's does.
    pub(super) async fn reopen(
        &self,
        caller: &Caller,
        request: pb::ReopenDeskSessionRequest,
    ) -> Result<pb::Operation, Status> {
        let id = session_id(request.session)?;
        let _held = self.locks.hold(&id).await;
        let (session, mut row) = self.load(&id).await?;
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(operation);
        }
        if !session.folder.is_dir() {
            return Err(failed_precondition(
                format!(
                    "the folder {} of Desk session {id} is gone, so the session can't be reopened",
                    session.folder.display()
                ),
                "put the folder back where it was; a Desk session needs its folder to run, so it stays closed until then",
            ));
        }
        let steps: Vec<(String, String)> = vec![
            ("launch".to_owned(), "Starting the agent again".to_owned()),
            (
                "verify".to_owned(),
                "Checking it took up its conversation".to_owned(),
            ),
        ];
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            request.idempotency_key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(operation),
        };
        let operation = progress.operation();
        row.closed_unix_ms = None;
        row.last_used_unix_ms = now_ms();
        self.resume_after_reopen(caller, &id, &row, progress).await?;
        self.tell
            .session(&id, pb::Change::Updated, Some(identity::actor(caller)))
            .await;
        Ok(operation)
    }

    /// Saves the reopened row, then hands the agent's resume to the same
    /// work a resumed channel session runs. A failure before the hand-off
    /// fails the operation it was given.
    pub(super) async fn resume_after_reopen(
        &self,
        caller: &Caller,
        id: &str,
        row: &humata2_store::DeskRow,
        progress: Progress,
    ) -> Result<(), Status> {
        let saved = self.save_row(row).await;
        self.resume_saved(caller, id, saved, progress).await
    }

    /// The resume Reopen and a restored channel Desk session share: reads
    /// the session back and hands its agent's resume to the same work a
    /// resumed channel session runs. `saved` is what saving the reopened
    /// row answered; a failure of it, or before the hand-off, fails the
    /// operation it was given.
    pub(super) async fn resume_saved(
        &self,
        caller: &Caller,
        id: &str,
        saved: Result<(), Status>,
        progress: Progress,
    ) -> Result<(), Status> {
        let prepared: Result<Loaded, Status> = async {
            saved?;
            let (store, key) = (self.starter.store.clone(), id.to_owned());
            let loaded = blocking(move || Loaded::read(&store, &key))
                .await?
                .map_err(|error| {
                    crate::ports::store_error(error, &self.machine, "read the Desk session to reopen")
                })?;
            loaded.ok_or_else(|| {
                status::not_found(
                    pb::EntityKind::Session,
                    id,
                    format!("no Desk session {id} is on this machine"),
                    "list the Desk's sessions to see their ids",
                )
            })
        }
        .await;
        match prepared {
            Ok(loaded) => {
                let (sessions, by) = (self.sessions.clone(), identity::actor(caller));
                tokio::spawn(async move { sessions.run_resume(by, loaded, progress).await });
                Ok(())
            }
            Err(error) => {
                progress.fail(humata2_rpc_kit::helpers::status_error_of(&error).unwrap_or_default());
                Err(error)
            }
        }
    }

    /// Stops the session's agent the way Stop does (SE-14): its holder is
    /// shut and the agent is saved as stopped, so nothing resumes it until
    /// Reopen. An agent still starting or resuming is not stopped now.
    pub(super) async fn stop_agent(&self, id: &str) -> Result<(), Status> {
        let (store, key) = (self.starter.store.clone(), id.to_owned());
        let agent = blocking(move || store.agent(&key))
            .await?
            .map_err(|error| {
                crate::ports::store_error(error, &self.machine, "read the Desk session's agent")
            })?;
        let Some(mut agent) = agent else {
            return Ok(());
        };
        let holder_pid = match agent.state {
            AgentState::Launching => {
                return Err(failed_precondition(
                    format!("Desk session {id}'s agent is still starting, so it can't be closed yet"),
                    "wait until the agent is running, then close the session again",
                ));
            }
            AgentState::Resuming => {
                return Err(failed_precondition(
                    format!("Desk session {id}'s agent is being resumed, so it can't be closed yet"),
                    "wait until the agent is running, then close the session again",
                ));
            }
            AgentState::Running { holder_pid } => holder_pid,
            AgentState::Exited { .. } | AgentState::Stopped | AgentState::ResumeFailed { .. } => {
                return Ok(());
            }
        };
        // A holder already gone is as stopped as one that was asked to.
        if let Err(error) = holder_end::shut_or_end(
            &agent.launch.holder_socket,
            holder_pid,
            &format!("Desk session {id}'s terminal"),
        )
        .await
        {
            humata2_rpc_kit::log::say!("humatad2: {error}");
        }
        self.terminals.forget_tab(&agent.tab);
        agent.state = AgentState::Stopped;
        agent.updated_unix_ms = now_ms();
        let store = self.starter.store.clone();
        blocking(move || store.put_agent(&agent))
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "save that the agent stopped"))
    }

    /// Shuts every shell of the session and forgets it: a closed session
    /// has no shell for restart to bring back (DK-11). Each shell's session
    /// token is revoked.
    pub(super) async fn stop_shells(&self, id: &str) -> Result<(), Status> {
        let store = self.starter.store.clone();
        let tabs = blocking(move || store.tabs())
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "list the session's tabs"))?;
        for tab in tabs
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(id) && tab.kind == TabKind::Shell)
        {
            if let Err(error) = holder_end::shut_or_end(
                &tab.holder_socket,
                tab.holder_pid,
                &format!("Desk session {id}'s shell {}", tab.id),
            )
            .await
            {
                humata2_rpc_kit::log::say!("humatad2: {error}");
            }
            self.terminals.forget_tab(&tab.id);
            let (store, tab_id) = (self.starter.store.clone(), tab.id.clone());
            blocking(move || store.remove_tab(&tab_id))
                .await?
                .map_err(|error| crate::ports::store_error(error, &self.machine, "forget the closed shell"))?;
            let (identities, tab_id) = (self.identities.clone(), tab.id.clone());
            let _ = blocking(move || identities.revoke(&tab_id)).await;
        }
        Ok(())
    }

    /// Ends the session's standing grants in the policy and in the store,
    /// and withdraws the Inbox requests it was waiting on (IN-22, TP-15).
    pub(super) async fn end_grants(&self, id: &str) -> Result<(), Status> {
        let ended = self.guard.end_grants_of(&AccessSessionId::new(id));
        for grant in ended {
            self.inbox
                .revoked(&grant.granted_from.to_string(), SystemTime::now())
                .await?;
        }
        self.inbox
            .withdraw_pending_of(id, pb::WithdrawReason::SessionStopped)
            .await
    }
}
