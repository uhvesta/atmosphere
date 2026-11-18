//! A channel's own Desk sessions (CD-1, D306): a scratch folder
//! `<root>/<channel>/desk/<name>` beside the channel's `main/`, `sessions/`
//! and `shared/`, named from the title as DK-2 names a Desk folder, with no
//! clones, branch, fork or board of its own. The record is filed under the
//! Desk's channel id, so it is in no channel's Sessions tree; its channel
//! Desk row says which channel owns it.

use std::collections::HashSet;
use std::path::Path;

use humata2_revive::Env;
use humata2_store::{ChannelDeskRow, SessionDetail, SessionRecord, StartStep, Store};

use crate::desk::{DESK_CHANNEL, DeskError, blocking, claim_scratch, init_scratch, store_error};
use crate::error::StartError;
use crate::names::branch_slug;
use crate::setup::unix_ms;
use crate::start::{Started, Starter};

/// The folder under a channel's folder its Desk sessions work in.
const DESK_FOLDER: &str = "desk";

/// A channel Desk session to start (CD-1, CD-6).
#[derive(Clone, Debug)]
pub struct ChannelDeskStart {
    /// The session's id, chosen by the caller.
    pub id: String,
    /// The title shown; the scratch folder's name comes from it.
    pub title: String,
    /// The agent as named in Settings › Agents, or a plain shell.
    pub agent: String,
    pub model: Option<String>,
    pub first_message: Option<String>,
    /// The machine it runs on: any linked machine, the channel's or not,
    /// and any other later by a move (CD-1, MV-17, D316).
    pub machine: String,
    /// The channel's id, and its folder's name under the channels root.
    pub channel: String,
    pub channel_folder: String,
    /// The session it looks after; `None` for the whole channel.
    pub target: Option<String>,
    /// The CD-3 switches it starts with.
    pub read_files: bool,
    pub send_notes: bool,
    pub type_into_terminal: bool,
    pub start_and_fork: bool,
    pub stop_and_archive: bool,
    pub change_board_items: bool,
    /// The operation that started the session, if any.
    pub operation: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

impl Starter {
    /// Claims the scratch folder, records the session and its channel Desk
    /// row, and starts its agent (or shell) the way any session's agent is
    /// started. A failure before the launch takes back the folder and the
    /// records; from the launch on they stay and revive owns them.
    pub async fn start_channel_desk(
        &self,
        request: ChannelDeskStart,
        env: Env<'_>,
        progress: &mut (dyn FnMut(&str) + Send),
    ) -> Result<Started, DeskError> {
        let agent_name = self
            .agents
            .get(&request.agent)
            .map_err(StartError::from)?
            .person_name();
        let root = self.layout.channel(&request.channel_folder).join(DESK_FOLDER);
        let (store, slug, claiming) = (
            self.store.clone(),
            branch_slug(&request.title),
            root.clone(),
        );
        // The folder's versions are authored under the agent's person name.
        let folder = blocking(move || {
            let taken = held_names(&store, &claiming)?;
            let folder = claim_scratch(&claiming, &slug, &taken)?;
            // A channel's Desk folder is a versions repository too (DV-1).
            init_scratch(&folder, &agent_name)?;
            Ok(folder)
        })
        .await?;

        let session = SessionRecord {
            id: request.id.clone(),
            channel: DESK_CHANNEL.to_owned(),
            machine: request.machine.clone(),
            title: request.title.clone(),
            agent: request.agent.clone(),
            folder,
            created_unix_ms: unix_ms(),
        };
        let detail = SessionDetail {
            session: request.id.clone(),
            repos: Vec::new(),
            model: request.model.clone(),
            first_message: request.first_message.clone(),
            start: StartStep::Recorded,
            operation: request.operation.clone(),
            revision: 1,
        };
        let row = ChannelDeskRow {
            session_id: request.id.clone(),
            channel: request.channel.clone(),
            target_session: request.target.clone(),
            read_files: request.read_files,
            send_notes: request.send_notes,
            type_into_terminal: request.type_into_terminal,
            start_and_fork: request.start_and_fork,
            stop_and_archive: request.stop_and_archive,
            change_board_items: request.change_board_items,
            last_used_unix_ms: unix_ms(),
            closed_unix_ms: None,
            target_gone: false,
            trash_item: None,
        };
        if let Err(error) = self.record_channel_desk(&session, &detail, &row).await {
            // Best effort: the start's own error is the one worth reporting.
            let _ = self.undo_desk(&session).await;
            return Err(error);
        }
        progress("write_record");
        // From the launch on, the records stay and revive owns them, as for
        // any session.
        let (detail, agent) = self
            .launch_session(&session, detail, request.cols, request.rows, env, progress)
            .await?;
        Ok(Started {
            session,
            detail,
            agent,
        })
    }

    /// Saves the session, its start and its channel Desk row in one
    /// blocking step.
    async fn record_channel_desk(
        &self,
        session: &SessionRecord,
        detail: &SessionDetail,
        row: &ChannelDeskRow,
    ) -> Result<(), DeskError> {
        let (store, session, detail, row) = (
            self.store.clone(),
            session.clone(),
            detail.clone(),
            row.clone(),
        );
        blocking(move || -> Result<(), DeskError> {
            store
                .put_session(&session)
                .map_err(store_error("save the session"))?;
            store
                .put_session_detail(&detail)
                .map_err(store_error("save the session's start"))?;
            store
                .put_channel_desk(&row)
                .map_err(store_error("save the channel Desk row"))
        })
        .await
    }
}

/// The folder names under `root` that the channel's Desk sessions hold,
/// closed ones included, so a restore from the trash finds its name free.
fn held_names(store: &Store, root: &Path) -> Result<HashSet<String>, DeskError> {
    let desks: HashSet<String> = store
        .channel_desks()
        .map_err(store_error("read the channel Desk rows"))?
        .into_iter()
        .map(|row| row.session_id)
        .collect();
    Ok(store
        .sessions()
        .map_err(store_error("read the sessions"))?
        .into_iter()
        .filter(|session| desks.contains(&session.id) && session.folder.parent() == Some(root))
        .filter_map(|session| {
            session
                .folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .collect())
}
