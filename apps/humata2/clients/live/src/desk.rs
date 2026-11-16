//! [`DeskClient`] for the live daemon, over `humata.v1.DeskService`. The
//! Desk's sessions, their repositories and documents load with the
//! workspace ([`Shared::load_desk`]). Moves to another machine are not in
//! this release (D182.2) and still answer [`DaemonError::NotServedYet`].

use std::time::{SystemTime, UNIX_EPOCH};

use humata2_clients::desk::{
    Access, CleanUp, CleanUpRow, Closed, DeskClient, DeskData, DeskSession, Document, Folder,
    FolderChoice, LastUsed, NewDeskSession,
};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{DaemonError, Reply};
use humata2_grpc::humata::v1 as pb;

use crate::connection::{LiveDaemon, Shared, fresh_key, not_in_this_release};
use crate::daemon::bad_repo;
use crate::map;

const DAY_MS: i64 = 86_400_000;

/// The last Clean up list this client answered, with the token that names
/// it. `CleanUpDesk` checks the token before it deletes anything (NV-27).
#[derive(Clone, Debug)]
pub(crate) struct HeldPlan {
    pub(crate) token: pb::PlanToken,
    pub(crate) rows: Vec<CleanUpRow>,
}

/// An answer that is not in this release.
fn later<T: Send + 'static>(what: &'static str) -> Reply<T> {
    Box::pin(async move { Err::<T, DaemonError>(not_in_this_release(what)) })
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// How long ago, in milliseconds, in the words the boards use.
fn ago(now: i64, then: i64) -> String {
    crate::boards::ago(now / 1_000, then / 1_000)
}

/// A repository written `owner/name`, or the refusal that says how to write it.
fn repo_wire(text: &str) -> Result<pb::RepoRef, DaemonError> {
    map::parse_repo(text).ok_or_else(|| bad_repo(text))
}

/// Whole days from `now` until `until`, rounded up; 0 once it has passed.
fn days_until(until: i64, now: i64) -> u32 {
    let left = until.saturating_sub(now).max(0);
    u32::try_from(left.saturating_add(DAY_MS - 1) / DAY_MS).unwrap_or(u32::MAX)
}

fn session_wire(session: &SessionId) -> pb::SessionId {
    pb::SessionId {
        value: session.as_str().to_owned(),
    }
}

fn access_wire(access: Access) -> pb::DeskAccess {
    pb::DeskAccess {
        boards: access.boards,
        channels_and_machines: access.channels_and_machines,
    }
}

/// One Desk session as the app's model. `None` when the daemon named no
/// session.
fn desk_session(wire: &pb::DeskSession, now: i64) -> Option<DeskSession> {
    let id = map::session_id(wire.session.as_ref());
    if id.as_str().is_empty() {
        return None;
    }
    let folder = wire.folder.as_ref();
    let path = folder.map(|folder| folder.path.clone()).unwrap_or_default();
    let folder = match folder.and_then(|folder| folder.kind.as_ref()) {
        Some(pb::desk_folder::Kind::Scratch(scratch)) => Folder::Scratch {
            path,
            kept: scratch.kept,
        },
        // A folder nobody made, or one the daemon gave no kind for, is
        // never deleted: the safe reading.
        Some(pb::desk_folder::Kind::Picked(_)) | None => Folder::Picked { path },
    };
    let access = wire.access.as_ref().map_or_else(Access::default, |access| Access {
        boards: access.boards,
        channels_and_machines: access.channels_and_machines,
    });
    let last_used = match wire.last_used_at.as_ref() {
        Some(at) => {
            let ms = map::unix_ms(at.seconds, at.nanos);
            LastUsed {
                order: u64::try_from(ms).unwrap_or(0),
                label: ago(now, ms),
            }
        }
        None => LastUsed {
            order: 0,
            label: String::new(),
        },
    };
    let closed = wire.closed_at.as_ref().map(|at| Closed {
        when: ago(now, map::unix_ms(at.seconds, at.nanos)),
        days_left: wire
            .delete_at
            .as_ref()
            .map(|at| days_until(map::unix_ms(at.seconds, at.nanos), now)),
    });
    Some(DeskSession {
        session: id,
        folder,
        repos: wire.repos.iter().map(map::repo_text).collect(),
        access,
        last_used,
        closed,
        unpushed: (!wire.held_work.is_empty()).then(|| wire.held_work.clone()),
    })
}

fn clean_up_row(row: &pb::DeskCleanUpRow) -> CleanUpRow {
    CleanUpRow {
        session: map::session_id(row.session.as_ref()),
        size_bytes: row.size_bytes,
        unpushed: (!row.unpushed.is_empty()).then(|| row.unpushed.clone()),
    }
}

impl Shared {
    /// Every Desk session on this machine, open and closed. A daemon that
    /// doesn't serve the Desk yet has none, rather than failing the load.
    pub(crate) async fn load_desk(&self) -> Result<DeskData, DaemonError> {
        let reply = match self
            .desk()
            .list_desk_sessions(pb::ListDeskSessionsRequest {})
            .await
        {
            Ok(reply) => reply.into_inner(),
            Err(status) if status.code() == tonic::Code::Unimplemented => {
                return Ok(DeskData::default());
            }
            Err(status) => return Err(self.error(&status, "The Desk")),
        };
        let now = now_ms();
        let sessions = reply
            .sessions
            .iter()
            .filter_map(|wire| desk_session(wire, now))
            .map(|session| (session.session.clone(), session))
            .collect();
        // The markdown files of each folder, in the order the sessions list
        // them, then by name as the daemon sorts them.
        let documents = reply
            .sessions
            .iter()
            .filter_map(|wire| {
                let session = map::session_id(wire.session.as_ref());
                (!session.as_str().is_empty()).then(|| {
                    wire.documents.iter().map(move |path| Document {
                        session: session.clone(),
                        path: path.clone(),
                    })
                })
            })
            .flatten()
            .collect();
        Ok(DeskData {
            sessions,
            documents,
            ..DeskData::default()
        })
    }

    async fn add_desk_repo(&self, session: &SessionId, repo: &str) -> Result<(), DaemonError> {
        const WHAT: &str = "Adding a repository to a desk session";
        let repo = repo_wire(repo)?;
        self.desk()
            .add_desk_repo(pb::AddDeskRepoRequest {
                session: Some(session_wire(session)),
                repo: Some(repo),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        Ok(())
    }

    async fn remove_desk_repo(&self, session: &SessionId, repo: &str) -> Result<(), DaemonError> {
        const WHAT: &str = "Removing a repository from a desk session";
        let repo = repo_wire(repo)?;
        self.desk()
            .remove_desk_repo(pb::RemoveDeskRepoRequest {
                session: Some(session_wire(session)),
                repo: Some(repo),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        Ok(())
    }

    async fn copy_desk_document(
        &self,
        document: &Document,
        channel: &ChannelId,
        machine: Option<&MachineId>,
    ) -> Result<(), DaemonError> {
        const WHAT: &str = "Copying a document to a channel";
        self.desk()
            .copy_desk_document(pb::CopyDeskDocumentRequest {
                idempotency_key: fresh_key(),
                session: Some(session_wire(&document.session)),
                path: document.path.clone(),
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
                machine: machine.map(|machine| pb::MachineId {
                    value: machine.as_str().to_owned(),
                }),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        Ok(())
    }

    async fn start_desk(&self, new: NewDeskSession) -> Result<SessionId, DaemonError> {
        const WHAT: &str = "Starting a desk session";
        let repos = new
            .repos
            .iter()
            .map(|text| repo_wire(text))
            .collect::<Result<Vec<_>, _>>()?;
        // A scratch folder is named by the daemon from the title, so no name
        // is sent for it.
        let picked_path = match new.folder {
            FolderChoice::Picked { path } => path,
            FolderChoice::Scratch { .. } => String::new(),
        };
        let reply = self
            .desk()
            .start_desk_session(pb::StartDeskSessionRequest {
                idempotency_key: fresh_key(),
                title: new.title,
                agent: Some(pb::AgentId {
                    value: self.agents.name(new.agent),
                }),
                model: String::new(),
                machine: Some(pb::MachineId {
                    value: new.machine.as_str().to_owned(),
                }),
                picked_path,
                repos,
                access: Some(access_wire(new.access)),
                first_message: String::new(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        let id = map::session_id(reply.session.as_ref());
        if id.as_str().is_empty() {
            return Err(DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon didn't name the session it started. Reload to see whether it exists".to_owned(),
            });
        }
        Ok(id)
    }

    async fn close_desk(&self, session: &SessionId) -> Result<(), DaemonError> {
        const WHAT: &str = "Closing a desk session";
        self.desk()
            .close_desk_session(pb::CloseDeskSessionRequest {
                idempotency_key: fresh_key(),
                session: Some(session_wire(session)),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?;
        Ok(())
    }

    async fn reopen_desk(&self, session: &SessionId) -> Result<(), DaemonError> {
        const WHAT: &str = "Reopening a desk session";
        let operation = self
            .desk()
            .reopen_desk_session(pb::ReopenDeskSessionRequest {
                idempotency_key: fresh_key(),
                session: Some(session_wire(session)),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        self.finish(operation, WHAT).await.map(|_| ())
    }

    async fn keep_desk(&self, session: &SessionId) -> Result<(), DaemonError> {
        self.desk()
            .keep_desk_folder(pb::KeepDeskFolderRequest {
                session: Some(session_wire(session)),
            })
            .await
            .map_err(|status| self.error(&status, "Keeping a desk session's folder"))?;
        Ok(())
    }

    async fn set_desk_access(&self, session: &SessionId, access: Access) -> Result<(), DaemonError> {
        self.desk()
            .set_desk_access(pb::SetDeskAccessRequest {
                session: Some(session_wire(session)),
                access: Some(access_wire(access)),
            })
            .await
            .map_err(|status| self.error(&status, "Changing a desk session's access"))?;
        Ok(())
    }

    /// Keeps the plan the daemon answered, so a clean-up can name it, and
    /// returns its rows.
    fn hold_plan(
        &self,
        plan: pb::DeskCleanUpPlan,
        what: &str,
    ) -> Result<Vec<CleanUpRow>, DaemonError> {
        let token = plan.token.ok_or_else(|| DaemonError::Refused {
            what: what.to_owned(),
            reason: "the daemon didn't give a token for the list. Reload and try again".to_owned(),
        })?;
        let rows: Vec<CleanUpRow> = plan.rows.iter().map(clean_up_row).collect();
        self.state().desk_plan = Some(HeldPlan {
            token,
            rows: rows.clone(),
        });
        Ok(rows)
    }

    async fn plan_clean_up(&self) -> Result<Vec<CleanUpRow>, DaemonError> {
        const WHAT: &str = "Listing closed desk sessions";
        let plan = self
            .desk()
            .plan_desk_clean_up(pb::PlanDeskCleanUpRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        self.hold_plan(plan, WHAT)
    }

    async fn clean_up_desk(
        &self,
        shown: Vec<CleanUpRow>,
        delete: Vec<SessionId>,
    ) -> Result<CleanUp, DaemonError> {
        const WHAT: &str = "Cleaning up closed desk sessions";
        let held = self.state().desk_plan.clone();
        let Some(held) = held.filter(|held| held.rows == shown) else {
            // The list on screen is not the one held, so nothing is deleted:
            // the list now goes back to ask again (NV-27).
            return Ok(CleanUp::Changed(self.plan_clean_up().await?));
        };
        let response = self
            .desk()
            .clean_up_desk(pb::CleanUpDeskRequest {
                idempotency_key: fresh_key(),
                plan: Some(held.token),
                delete: delete.iter().map(session_wire).collect(),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        match response.result {
            Some(pb::clean_up_desk_response::Result::Deleted(done)) => {
                self.state().desk_plan = None;
                Ok(CleanUp::Deleted {
                    sessions: usize::try_from(done.sessions).unwrap_or(usize::MAX),
                    freed_bytes: done.freed_bytes,
                })
            }
            Some(pb::clean_up_desk_response::Result::Changed(plan)) => {
                Ok(CleanUp::Changed(self.hold_plan(plan, WHAT)?))
            }
            None => Err(DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon answered neither a deletion nor the list now. Reload to see what is left".to_owned(),
            }),
        }
    }
}

impl DeskClient for LiveDaemon {
    fn start(&self, new: NewDeskSession) -> Reply<SessionId> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.start_desk(new).await })
    }

    fn close(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.close_desk(&session).await })
    }

    fn reopen(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.reopen_desk(&session).await })
    }

    fn keep(&self, session: &SessionId) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.keep_desk(&session).await })
    }

    fn clean_up_list(&self) -> Reply<Vec<CleanUpRow>> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.plan_clean_up().await })
    }

    fn clean_up(&self, shown: Vec<CleanUpRow>, delete: Vec<SessionId>) -> Reply<CleanUp> {
        let shared = self.shared.clone();
        Box::pin(async move { shared.clean_up_desk(shown, delete).await })
    }

    fn copy_document(
        &self,
        document: &Document,
        channel: &ChannelId,
        machine: Option<&MachineId>,
    ) -> Reply<()> {
        let shared = self.shared.clone();
        let (document, channel) = (document.clone(), channel.clone());
        let machine = machine.cloned();
        Box::pin(async move {
            shared
                .copy_desk_document(&document, &channel, machine.as_ref())
                .await
        })
    }

    fn add_repo(&self, session: &SessionId, repo: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let (session, repo) = (session.clone(), repo.to_owned());
        Box::pin(async move { shared.add_desk_repo(&session, &repo).await })
    }

    fn remove_repo(&self, session: &SessionId, repo: &str) -> Reply<()> {
        let shared = self.shared.clone();
        let (session, repo) = (session.clone(), repo.to_owned());
        Box::pin(async move { shared.remove_desk_repo(&session, &repo).await })
    }

    fn move_to(&self, _session: &SessionId, _machine: &MachineId) -> Reply<()> {
        later("Moving a desk session to another machine")
    }

    fn set_access(&self, session: &SessionId, access: Access) -> Reply<()> {
        let shared = self.shared.clone();
        let session = session.clone();
        Box::pin(async move { shared.set_desk_access(&session, access).await })
    }
}
