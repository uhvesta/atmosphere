//! [`DeskClient`] for the live daemon, over `humata.v1.DeskService`. The
//! Desk's sessions, their repositories and documents load with the
//! workspace ([`load_desk`]). Moves to another machine are not in
//! this release (D182.2) and still answer [`DaemonError::NotServedYet`].

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_clients::desk::{
    Access, ChannelDeskSession, CleanUp, CleanUpRow, Closed, DeskClient, DeskData, DeskSession,
    Document, Folder, FolderChoice, LastUsed, NewChannelDesk, NewDeskSession, Switch, Switches,
};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::{DaemonError, Reply};
use humata2_clients_live_core::{
    LiveConnection, MESSAGE_MAX_BYTES, Wire, bad_repo, fresh_key, local_time, map,
    not_in_this_release,
};
use crate::pb;
use crate::pb::desk_service_client::DeskServiceClient;

const DAY_MS: i64 = 86_400_000;

/// The last Clean up list this client answered, with the token that names
/// it. `CleanUpDesk` checks the token before it deletes anything (NV-27).
#[derive(Clone, Debug)]
struct HeldPlan {
    token: pb::PlanToken,
    rows: Vec<CleanUpRow>,
}

/// The app's Desk client over one daemon connection. Cheap to clone.
#[derive(Clone)]
pub struct LiveDesk {
    pub(crate) conn: LiveConnection,
    held_plan: Arc<Mutex<Option<HeldPlan>>>,
}

impl LiveDesk {
    /// Desk operations over `conn`'s daemon connection.
    #[must_use]
    pub fn new(conn: impl AsRef<LiveConnection>) -> Self {
        Self {
            conn: conn.as_ref().clone(),
            held_plan: Arc::new(Mutex::new(None)),
        }
    }
}

pub(crate) fn desk_client(conn: &LiveConnection) -> DeskServiceClient<Wire> {
    DeskServiceClient::new(conn.wire())
        .max_decoding_message_size(MESSAGE_MAX_BYTES)
        .max_encoding_message_size(MESSAGE_MAX_BYTES)
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
    local_time::ago(now / 1_000, then / 1_000)
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
        has_versions: wire.has_versions,
        doc_head: wire.doc_head.clone(),
        version_count: wire.version_count,
        holds_files: wire.holds_files,
    })
}

fn switches_of(wire: &pb::ChannelDeskSwitches) -> Switches {
    Switches {
        read_files: wire.read_files,
        send_notes: wire.send_notes,
        type_into_terminal: wire.type_into_terminal,
        start_and_fork: wire.start_and_fork,
        stop_and_archive: wire.stop_and_archive,
        change_board_items: wire.change_board_items,
    }
}

fn switches_wire(switches: Switches) -> pb::ChannelDeskSwitches {
    pb::ChannelDeskSwitches {
        read_files: switches.read_files,
        send_notes: switches.send_notes,
        type_into_terminal: switches.type_into_terminal,
        start_and_fork: switches.start_and_fork,
        stop_and_archive: switches.stop_and_archive,
        change_board_items: switches.change_board_items,
    }
}

fn blocked_switch(value: i32) -> Option<Switch> {
    match pb::ChannelDeskSwitch::try_from(value).ok()? {
        pb::ChannelDeskSwitch::Unspecified => None,
        pb::ChannelDeskSwitch::ReadFiles => Some(Switch::ReadFiles),
        pb::ChannelDeskSwitch::SendNotes => Some(Switch::SendNotes),
        pb::ChannelDeskSwitch::TypeIntoTerminal => Some(Switch::TypeIntoTerminal),
        pb::ChannelDeskSwitch::StartAndFork => Some(Switch::StartAndFork),
        pb::ChannelDeskSwitch::StopAndArchive => Some(Switch::StopAndArchive),
        pb::ChannelDeskSwitch::ChangeBoardItems => Some(Switch::ChangeBoardItems),
    }
}

/// One channel Desk session as the app's model, through [`desk_session`].
/// `None` when the daemon named no session or sent no channel Desk part.
fn channel_desk_session(wire: &pb::DeskSession, now: i64) -> Option<ChannelDeskSession> {
    let desk = desk_session(wire, now)?;
    let part = wire.channel_desk.as_ref()?;
    let target = map::session_id(part.target.as_ref());
    Some(ChannelDeskSession {
        desk,
        channel: map::channel_id(part.channel.as_ref()),
        target: (!target.as_str().is_empty()).then_some(target),
        switches: part.switches.as_ref().map_or_else(Switches::default, switches_of),
        blocked: part.blocked.iter().filter_map(|value| blocked_switch(*value)).collect(),
        target_gone: part.target_gone,
        documents: wire.documents.clone(),
        document_lines: wire
            .document_lines
            .iter()
            .map(|(name, lines)| (name.clone(), *lines))
            .collect(),
    })
}

