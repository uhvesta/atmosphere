//! What a channel's setup checks before it asks a machine, and how its
//! clones keep out of each other's way (D164).
//!
//! A machine is named in a channel only when the host's machine list has it:
//! one the host never linked would otherwise show Waiting for ever, since
//! nothing would ever bring its link up. And a channel clones one call at a
//! time: a second add while a main clone still runs waits for it instead of
//! cloning into the same folder, then reports what that clone became.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use humata2_grpc::humata::v1 as pb;
use humata2_machines::ManageError;
use humata2_sessions::Starter;
use humata2_store::{ChannelRecord, MainClone};
use tonic::{Code, Status};

use super::clone_in_background;
use crate::machine_service::{self, Work};
use crate::remote_work::HostOp;
use crate::status;

/// Refuses `machine`, named in `field`, when the host's machine list doesn't
/// have it. Reads the list (a small file); call it off the async threads
/// when that matters.
pub(super) fn check_known(work: &Work, field: &str, machine: &str) -> Result<(), Status> {
    match work.machines.get(machine) {
        Ok(_) => Ok(()),
        Err(ManageError::NotFound { .. }) => Err(status::failure(
            Code::NotFound,
            pb::Error {
                message: format!(
                    "{field} ({machine}) is not a machine Humata knows, so the channel can't use it"
                ),
                fix: format!(
                    "add {machine} on the Machines page first (or check the name against the machines Humata lists), then try again"
                ),
                retry: None,
                kind: Some(pb::error::Kind::NotFound(pb::NotFound {
                    kind: pb::EntityKind::Machine as i32,
                    id: machine.to_owned(),
                    in_trash: false,
                })),
            },
        )),
        Err(error) => Err(machine_service::status_of(&error)),
    }
}

/// Whether the host's machine list has `machine`. A machine the list can't
/// be read for counts as known, so the call goes on and its step says what
/// became of it.
pub(super) fn is_known(work: &Work, machine: &str) -> bool {
    !matches!(work.machines.get(machine), Err(ManageError::NotFound { .. }))
}

/// One lock per channel, held while that channel's main clones run.
#[derive(Default)]
pub(super) struct CloneLocks {
    channels: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl CloneLocks {
    fn for_channel(&self, channel: &str) -> Arc<Mutex<()>> {
        let mut channels = self.channels.lock().unwrap_or_else(PoisonError::into_inner);
        channels.entry(channel.to_owned()).or_default().clone()
    }
}

/// Clones `channel`'s mains as [`clone_in_background`] does, once no other
/// call is cloning for the same channel. Then each listed step whose clone
/// another call made meanwhile is set to how that clone ended, so nothing is
/// left pending. Blocking.
pub(super) fn clone_one_at_a_time(
    starter: &Starter,
    channel: &ChannelRecord,
    progress: &HostOp,
    locks: &CloneLocks,
) -> Result<(), pb::Error> {
    let lock = locks.for_channel(&channel.id);
    let _held = lock.lock().unwrap_or_else(PoisonError::into_inner);
    let outcome = clone_in_background(starter, channel, progress);
    tell_settled(starter, &channel.id, progress);
    outcome
}

/// Sets each `clone_main:` step `progress` lists to the clone's state in the
/// store, when that clone has ended.
fn tell_settled(starter: &Starter, channel: &str, progress: &HostOp) {
    let Some(listed) = progress.operation().map(|operation| {
        operation
            .steps
            .into_iter()
            .map(|step| step.key)
            .collect::<HashSet<String>>()
    }) else {
        return;
    };
    let Ok(Some(now)) = starter.store.channel(channel) else {
        return;
    };
    for repo in now.repos {
        let key = format!("clone_main:{}/{}", repo.owner, repo.name);
        if !listed.contains(&key) {
            continue;
        }
        match repo.main {
            MainClone::Ready => progress.set_step(&key, pb::StepState::Done, None),
            MainClone::Failed { reason } => progress.set_step(&key, pb::StepState::Failed, Some(reason)),
            _ => {}
        }
    }
}
