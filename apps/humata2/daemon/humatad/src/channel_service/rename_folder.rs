//! A rename moves the channel's folder (MV-16, D314): `<root>/<old>/`
//! becomes `<root>/<new>/` with one `rename(2)`, and the records and the
//! files that name the old folder follow.
//!
//! D801 D: on one filesystem the channel's agents and shells keep running.
//! Each running agent keeps the folder and conversation key it started
//! under, with a note beside its holder (`humata2_revive::RekeyNote`) that
//! its next resume carries its conversation to the new folder's key from; the
//! old folder name stays as a link to the new one while a holder that
//! started under it runs (`rename_alias.rs`). Across filesystems the
//! sessions are stopped before the rename and resumed after it, as before.
//!
//! The order, so a crash at any point leaves something a restart finishes:
//!
//! 1. the name is checked against the folders there are, and, across
//!    filesystems, the channel's agents are checked: one in a turn needs the
//!    person's say-so;
//! 2. a note naming the move, the conversations it carries and the sessions
//!    it stops or keeps is written under `<root>/.renaming`;
//! 3. across filesystems the agents are stopped; on one, each running agent's
//!    re-key note and the record of the old name's link are written;
//! 4. the folder is renamed (swapped with the link, where it can be);
//! 5. the files that name the old folder are rewritten, the conversations
//!    move, and the records change in one transaction; the running agents'
//!    records are put back to what they started under;
//! 6. the note goes and the stopped agents are resumed.
//!
//! A restart that finds a note with the folder moved and the records not
//! does steps 5 and 6; one that finds the folder not moved only resumes.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use humata2_revive::{FollowStep, RekeyError, RekeyNote, Started};
use humata2_sessions::{Agents, FollowError, SetupError, channel_folder};
use humata2_store::{AgentRecord, AgentState, ChannelRecord, Placement, Store, StoreError, TabKind};
use serde::{Deserialize, Serialize};
use tonic::{Code, Request, Status};

use super::move_folder::{self, MoveError};
use super::{ChannelSvc, blocking, failed, setup_status, tell_channel};
use crate::holder_end;
use crate::identity::{self, Caller, UserVia};
use crate::status;
use crate::trash_service::now_ms;

pub(crate) use humata2_svc_channels::rename_alias;

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
    /// The sessions whose agents were stopped, and run again once it is done.
    resume: Vec<String>,
    /// D801 D: the sessions whose agents keep running and whose conversations
    /// are carried to the new folder's key by their next resume.
    #[serde(default)]
    pending: Vec<String>,
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
    #[error(
        "the folder was renamed to {}, but the note that carries session {session}'s conversation to it when the agent is next resumed could not be kept",
        to.display()
    )]
    Rekey {
        to: PathBuf,
        session: String,
        #[source]
        source: RekeyError,
    },
    #[error("the folder's old name could not be kept as a link to the new one")]
    Alias {
        #[source]
        source: rename_alias::AliasError,
    },
    #[error("the folder was renamed to {}, but the channel's records could not be saved under it", to.display())]
    Records {
        to: PathBuf,
        #[source]
        source: StoreError,
    },
}

/// An agent of the channel and the session it belongs to.
#[derive(Clone)]
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

/// Fallback sweep interval for links to renamed folders; `Ended` and
/// `CloseTab` events wake the watcher immediately via [`wake_aliases`].
const ALIAS_POLL: std::time::Duration = std::time::Duration::from_secs(30);

fn alias_wake() -> &'static tokio::sync::Notify {
    static WAKE: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();
    WAKE.get_or_init(tokio::sync::Notify::new)
}

/// Wakes the rename-alias cleanup task immediately (e.g. when a session ends
/// or a tab closes).
pub(crate) fn wake_aliases() {
    alias_wake().notify_one();
}

/// Where the conversations that follow a rename are staged, in the channels
/// folder.
const FOLLOW_SCRATCH: &str = ".renaming-stage";

/// What the conversations follow with.
#[derive(Clone, Copy)]
struct Following<'a> {
    store: &'a Store,
    agents: &'a Agents,
    home: &'a Path,
    scratch: &'a Path,
    here: &'a str,
}

