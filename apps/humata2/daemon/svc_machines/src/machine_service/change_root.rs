//! What changing a machine's Humata folder touches (PL-1, FC-10, ST-5): the
//! channels that live under its old root, the sessions running in them, and
//! This Mac's own root. This Mac isn't in the machine registry (that holds
//! the boxes), so its stored root is a setting of the host's store; until the
//! user changes it, its root is the folder the daemon was started to keep
//! channels in.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use humata2_daemon_ports::Caller;
use humata2_grpc::humata::v1 as pb;
use humata2_rpc_kit::log::say;
use humata2_rpc_kit::{chain, error, error_of, failed, failure, internal, lock, now_ms};
use humata2_sessions::{DESK_CHANNEL, Layout};
use humata2_store::{AgentState, ChannelRecord, Placement, Store, StoreError};
use tonic::{Code, Status};

use super::{PLAN_LIFETIME, machine_id, root_plan_key};
use crate::ports::{
    MachineChannelsPort, MachineOperationsPort, MachineStoragePort, MachineTrashPort, MoveError,
    OperationProgress, OperationStarted,
};

/// The name the CLI and the app call this machine by, beside its id.
pub(super) const THIS_MAC_ALIAS: &str = "this-mac";

/// The setting that holds This Mac's root once the user changed it.
const THIS_MAC_ROOT: &str = "this_machine_root";

/// The setting that holds the roots channels were left in when the root
/// changed for new channels only, newest first, one per line.
const THIS_MAC_OLDER_ROOTS: &str = "this_machine_older_roots";

/// Folders under a root that belong to the machine, not to a channel, and
/// move with the channels: the trash and what other machines sent.
const MACHINE_FOLDERS: [&str; 2] = [".trash", "received"];

/// What a root change reaches on one machine.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Affected {
    /// Ids of the channels whose folders are under the machine's root.
    pub(super) channels: Vec<String>,
    /// Ids of the sessions with an agent running or starting in them.
    pub(super) running: Vec<String>,
}

/// Maps a store error into a tonic `Status`.
fn store_status(err: StoreError, machine: &str, doing: &str) -> Status {
    failed(
        Code::Internal,
        &err,
        &format!("couldn't {doing} for {machine}; check that humatad2's state folder is accessible"),
        None,
    )
}

/// The channels with a folder on `machine`, and their running sessions
/// there. `here` is this daemon's own machine id: a channel naming no
/// machine, and a session recording none, live on it.
pub(super) fn affected(store: &Store, machine: &str, here: &str) -> Result<Affected, StoreError> {
    let channels: Vec<String> = store
        .channels()?
        .into_iter()
        .filter(|channel| {
            if machine == here && channel.machines.is_empty() {
                return true;
            }
            channel.machines.iter().any(|named| named == machine)
        })
        .map(|channel| channel.id)
        .collect();
    let wanted: HashSet<&str> = channels.iter().map(String::as_str).collect();
    let desks: HashSet<String> = store
        .channel_desks()?
        .into_iter()
        .filter(|row| wanted.contains(row.channel.as_str()) && row.closed_unix_ms.is_none())
        .map(|row| row.session_id)
        .collect();
    let mut running = Vec::new();
    for session in store.sessions()? {
        if !wanted.contains(session.channel.as_str()) && !desks.contains(&session.id) {
            continue;
        }
        let lives_on = if session.machine.is_empty() {
            here
        } else {
            session.machine.as_str()
        };
        if lives_on != machine {
            continue;
        }
        let live = store.agent(&session.id)?.is_some_and(|agent| {
            matches!(
                agent.state,
                AgentState::Launching | AgentState::Running { .. } | AgentState::Resuming
            )
        });
        if live {
            running.push(session.id);
        }
    }
    Ok(Affected { channels, running })
}

