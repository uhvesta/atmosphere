//! A rename moves the channel's folder (MV-16, D314): `<root>/<old>/`
//! becomes `<root>/<new>/` with one `rename(2)`, and every session of the
//! channel, its channel Desk sessions included, is stopped before it and
//! resumed after it in its new folder with its conversation moved to the
//! key the new folder has.
//!
//! The order, so a crash at any point leaves something a restart finishes:
//!
//! 1. the name is checked against the folders there are, and the channel's
//!    agents are checked: one in a turn needs the person's say-so;
//! 2. a note naming the move, the conversations it carries and the sessions
//!    it stops is written under `<root>/.renaming`;
//! 3. the agents are stopped and the folder is renamed;
//! 4. the files that name the old folder are rewritten, the conversations
//!    move, and the records change in one transaction;
//! 5. the note goes and the agents are resumed.
//!
//! A restart that finds a note with the folder moved and the records not
//! does steps 4 and 5; one that finds the folder not moved only resumes.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{Agents, FollowError, SetupError, channel_folder};
use humata2_store::{AgentRecord, AgentState, ChannelRecord, Placement, Store, StoreError};
use serde::{Deserialize, Serialize};
use tonic::{Code, Request, Status};

use super::move_folder::{self, MoveError};
use super::{ChannelSvc, blocking, failed, setup_status, tell_channel};
use crate::holder_end;
use crate::identity::{self, Caller, UserVia};
use crate::status;
use crate::trash_service::now_ms;

/// Where moves in progress are written, in the channels folder.
const NOTES: &str = ".renaming";

/// A rename in progress, as the note under [`NOTES`] keeps it.
#[derive(Debug, Serialize, Deserialize)]
struct Intent {
    channel: String,
    name: String,
    old_folder: String,
    new_folder: String,
    /// The conversation files that go with the folder.
    moves: Vec<(PathBuf, PathBuf)>,
    /// The sessions whose agents ran, and run again once it is done.
    resume: Vec<String>,
}

/// Why a channel's rename stopped.
#[derive(Debug, thiserror::Error)]
enum RenameError {
    #[error("the folder {} is already there, so channel {name:?} can't be moved to it; nothing was changed", folder.display())]
    Taken { name: String, folder: PathBuf },
    #[error("session {title:?} is in a turn, and a rename stops it and starts it again in the new folder; nothing was changed")]
    Busy { title: String },
    #[error("session {title:?} is still starting, so its agent can't be stopped yet; nothing was changed")]
    Starting { title: String },
    #[error("couldn't stop session {session}'s agent for the rename; nothing was renamed")]
    Stop {
        session: String,
        #[source]
        source: Status,
    },
    #[error("couldn't write the note of the rename at {}", path.display())]
    Note {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("couldn't rename the folder {} to {}", from.display(), to.display())]
    Move {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the folder was renamed to {}, but the files inside it still name the old one", to.display())]
    Files {
        to: PathBuf,
        #[source]
        source: MoveError,
    },
    #[error(
        "the folder was renamed to {}, but the conversation of session {session} could not follow its agent to the new folder",
        to.display()
    )]
    Follow {
        to: PathBuf,
        session: String,
        #[source]
        source: FollowError,
    },
    #[error("the folder was renamed to {}, but the channel's records could not be saved under it", to.display())]
    Records {
        to: PathBuf,
        #[source]
        source: StoreError,
    },
}

/// An agent of the channel and the session it belongs to.
struct Live {
    session: String,
    title: String,
    agent: AgentRecord,
}

/// The agents of `channel`'s sessions and of its open Desk sessions. A
/// closed Desk session's agent is stopped, and is not resumed.
fn live_agents(store: &Store, channel: &str) -> Result<Vec<Live>, StoreError> {
    let desks: HashSet<String> = store
        .channel_desks()?
        .into_iter()
        .filter(|row| row.channel == channel && row.closed_unix_ms.is_none())
        .map(|row| row.session_id)
        .collect();
    let mut found = Vec::new();
    for session in store.sessions()? {
        if session.channel != channel && !desks.contains(&session.id) {
            continue;
        }
        if let Some(agent) = store.agent(&session.id)? {
            found.push(Live {
                session: session.id,
                title: session.title,
                agent,
            });
        }
    }
    Ok(found)
}

/// Where the conversations that follow a rename are staged, in the channels
/// folder.
const FOLLOW_SCRATCH: &str = ".renaming-stage";

