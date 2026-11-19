//! A Desk session's part of a carry (MV-16, MV-17, D313, D316): where its
//! folder is in the channels tree on any machine, and the rows only a Desk
//! knows about it, which travel in the stage beside the manifest.
//!
//! A Desk session is named for a carry by [`CHANNELS_ROOT`], the channels
//! folder itself, and a folder relative to it: `desk/<name>` for a plain
//! Desk session, `<channel folder>/desk/<name>` for a channel's own. So a
//! machine that holds no record of the channel can still take the session.

use std::fs;
use std::path::{Path, PathBuf};

use humata2_store::{ChannelDeskRow, ChannelRecord, DeskFolderKind, DeskRow, SessionRecord, Take};
use serde::{Deserialize, Serialize};

use super::{CarryError, Manifest, ManifestProblem, folder_name, io, store};
use humata2_sessions_core::names::DESK_CHANNEL;
use humata2_sessions_core::start::Starter;

/// What a carry calls the channels folder itself, for the sessions that live
/// in a Desk: no channel has this id.
pub use humata2_sessions_core::names::CHANNELS_ROOT;

/// The stage's file with a Desk session's rows.
const ROWS_FILE: &str = "desk.json";

/// The rows only a Desk knows about a session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Rows {
    Plain {
        kept: bool,
        boards: bool,
        channels_and_machines: bool,
        last_used_unix_ms: i64,
        closed_unix_ms: Option<i64>,
        held_work: String,
    },
    Channel {
        channel: String,
        target_session: Option<String>,
        read_files: bool,
        send_notes: bool,
        type_into_terminal: bool,
        start_and_fork: bool,
        stop_and_archive: bool,
        change_board_items: bool,
        last_used_unix_ms: i64,
        closed_unix_ms: Option<i64>,
        target_gone: bool,
    },
}

impl Rows {
    fn plain(row: &DeskRow, kept: bool) -> Self {
        Self::Plain {
            kept,
            boards: row.boards,
            channels_and_machines: row.channels_and_machines,
            last_used_unix_ms: row.last_used_unix_ms,
            closed_unix_ms: row.closed_unix_ms,
            held_work: row.held_work.clone(),
        }
    }

    fn channel(row: &ChannelDeskRow) -> Self {
        Self::Channel {
            channel: row.channel.clone(),
            target_session: row.target_session.clone(),
            read_files: row.read_files,
            send_notes: row.send_notes,
            type_into_terminal: row.type_into_terminal,
            start_and_fork: row.start_and_fork,
            stop_and_archive: row.stop_and_archive,
            change_board_items: row.change_board_items,
            last_used_unix_ms: row.last_used_unix_ms,
            closed_unix_ms: row.closed_unix_ms,
            target_gone: row.target_gone,
        }
    }
}

/// Where a session lives in the channels tree, for a carry.
pub(super) struct Home {
    /// The channel the carry names: the session's own, or the channels
    /// folder for a Desk session.
    pub(super) channel: ChannelRecord,
    /// The session's folder under the channel's folder: `sessions/<name>`,
    /// `desk/<name>` or `<channel folder>/desk/<name>`.
    pub(super) rel: String,
    /// A Desk session's rows.
    pub(super) rows: Option<Rows>,
}

