//! Changing a box's Humata folder from the host. The box's own daemon moves
//! its channels; the host's operation for the change is made before the box
//! is asked, and kept in the host's store until the box has answered, so a
//! host that stops part-way leaves an operation that is brought back and
//! asks the box again, and Retry asks it again too. A box that already has
//! the folder is not moved twice: the second ask starts from a plan of the
//! box's own, which names the folder it has now.
//!
//! When the box moved its channels, the host's records of the box's Desk
//! sessions follow: they name the box's own paths (the host records the
//! box's other sessions in its own layout, which doesn't change).

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::DESK_CHANNEL;
use humata2_store::{Placement, Store, StoreError};
use prost::Message;
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tonic::{Code, Request, Status};

use super::change_root::{self, FOLLOW_SESSIONS, MOVE_CHANNELS, SAVE_ROOT};
use super::{MachineSvc, status_of};
use crate::channel_service::failed;
use crate::host_relay::{LinkState, assigned};
use crate::identity::Caller;
use crate::operations::{Origin, Progress, Started};
use crate::status;
use crate::trash_service::now_ms;

/// Where a change waits in the host's store, per box.
const JOURNAL: &str = "box_root_change:";

/// One ask of a box to change its Humata folder.
#[derive(Clone)]
struct Change {
    machine: String,
    /// The folder the user asked for, as typed.
    new_root: String,
    /// The folder the host kept for the box before the change.
    old_root: String,
    /// The box's plan the user was shown; later asks make their own.
    plan: Option<pb::PlanToken>,
    move_channels: bool,
    /// Which ask of the box this is; each Retry asks under a new one.
    attempt: u32,
    /// Ask the box for a plan of its own first, because the one the user
    /// was given is spent, or the box that made it restarted.
    fresh: bool,
}

/// What the host keeps of a change until it ends.
struct Journal {
    operation: pb::Operation,
    origin: Origin,
    change: Change,
}

impl Journal {
    fn value(&self) -> Value {
        json!({
            "operation": self.operation.encode_to_vec(),
            "owner": self.origin.owner,
            "scope": self.origin.scope,
            "machine": self.change.machine,
            "new_root": self.change.new_root,
            "old_root": self.change.old_root,
            "move_channels": self.change.move_channels,
            "attempt": self.change.attempt,
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let bytes: Vec<u8> = value
            .get("operation")?
            .as_array()?
            .iter()
            .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
            .collect::<Option<_>>()?;
        let scope = match value.get("scope")? {
            Value::Null => None,
            Value::Array(pair) => match pair.as_slice() {
                [channel, session] => Some((channel.as_str()?.to_owned(), session.as_str()?.to_owned())),
                _ => return None,
            },
            _ => return None,
        };
        Some(Self {
            operation: pb::Operation::decode(bytes.as_slice()).ok()?,
            origin: Origin { owner: text("owner")?, scope },
            change: Change {
                machine: text("machine")?,
                new_root: text("new_root")?,
                old_root: text("old_root")?,
                plan: None,
                move_channels: value.get("move_channels")?.as_bool()?,
                attempt: u32::try_from(value.get("attempt")?.as_u64()?).ok()?,
                fresh: true,
            },
        })
    }
}

fn keep(store: &Store, entry: &Journal) -> Result<(), StoreError> {
    let key = format!("{JOURNAL}{}", entry.change.machine);
    let revision = store.setting(&key)?.map_or(1, |saved| saved.revision + 1);
    store.put_setting(&key, &entry.value().to_string(), revision)
}

fn forget(store: &Store, machine: &str) -> Result<(), StoreError> {
    let key = format!("{JOURNAL}{machine}");
    match store.setting(&key)? {
        Some(saved) if !saved.value.is_empty() => store.put_setting(&key, "", saved.revision + 1),
        _ => Ok(()),
    }
}

fn read(store: &Store, machine: &str) -> Result<Option<Journal>, StoreError> {
    let Some(saved) = store.setting(&format!("{JOURNAL}{machine}"))?.filter(|saved| !saved.value.is_empty()) else {
        return Ok(None);
    };
    let found = serde_json::from_str::<Value>(&saved.value).ok().and_then(|value| Journal::from_value(&value));
    if found.is_none() {
        crate::log::say!(
            "humatad2: couldn't read the change of {machine}'s Humata folder the last run kept, so it isn't asked again. Look at {machine}'s folder on the Machines page and change it again if it is still the old one"
        );
    }
    Ok(found)
}

/// Why a box's answer can't be taken as done.
#[derive(Debug, thiserror::Error)]
enum BoxRootError {
    #[error(
        "{machine} answered the change of its Humata folder without an operation to follow, so the host can't tell whether it moved. Look at {machine}'s Humata folder on the Machines page, then change it again if it is still the old one"
    )]
    NoOperation { machine: String },
    #[error(
        "{machine}'s change of its Humata folder ended {state} before it was done. Look at {machine}'s Humata folder on the Machines page, then Retry the change or change it again"
    )]
    Unfinished { machine: String, state: String },
}