/// MV-16: the conversations that keep their agent's folder inside them
/// (Codex's, Copilot's) are placed again where they are by their adapter, so
/// their files and, for Codex, its thread row name the new folder; Claude's
/// move with their key (`carry_conversations`). Each session's folders are
/// its own and its clones' and the one the conversation records for itself
/// (the agent's, for the current one), that one last, as the thread row
/// keeps the last. A previous conversation's own folder is read from its
/// file. Each step can run again, and runs before the records change.
fn follow_conversations(
    store: &Store,
    agents: &Agents,
    home: &Path,
    scratch: &Path,
    placement: &Placement,
    channel: &str,
    here: &str,
) -> Result<(), RenameError> {
    let records = |source: StoreError| RenameError::Records { to: placement.new_path.clone(), source };
    let desks: HashSet<String> = store
        .channel_desks()
        .map_err(records)?
        .into_iter()
        .filter(|row| row.channel == channel)
        .map(|row| row.session_id)
        .collect();
    for session in store.sessions().map_err(records)? {
        if session.machine != here || (session.channel != channel && !desks.contains(&session.id)) {
            continue;
        }
        let Some(agent) = store.agent(&session.id).map_err(records)? else { continue };
        let Ok(kind) = agents.get(&agent.launch.agent) else { continue };
        let moved = |old: &Path| old.strip_prefix(&placement.old_path).ok().map(|rest| placement.new_path.join(rest));
        let mut above: Vec<PathBuf> = vec![session.folder.clone()];
        if let Some(detail) = store.session_detail(&session.id).map_err(records)? {
            above.extend(detail.repos.into_iter().map(|repo| repo.path));
        }
        let mut conversations = vec![(agent.launch.conversation_id.clone(), agent.launch.conversation_file.clone())];
        conversations.extend(
            store
                .previous_conversations(&session.id)
                .map_err(records)?
                .into_iter()
                .map(|one| (one.conversation_id, one.path)),
        );
        let stage = scratch.join(&session.id);
        for (id, file) in &conversations {
            // The folder the conversation itself records; the agent's for the
            // current one.
            let cwd = if *file == agent.launch.conversation_file {
                Some(agent.launch.folder.clone())
            } else {
                kind.recorded_folder(file)
            };
            // A conversation a run before this one followed already records
            // the new folder: it is still its own folder, followed last, so
            // the CLI's thread row keeps naming it on a run again.
            let cwd = cwd.map(|cwd| match cwd.strip_prefix(&placement.new_path) {
                Ok(rest) if !cwd.starts_with(&placement.old_path) => placement.old_path.join(rest),
                _ => cwd,
            });
            let mut others = above.clone();
            others.retain(|old| Some(old) != cwd.as_ref());
            others.sort_by_key(|old| old.as_os_str().len());
            others.dedup();
            others.extend(cwd);
            let folders: Vec<(PathBuf, PathBuf)> =
                others.into_iter().filter_map(|old| moved(&old).map(|new| (old, new))).collect();
            for (from, to) in &folders {
                kind.follow_folder(home, id, file, from, to, &stage).map_err(|source| RenameError::Follow {
                    to: placement.new_path.clone(),
                    session: session.id.clone(),
                    source,
                })?;
            }
        }
    }
    Ok(())
}

fn note_path(root: &Path, channel: &str) -> PathBuf {
    root.join(NOTES).join(channel)
}

fn write_note(path: &Path, intent: &Intent) -> Result<(), RenameError> {
    let note = |source| RenameError::Note {
        path: path.to_owned(),
        source,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(note)?;
    }
    let text = serde_json::to_vec(intent).map_err(|error| note(std::io::Error::other(error)))?;
    humata2_store::write_atomic(path, &text).map_err(|error| note(std::io::Error::other(error)))
}

impl ChannelSvc {
    /// The folder `name` gives `channel`, when it is not the one it has.
    /// The PR reviews channel's folder is Humata's own and stays.
    pub(super) fn folder_for(&self, channel: &ChannelRecord, name: &str) -> Result<Option<String>, Status> {
        if channel.folder == crate::review::PR_REVIEWS_FOLDER {
            return Ok(None);
        }
        let folder = channel_folder(name).map_err(|error| setup_status(SetupError::Name(error), &self.machine))?;
        Ok((folder != channel.folder).then_some(folder))
    }