/// MV-16: the conversations that keep their agent's folder inside them
/// (Codex's, Copilot's) are placed again where they are by their adapter, so
/// their files and, for Codex, its thread row name the new folder; Claude's
/// move with their key (`carry_conversations`). Each session's folders are
/// its own and its clones' and the one the conversation records for itself
/// (the agent's, for the current one), that one last, as the thread row
/// keeps the last. A previous conversation's own folder is read from its
/// file. Each step can run again, and runs before the records change.
///
/// D801 D: a session of `pending` has an agent that keeps running under its
/// old folder, so its steps are not run: they are answered, by session, for
/// the note its next resume follows them from, worked out from the folder
/// the agent started under.
fn follow_or_defer(
    with: Following<'_>,
    placement: &Placement,
    channel: &str,
    pending: &HashSet<String>,
) -> Result<Vec<(String, Vec<FollowStep>)>, RenameError> {
    let Following { store, agents, home, scratch, here } = with;
    let records = |source: StoreError| RenameError::Records { to: placement.new_path.clone(), source };
    let desks: HashSet<String> = store
        .channel_desks()
        .map_err(records)?
        .into_iter()
        .filter(|row| row.channel == channel)
        .map(|row| row.session_id)
        .collect();
    let mut deferred = Vec::new();
    for session in store.sessions().map_err(records)? {
        if session.machine != here || (session.channel != channel && !desks.contains(&session.id)) {
            continue;
        }
        let Some(agent) = store.agent(&session.id).map_err(records)? else { continue };
        let Ok(kind) = agents.get_as_ran(&session.id, &agent.launch.agent) else { continue };
        let later = pending.contains(&session.id);
        let started_under;
        let placement = match humata2_revive::read_note(&agent.launch.holder_socket) {
            Ok(Some(note)) if later => {
                started_under = Placement { old_path: note.path, ..placement.clone() };
                &started_under
            }
            _ => placement,
        };
        let stage = scratch.join(&session.id);
        let mut steps = Vec::new();
        for (id, file, from, to) in folder_moves(store, &kind, &session.id, &session.folder, &agent, placement).map_err(records)? {
            if later {
                steps.push(FollowStep {
                    agent: agent.launch.agent.clone(),
                    id,
                    file,
                    from,
                    to,
                    home: home.to_owned(),
                    scratch: stage.clone(),
                });
                continue;
            }
            kind.follow_folder(home, &id, &file, &from, &to, &stage).map_err(|source| RenameError::Follow {
                to: placement.new_path.clone(),
                session: session.id.clone(),
                source,
            })?;
        }
        if later {
            deferred.push((session.id.clone(), steps));
        }
    }
    Ok(deferred)
}