impl crate::CarryOps<'_> {
    /// The channels folder as a channel a carry can name: no repositories, no
    /// machines, and a folder that is the root itself.
    pub(super) fn channels_root_record(&self) -> ChannelRecord {
        ChannelRecord {
            id: CHANNELS_ROOT.to_owned(),
            name: "Desk".to_owned(),
            folder: String::new(),
            branch_prefix: None,
            default_agent: String::new(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 0,
            created_unix_ms: 0,
        }
    }

    /// Where `record`'s session lives, by what kind of session it is (MV-16).
    pub(super) fn home_of(&self, record: &SessionRecord) -> Result<Home, CarryError> {
        if record.channel != DESK_CHANNEL {
            let channel = self.carry_channel(&record.channel)?;
            let sessions = self.layout.sessions(&channel.folder);
            let name = match record.folder.parent() {
                Some(parent) if parent == sessions => record.folder.file_name().map(|name| name.to_string_lossy().into_owned()),
                _ => None,
            }
            .ok_or_else(|| CarryError::NotASessionFolder { folder: record.folder.clone(), sessions: sessions.clone() })?;
            let rel = format!("sessions/{name}");
            folder_name(&rel)?;
            return Ok(Home { channel, rel, rows: None });
        }
        let plain = self.store.desk(&record.id).map_err(store("read the Desk session"))?;
        let own = self.store.channel_desk(&record.id).map_err(store("read the channel's Desk session"))?;
        let name = record.folder.file_name().map(|name| name.to_string_lossy().into_owned());
        let channel = self.channels_root_record();
        let rel = match (plain, own, name) {
            (_, Some(row), Some(name)) => {
                let desk = record.folder.parent().map(Path::to_owned).unwrap_or_default();
                let channel_dir = desk.parent().map(Path::to_owned).unwrap_or_default();
                let folder = channel_dir.file_name().map(|folder| folder.to_string_lossy().into_owned());
                match folder {
                    Some(folder)
                        if desk.file_name().is_some_and(|last| last == "desk")
                            && channel_dir.parent().is_some_and(|parent| self.layout.is_a_root(parent)) =>
                    {
                        (format!("{folder}/desk/{name}"), Rows::channel(&row))
                    }
                    _ => return Err(CarryError::NotADeskFolder { folder: record.folder.clone(), desk }),
                }
            }
            (Some(row), None, Some(name)) => {
                let desk = self.layout.root().join("desk");
                match row.kind {
                    DeskFolderKind::Scratch { kept } if record.folder.parent() == Some(desk.as_path()) => {
                        (format!("desk/{name}"), Rows::plain(&row, kept))
                    }
                    _ => return Err(CarryError::NotADeskFolder { folder: record.folder.clone(), desk }),
                }
            }
            _ => {
                return Err(CarryError::NotADeskFolder {
                    folder: record.folder.clone(),
                    desk: self.layout.root().join("desk"),
                });
            }
        };
        self.carried_folder(&channel, &rel.0).map(drop)?;
        Ok(Home { channel, rel: rel.0, rows: Some(rel.1) })
    }

    /// The folder `rel` names under `channel` on this machine, once `rel` is
    /// known to be one plain place of that kind of session.
    pub(super) fn carried_folder(&self, channel: &ChannelRecord, rel: &str) -> Result<PathBuf, CarryError> {
        let plain = |name: &str| {
            !name.is_empty() && name != "." && name != ".." && !name.contains('\0') && !name.contains('\\') && !name.starts_with('.')
        };
        let parts: Vec<&str> = rel.split('/').collect();
        let valid = if channel.id == CHANNELS_ROOT {
            match parts.as_slice() {
                ["desk", name] => plain(*name),
                [channel_folder, "desk", name] => *channel_folder != "desk" && plain(*channel_folder) && plain(*name),
                _ => false,
            }
        } else {
            folder_name(rel).is_ok()
        };
        if !valid {
            return Err(CarryError::BadFolder { path: rel.to_owned() });
        }
        Ok(self.layout.channel(&channel.folder).join(rel))
    }

    /// Writes a Desk session's rows into `stage`.
    pub(super) fn stage_rows(&self, home: &Home, stage: &Path) -> Result<(), CarryError> {
        let Some(rows) = &home.rows else {
            return Ok(());
        };
        let path = stage.join(ROWS_FILE);
        let bytes = serde_json::to_vec_pretty(rows).map_err(|source| CarryError::Manifest {
            path: path.clone(),
            source: ManifestProblem::Parse(source),
        })?;
        fs::write(&path, bytes).map_err(io("write", &path))
    }

    /// Brings a Desk session's rows in, once its records are. A machine that
    /// already holds the session (the host) keeps its own.
    pub(super) fn bring_rows(&self, manifest: &Manifest, stage: &Path, take: Take) -> Result<(), CarryError> {
        if manifest.channel != CHANNELS_ROOT || take != Take::Everything {
            return Ok(());
        }
        let path = stage.join(ROWS_FILE);
        let bytes = fs::read(&path).map_err(|_| CarryError::Missing { item: "the Desk session's rows".to_owned() })?;
        let rows: Rows = serde_json::from_slice(&bytes).map_err(|source| CarryError::Manifest {
            path: path.clone(),
            source: ManifestProblem::Parse(source),
        })?;
        let session = manifest.session.clone();
        match rows {
            Rows::Plain { kept, boards, channels_and_machines, last_used_unix_ms, closed_unix_ms, held_work } => self
                .store
                .put_desk(&DeskRow {
                    session_id: session,
                    kind: DeskFolderKind::Scratch { kept },
                    boards,
                    channels_and_machines,
                    last_used_unix_ms,
                    closed_unix_ms,
                    deleted_unix_ms: None,
                    held_work,
                })
                .map_err(store("save the Desk session's row")),
            Rows::Channel {
                channel,
                target_session,
                read_files,
                send_notes,
                type_into_terminal,
                start_and_fork,
                stop_and_archive,
                change_board_items,
                last_used_unix_ms,
                closed_unix_ms,
                target_gone,
            } => self
                .store
                .put_channel_desk(&ChannelDeskRow {
                    session_id: session,
                    channel,
                    target_session,
                    read_files,
                    send_notes,
                    type_into_terminal,
                    start_and_fork,
                    stop_and_archive,
                    change_board_items,
                    last_used_unix_ms,
                    closed_unix_ms,
                    target_gone,
                    trash_item: None,
                })
                .map_err(store("save the channel's Desk session's row")),
        }
    }

    /// The folder a move of `record`'s session leaves behind. A Desk
    /// session's record on the host may already name the new machine's
    /// folder, so its agent's folder says where it was. Without its agent
    /// a release already took it (a release done again after a restart):
    /// none, as the record's folder may be the new machine's.
    pub(super) fn folder_to_release(&self, record: &SessionRecord) -> Result<Option<PathBuf>, CarryError> {
        if record.channel != DESK_CHANNEL {
            return Ok(Some(record.folder.clone()));
        }
        let agent = self.store.agent(&record.id).map_err(store("read the session's agent"))?;
        Ok(agent.map(|agent| agent.launch.folder))
    }
}