    /// Renames `channel` to `name` and moves its folder to `folder`.
    /// `confirmed` says the person agreed to cut off a turn.
    pub(super) async fn rename_moving(
        &self,
        caller: &Caller,
        channel: ChannelRecord,
        name: String,
        folder: String,
        expected: Option<u64>,
        confirmed: bool,
    ) -> Result<ChannelRecord, Status> {
        if let Some(expected) = expected
            && expected != channel.revision
        {
            return Err(self.rename_failed(
                StoreError::StaleChannel {
                    id: channel.id.clone(),
                    expected,
                    actual: channel.revision,
                },
                &channel.id,
            ));
        }
        let layout = &self.starter.layout;
        let (old_path, new_path) = (layout.channel(&channel.folder), layout.channel(&folder));
        self.check_free(&channel, &name, &folder, &new_path).await?;
        let live = self.live_of(&channel.id).await?;
        for item in &live {
            match item.agent.state {
                AgentState::Launching | AgentState::Resuming => {
                    return Err(failed(
                        Code::FailedPrecondition,
                        &RenameError::Starting { title: item.title.clone() },
                        "wait until the agent has started, then rename the channel again",
                        Some(busy_kind(item)),
                    ));
                }
                AgentState::Running { holder_pid }
                    if !confirmed
                        && crate::session_service::running_turn(&item.session, holder_pid) != pb::SessionStatus::Idle =>
                {
                    return Err(failed(
                        Code::FailedPrecondition,
                        &RenameError::Busy { title: item.title.clone() },
                        "wait until it is between turns and rename the channel again, or rename it with confirm_running set (humata channel rename --confirm-running) to cut the turn off: the agent resumes its conversation in the new folder",
                        Some(busy_kind(item)),
                    ));
                }
                _ => {}
            }
        }
        let placement = Placement {
            name,
            folder,
            old_path,
            new_path,
            now_unix_ms: now_ms(),
        };
        let running: Vec<&Live> = live
            .iter()
            .filter(|item| matches!(item.agent.state, AgentState::Running { .. }))
            .collect();
        let (store, id, planned, here) =
            (self.starter.store.clone(), channel.id.clone(), placement.clone(), self.machine.clone());
        let moves = blocking(move || store.conversation_moves(&id, Some(&here), &planned))
            .await?
            .map_err(|error| status::store(error, &self.machine, "list the conversations that move with the folder"))?;
        let intent = Intent {
            channel: channel.id.clone(),
            name: placement.name.clone(),
            old_folder: channel.folder.clone(),
            new_folder: placement.folder.clone(),
            moves,
            resume: running.iter().map(|item| item.session.clone()).collect(),
        };
        let note = note_path(&layout.root(), &channel.id);
        let (at, writing) = (note.clone(), intent);
        let intent = blocking(move || write_note(&at, &writing).map(|()| writing))
            .await?
            .map_err(|error| self.rename_status(&error))?;
        for item in &running {
            if let Err(source) = self.stop_for_rename(item).await {
                let error = RenameError::Stop {
                    session: item.session.clone(),
                    source,
                };
                self.give_up(&note, &intent);
                return Err(self.rename_status(&error));
            }
        }
        // A kill here leaves the note, the stopped agents and the folder
        // where it was: the restart resumes them.
        crate::fault::point("rename_channel.stopped");
        let (from, to) = (placement.old_path.clone(), placement.new_path.clone());
        let root = layout.root();
        let renamed = blocking(move || {
            humata2_durable::rename(&from, &to).and_then(|()| humata2_durable::sync_dir(&root)).map_err(|source| {
                RenameError::Move {
                    from,
                    to,
                    source,
                }
            })
        })
        .await?;
        if let Err(error) = renamed {
            self.give_up(&note, &intent);
            return Err(self.rename_status(&error));
        }
        // A kill here leaves the folder moved and the records not: the
        // restart finishes the move from the note.
        crate::fault::point("rename_channel.moved");
        let done = self.finish_move(&placement, intent.moves.clone(), &channel.id, expected).await?;
        let _ = std::fs::remove_file(&note);
        let (feed, store, id, by) = (
            self.operations.feed(),
            self.starter.store.clone(),
            done.id.clone(),
            identity::actor(caller),
        );
        let _ = blocking(move || tell_channel(&feed, &store, &id, pb::Change::Updated, Some(by))).await;
        self.resume_all(&intent.resume);
        Ok(done)
    }