/// Why the host's records of a box's Desk sessions couldn't follow its folder.
#[derive(Debug, thiserror::Error)]
enum FollowError {
    #[error("couldn't read the host's records of {machine}'s Desk sessions: {source}; check that humatad2's state folder can be read, then Retry the change")]
    Read {
        machine: String,
        #[source]
        source: StoreError,
    },
    #[error("couldn't save the new folders of {machine}'s Desk sessions: {source}; check that humatad2's state folder can be written, then Retry the change")]
    Save {
        machine: String,
        #[source]
        source: StoreError,
    },
    #[error(
        "{machine} now keeps its channels in {new_root}, but the host can't tell where that is on {machine}, so its Desk sessions in {} still show the old folder. Start them again to see where they work, and give a full path (one starting with /) the next time you change the folder",
        old.display()
    )]
    NeedsHome { machine: String, new_root: String, old: PathBuf },
}

impl FollowError {
    fn status(&self) -> Status {
        match self {
            Self::Read { .. } | Self::Save { .. } => failed(Code::Internal, self, "follow the message, then Retry the change", None),
            Self::NeedsHome { .. } => failed(Code::FailedPrecondition, self, "start the Desk sessions again to see their folders", None),
        }
    }
}

/// The error an operation records for `status`: the one it carries, else
/// its message.
fn failure_of(status: &Status) -> pb::Error {
    status::error_of(status).unwrap_or_else(|| pb::Error {
        message: status.message().to_owned(),
        fix: "check the message, then Retry the change".to_owned(),
        retry: None,
        kind: None,
    })
}

/// The roots the host's records of `machine`'s Desk sessions are kept under:
/// a plain Desk session works in `<root>/desk/<name>`, a channel's in
/// `<root>/<channel folder>/desk/<name>`. A session in a folder the user
/// picked is under no root. Blocking.
fn roots_of(store: &Store, machine: &str) -> Result<BTreeSet<PathBuf>, StoreError> {
    let channels: HashSet<String> = store.channels()?.into_iter().map(|channel| channel.folder).collect();
    let mut roots = BTreeSet::new();
    for record in store.sessions()? {
        if record.machine != machine || record.channel != DESK_CHANNEL {
            continue;
        }
        let Some(desk) = record.folder.parent().filter(|desk| desk.file_name().is_some_and(|name| name == "desk")) else {
            continue;
        };
        let Some(base) = desk.parent() else { continue };
        let named = base.file_name().and_then(|name| name.to_str()).is_some_and(|name| channels.contains(name));
        let root = if named { base.parent() } else { Some(base) };
        roots.extend(root.map(Path::to_owned));
    }
    Ok(roots)
}