/// A Desk event's session as the app's change: a deletion names only the
/// session. `None` when the daemon named no session.
#[must_use]
pub fn change(
    change: i32,
    wire: &pb::DeskSession,
) -> Option<humata2_clients::desk::DeskChange> {
    use humata2_clients::desk::DeskChange;
    if change == pb::Change::Deleted as i32 {
        let id = map::session_id(wire.session.as_ref());
        return (!id.as_str().is_empty()).then_some(DeskChange::Gone(id));
    }
    let now = now_ms();
    if wire.channel_desk.is_some() {
        return channel_desk_session(wire, now).map(DeskChange::ChannelSession);
    }
    desk_session(wire, now).map(|desk| DeskChange::Session {
        desk,
        documents: wire.documents.clone(),
    })
}

fn clean_up_row(row: &pb::DeskCleanUpRow) -> CleanUpRow {
    CleanUpRow {
        session: map::session_id(row.session.as_ref()),
        size_bytes: row.size_bytes,
        unpushed: (!row.unpushed.is_empty()).then(|| row.unpushed.clone()),
    }
}

/// Every Desk session on this machine, open and closed. A daemon that
/// doesn't serve the Desk yet has none, rather than failing the load.
pub async fn load_desk(
    conn: &LiveConnection,
    channels: &[ChannelId],
) -> Result<DeskData, DaemonError> {
    let reply = match desk_client(conn)
        .list_desk_sessions(pb::ListDeskSessionsRequest::default())
        .await
    {
        Ok(reply) => reply.into_inner(),
        Err(status) if status.code() == tonic::Code::Unimplemented => {
            return Ok(DeskData::default());
        }
        Err(status) => return Err(conn.error(&status, "The Desk")),
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
    let channel_sessions = load_channel_desks(conn, channels, now).await?;
    Ok(DeskData {
        sessions,
        documents,
        channel_sessions,
        ..DeskData::default()
    })
}

/// Every channel's own Desk sessions, open and closed (CD-2): one
/// listing per channel, since the plain listing leaves them out.
async fn load_channel_desks(
    conn: &LiveConnection,
    channels: &[ChannelId],
    now: i64,
) -> Result<BTreeMap<SessionId, ChannelDeskSession>, DaemonError> {
    let mut found = BTreeMap::new();
    for channel in channels {
        let reply = match desk_client(conn)
            .list_desk_sessions(pb::ListDeskSessionsRequest {
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
                target: None,
            })
            .await
        {
            Ok(reply) => reply.into_inner(),
            Err(status) if status.code() == tonic::Code::Unimplemented => return Ok(found),
            Err(status) => return Err(conn.error(&status, "A channel's Desk")),
        };
        for wire in &reply.sessions {
            if let Some(session) = channel_desk_session(wire, now) {
                found.insert(session.desk.session.clone(), session);
            }
        }
    }
    Ok(found)
}

impl LiveDesk {
    async fn list_channel_desk(
        &self,
        channel: &ChannelId,
    ) -> Result<Vec<ChannelDeskSession>, DaemonError> {
        let reply = desk_client(&self.conn)
            .list_desk_sessions(pb::ListDeskSessionsRequest {
                channel: Some(pb::ChannelId {
                    value: channel.as_str().to_owned(),
                }),
                target: None,
            })
            .await
            .map_err(|status| self.conn.error(&status, "A channel's Desk"))?
            .into_inner();
        let now = now_ms();
        Ok(reply
            .sessions
            .iter()
            .filter_map(|wire| channel_desk_session(wire, now))
            .collect())
    }

    async fn list_target_desk(
        &self,
        target: &SessionId,
    ) -> Result<Vec<ChannelDeskSession>, DaemonError> {
        let reply = desk_client(&self.conn)
            .list_desk_sessions(pb::ListDeskSessionsRequest {
                channel: None,
                target: Some(session_wire(target)),
            })
            .await
            .map_err(|status| self.conn.error(&status, "The Desk sessions looking after a session"))?
            .into_inner();
        let now = now_ms();
        Ok(reply
            .sessions
            .iter()
            .filter_map(|wire| channel_desk_session(wire, now))
            .collect())
    }

    async fn start_channel_desk(&self, new: NewChannelDesk) -> Result<SessionId, DaemonError> {
        const WHAT: &str = "Starting a channel desk session";
        let reply = desk_client(&self.conn)
            .start_desk_session(pb::StartDeskSessionRequest {
                idempotency_key: fresh_key(),
                title: new.title,
                agent: Some(pb::AgentId {
                    value: self.conn.agents().name(new.agent),
                }),
                machine: Some(pb::MachineId {
                    value: new.machine.as_str().to_owned(),
                }),
                channel: Some(pb::ChannelId {
                    value: new.channel.as_str().to_owned(),
                }),
                target: new.target.as_ref().map(session_wire),
                switches: Some(switches_wire(new.switches)),
                ..Default::default()
            })
            .await
            .map_err(|status| self.conn.error(&status, WHAT))?
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

    async fn set_channel_switches(
        &self,
        session: &SessionId,
        switches: Switches,
    ) -> Result<(), DaemonError> {
        desk_client(&self.conn)
            .set_desk_access(pb::SetDeskAccessRequest {
                session: Some(session_wire(session)),
                access: None,
                switches: Some(switches_wire(switches)),
            })
            .await
            .map_err(|status| self.conn.error(&status, "Changing a channel desk session's switches"))?;
        Ok(())
    }

    async fn set_channel_target(
        &self,
        session: &SessionId,
        target: Option<&SessionId>,
    ) -> Result<(), DaemonError> {
        desk_client(&self.conn)
            .set_desk_target(pb::SetDeskTargetRequest {
                session: Some(session_wire(session)),
                target: target.map(session_wire),
            })
            .await
            .map_err(|status| {
                self.conn.error(&status, "Changing what a channel desk session looks after")
            })?;
        Ok(())
    }

    async fn add_desk_repo(&self, session: &SessionId, repo: &str) -> Result<(), DaemonError> {
        const WHAT: &str = "Adding a repository to a desk session";
        let repo = repo_wire(repo)?;
        desk_client(&self.conn)
            .add_desk_repo(pb::AddDeskRepoRequest {
                session: Some(session_wire(session)),
                repo: Some(repo),
            })
            .await
            .map_err(|status| self.conn.error(&status, WHAT))?;
        Ok(())
    }

    async fn remove_desk_repo(&self, session: &SessionId, repo: &str) -> Result<(), DaemonError> {
        const WHAT: &str = "Removing a repository from a desk session";
        let repo = repo_wire(repo)?;
        desk_client(&self.conn)
            .remove_desk_repo(pb::RemoveDeskRepoRequest {
                session: Some(session_wire(session)),
                repo: Some(repo),
            })
            .await
            .map_err(|status| self.conn.error(&status, WHAT))?;
        Ok(())
    }

    async fn copy_desk_document(
        &self,
        document: &Document,
        channel: &ChannelId,
        machine: Option<&MachineId>,
    ) -> Result<(), DaemonError> {
        const WHAT: &str = "Copying a document to a channel";
        desk_client(&self.conn)
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
            .map_err(|status| self.conn.error(&status, WHAT))?;
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
        // Nobody picks a folder (D451, D460): the daemon places the session.
        let FolderChoice::Scratch { .. } = new.folder;
        let reply = desk_client(&self.conn)
            .start_desk_session(pb::StartDeskSessionRequest {
                idempotency_key: fresh_key(),
                title: new.title,
                agent: Some(pb::AgentId {
                    value: self.conn.agents().name(new.agent),
                }),
                model: String::new(),
                machine: Some(pb::MachineId {
                    value: new.machine.as_str().to_owned(),
                }),
                picked_path: String::new(),
                repos,
                access: Some(access_wire(new.access)),
                first_message: String::new(),
                channel: None,
                target: None,
                switches: None,
            })
            .await
            .map_err(|status| self.conn.error(&status, WHAT))?
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
        desk_client(&self.conn)
            .close_desk_session(pb::CloseDeskSessionRequest {
                idempotency_key: fresh_key(),
                session: Some(session_wire(session)),
                ..Default::default()
            })
            .await
            .map_err(|status| self.conn.error(&status, WHAT))?;
        Ok(())
    }

    async fn reopen_desk(&self, session: &SessionId) -> Result<(), DaemonError> {
        const WHAT: &str = "Reopening a desk session";
        let operation = desk_client(&self.conn)
            .reopen_desk_session(pb::ReopenDeskSessionRequest {
                idempotency_key: fresh_key(),
                session: Some(session_wire(session)),
            })
            .await
            .map_err(|status| self.conn.error(&status, WHAT))?
            .into_inner();
        self.conn.finish_operation(operation, WHAT).await.map(|_| ())
    }

    async fn keep_desk(&self, session: &SessionId) -> Result<(), DaemonError> {
        desk_client(&self.conn)
            .keep_desk_folder(pb::KeepDeskFolderRequest {
                session: Some(session_wire(session)),
            })
            .await
            .map_err(|status| self.conn.error(&status, "Keeping a desk session's folder"))?;
        Ok(())
    }

    async fn set_desk_access(&self, session: &SessionId, access: Access) -> Result<(), DaemonError> {
        desk_client(&self.conn)
            .set_desk_access(pb::SetDeskAccessRequest {
                session: Some(session_wire(session)),
                access: Some(access_wire(access)),
                switches: None,
            })
            .await
            .map_err(|status| self.conn.error(&status, "Changing a desk session's access"))?;
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
        *self.held_plan.lock().unwrap_or_else(PoisonError::into_inner) = Some(HeldPlan {
            token,
            rows: rows.clone(),
        });
        Ok(rows)
    }

    async fn plan_clean_up(&self) -> Result<Vec<CleanUpRow>, DaemonError> {
        const WHAT: &str = "Listing closed desk sessions";
        let plan = desk_client(&self.conn)
            .plan_desk_clean_up(pb::PlanDeskCleanUpRequest {})
            .await
            .map_err(|status| self.conn.error(&status, WHAT))?
            .into_inner();
        self.hold_plan(plan, WHAT)
    }

    async fn clean_up_desk(
        &self,
        shown: Vec<CleanUpRow>,
        delete: Vec<SessionId>,
    ) -> Result<CleanUp, DaemonError> {
        const WHAT: &str = "Cleaning up closed desk sessions";
        let held = self
            .held_plan
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(held) = held.filter(|held| held.rows == shown) else {
            // The list on screen is not the one held, so nothing is deleted:
            // the list now goes back to ask again (NV-27).
            return Ok(CleanUp::Changed(self.plan_clean_up().await?));
        };
        let response = desk_client(&self.conn)
            .clean_up_desk(pb::CleanUpDeskRequest {
                idempotency_key: fresh_key(),
                plan: Some(held.token),
                delete: delete.iter().map(session_wire).collect(),
            })
            .await
            .map_err(|status| self.conn.error(&status, WHAT))?
            .into_inner();
        match response.result {
            Some(pb::clean_up_desk_response::Result::Deleted(done)) => {
                *self.held_plan.lock().unwrap_or_else(PoisonError::into_inner) = None;
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

impl DeskClient for LiveDesk {
    fn start(&self, new: NewDeskSession) -> Reply<SessionId> {
        let this = self.clone();
        Box::pin(async move { this.start_desk(new).await })
    }

    fn close(&self, session: &SessionId) -> Reply<()> {
        let this = self.clone();
        let session = session.clone();
        Box::pin(async move { this.close_desk(&session).await })
    }

    fn reopen(&self, session: &SessionId) -> Reply<()> {
        let this = self.clone();
        let session = session.clone();
        Box::pin(async move { this.reopen_desk(&session).await })
    }

    fn keep(&self, session: &SessionId) -> Reply<()> {
        let this = self.clone();
        let session = session.clone();
        Box::pin(async move { this.keep_desk(&session).await })
    }

    fn clean_up_list(&self) -> Reply<Vec<CleanUpRow>> {
        let this = self.clone();
        Box::pin(async move { this.plan_clean_up().await })
    }

    fn clean_up(&self, shown: Vec<CleanUpRow>, delete: Vec<SessionId>) -> Reply<CleanUp> {
        let this = self.clone();
        Box::pin(async move { this.clean_up_desk(shown, delete).await })
    }

    fn copy_document(
        &self,
        document: &Document,
        channel: &ChannelId,
        machine: Option<&MachineId>,
    ) -> Reply<()> {
        let this = self.clone();
        let (document, channel) = (document.clone(), channel.clone());
        let machine = machine.cloned();
        Box::pin(async move {
            this.copy_desk_document(&document, &channel, machine.as_ref())
                .await
        })
    }

    fn add_repo(&self, session: &SessionId, repo: &str) -> Reply<()> {
        let this = self.clone();
        let (session, repo) = (session.clone(), repo.to_owned());
        Box::pin(async move { this.add_desk_repo(&session, &repo).await })
    }

    fn remove_repo(&self, session: &SessionId, repo: &str) -> Reply<()> {
        let this = self.clone();
        let (session, repo) = (session.clone(), repo.to_owned());
        Box::pin(async move { this.remove_desk_repo(&session, &repo).await })
    }

    fn move_to(&self, _session: &SessionId, _machine: &MachineId) -> Reply<()> {
        later("Moving a desk session to another machine")
    }

    fn list_versions(
        &self,
        session: &SessionId,
        path: &str,
    ) -> Reply<humata2_clients::docs::DocVersions> {
        let this = self.clone();
        let (session, path) = (session.clone(), path.to_owned());
        Box::pin(async move { this.list_doc_versions(&session, &path).await })
    }

    fn read_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<humata2_clients::docs::VersionText> {
        let this = self.clone();
        let (session, path, id) = (session.clone(), path.to_owned(), id.to_owned());
        Box::pin(async move { this.read_doc_version(&session, &path, &id).await })
    }

    fn save_version(
        &self,
        session: &SessionId,
        message: &str,
    ) -> Reply<humata2_clients::docs::DocVersion> {
        let this = self.clone();
        let (session, message) = (session.clone(), message.to_owned());
        Box::pin(async move { this.save_doc_version(&session, &message).await })
    }

    fn restore_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<humata2_clients::docs::Restored> {
        let this = self.clone();
        let (session, path, id) = (session.clone(), path.to_owned(), id.to_owned());
        Box::pin(async move { this.restore_doc_version(&session, &path, &id).await })
    }

    fn diff_version(
        &self,
        session: &SessionId,
        path: &str,
        id: &str,
    ) -> Reply<humata2_clients::docs::VersionDiff> {
        let this = self.clone();
        let (session, path, id) = (session.clone(), path.to_owned(), id.to_owned());
        Box::pin(async move { this.diff_doc_version(&session, &path, &id).await })
    }

    fn set_access(&self, session: &SessionId, access: Access) -> Reply<()> {
        let this = self.clone();
        let session = session.clone();
        Box::pin(async move { this.set_desk_access(&session, access).await })
    }

    fn channel_sessions(&self, channel: &ChannelId) -> Reply<Vec<ChannelDeskSession>> {
        let this = self.clone();
        let channel = channel.clone();
        Box::pin(async move { this.list_channel_desk(&channel).await })
    }

    fn target_sessions(&self, target: &SessionId) -> Reply<Vec<ChannelDeskSession>> {
        let this = self.clone();
        let target = target.clone();
        Box::pin(async move { this.list_target_desk(&target).await })
    }

    fn start_channel(&self, new: NewChannelDesk) -> Reply<SessionId> {
        let this = self.clone();
        Box::pin(async move { this.start_channel_desk(new).await })
    }

    fn set_switches(&self, session: &SessionId, switches: Switches) -> Reply<()> {
        let this = self.clone();
        let session = session.clone();
        Box::pin(async move { this.set_channel_switches(&session, switches).await })
    }

    fn set_target(&self, session: &SessionId, target: Option<&SessionId>) -> Reply<()> {
        let this = self.clone();
        let session = session.clone();
        let target = target.cloned();
        Box::pin(async move { this.set_channel_target(&session, target.as_ref()).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel_wire(documents: &[&str], lines: &[(&str, u32)]) -> pb::DeskSession {
        pb::DeskSession {
            session: Some(pb::SessionId {
                value: "se_1".to_owned(),
            }),
            documents: documents.iter().map(|name| (*name).to_owned()).collect(),
            document_lines: lines
                .iter()
                .map(|(name, count)| ((*name).to_owned(), *count))
                .collect(),
            channel_desk: Some(pb::ChannelDesk::default()),
            ..Default::default()
        }
    }

    #[test]
    fn a_channel_desk_session_carries_the_daemons_line_counts_by_file() {
        let wire = channel_wire(&["a.md", "b.md"], &[("a.md", 3)]);
        let shown = channel_desk_session(&wire, 0).expect("a channel Desk session");
        let expected: BTreeMap<String, u32> = [("a.md".to_owned(), 3)].into_iter().collect();
        assert_eq!(shown.document_lines, expected);
        assert_eq!(shown.documents, vec!["a.md".to_owned(), "b.md".to_owned()]);
    }
}