    /// Refuses a folder that is already there, or that another channel has.
    async fn check_free(&self, channel: &ChannelRecord, name: &str, folder: &str, at: &Path) -> Result<(), Status> {
        let (store, id, wanted, path) = (
            self.starter.store.clone(),
            channel.id.clone(),
            folder.to_owned(),
            at.to_owned(),
        );
        let taken = blocking(move || -> Result<bool, StoreError> {
            let held = store.channels()?.into_iter().any(|other| other.id != id && other.folder == wanted);
            Ok(held || fs::symlink_metadata(&path).is_ok())
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "check the channel's new folder"))?;
        if taken {
            return Err(failed(
                Code::AlreadyExists,
                &RenameError::Taken {
                    name: name.to_owned(),
                    folder: at.to_owned(),
                },
                "pick another name for the channel, or move what is at that folder away and rename it again",
                Some(pb::error::Kind::NameTaken(pb::NameTaken {
                    kind: pb::EntityKind::Channel as i32,
                    name: folder.to_owned(),
                    suggestion: format!("{folder}-2"),
                })),
            ));
        }
        Ok(())
    }

    async fn live_of(&self, channel: &str) -> Result<Vec<Live>, Status> {
        let (store, id) = (self.starter.store.clone(), channel.to_owned());
        blocking(move || live_agents(&store, &id))
            .await?
            .map_err(|error| status::store(error, &self.machine, "read the channel's sessions"))
    }

    /// Stops the agent as StopSession does, but keeps the session's pending
    /// requests: the one that asked for the rename may be the agent's own.
    async fn stop_for_rename(&self, item: &Live) -> Result<(), Status> {
        let AgentState::Running { holder_pid } = item.agent.state else {
            return Ok(());
        };
        // A holder already gone is as stopped as one that was asked to.
        if let Err(error) = holder_end::shut_or_end(
            &item.agent.launch.holder_socket,
            holder_pid,
            &format!("session {}'s terminal", item.session),
        )
        .await
        {
            crate::log::say!("humatad2: {error}");
        }
        let mut agent = item.agent.clone();
        agent.state = AgentState::Stopped;
        agent.updated_unix_ms = now_ms();
        let store = self.starter.store.clone();
        blocking(move || store.put_agent(&agent))
            .await?
            .map_err(|error| status::store(error, &self.machine, "save that the agent stopped"))
    }

    /// The steps after the folder moved, each safe to run again: the files
    /// that name the old folder, the conversations, and the records in one
    /// transaction. `expected` is the revision the rename was asked against.
    async fn finish_move(
        &self,
        placement: &Placement,
        moves: Vec<(PathBuf, PathBuf)>,
        channel: &str,
        expected: Option<u64>,
    ) -> Result<ChannelRecord, Status> {
        let (store, placement, id, here) =
            (self.starter.store.clone(), placement.clone(), channel.to_owned(), self.machine.clone());
        let (agents, home) = (self.starter.agents.clone(), self.starter.home.clone());
        let scratch = self.starter.layout.root().join(FOLLOW_SCRATCH);
        let done = blocking(move || -> Result<ChannelRecord, RenameError> {
            let to = placement.new_path.clone();
            move_folder::rewrite_folders(&placement)
                .and_then(|()| move_folder::carry_conversations(&placement, &moves))
                .map_err(|source| RenameError::Files { to: to.clone(), source })?;
            follow_conversations(&store, &agents, &home, &scratch, &placement, &id, &here)?;
            // A kill here leaves the files and conversations moved: the
            // restart runs the steps again.
            crate::fault::point("rename_channel.files");
            store
                .move_channel(&id, Some(&here), &placement, expected)
                .map_err(|source| RenameError::Records { to, source })
        })
        .await?;
        done.map_err(|error| self.rename_status(&error))
    }

    /// Takes a rename back that stopped before the folder moved: the note
    /// goes and the agents it stopped run again.
    fn give_up(&self, note: &Path, intent: &Intent) {
        let _ = fs::remove_file(note);
        self.resume_all(&intent.resume);
    }

    fn rename_status(&self, error: &RenameError) -> Status {
        match error {
            RenameError::Taken { .. } | RenameError::Busy { .. } | RenameError::Starting { .. } => {
                failed(Code::FailedPrecondition, error, "rename the channel again once that is sorted out", None)
            }
            RenameError::Stop { .. } | RenameError::Note { .. } | RenameError::Move { .. } => failed(
                Code::Unavailable,
                error,
                "check that the Humata folder can be written and that the disk has room, then rename the channel again; its agents were started again where they were",
                None,
            ),
            RenameError::Files { .. } | RenameError::Follow { .. } | RenameError::Records { .. } => failed(
                Code::Internal,
                error,
                "restart humatad2: it finishes the move from the note it kept, and nothing is lost",
                None,
            ),
        }
    }