/// Where the folder `old` (a root the host's records name) is after the box
/// changed its folder from `old_root` to `new_root`, both as typed: `new_root`
/// itself when it is a full path, else under the box's home, which `old`
/// shows when the old folder was under it too.
fn new_place(machine: &str, old: &Path, old_root: &str, new_root: &str) -> Result<PathBuf, FollowError> {
    if new_root.starts_with('/') {
        return Ok(PathBuf::from(new_root));
    }
    let home = new_root.strip_prefix("~/").and_then(|rest| {
        let under = old_root.strip_prefix("~/")?;
        let depth = Path::new(under).components().count();
        old.ends_with(under).then(|| old.ancestors().nth(depth).map(|home| home.join(rest)))?
    });
    home.ok_or_else(|| FollowError::NeedsHome {
        machine: machine.to_owned(),
        new_root: new_root.to_owned(),
        old: old.to_owned(),
    })
}

/// Points the host's records of `machine`'s Desk sessions at the folders the
/// box moved them to: each folder, clone and conversation path under the old
/// root becomes the same one under the new. Safe to run again. Blocking.
fn follow_desks(store: &Store, machine: &str, old_root: &str, new_root: &str) -> Result<(), FollowError> {
    let roots = roots_of(store, machine).map_err(|source| FollowError::Read { machine: machine.to_owned(), source })?;
    for old in roots {
        let new = new_place(machine, &old, old_root, new_root)?;
        let placement = Placement {
            name: String::new(),
            folder: String::new(),
            old_path: old,
            new_path: new,
            now_unix_ms: now_ms(),
        };
        store
            .move_machine_sessions(DESK_CHANNEL, machine, &placement)
            .map_err(|source| FollowError::Save { machine: machine.to_owned(), source })?;
    }
    Ok(())
}

