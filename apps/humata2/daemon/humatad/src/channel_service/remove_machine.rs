//! PlanRemoveMachine (CH-8, LD-10, MV-9): before a machine leaves a channel,
//! the plan lists its live sessions there with their uncommitted and
//! unpushed work, and a token RemoveMachine takes once. Read-only.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use tonic::Status;

use humata2_access::{Capability, ChannelId, Target};
use humata2_sessions::repo_work;
use humata2_store::SessionDetail;
use tonic::Code;

use super::{ChannelSvc, Gated, blocking};
use crate::identity::Caller;
use crate::status;

/// Plans not yet taken: token -> (channel, machine, when it was made).
#[derive(Default)]
pub(super) struct Plans(Mutex<HashMap<String, (String, String, Instant)>>);

/// How long a plan's token stays good.
const PLAN_LIFETIME: Duration = Duration::from_secs(10 * 60);

impl Plans {
    /// Makes a token for a plan of `machine` leaving `channel`, forgetting
    /// plans older than their lifetime.
    fn make(&self, channel: &str, machine: &str) -> String {
        let token = format!("pl_{}", uuid::Uuid::new_v4().simple());
        let mut plans = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        plans.retain(|_, (_, _, made)| made.elapsed() < PLAN_LIFETIME);
        plans.insert(
            token.clone(),
            (channel.to_owned(), machine.to_owned(), Instant::now()),
        );
        token
    }

    /// Uses `token` up: true when it is a live plan of `machine` leaving `channel`.
    pub(super) fn take(&self, token: &str, channel: &str, machine: &str) -> bool {
        let mut plans = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let live = plans
            .get(token)
            .is_some_and(|(of, leaving, made)| of == channel && leaving == machine && made.elapsed() < PLAN_LIFETIME);
        if live {
            plans.remove(token);
        }
        live
    }
}

/// The machine a session lives on: a record with none is this machine's.
fn lives_on<'a>(recorded: &'a str, this_machine: &'a str) -> &'a str {
    if recorded.is_empty() { this_machine } else { recorded }
}

/// The uncommitted files and unpushed commits in a session's clones. A
/// clone whose folder is gone was freed and counts nothing.
fn work_of(
    detail: Option<SessionDetail>,
) -> Result<(u32, u32), humata2_sessions::GitError> {
    let mut dirty = 0u32;
    let mut unpushed = 0u32;
    for repo in detail.into_iter().flat_map(|detail| detail.repos) {
        if !repo.path.exists() {
            continue;
        }
        let work = repo_work(&repo.path)?;
        dirty = dirty.saturating_add(work.dirty_files);
        unpushed = unpushed.saturating_add(work.unpushed_commits);
    }
    Ok((dirty, unpushed))
}

impl ChannelSvc {
    pub(super) async fn plan_remove_machine_of(
        &self,
        caller: &Caller,
        request: pb::PlanRemoveChannelMachineRequest,
        approved: bool,
    ) -> Result<Gated<pb::RemoveChannelMachinePlan>, Status> {
        let id = request
            .channel
            .map(|id| id.value)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        let machine = request
            .machine
            .map(|machine| machine.value)
            .filter(|machine| !machine.is_empty())
            .ok_or_else(|| status::missing("machine"))?;
        let channel = self.find(&id).await?;
        if let Some(asked) = self.gate(
            caller,
            Capability::ChannelChange,
            Target::Channel(ChannelId::new(&id)),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        if !channel.machines.contains(&machine) {
            return Err(status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: format!("machine {machine} is not in channel {id}"),
                    fix: "that machine is not in the channel; list the channel's machines and pick one of them"
                        .to_owned(),
                    retry: None,
                    kind: None,
                },
            ));
        }
        if machine != self.machine {
            // The sessions of another machine are listed from what it last reported of them.
            let sessions = self.remote_sessions_of(&id, &machine).await?;
            return Ok(Gated::Done(self.plan_of(&id, &machine, sessions)));
        }
        let store = self.starter.store.clone();
        let (key, here, mine) = (id.clone(), machine.clone(), self.machine.clone());
        let sessions = blocking(move || -> Result<Vec<pb::SessionWork>, WorkError> {
            let mut listed = Vec::new();
            for session in store.sessions()? {
                if session.channel != key
                    || lives_on(&session.machine, &mine) != here
                    || store.archive(&session.id)?.is_some()
                {
                    continue;
                }
                let (dirty_files, unpushed_commits) =
                    work_of(store.session_detail(&session.id)?)?;
                listed.push(pb::SessionWork {
                    session: Some(pb::SessionId { value: session.id }),
                    title: session.title,
                    dirty_files,
                    unpushed_commits,
                });
            }
            Ok(listed)
        })
        .await?
        .map_err(|error| match error {
            WorkError::Store(error) => {
                status::store(error, &self.machine, "list the machine's sessions")
            }
            WorkError::Git(error) => status::failure(
                Code::Unavailable,
                pb::Error {
                    message: format!(
                        "could not read a session's uncommitted and unpushed work: {}",
                        status::chain(&error)
                    ),
                    fix: "check that git runs and the session's clones are readable, then plan again; nothing was changed"
                        .to_owned(),
                    retry: None,
                    kind: None,
                },
            ),
        })?;
        Ok(Gated::Done(self.plan_of(&id, &machine, sessions)))
    }

    fn plan_of(
        &self,
        channel: &str,
        machine: &str,
        sessions: Vec<pb::SessionWork>,
    ) -> pb::RemoveChannelMachinePlan {
        let token = self.plans.make(channel, machine);
        pb::RemoveChannelMachinePlan {
            channel: Some(pb::ChannelId {
                value: channel.to_owned(),
            }),
            machine: Some(pb::MachineId {
                value: machine.to_owned(),
            }),
            sessions,
            token: Some(pb::PlanToken { value: token }),
        }
    }
}

/// What listing a machine's work can fail on.
#[derive(Debug, thiserror::Error)]
enum WorkError {
    #[error("the store could not be read")]
    Store(#[from] humata2_store::StoreError),
    #[error("git could not read a clone")]
    Git(#[from] humata2_sessions::GitError),
}