    /// Resumes each of `sessions` as the person's own Resume does, in the
    /// folder its record names now, without waiting: a resume may wait for
    /// the Mac's rules. One that can't be is logged: it stays stopped, and
    /// its session shows how to resume it.
    fn resume_all(&self, sessions: &[String]) {
        if sessions.is_empty() {
            return;
        }
        let (me, sessions) = (self.clone(), sessions.to_vec());
        tokio::spawn(async move { me.resume_each(&sessions).await });
    }

    async fn resume_each(&self, sessions: &[String]) {
        let Some(svc) = self.sessions.get() else {
            crate::log::say!(
                "humatad2: the sessions {sessions:?} were stopped for a channel's rename and can't be resumed here: resume them from their menus"
            );
            return;
        };
        for session in sessions {
            let mut request = Request::new(pb::ResumeSessionRequest {
                id: Some(pb::SessionId { value: session.clone() }),
                ..pb::ResumeSessionRequest::default()
            });
            request.extensions_mut().insert(Caller::User(UserVia::Host));
            if let Err(error) = pb::session_service_server::SessionService::resume_session(svc, request).await {
                crate::log::say!(
                    "humatad2: session {session} was stopped for a channel's rename and didn't resume: {}; resume it from its menu",
                    error.message()
                );
            }
        }
    }

    /// Finishes the renames a restart cut short, from the notes they left.
    pub(super) async fn recover_renames(&self) {
        let root = self.starter.layout.root();
        let notes = blocking(move || entries(&root.join(NOTES))).await.unwrap_or_default();
        for (path, intent) in notes {
            self.recover_one(path, intent).await;
        }
    }

    async fn recover_one(&self, note: PathBuf, intent: Intent) {
        let layout = &self.starter.layout;
        let (old, new) = (layout.channel(&intent.old_folder), layout.channel(&intent.new_folder));
        let (store, id) = (self.starter.store.clone(), intent.channel.clone());
        let channel = blocking(move || store.channel(&id)).await.ok().and_then(Result::ok).flatten();
        let moved_only_on_disk = channel.as_ref().is_some_and(|channel| channel.folder == intent.old_folder)
            && fs::symlink_metadata(&new).is_ok()
            && fs::symlink_metadata(&old).is_err();
        if moved_only_on_disk {
            let placement = Placement {
                name: intent.name.clone(),
                folder: intent.new_folder.clone(),
                old_path: old,
                new_path: new,
                now_unix_ms: now_ms(),
            };
            match self.finish_move(&placement, intent.moves.clone(), &intent.channel, None).await {
                Ok(done) => {
                    let (feed, store, id) = (self.operations.feed(), self.starter.store.clone(), done.id);
                    let _ = blocking(move || tell_channel(&feed, &store, &id, pb::Change::Updated, None)).await;
                }
                Err(error) => {
                    // The note stays: the next start tries again.
                    crate::log::say!(
                        "humatad2: couldn't finish renaming channel {}: {}",
                        intent.channel,
                        error.message()
                    );
                    return;
                }
            }
        } else if fs::symlink_metadata(&old).is_ok() && fs::symlink_metadata(&new).is_ok() {
            crate::log::say!(
                "humatad2: the folders {} and {} both exist, so channel {}'s rename can't be finished by itself: keep the one that holds the channel's work and move the other away",
                old.display(),
                new.display(),
                intent.channel
            );
        }
        let _ = fs::remove_file(&note);
        self.resume_all(&intent.resume);
    }
}

/// The notes under `dir` that read, each with the file it was read from.
fn entries(dir: &Path) -> Vec<(PathBuf, Intent)> {
    let Ok(listing) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in listing.flatten() {
        let path = entry.path();
        match fs::read(&path).ok().and_then(|bytes| serde_json::from_slice::<Intent>(&bytes).ok()) {
            Some(intent) => found.push((path, intent)),
            None => crate::log::say!(
                "humatad2: {} is not a note of a rename in progress; remove it if no rename is waiting",
                path.display()
            ),
        }
    }
    found
}

#[cfg(test)]
#[path = "rename_follow_tests.rs"]
mod follow_tests;

/// How a session that blocks a rename is named in the error.
fn busy_kind(item: &Live) -> pb::error::Kind {
    pb::error::Kind::SessionBusy(pb::SessionBusy {
        session: Some(pb::SessionId { value: item.session.clone() }),
        running: None,
        running_title: item.title.clone(),
    })
}