/// This Mac's root: the one the user saved, else the folder the daemon keeps
/// its channels in.
pub(super) fn this_root(store: &Store, started_with: &Path) -> Result<String, StoreError> {
    Ok(store
        .setting(THIS_MAC_ROOT)?
        .map_or_else(|| started_with.display().to_string(), |saved| saved.value))
}

/// Saves `root` as This Mac's.
pub(super) fn save_this_root(store: &Store, root: &str) -> Result<(), StoreError> {
    let revision = store
        .setting(THIS_MAC_ROOT)?
        .map_or(1, |saved| saved.revision + 1);
    store.put_setting(THIS_MAC_ROOT, root, revision)
}

/// Saves the roots channels were left in.
pub(super) fn save_older_roots(store: &Store, older: &[PathBuf]) -> Result<(), StoreError> {
    let revision = store
        .setting(THIS_MAC_OLDER_ROOTS)?
        .map_or(1, |saved| saved.revision + 1);
    let lines: Vec<String> = older
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    store.put_setting(THIS_MAC_OLDER_ROOTS, &lines.join("\n"), revision)
}

/// `root` as a path: a leading `~` is `home`.
pub(super) fn expand(root: &str, home: &Path) -> PathBuf {
    match root.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None if root == "~" => home.to_owned(),
        None => PathBuf::from(root),
    }
}

/// The layout the daemon starts with: the root the user saved for this Mac
/// if there is one, else `started_with`, and the older roots their channels
/// stayed in. A setting that can't be read is logged and the start's own
/// folder is used, so the daemon still comes up.
pub fn start_layout(store: &Store, started_with: PathBuf, home: &Path) -> Layout {
    let read = |key: &str| match store.setting(key) {
        Ok(saved) => saved.map(|saved| saved.value),
        Err(err) => {
            say!(
                "humatad2: couldn't read the saved Humata folder ({key}): {}; using {} until it is fixed",
                chain(&err),
                started_with.display()
            );
            None
        }
    };
    let layout = Layout::new(
        read(THIS_MAC_ROOT)
            .map_or_else(|| started_with.clone(), |saved| expand(&saved, home)),
    );
    if let Some(older) = read(THIS_MAC_OLDER_ROOTS) {
        layout.set_older_roots(
            older
                .lines()
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .collect(),
        );
    }
    layout
}

/// Why changing this Mac's Humata folder stopped.
#[derive(Debug, thiserror::Error)]
pub(super) enum RootError {
    #[error("couldn't {doing}: {source}; check that humatad2's state folder can be written, then change the Humata folder again")]
    Store {
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error(
        "the new Humata folder {} and the old one {} are inside each other, so the channels can't be moved from one to the other; pick a folder beside the old one",
        new.display(), old.display()
    )]
    Nested { old: PathBuf, new: PathBuf },
    #[error(
        "{what} would move to {}, but something is already there; nothing was moved. Move or rename what is there, or pick another folder, then change the Humata folder again",
        path.display()
    )]
    Taken { what: String, path: PathBuf },
    #[error(
        "{} is on another volume than {}, and a channel moves between volumes only by copying it, which isn't built; nothing was moved. Pick a folder on the same volume, or change the folder for new channels only",
        new.display(), old.display()
    )]
    OtherVolume { old: PathBuf, new: PathBuf },
    #[error("couldn't make the folder {}: {source}; make it yourself or pick another one, then change the Humata folder again", path.display())]
    Make {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "couldn't move {what} from {} to {}: {source}; everything moved before it was put back, so nothing changed. Fix what the message names and change the Humata folder again",
        from.display(), to.display()
    )]
    PutBack {
        what: String,
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: StepError,
    },
    #[error(
        "couldn't move {what} from {} to {}: {source}, and {left} couldn't be put back either. Move each folder named there back to the old Humata folder by hand, then restart humatad2",
        from.display(), to.display()
    )]
    Stuck {
        what: String,
        from: PathBuf,
        to: PathBuf,
        left: String,
        #[source]
        source: StepError,
    },
}