impl MachineSvc {
    /// Changes a box's Humata folder on the box's own daemon, with the box's
    /// plan, moving its channels when asked. The host's operation is made
    /// first and ends when the box has answered: failed with the box's own
    /// error when it refuses or can't be reached (TR-6), done once the host
    /// keeps the new folder as the box's. The call answers with the
    /// operation as it started once that is so, and with the box's refusal
    /// otherwise; the operation can be Retried either way.
    pub(super) async fn change_box_root(
        &self,
        caller: &Caller,
        name: &str,
        new_root: String,
        plan: Option<pb::PlanToken>,
        move_channels: bool,
        key: Option<pb::IdempotencyKey>,
    ) -> Result<pb::Operation, Status> {
        if let Some(operation) = self.operations.existing(caller, key.as_ref())? {
            return Ok(operation);
        }
        let old_root = self.work.machines.get(name).map_err(|error| status_of(&error))?.root;
        let progress = match change_root::start_box_operation(&self.operations, caller, name, move_channels, key)? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(operation),
        };
        let started = progress.operation();
        let change = Change {
            machine: name.to_owned(),
            new_root,
            old_root,
            plan,
            move_channels,
            attempt: 0,
            fresh: false,
        };
        // On a task of its own: a caller that goes away mustn't leave the
        // operation running with nothing to end it.
        let svc = self.clone();
        tokio::spawn(async move { svc.drive(progress, change).await })
            .await
            .map_err(|error| status::internal(format!("changing the Humata folder stopped: {error}"), ""))??;
        Ok(started)
    }

    /// Runs one ask of the box to its end, and ends the operation with it.
    async fn drive(&self, progress: Progress, change: Change) -> Result<(), Status> {
        let again = (self.clone(), change.clone());
        progress.rerun_with(Arc::new(move |progress| {
            let mut next = again.1.clone();
            next.attempt += 1;
            next.fresh = true;
            again.0.spawn_drive(progress, next);
        }));
        let outcome = self.go(&progress, &change).await;
        let result = match outcome {
            Ok(()) => {
                progress.succeed(None);
                Ok(())
            }
            Err(status) => {
                progress.fail(failure_of(&status));
                Err(status)
            }
        };
        let machine = change.machine.clone();
        let forgotten = self
            .storage_blocking(move |storage| {
                forget(storage.store(), &machine).map_err(|error| status::store(error, &machine, "forget a finished change of the Humata folder"))
            })
            .await;
        if let Err(status) = forgotten {
            crate::log::say!("humatad2: {}", status.message());
        }
        result
    }

    fn spawn_drive(&self, progress: Progress, change: Change) {
        let svc = self.clone();
        tokio::spawn(async move {
            // The operation records how it ended.
            let _ = svc.drive(progress, change).await;
        });
    }

    async fn go(&self, progress: &Progress, change: &Change) -> Result<(), Status> {
        let entry = Journal { operation: progress.operation(), origin: progress.origin(), change: change.clone() };
        let machine = change.machine.clone();
        self.storage_blocking(move |storage| {
            keep(storage.store(), &entry)
                .map_err(|error| status::store(error, &machine, "keep the change of the Humata folder until the box answers"))
        })
        .await?;
        if change.move_channels {
            progress.set_step(MOVE_CHANNELS, pb::StepState::Running, None);
        }
        let id = progress.operation().id.map(|id| id.value).unwrap_or_default();
        self.ask_box(&id, change).await?;
        if change.move_channels {
            progress.set_step(MOVE_CHANNELS, pb::StepState::Done, None);
        }
        let (work, machine, root) = (self.work.clone(), change.machine.clone(), change.new_root.clone());
        tokio::task::spawn_blocking(move || work.machines.set_root(&machine, &root))
            .await
            .map_err(|error| status::internal(format!("saving the Humata folder stopped: {error}"), ""))?
            .map_err(|error| status_of(&error))?;
        progress.set_step(SAVE_ROOT, pb::StepState::Done, None);
        if change.move_channels {
            progress.set_step(FOLLOW_SESSIONS, pb::StepState::Running, None);
            let (machine, old_root, new_root) = (change.machine.clone(), change.old_root.clone(), change.new_root.clone());
            self.storage_blocking(move |storage| {
                follow_desks(storage.store(), &machine, &old_root, &new_root).map_err(|error| error.status())
            })
            .await?;
            progress.set_step(FOLLOW_SESSIONS, pb::StepState::Done, None);
        }
        Ok(())
    }

    /// Asks the box to change its folder, and waits for its operation to
    /// end. A repeated ask first reads the box's own plan: a box that has
    /// the folder already is done, and the box dedups an ask it is still
    /// working on by its key.
    async fn ask_box(&self, operation: &str, change: &Change) -> Result<(), Status> {
        let mut client = self.storage_client(&change.machine)?;
        let plan = if change.fresh {
            let fresh = client
                .plan_change_root(Request::new(pb::PlanChangeRootRequest { id: None, root: change.new_root.clone() }))
                .await?
                .into_inner();
            if fresh.old_root == change.new_root {
                return Ok(());
            }
            fresh.token
        } else {
            change.plan.clone()
        };
        let reply = client
            .change_root(Request::new(pb::ChangeRootRequest {
                idempotency_key: Some(pb::IdempotencyKey { value: format!("{operation}.{}", change.attempt) }),
                id: None,
                root: change.new_root.clone(),
                presence: None,
                plan,
                move_channels: change.move_channels,
            }))
            .await?
            .into_inner();
        self.box_ended(&change.machine, reply).await
    }

    /// Waits for the box's operation `reply` names to end, and answers its
    /// refusal when it failed.
    async fn box_ended(&self, machine: &str, reply: pb::Operation) -> Result<(), Status> {
        let Some(id) = reply.id.clone() else {
            let error = BoxRootError::NoOperation { machine: machine.to_owned() };
            return Err(failed(Code::Internal, &error, "look at the box's folder, then change it again if it is still the old one", None));
        };
        let request = assigned(pb::WatchOperationRequest { id: Some(id), from_version: 0 }, None).map_err(Status::from)?;
        let mut watch = self
            .work
            .relay_links
            .relayed(machine)
            .map_err(Status::from)?
            .operation()
            .watch_operation(request)
            .await?
            .into_inner();
        let mut last = reply;
        while let Some(next) = watch.message().await? {
            last = next;
        }
        match pb::OperationState::try_from(last.state) {
            Ok(pb::OperationState::Succeeded) => Ok(()),
            Ok(pb::OperationState::Failed) if last.error.is_some() => {
                Err(status::failure(Code::FailedPrecondition, last.error.unwrap_or_default()))
            }
            other => {
                let state = other.map_or_else(|_| "in a state this host doesn't know".to_owned(), |state| format!("{state:?}"));
                let error = BoxRootError::Unfinished { machine: machine.to_owned(), state };
                Err(failed(Code::Unavailable, &error, "look at the box's folder, then Retry the change", None))
            }
        }
    }

    /// At start: brings back each change of a box's folder the last run kept
    /// and didn't end, under its own operation, and asks the box again once
    /// it links. Call it once, before anything is served.
    pub(super) fn recover_box_roots(&self) {
        let Ok(runtime) = Handle::try_current() else { return };
        let Ok(entries) = self.work.machines.list() else { return };
        for entry in entries {
            let kept = match read(self.storage.store(), &entry.name) {
                Ok(kept) => kept,
                Err(error) => {
                    crate::log::say!(
                        "humatad2: couldn't read the change of {}'s Humata folder the last run kept: {}; look at its folder on the Machines page and change it again if it is still the old one",
                        entry.name,
                        status::chain(&error)
                    );
                    continue;
                }
            };
            let Some(kept) = kept else { continue };
            let Some(progress) = self.operations.restore(kept.operation, &kept.origin) else {
                crate::log::say!(
                    "humatad2: couldn't bring back the change of {}'s Humata folder the last run kept, so it isn't asked again; look at its folder on the Machines page and change it again if it is still the old one",
                    entry.name
                );
                continue;
            };
            let (svc, change) = (self.clone(), kept.change);
            runtime.spawn(async move {
                if svc.wait_up(&change.machine).await {
                    let _ = svc.drive(progress, change).await;
                } else {
                    progress.fail(failure_of(&failed(
                        Code::Unavailable,
                        &BoxRootError::Unfinished { machine: change.machine.clone(), state: "when the machine was removed".to_owned() },
                        "add the machine again if you still want it",
                        None,
                    )));
                }
            });
        }
    }

    /// Waits until `machine`'s link is up; false when it is forgotten first.
    async fn wait_up(&self, machine: &str) -> bool {
        let mut state = self.work.relay_links.watch(machine);
        loop {
            if matches!(*state.borrow_and_update(), LinkState::Up { .. }) {
                return true;
            }
            if state.changed().await.is_err() {
                return false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_path_is_the_new_place_whatever_the_old_folder_was() {
        let there = new_place("box-1", Path::new("/home/me/Humata"), "~/Humata", "/srv/humata").expect("a full path needs no home");
        assert_eq!(there, PathBuf::from("/srv/humata"));
    }

    #[test]
    fn a_folder_under_home_follows_the_home_the_old_folder_shows() {
        let there = new_place("box-1", Path::new("/home/me/Humata"), "~/Humata", "~/Work").expect("the old folder shows the home");
        assert_eq!(there, PathBuf::from("/home/me/Work"));
        let deeper = new_place("box-1", Path::new("/home/me/a/b"), "~/a/b", "~/c").expect("the old folder shows the home");
        assert_eq!(deeper, PathBuf::from("/home/me/c"));
    }

    #[test]
    fn a_home_folder_that_cannot_be_placed_is_an_error_not_a_guess() {
        let refused = new_place("box-1", Path::new("/srv/humata"), "/srv/humata", "~/Work");
        assert!(matches!(refused, Err(FollowError::NeedsHome { .. })));
    }
}
