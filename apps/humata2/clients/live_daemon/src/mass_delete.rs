//! MASSDELETE over the live daemon: `PlanDeleteSessions` and
//! `DeleteSessions` on SessionService. The delete names the plan's token;
//! when the sessions changed since the plan, the daemon deletes nothing and
//! answers the new plan instead (NV-27).

use humata2_clients::mass_delete::{
    DeleteScope, DeleteSessionsOutcome, DeleteSessionsPlan, EndingState, LeftAlone, SessionToDelete,
    SessionWork, TerminalEnding,
};
use humata2_clients::DaemonError;

use crate::connection::{Shared, fresh_key};
use crate::daemon::unpushed;
use crate::humata2_grpc::humata::v1 as pb;
use crate::map;

fn scope_wire(scope: &DeleteScope) -> pb::DeleteScope {
    let scope = match scope {
        DeleteScope::Picked(ids) => pb::delete_scope::Scope::Sessions(pb::delete_scope::Picked {
            ids: ids
                .iter()
                .map(|id| pb::SessionId {
                    value: id.as_str().to_owned(),
                })
                .collect(),
        }),
        DeleteScope::Channel(channel) => pb::delete_scope::Scope::Channel(pb::ChannelId {
            value: channel.as_str().to_owned(),
        }),
        DeleteScope::Machine(machine) => pb::delete_scope::Scope::Machine(pb::MachineId {
            value: machine.as_str().to_owned(),
        }),
        DeleteScope::All => pb::delete_scope::Scope::All(true),
    };
    pb::DeleteScope { scope: Some(scope) }
}

fn ending_state(raw: i32) -> EndingState {
    match pb::terminal_ending::State::try_from(raw) {
        Ok(pb::terminal_ending::State::Working) => EndingState::Working,
        Ok(pb::terminal_ending::State::WaitingForYou) => EndingState::WaitingForYou,
        _ => EndingState::Idle,
    }
}

fn session_work(work: pb::SessionWork) -> SessionWork {
    SessionWork {
        session: map::session_id(work.session.as_ref()),
        title: work.title,
        dirty_files: work.dirty_files,
        unpushed_commits: work.unpushed_commits,
    }
}

fn to_delete(found: pb::SessionToDelete) -> SessionToDelete {
    SessionToDelete {
        session: map::session_id(found.session.as_ref()),
        title: found.title,
        channel: map::channel_id(found.channel.as_ref()),
        machine: map::machine_id(found.machine.as_ref()),
        archived: found.archived,
        unpushed: found.unpushed.iter().map(unpushed).collect(),
        running_terminals: found.running_terminals,
        goes_with: found
            .goes_with
            .filter(|id| !id.value.is_empty())
            .map(|id| map::session_id(Some(&id))),
        terminals: found
            .terminals
            .into_iter()
            .map(|terminal| TerminalEnding {
                name: terminal.name,
                state: ending_state(terminal.state),
            })
            .collect(),
        exited: found.exited,
    }
}

/// The daemon's plan as the app holds it. A plan without its token can't be
/// deleted, so it is refused here rather than deleted later with an empty one.
fn plan_from(plan: pb::DeleteSessionsPlan, what: &str) -> Result<DeleteSessionsPlan, DaemonError> {
    let token = plan
        .token
        .map(|token| token.value)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| DaemonError::Refused {
            what: what.to_owned(),
            reason: "the daemon didn't give a token for the plan. Plan the delete again".to_owned(),
        })?;
    Ok(DeleteSessionsPlan {
        sessions: plan.sessions.into_iter().map(to_delete).collect(),
        left_alone: plan
            .left_alone
            .into_iter()
            .map(|left| LeftAlone {
                machine: map::machine_id(left.machine.as_ref()),
                sessions: left.sessions.into_iter().map(session_work).collect(),
            })
            .collect(),
        forks_staying: plan.forks_staying.into_iter().map(session_work).collect(),
        desk_sessions_left_out: plan.desk_sessions_left_out,
        token,
        needs_count: plan.needs_count,
    })
}

impl Shared {
    pub(crate) async fn plan_delete_sessions(
        &self,
        scope: DeleteScope,
    ) -> Result<DeleteSessionsPlan, DaemonError> {
        const WHAT: &str = "Planning the delete of sessions";
        let plan = self
            .sessions()
            .plan_delete_sessions(pb::PlanDeleteSessionsRequest {
                scope: Some(scope_wire(&scope)),
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        plan_from(plan, WHAT)
    }

    pub(crate) async fn delete_sessions(
        &self,
        token: &str,
        delete_unpushed_work: bool,
        typed_count: u32,
    ) -> Result<DeleteSessionsOutcome, DaemonError> {
        const WHAT: &str = "Deleting the sessions";
        let response = self
            .sessions()
            .delete_sessions(pb::DeleteSessionsRequest {
                idempotency_key: fresh_key(),
                plan: Some(pb::PlanToken {
                    value: token.to_owned(),
                }),
                delete_unpushed_work,
                typed_count,
                check_only: false,
            })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        match response.result {
            Some(pb::delete_sessions_response::Result::Deleted(done)) => {
                if let Some(operation) = done.operation {
                    self.finish(operation, WHAT).await?;
                }
                Ok(DeleteSessionsOutcome::Deleted {
                    sessions: done.sessions,
                })
            }
            Some(pb::delete_sessions_response::Result::Changed(changed)) => {
                let plan = changed.plan.ok_or_else(|| DaemonError::Refused {
                    what: WHAT.to_owned(),
                    reason: "the daemon changed the plan but sent no new list. Plan the delete again".to_owned(),
                })?;
                Ok(DeleteSessionsOutcome::Changed {
                    plan: plan_from(plan, WHAT)?,
                    changes: changed.changes,
                })
            }
            None => Err(DaemonError::Refused {
                what: WHAT.to_owned(),
                reason: "the daemon answered neither a deletion nor the new list. Reload to see which sessions are left".to_owned(),
            }),
        }
    }
}