/// What went wrong in one step of moving a folder.
#[derive(Debug, thiserror::Error)]
pub(super) enum StepError {
    #[error("the folder couldn't be renamed: {0}")]
    Rename(#[source] std::io::Error),
    #[error("the files inside couldn't be brought up to date: {0}")]
    Files(#[source] MoveError),
    #[error("the records couldn't be saved: {0}")]
    Records(#[source] StoreError),
}

/// How a root error answers.
pub(super) fn status_of(err: &RootError) -> Status {
    let (code, fix) = match err {
        RootError::Nested { .. } | RootError::Taken { .. } | RootError::OtherVolume { .. } => (
            Code::FailedPrecondition,
            "change what the message names and ask for a new plan",
        ),
        RootError::Make { .. } | RootError::PutBack { .. } => (
            Code::Unavailable,
            "fix what the message names, ask for a new plan and change the Humata folder again",
        ),
        RootError::Store { .. } | RootError::Stuck { .. } => (
            Code::Internal,
            "follow the message, then ask for a new plan and change the Humata folder again",
        ),
    };
    failed(code, err, fix, None)
}

/// What moves: its name for messages, where it is and goes, and which
/// records follow it.
struct Job {
    what: String,
    old: PathBuf,
    new: PathBuf,
    records: Records,
    placement: Placement,
}

enum Records {
    /// A channel's record and its sessions.
    Channel(String),
    /// The Desk's sessions.
    Desk,
    /// Nothing records it.
    None,
}

/// The device a path's folder is on, or its nearest existing parent's.
fn device_of(path: &Path) -> Option<u64> {
    path.ancestors()
        .find_map(|folder| fs::metadata(folder).ok().map(|meta| meta.dev()))
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Everything that moves with the root, or the error that says why nothing
/// can. Changes nothing.
fn jobs(
    store: &Store,
    layout: &Layout,
    channels: &[String],
    new_root: &Path,
) -> Result<Vec<Job>, RootError> {
    let old_root = layout.root();
    if new_root.starts_with(&old_root) || old_root.starts_with(new_root) {
        return Err(RootError::Nested {
            old: old_root,
            new: new_root.to_owned(),
        });
    }
    let mut records: Vec<ChannelRecord> = Vec::new();
    for id in channels {
        let found = store
            .channel(id)
            .map_err(|source| RootError::Store {
                doing: "read a channel",
                source,
            })?;
        records.extend(found);
    }
    let placement = |name: &str, folder: &str, old: &Path, new: &Path| Placement {
        name: name.to_owned(),
        folder: folder.to_owned(),
        old_path: old.to_owned(),
        new_path: new.to_owned(),
        now_unix_ms: now_ms(),
    };
    let mut found = Vec::new();
    for record in &records {
        let old = layout.channel(&record.folder);
        let new = new_root.join(&record.folder);
        if old == new || !exists(&old) {
            continue;
        }
        found.push(Job {
            what: format!("channel {}", record.name),
            placement: placement(&record.name, &record.folder, &old, &new),
            records: Records::Channel(record.id.clone()),
            old,
            new,
        });
    }
    let desk = layout.channel(DESK_CHANNEL);
    if exists(&desk) && desk != new_root.join(DESK_CHANNEL) {
        let new = new_root.join(DESK_CHANNEL);
        found.push(Job {
            what: "the Desk".to_owned(),
            placement: placement(DESK_CHANNEL, DESK_CHANNEL, &desk, &new),
            records: Records::Desk,
            old: desk,
            new,
        });
    }
    for name in MACHINE_FOLDERS {
        let old = layout.channel(name);
        let new = new_root.join(name);
        if exists(&old) && old != new {
            found.push(Job {
                what: format!("the {name} folder"),
                placement: placement(name, name, &old, &new),
                records: Records::None,
                old,
                new,
            });
        }
    }
    for job in &found {
        if exists(&job.new) {
            return Err(RootError::Taken {
                what: job.what.clone(),
                path: job.new.clone(),
            });
        }
        if device_of(&job.old) != device_of(&job.new) {
            return Err(RootError::OtherVolume {
                old: job.old.clone(),
                new: job.new.clone(),
            });
        }
    }
    Ok(found)
}

fn rename_folder(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)?;
    for parent in [to.parent(), from.parent()].into_iter().flatten() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

fn forward(
    store: &Store,
    channels_port: &dyn MachineChannelsPort,
    here: &str,
    job: &Job,
    moved: &mut Carried,
) -> Result<(), StepError> {
    let id = match &job.records {
        Records::Channel(id) => id.as_str(),
        Records::Desk => DESK_CHANNEL,
        Records::None => "",
    };
    if !id.is_empty() {
        moved.moves = store
            .conversation_moves(id, Some(here), &job.placement)
            .map_err(StepError::Records)?;
    }
    rename_folder(&job.old, &job.new).map_err(StepError::Rename)?;
    moved.renamed = true;
    if matches!(job.records, Records::Channel(_)) {
        channels_port
            .rewrite_folders(store, &job.placement.old_path, &job.placement.new_path)
            .map_err(|s| {
                StepError::Files(MoveError {
                    message: s.message().to_owned(),
                    fix: String::new(),
                })
            })?;
    }
    if !id.is_empty() {
        channels_port
            .carry_conversations(store, &job.placement.old_path, &job.placement.new_path)
            .map_err(StepError::Files)?;
    }
    let recorded = match &job.records {
        Records::Channel(id) => store
            .move_channel(id, Some(here), &job.placement, None)
            .map(|_| ()),
        Records::Desk => store.move_machine_sessions(DESK_CHANNEL, here, &job.placement),
        Records::None => Ok(()),
    };
    recorded.map_err(StepError::Records)?;
    moved.recorded = true;
    Ok(())
}

/// What `forward` got done of one job.
#[derive(Default)]
struct Carried {
    moves: Vec<(PathBuf, PathBuf)>,
    renamed: bool,
    recorded: bool,
}

/// Puts a job back as it was; each step can run again.
fn undo(
    store: &Store,
    channels_port: &dyn MachineChannelsPort,
    here: &str,
    job: &Job,
    done: &Carried,
) -> Result<(), StepError> {
    let back = Placement {
        old_path: job.placement.new_path.clone(),
        new_path: job.placement.old_path.clone(),
        ..job.placement.clone()
    };
    if done.renamed && exists(&job.new) && !exists(&job.old) {
        rename_folder(&job.new, &job.old).map_err(StepError::Rename)?;
    }
    if done.renamed {
        if matches!(job.records, Records::Channel(_)) {
            channels_port
                .rewrite_folders(store, &back.old_path, &back.new_path)
                .map_err(|s| {
                    StepError::Files(MoveError {
                        message: s.message().to_owned(),
                        fix: String::new(),
                    })
                })?;
        }
        channels_port
            .carry_conversations(store, &back.old_path, &back.new_path)
            .map_err(StepError::Files)?;
    }
    if done.recorded {
        let recorded = match &job.records {
            Records::Channel(id) => store.move_channel(id, Some(here), &back, None).map(|_| ()),
            Records::Desk => store.move_machine_sessions(DESK_CHANNEL, here, &back),
            Records::None => Ok(()),
        };
        recorded.map_err(StepError::Records)?;
    }
    Ok(())
}

/// Moves `channels` (and the Desk, the trash and what other machines sent)
/// from the layout's root into `new_root`, with their files and records.
/// Then `finish` runs (it saves the new root). One that fails, or a `finish`
/// that does, puts the ones already moved back. Changes nothing when it
/// refuses up front. Blocking.
fn move_everything(
    store: &Store,
    channels_port: &dyn MachineChannelsPort,
    layout: &Layout,
    here: &str,
    channels: &[String],
    new_root: &Path,
    finish: impl FnOnce() -> Result<(), StoreError>,
) -> Result<(), RootError> {
    let all = jobs(store, layout, channels, new_root)?;
    fs::create_dir_all(new_root).map_err(|source| RootError::Make {
        path: new_root.to_owned(),
        source,
    })?;
    let mut done: Vec<(Job, Carried)> = Vec::new();
    for job in all {
        let mut progress = Carried::default();
        match forward(store, channels_port, here, &job, &mut progress) {
            Ok(()) => done.push((job, progress)),
            Err(source) => {
                let (what, from, to) = (job.what.clone(), job.old.clone(), job.new.clone());
                done.push((job, progress));
                return Err(put_back(
                    store,
                    channels_port,
                    here,
                    done,
                    what,
                    from,
                    to,
                    source,
                ));
            }
        }
    }
    if let Err(source) = finish() {
        let what = "the setting that names the new Humata folder".to_owned();
        return Err(put_back(
            store,
            channels_port,
            here,
            done,
            what,
            layout.root(),
            new_root.to_owned(),
            StepError::Records(source),
        ));
    }
    Ok(())
}

/// Undoes `done`, newest first.
#[allow(clippy::too_many_arguments)]
fn put_back(
    store: &Store,
    channels_port: &dyn MachineChannelsPort,
    here: &str,
    done: Vec<(Job, Carried)>,
    what: String,
    from: PathBuf,
    to: PathBuf,
    source: StepError,
) -> RootError {
    let mut left: Vec<String> = Vec::new();
    for (job, progress) in done.iter().rev() {
        if let Err(err) = undo(store, channels_port, here, job, progress) {
            say!(
                "humatad2: couldn't put {} back at {}: {err}",
                job.what,
                job.old.display()
            );
            left.push(format!("{} (now at {})", job.what, job.new.display()));
        }
    }
    if left.is_empty() {
        RootError::PutBack {
            what,
            from,
            to,
            source,
        }
    } else {
        RootError::Stuck {
            what,
            from,
            to,
            left: left.join(", "),
            source,
        }
    }
}

/// Makes `new_root` the one new channels are made in; the channels that are
/// there stay where they are, found under the older roots.
pub(super) fn switch_for_new_channels(
    store: &Store,
    layout: &Layout,
    raw: &str,
    new_root: PathBuf,
) -> Result<(), RootError> {
    let old = layout.root();
    let mut older = layout.older_roots();
    if old != new_root {
        older.retain(|path| *path != old);
        older.insert(0, old);
    }
    older.retain(|path| *path != new_root);
    save_older_roots(store, &older).map_err(|source| RootError::Store {
        doing: "save the old Humata folders",
        source,
    })?;
    save_this_root(store, raw).map_err(|source| RootError::Store {
        doing: "save the new Humata folder",
        source,
    })?;
    layout.set_older_roots(older);
    layout.set_root(new_root);
    Ok(())
}

/// Moves the channels into `new_root`, then makes it the root: saved, and
/// the one every service reads, the trash included.
#[allow(clippy::too_many_arguments)]
pub(super) fn switch_and_move(
    store: &Store,
    channels_port: &dyn MachineChannelsPort,
    layout: &Layout,
    bin: Option<&dyn MachineTrashPort>,
    here: &str,
    raw: &str,
    new_root: PathBuf,
) -> Result<(), RootError> {
    let found = affected(store, here, here).map_err(|source| RootError::Store {
        doing: "list the channels to move",
        source,
    })?;
    let mut older = layout.older_roots();
    older.retain(|path| *path != new_root);
    move_everything(
        store,
        channels_port,
        layout,
        here,
        &found.channels,
        &new_root,
        || {
            save_older_roots(store, &older)?;
            save_this_root(store, raw)
        },
    )?;
    layout.set_older_roots(older);
    layout.set_root(new_root.clone());
    if let Some(bin) = bin {
        bin.move_to_folder(new_root.join(".trash"));
    }
    Ok(())
}

/// Plans handed out: token to what it was made for, and when.
pub(super) type RootPlans = Mutex<HashMap<String, (String, Instant)>>;

/// A plan to change `machine`'s Humata folder to `new_root`: its token,
/// good for ten minutes and once.
pub(super) fn issue_plan(plans: &RootPlans, machine: &str, new_root: &str) -> String {
    let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
    let mut plans = lock(plans);
    plans.retain(|_, (_, made)| made.elapsed() < PLAN_LIFETIME);
    plans.insert(
        token.clone(),
        (root_plan_key(machine, new_root), Instant::now()),
    );
    token
}

/// Uses up the plan `token` if it was made for changing `machine`'s folder
/// to `new_root` and is still good. A token asked for another root stays
/// good for its own.
pub(super) fn take_plan(
    plans: &RootPlans,
    token: &str,
    machine: &str,
    new_root: &str,
) -> Result<(), Status> {
    let key = root_plan_key(machine, new_root);
    let mut plans = lock(plans);
    let found = plans
        .get(token)
        .map(|(planned, made)| (*planned == key, made.elapsed() < PLAN_LIFETIME));
    match found {
        Some((true, true)) => {
            plans.remove(token);
            return Ok(());
        }
        Some((_, false)) => {
            plans.remove(token);
        }
        _ => {}
    }
    Err(failure(
        Code::FailedPrecondition,
        error(
            format!("the plan to change {machine}'s Humata folder to {new_root} is unknown, used or older than ten minutes"),
            "ask for a new plan (PlanChangeRoot) for the folder you want, check what it says, then change it with its token",
            pb::error::Kind::PlanStale(pb::PlanStale {
                changes: Vec::new(),
            }),
        ),
    ))
}

/// This daemon's own root now, and what changing it reaches: its channels
/// and their running sessions. Blocking.
pub(super) fn own_facts(
    storage: &dyn MachineStoragePort,
    here: &str,
) -> Result<(String, Affected), Status> {
    let store = storage.store();
    let root = this_root(store, &storage.root())
        .map_err(|err| store_status(err, here, "read this machine's Humata folder"))?;
    let found = affected(store, here, here)
        .map_err(|err| store_status(err, here, "list the channels under the Humata folder"))?;
    Ok((root, found))
}

/// The refusal for moving channels while sessions run in them, naming the
/// first; none when nothing runs.
pub(super) fn busy_refusal(name: &str, running: &[String]) -> Option<Status> {
    let first = running.first()?;
    Some(failure(
        Code::FailedPrecondition,
        error(
            format!(
                "{} session(s) are running in the channels under {name}'s Humata folder, including {first}, so the channels can't be moved now",
                running.len()
            ),
            "stop those sessions and change the folder again, or change it for new channels only and leave the existing ones where they are",
            pb::error::Kind::SessionBusy(pb::SessionBusy {
                session: Some(pb::SessionId {
                    value: first.clone(),
                }),
                running: None,
                running_title: String::new(),
            }),
        ),
    ))
}

fn pending_step(key: &str, title: &str) -> pb::OperationStep {
    pb::OperationStep {
        key: key.to_owned(),
        title: title.to_owned(),
        state: pb::StepState::Waiting as i32,
        ..pb::OperationStep::default()
    }
}

/// Starts the operation for changing `name`'s Humata folder.
pub(super) fn start_operation(
    operations: &dyn MachineOperationsPort,
    caller: &Caller,
    name: &str,
    move_channels: bool,
    key: Option<pb::IdempotencyKey>,
) -> Result<OperationStarted, Status> {
    let _target = pb::EntityRef {
        entity: Some(pb::entity_ref::Entity::Machine(machine_id(name))),
    };
    let mut steps = Vec::new();
    if move_channels {
        steps.push(pending_step(
            MOVE_CHANNELS,
            "Move the channels to the new Humata folder",
        ));
    }
    steps.push(pending_step(SAVE_ROOT, "Save the new Humata folder"));
    operations.start_steps(caller, pb::OperationKind::ChangeRoot, key, steps)
}

/// Starts the host's operation for changing a box's Humata folder. Besides
/// the box's own steps it has one for the host's records of the box's Desk
/// sessions, which follow the folder when the channels move.
pub(super) fn start_box_operation(
    operations: &dyn MachineOperationsPort,
    caller: &Caller,
    name: &str,
    move_channels: bool,
    key: Option<pb::IdempotencyKey>,
) -> Result<OperationStarted, Status> {
    let _target = pb::EntityRef {
        entity: Some(pb::entity_ref::Entity::Machine(machine_id(name))),
    };
    let mut steps = Vec::new();
    if move_channels {
        steps.push(pending_step(
            MOVE_CHANNELS,
            "Move the channels to the new Humata folder",
        ));
    }
    steps.push(pending_step(SAVE_ROOT, "Save the new Humata folder"));
    if move_channels {
        steps.push(pending_step(
            FOLLOW_SESSIONS,
            "Point the records of its Desk sessions at the new folders",
        ));
    }
    operations.start_steps(caller, pb::OperationKind::ChangeRoot, key, steps)
}

/// The keys of a root change's steps.
pub(super) const MOVE_CHANNELS: &str = "move_channels";
pub(super) const SAVE_ROOT: &str = "save_root";
pub(super) const FOLLOW_SESSIONS: &str = "follow_sessions";

/// Makes `raw` the root of this daemon's own machine, moving its channels
/// into it when asked, and ends `progress` with the outcome. The same for
/// This Mac and for a box's own daemon.
pub(super) async fn apply_here(
    progress: Arc<dyn OperationProgress>,
    storage: Arc<dyn MachineStoragePort>,
    channels_port: Arc<dyn MachineChannelsPort>,
    here: String,
    raw: String,
    move_channels: bool,
) -> Result<pb::Operation, Status> {
    let started = progress.snapshot();
    if move_channels {
        progress.step(0, pb::StepState::Running, String::new());
    }
    let applied = tokio::task::spawn_blocking(move || {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        let path = expand(&raw, &home);
        let layout = Layout::new(storage.root());
        if move_channels {
            let bin = storage.trash_bin();
            switch_and_move(
                storage.store(),
                channels_port.as_ref(),
                &layout,
                bin.as_deref(),
                &here,
                &raw,
                path.clone(),
            )?;
            storage.set_root(path.clone()).map_err(|err| RootError::Store {
                doing: "update the runtime Humata folder",
                source: StoreError::Write {
                    path,
                    source: std::io::Error::other(err.message().to_owned()),
                },
            })
        } else {
            switch_for_new_channels(storage.store(), &layout, &raw, path.clone())?;
            storage.set_root(path.clone()).map_err(|err| RootError::Store {
                doing: "update the runtime Humata folder",
                source: StoreError::Write {
                    path,
                    source: std::io::Error::other(err.message().to_owned()),
                },
            })
        }
    })
    .await
    .map_err(|err| internal(format!("changing the Humata folder stopped: {err}"), ""))?;
    match applied {
        Ok(()) => {
            if move_channels {
                progress.step(0, pb::StepState::Done, String::new());
                progress.step(1, pb::StepState::Done, String::new());
            } else {
                progress.step(0, pb::StepState::Done, String::new());
            }
            progress.finish_ok("", Vec::new());
            Ok(started)
        }
        Err(err) => {
            let status = status_of(&err);
            if let Some(err) = humata2_rpc_kit::status_error_of(&status) {
                progress.finish_err(err);
            }
            Err(status)
        }
    }
}