/// Each conversation of `session`'s agent (its current one and the ones
/// before it) with each folder it can name that `placement` moves: the
/// session's own, its clones' and the one the conversation records for
/// itself (the agent's, for the current one), that one last, as a thread
/// row keeps the last.
fn folder_moves(
    store: &Store,
    kind: &humata2_sessions::AgentKind,
    session: &str,
    folder: &Path,
    agent: &AgentRecord,
    placement: &Placement,
) -> Result<Vec<(String, PathBuf, PathBuf, PathBuf)>, StoreError> {
    // The moved folder itself maps to the new one as is: joining an empty
    // rest would add a trailing slash the CLI then writes into its paths.
    let moved = |old: &Path| {
        old.strip_prefix(&placement.old_path).ok().map(|rest| {
            if rest.as_os_str().is_empty() { placement.new_path.clone() } else { placement.new_path.join(rest) }
        })
    };
    let mut above: Vec<PathBuf> = vec![folder.to_owned()];
    if let Some(detail) = store.session_detail(session)? {
        above.extend(detail.repos.into_iter().map(|repo| repo.path));
    }
    let mut conversations = vec![(agent.launch.conversation_id.clone(), agent.launch.conversation_file.clone())];
    conversations.extend(store.previous_conversations(session)?.into_iter().map(|one| (one.conversation_id, one.path)));
    let mut found = Vec::new();
    for (id, file) in conversations {
        // The folder the conversation itself records; the agent's for the
        // current one.
        let cwd = if file == agent.launch.conversation_file {
            Some(agent.launch.folder.clone())
        } else {
            kind.recorded_folder(&file)
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
        for old in others {
            if let Some(new) = moved(&old) {
                found.push((id.clone(), file.clone(), old, new));
            }
        }
    }
    Ok(found)
}

/// D801 D: how the conversations of `session`'s running agent follow its
/// folder from `from` to `to` at its next resume, for its re-key note. The
/// resume runs each with the agent's kind, which changes only what records
/// its folder inside a conversation (Codex's, Copilot's). An agent of a kind
/// this machine doesn't know has none. `channels` is the channels folder,
/// whose staging folder they use.
pub(crate) fn session_follows(
    store: &Store,
    agents: &Agents,
    home: &Path,
    channels: &Path,
    session: &str,
    from: &Path,
    to: &Path,
) -> Result<Vec<FollowStep>, StoreError> {
    let Some(agent) = store.agent(session)? else { return Ok(Vec::new()) };
    let Ok(kind) = agents.get_as_ran(session, &agent.launch.agent) else { return Ok(Vec::new()) };
    let placement = Placement {
        name: String::new(),
        folder: String::new(),
        old_path: from.to_owned(),
        new_path: to.to_owned(),
        now_unix_ms: 0,
    };
    let stage = channels.join(FOLLOW_SCRATCH).join(session);
    Ok(folder_moves(store, &kind, session, from, &agent, &placement)?
        .into_iter()
        .map(|(id, file, from, to)| FollowStep {
            agent: agent.launch.agent.clone(),
            id,
            file,
            from,
            to,
            home: home.to_owned(),
            scratch: stage.clone(),
        })
        .collect())
}

/// [`follow_or_defer`] for a channel none of whose agents keeps running.
#[cfg(test)]
fn follow_conversations(
    store: &Store,
    agents: &Agents,
    home: &Path,
    scratch: &Path,
    placement: &Placement,
    channel: &str,
    here: &str,
) -> Result<(), RenameError> {
    let with = Following { store, agents, home, scratch, here };
    follow_or_defer(with, placement, channel, &HashSet::new()).map(|_| ())
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

/// Whether a folder renamed inside `root` stays on one filesystem, so a
/// `rename(2)` of it works under what runs in it.
fn same_filesystem(folder: &Path, root: &Path) -> bool {
    match (fs::metadata(folder), fs::metadata(root)) {
        (Ok(folder), Ok(root)) => folder.dev() == root.dev(),
        _ => true,
    }
}

/// The sessions of `channel`, and of its open Desk sessions.
fn channel_sessions(store: &Store, channel: &str) -> Result<HashSet<String>, StoreError> {
    let desks: HashSet<String> = store
        .channel_desks()?
        .into_iter()
        .filter(|row| row.channel == channel && row.closed_unix_ms.is_none())
        .map(|row| row.session_id)
        .collect();
    Ok(store
        .sessions()?
        .into_iter()
        .filter(|session| session.channel == channel || desks.contains(&session.id))
        .map(|session| session.id)
        .collect())
}

/// The holders that run in the channel's folder now and started under the
/// name it has: its running agents' and its sessions' shells'.
fn channel_holders(
    store: &Store,
    channel: &str,
    live: &[Live],
) -> Result<Vec<rename_alias::Holder>, StoreError> {
    let sessions = channel_sessions(store, channel)?;
    let mut found: Vec<rename_alias::Holder> = Vec::new();
    for item in live {
        if let AgentState::Running { holder_pid } = item.agent.state {
            found.push(rename_alias::Holder { socket: item.agent.launch.holder_socket.clone(), pid: holder_pid });
        }
    }
    for tab in store.tabs()? {
        if tab.kind == TabKind::Shell && tab.session.as_ref().is_some_and(|session| sessions.contains(session)) {
            found.push(rename_alias::Holder { socket: tab.holder_socket, pid: tab.holder_pid });
        }
    }
    found.retain(rename_alias::alive);
    found.dedup();
    Ok(found)
}

/// D801 D: writes, for each agent of `live` that runs or already has a note
/// waiting, the note its next resume follows to carry the conversation to
/// the folder `to` names. Answers the sessions that have one and the
/// conversation files they started under, which the rename's own carrying
/// leaves where they are. An agent whose channel is back at the name it
/// started under has nothing left to carry.
fn plan_pending(
    live: &[Live],
    from: &Path,
    to: &Path,
) -> Result<(Vec<String>, Vec<PathBuf>), RenameError> {
    let rekey = |session: &str| {
        let (to, session) = (to.to_owned(), session.to_owned());
        move |source: RekeyError| RenameError::Rekey { to, session, source }
    };
    let (mut sessions, mut files) = (Vec::new(), Vec::new());
    for item in live {
        let launch = &item.agent.launch;
        let socket = &launch.holder_socket;
        let existing = humata2_revive::read_note(socket).map_err(rekey(&item.session))?;
        let mut note = match existing {
            // Back at the name it started under: nothing to carry.
            Some(note) if note.path == to => {
                humata2_revive::remove_note(socket);
                continue;
            }
            // Renamed before and not resumed since: the same start, a newer
            // folder. A note of a rename that never happened (its start is
            // the folder this one moves) is no such thing.
            Some(note) if note.path != from => RekeyNote { target: to.to_owned(), ..note },
            _ if matches!(item.agent.state, AgentState::Running { .. }) => RekeyNote {
                path: from.to_owned(),
                target: to.to_owned(),
                started: Started {
                    folder: launch.folder.clone(),
                    argv: launch.argv.clone(),
                    resume_argv: launch.resume_argv.clone(),
                    conversation_file: launch.conversation_file.clone(),
                },
                known: Vec::new(),
                follows: Vec::new(),
            },
            _ => continue,
        };
        let seen = Placement {
            name: String::new(),
            folder: String::new(),
            old_path: note.path.clone(),
            new_path: from.to_owned(),
            now_unix_ms: 0,
        };
        note.known = move_folder::known_folders(&seen).map_err(|source| RenameError::Files { to: to.to_owned(), source })?;
        humata2_revive::write_note(socket, &note).map_err(rekey(&item.session))?;
        files.push(note.started.conversation_file.clone());
        sessions.push(item.session.clone());
    }
    Ok((sessions, files))
}

/// Keeps `steps`, how the conversations of `session`'s agent follow its
/// folder, in the note its next resume reads.
fn keep_follows(store: &Store, session: &str, steps: Vec<FollowStep>, to: &Path) -> Result<(), RenameError> {
    let rekey = |source| RenameError::Rekey { to: to.to_owned(), session: session.to_owned(), source };
    let records = |source| RenameError::Records { to: to.to_owned(), source };
    let Some(agent) = store.agent(session).map_err(records)? else { return Ok(()) };
    let socket = &agent.launch.holder_socket;
    let Some(mut note) = humata2_revive::read_note(socket).map_err(rekey)? else { return Ok(()) };
    if note.follows != steps {
        note.follows = steps;
        humata2_revive::write_note(socket, &note).map_err(rekey)?;
    }
    Ok(())
}

/// Puts `session`'s agent record back to what its agent started under, as
/// its note keeps it: the agent still runs there, and what reads its
/// conversation finds it where it is, until its next resume re-keys it.
fn restore_started(store: &Store, session: &str, to: &Path) -> Result<(), RenameError> {
    let records = |source| RenameError::Records { to: to.to_owned(), source };
    let Some(mut agent) = store.agent(session).map_err(records)? else { return Ok(()) };
    let note = humata2_revive::read_note(&agent.launch.holder_socket)
        .map_err(|source| RenameError::Rekey { to: to.to_owned(), session: session.to_owned(), source })?;
    let Some(note) = note else { return Ok(()) };
    let started = note.started;
    let launch = &mut agent.launch;
    if launch.folder == started.folder
        && launch.argv == started.argv
        && launch.resume_argv == started.resume_argv
        && launch.conversation_file == started.conversation_file
    {
        return Ok(());
    }
    launch.folder = started.folder;
    launch.argv = started.argv;
    launch.resume_argv = started.resume_argv;
    launch.conversation_file = started.conversation_file;
    agent.updated_unix_ms = now_ms();
    store.put_agent(&agent).map_err(records)
}

impl ChannelSvc {
    /// The folder `name` gives `channel`, when it is not the one it has.
    /// The PR reviews channel's folder is Humata's own and stays.
    pub(super) fn folder_for(&self, channel: &ChannelRecord, name: &str) -> Result<Option<String>, Status> {
        if channel.folder == super::PR_REVIEWS_FOLDER {
            return Ok(None);
        }
        let folder = channel_folder(name).map_err(|error| setup_status(SetupError::Name(error), &self.machine))?;
        Ok((folder != channel.folder).then_some(folder))
    }

    /// Renames `channel` to `name` and moves its folder to `folder`.
    /// `confirmed` says the person agreed to cut off a turn, which only a
    /// rename across filesystems does (D801 D).
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
        let root = layout.root();
        self.check_free(&channel, &name, &folder, &new_path).await?;
        let live = self.live_of(&channel.id).await?;
        // Within one filesystem the folder is renamed under what runs in it;
        // across two it can't be, so the agents stop and start again.
        let keeps_running = same_filesystem(&old_path, &root);
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
                    if !keeps_running
                        && !confirmed
                        && crate::session_service::running_turn(&item.session, holder_pid) != pb::SessionStatus::Idle =>
                {
                    return Err(failed(
                        Code::FailedPrecondition,
                        &RenameError::Busy { title: item.title.clone() },
                        "this channel's folder is on another disk than the Humata folder, so the rename stops the agent and starts it again in the new folder; wait until it is between turns and rename the channel again, or rename it with confirm_running set (humata channel rename --confirm-running) to cut the turn off",
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
        let running: Vec<Live> = live
            .iter()
            .filter(|item| matches!(item.agent.state, AgentState::Running { .. }))
            .cloned()
            .collect();
        let stopping: Vec<&Live> = if keeps_running { Vec::new() } else { running.iter().collect() };
        let (store, id, planned, here) =
            (self.starter.store.clone(), channel.id.clone(), placement.clone(), self.machine.clone());
        let mut moves = blocking(move || store.conversation_moves(&id, Some(&here), &planned))
            .await?
            .map_err(|error| status::store(error, &self.machine, "list the conversations that move with the folder"))?;
        let mut intent = Intent {
            channel: channel.id.clone(),
            name: placement.name.clone(),
            old_folder: channel.folder.clone(),
            new_folder: placement.folder.clone(),
            moves: Vec::new(),
            resume: stopping.iter().map(|item| item.session.clone()).collect(),
            pending: Vec::new(),
        };
        // D801 D: each agent that keeps running keeps its conversation where
        // it is, and the rename's own carrying leaves it be: its next resume
        // carries it. Written before the folder moves, so a restart that
        // finds the folder moved finds the notes too.
        let mut holders = Vec::new();
        if keeps_running {
            let (store, live_now) = (self.starter.store.clone(), live.clone());
            let (from, to, channel_id) = (placement.old_path.clone(), placement.new_path.clone(), channel.id.clone());
            let planned = blocking(move || -> Result<(Vec<String>, Vec<PathBuf>, Vec<rename_alias::Holder>), RenameError> {
                let (pending, files) = plan_pending(&live_now, &from, &to)?;
                let holders = channel_holders(&store, &channel_id, &live_now)
                    .map_err(|source| RenameError::Records { to: to.clone(), source })?;
                Ok((pending, files, holders))
            })
            .await?
            .map_err(|error| self.rename_status(&error))?;
            let (pending, files, found) = planned;
            moves.retain(|(from, _)| !files.contains(from));
            intent.pending = pending;
            holders = found;
        }
        intent.moves = moves;
        let note = note_path(&layout.root(), &channel.id);
        let (at, writing) = (note.clone(), intent);
        let intent = blocking(move || write_note(&at, &writing).map(|()| writing))
            .await?
            .map_err(|error| self.rename_status(&error))?;
        for item in &stopping {
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
        let (root, channel_id, link) = (root.clone(), channel.id.clone(), !holders.is_empty());
        let renamed = blocking(move || -> Result<rename_alias::Aliases, RenameError> {
            // The rename back to a name this channel's own link holds takes
            // the name from it.
            if rename_alias::is_alias(&root, &channel_id, &to) {
                rename_alias::release(&root, &channel_id, &to).map_err(|source| RenameError::Alias { source })?;
            }
            let before = rename_alias::begin(&root, &channel_id, &from, &to, &holders)
                .map_err(|source| RenameError::Alias { source })?;
            let moved = rename_alias::rename_folder(&from, &to, link)
                .and_then(|()| humata2_durable::sync_dir(&root))
                .map_err(|source| RenameError::Move { from: from.clone(), to: to.clone(), source });
            if let Err(error) = moved {
                rename_alias::undo(&root, &channel_id, &before);
                return Err(error);
            }
            Ok(before)
        })
        .await?;
        if let Err(error) = renamed {
            self.give_up(&note, &intent);
            return Err(self.rename_status(&error));
        }
        // A kill here leaves the folder moved and the records not: the
        // restart finishes the move from the note.
        crate::fault::point("rename_channel.moved");
        let done = self.finish_move(&placement, intent.moves.clone(), &channel.id, expected, &intent.pending).await?;
        let _ = std::fs::remove_file(&note);
        self.settle_aliases(&channel.id, &placement.old_path).await;
        let (feed, store, agents, id, by) = (
            self.operations.feed(),
            self.starter.store.clone(),
            self.starter.agents.clone(),
            done.id.clone(),
            identity::actor(caller),
        );
        let _ = blocking(move || tell_channel(&feed, &store, &agents, &id, pb::Change::Updated, Some(by))).await;
        self.resume_all(&intent.resume);
        Ok(done)
    }

    /// D801 D: after a rename, the channel's other links point at the new
    /// folder, and the links are watched until the holders under them end.
    pub(super) async fn settle_aliases(&self, channel: &str, renamed: &Path) {
        let (root, id, old) = (self.starter.layout.root(), channel.to_owned(), renamed.to_owned());
        let settled = blocking(move || {
            rename_alias::repoint(&root, &id, &old)?;
            // A Desk session moved into the channel keeps its own link.
            let new = rename_alias::load(&root, &id)?.target;
            if new.as_os_str().is_empty() {
                return Ok(());
            }
            rename_alias::repoint_under(&root, &id, &old, &new)
        })
        .await;
        match settled {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::log::say!("humatad2: {}", status::chain(&error)),
            Err(error) => crate::log::say!(
                "humatad2: couldn't point the old names of channel {channel}'s folder at it: {}",
                error.message()
            ),
        }
        self.watch_aliases();
    }

    /// Removes the links to renamed folders whose holders have all ended, as
    /// they do and now, and keeps looking while any remains. One watcher
    /// runs for the daemon.
    pub(crate) fn watch_aliases(&self) {
        static WATCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        use std::sync::atomic::Ordering;
        if WATCHING.swap(true, Ordering::AcqRel) {
            wake_aliases();
            return;
        }
        let root = self.starter.layout.root();
        tokio::spawn(async move {
            let mut wait = std::time::Duration::from_millis(250);
            loop {
                let at = root.clone();
                let kept = blocking(move || rename_alias::sweep(&at)).await.unwrap_or(0);
                if kept == 0 {
                    WATCHING.store(false, Ordering::Release);
                    // One that was recorded as this one looked stays watched.
                    let at = root.clone();
                    let again = blocking(move || rename_alias::sweep(&at)).await.unwrap_or(0);
                    if again == 0 || WATCHING.swap(true, Ordering::AcqRel) {
                        return;
                    }
                }
                match tokio::time::timeout(wait, alias_wake().notified()).await {
                    Ok(()) => {
                        // A wake can arrive right before the exiting holder's PID
                        // disappears; retry quickly once before backing off to ALIAS_POLL.
                        wait = std::time::Duration::from_millis(250);
                    }
                    Err(_) => {
                        wait = (wait * 4).min(ALIAS_POLL);
                    }
                }
            }
        });
    }

    /// Refuses a folder that is already there, or that another channel has.
    async fn check_free(&self, channel: &ChannelRecord, name: &str, folder: &str, at: &Path) -> Result<(), Status> {
        let (store, id, wanted, path, root) = (
            self.starter.store.clone(),
            channel.id.clone(),
            folder.to_owned(),
            at.to_owned(),
            self.starter.layout.root(),
        );
        let taken = blocking(move || -> Result<bool, StoreError> {
            let held = store.channels()?.into_iter().any(|other| other.id != id && other.folder == wanted);
            // The link that keeps this channel's own old name is the
            // channel's to take back (D801 D: undo is the rename back).
            Ok(held || (fs::symlink_metadata(&path).is_ok() && !rename_alias::is_alias(&root, &id, &path)))
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
    /// The agents of `pending` sessions kept running: their conversations
    /// wait for their next resume, and their records are put back to what
    /// they started under.
    async fn finish_move(
        &self,
        placement: &Placement,
        moves: Vec<(PathBuf, PathBuf)>,
        channel: &str,
        expected: Option<u64>,
        pending: &[String],
    ) -> Result<ChannelRecord, Status> {
        let (store, placement, id, here) =
            (self.starter.store.clone(), placement.clone(), channel.to_owned(), self.machine.clone());
        let (agents, home) = (self.starter.agents.clone(), self.starter.home.clone());
        let scratch = self.starter.layout.root().join(FOLLOW_SCRATCH);
        let pending = pending.to_vec();
        let done = blocking(move || -> Result<ChannelRecord, RenameError> {
            let to = placement.new_path.clone();
            move_folder::rewrite_folders(&placement)
                .and_then(|()| move_folder::carry_conversations(&placement, &moves))
                .map_err(|source| RenameError::Files { to: to.clone(), source })?;
            let waiting: HashSet<String> = pending.iter().cloned().collect();
            let with = Following { store: &store, agents: &agents, home: &home, scratch: &scratch, here: &here };
            let deferred = follow_or_defer(with, &placement, &id, &waiting)?;
            for (session, steps) in deferred {
                keep_follows(&store, &session, steps, &to)?;
            }
            // A kill here leaves the files and conversations moved: the
            // restart runs the steps again.
            crate::fault::point("rename_channel.files");
            let done = store
                .move_channel(&id, Some(&here), &placement, expected)
                .map_err(|source| RenameError::Records { to: to.clone(), source })?;
            for session in &pending {
                restore_started(&store, session, &to)?;
            }
            Ok(done)
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
            RenameError::Alias { .. } => failed(
                Code::Unavailable,
                error,
                "check that the Humata folder can be written and that the disk has room, then rename the channel again; nothing was renamed",
                None,
            ),
            RenameError::Files { .. }
            | RenameError::Follow { .. }
            | RenameError::Rekey { .. }
            | RenameError::Records { .. } => failed(
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

    /// Finishes the renames a restart cut short, from the notes they left,
    /// and takes away the links to renamed folders that no holder keeps.
    pub(super) async fn recover_renames(&self) {
        let root = self.starter.layout.root();
        let notes = blocking(move || entries(&root.join(NOTES))).await.unwrap_or_default();
        for (path, intent) in notes {
            self.recover_one(path, intent).await;
        }
        self.watch_aliases();
    }

    async fn recover_one(&self, note: PathBuf, intent: Intent) {
        let layout = &self.starter.layout;
        let (old, new) = (layout.channel(&intent.old_folder), layout.channel(&intent.new_folder));
        let (store, id) = (self.starter.store.clone(), intent.channel.clone());
        let channel = blocking(move || store.channel(&id)).await.ok().and_then(Result::ok).flatten();
        // The old name is gone, or a link to the new folder (D801 D).
        let old_is_link = fs::symlink_metadata(&old).is_ok_and(|meta| meta.file_type().is_symlink());
        let moved_only_on_disk = channel.as_ref().is_some_and(|channel| channel.folder == intent.old_folder)
            && fs::symlink_metadata(&new).is_ok_and(|meta| meta.is_dir())
            && (fs::symlink_metadata(&old).is_err() || old_is_link);
        if moved_only_on_disk {
            let placement = Placement {
                name: intent.name.clone(),
                folder: intent.new_folder.clone(),
                old_path: old,
                new_path: new,
                now_unix_ms: now_ms(),
            };
            match self.finish_move(&placement, intent.moves.clone(), &intent.channel, None, &intent.pending).await {
                Ok(done) => {
                    self.settle_aliases(&intent.channel, &placement.old_path).await;
                    let (feed, store, agents, id) =
                        (self.operations.feed(), self.starter.store.clone(), self.starter.agents.clone(), done.id);
                    let _ = blocking(move || tell_channel(&feed, &store, &agents, &id, pb::Change::Updated, None)).await;
                }
                Err(error) => {
                    // The note stays: the next start tries again.
                    crate::log::say!(
                        "humatad2: couldn't finish renaming channel {}: {}",
                        intent.channel,
                        error.message()
                    );
                    // SMB-065: resume the agents the rename stopped even when
                    // recovery couldn't finish the move, so they don't stay
                    // stopped until a later start succeeds.
                    self.resume_all(&intent.resume);
                    return;
                }
            }
        } else if fs::symlink_metadata(&old).is_ok_and(|meta| meta.is_dir()) && fs::symlink_metadata(&new).is_ok() {
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
