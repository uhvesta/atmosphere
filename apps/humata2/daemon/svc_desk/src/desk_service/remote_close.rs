//! Closing, cleaning up and sweeping a Desk session that works on another
//! machine (MV-17, D316). Its folder is that machine's, so the machine acts
//! on it: Close stops its agent there, and Clean up and the sweep send its
//! folder to that machine's Bin. The host's own record changes after the
//! machine's did, and the host's disk is never touched. A machine whose
//! link is down answers machine-offline at once and nothing is queued, as
//! every other action on a session there does.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::pb;
use humata2_store::{ChannelDeskRow, SessionRecord};
use tonic::Status;
use tonic::transport::Channel;

use super::{DeskSvc, failed_precondition, now_ms};
use crate::ports::qualified;
use crate::ports::assigned;
use humata2_daemon_ports::Caller;
use crate::ports as identity;
use crate::ports::call_failed;

type Desks = pb::desk_service_client::DeskServiceClient<Channel>;

/// What one machine deleted for a Clean up.
pub(super) struct Gone {
    pub(super) sessions: u32,
    pub(super) freed_bytes: u64,
    pub(super) trash_items: Vec<pb::TrashItemId>,
}

impl DeskSvc {
    /// The machine `session` works on, when it isn't this one and this
    /// daemon has the host's way to it. None for a session here, and always
    /// on a machine's own daemon.
    pub(super) fn elsewhere(&self, session: &SessionRecord) -> Option<String> {
        if self.remote.is_none() || session.machine.is_empty() || session.machine == self.machine {
            return None;
        }
        Some(session.machine.clone())
    }

    /// The Desk service on `machine`'s live link, or the machine-offline
    /// error when the link is down.
    pub(super) fn desk_on(&self, machine: &str) -> Result<Desks, Status> {
        let Some(remote) = &self.remote else {
            return Err(failed_precondition(
                format!("this machine has no way to reach {machine}, so the Desk session there was not changed"),
                "change the Desk session from the host",
            ));
        };
        Ok(remote.relayed(machine).map_err(Status::from)?.desk())
    }

    /// Closes `id` on `machine`: its agent and shells stop there, and its
    /// folder goes to that machine's Bin when it is a channel's Desk
    /// session. Answers the session as the machine shows it.
    pub(super) async fn close_there(&self, machine: &str, id: &str) -> Result<pb::DeskSession, Status> {
        self.close_there_as(machine, id, false).await
    }

    /// As [`Self::close_there`]; `trash_versioned` has a channel's Desk
    /// folder that holds a version go to that machine's Bin too, as a
    /// channel delete wants.
    async fn close_there_as(&self, machine: &str, id: &str, trash_versioned: bool) -> Result<pb::DeskSession, Status> {
        let mut desk = self.desk_on(machine)?;
        let request = assigned(
            pb::CloseDeskSessionRequest {
                idempotency_key: None,
                session: Some(pb::SessionId { value: id.to_owned() }),
                trash_versioned,
            },
            None,
        )
        .map_err(Status::from)?;
        desk.close_desk_session(request)
            .await
            .map(tonic::Response::into_inner)
            .map_err(|source| Status::from(call_failed(machine, "close the Desk session", source)))
    }

    /// A channel's Desk session closed on `machine`, then marked closed in
    /// the host's row. The machine's Bin holds its folder, so the host's row
    /// names that item as the host's Bin view lists it (`machine:item`):
    /// restoring it there restores the session on the machine. `by` is who
    /// asked, or `None` when the system closes it.
    pub(super) async fn close_channel_desk_there(
        &self,
        by: Option<&Caller>,
        machine: &str,
        session: &SessionRecord,
        row: &ChannelDeskRow,
        keep_versions: bool,
    ) -> Result<ChannelDeskRow, Status> {
        let shown = self.close_there_as(machine, &session.id, !keep_versions).await?;
        let now = now_ms();
        let mut closed = row.clone();
        closed.closed_unix_ms = Some(now);
        closed.last_used_unix_ms = now;
        closed.trash_item = shown
            .channel_desk
            .and_then(|desk| desk.trash_item)
            .map(|item| qualified(machine, &item.value));
        self.save_channel_desk_row(&closed).await?;
        self.end_grants(&session.id).await?;
        self.tell
            .session(&session.id, pb::Change::Updated, by.map(identity::actor))
            .await;
        Ok(closed)
    }

    /// What `machine`'s plan of a clean up says of each closed session it
    /// holds: the size of its folder and the work that would be lost. None
    /// when the machine can't be reached; Clean up then refuses it.
    pub(super) async fn plan_there(&self, machine: &str) -> Option<HashMap<String, (Option<u64>, String)>> {
        let mut desk = self.desk_on(machine).ok()?;
        let request = assigned(pb::PlanDeskCleanUpRequest::default(), None).ok()?;
        let plan = desk.plan_desk_clean_up(request).await.ok()?.into_inner();
        Some(
            plan.rows
                .into_iter()
                .filter_map(|row| Some((row.session?.value, (row.size_bytes, row.unpushed))))
                .collect(),
        )
    }

    /// The machines the sessions `ids` work on, with the ids each holds.
    /// Refuses at once, before anything is deleted, when a machine's link is
    /// down.
    pub(super) async fn remote_ids(&self, ids: &[String]) -> Result<BTreeMap<String, Vec<String>>, Status> {
        let mut by_machine: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for id in ids {
            let (session, _) = self.load(id).await?;
            if let Some(machine) = self.elsewhere(&session) {
                by_machine.entry(machine).or_default().push(id.clone());
            }
        }
        for machine in by_machine.keys() {
            self.desk_on(machine)?;
        }
        Ok(by_machine)
    }

    /// Cleans up `ids` on `machine`: the machine plans, then deletes them
    /// into its own Bin, and the host's rows follow the machine's.
    pub(super) async fn clean_up_there(&self, machine: &str, ids: &[String]) -> Result<Gone, Status> {
        let mut held = Vec::with_capacity(ids.len());
        for id in ids {
            held.push(self.locks.hold(id).await);
        }
        let mut desk = self.desk_on(machine)?;
        let doing = |source| Status::from(call_failed(machine, "clean up the Desk", source));
        let plan = desk
            .plan_desk_clean_up(assigned(pb::PlanDeskCleanUpRequest::default(), None).map_err(Status::from)?)
            .await
            .map_err(doing)?
            .into_inner();
        let request = pb::CleanUpDeskRequest {
            idempotency_key: None,
            plan: plan.token,
            delete: ids.iter().map(|id| pb::SessionId { value: id.clone() }).collect(),
        };
        let answered = desk
            .clean_up_desk(assigned(request, None).map_err(Status::from)?)
            .await
            .map_err(doing)?
            .into_inner();
        drop(held);
        match answered.result {
            Some(pb::clean_up_desk_response::Result::Deleted(done)) => {
                self.settle_remote(machine).await?;
                Ok(Gone {
                    sessions: done.sessions,
                    freed_bytes: done.freed_bytes,
                    trash_items: done.trash_items,
                })
            }
            _ => Err(failed_precondition(
                format!("the closed Desk sessions on {machine} changed while the clean up ran, so nothing more was deleted there"),
                "plan the clean up again and delete from the new list",
            )),
        }
    }

    /// Marks deleted the host's rows of closed sessions that `machine` no
    /// longer lists: it deleted them, by Clean up or its own sweep. Changes
    /// no folder. The caller holds no session lock but its own.
    pub(super) async fn settle_remote(&self, machine: &str) -> Result<(), Status> {
        let mut desk = self.desk_on(machine)?;
        let listed = desk
            .list_desk_sessions(assigned(pb::ListDeskSessionsRequest::default(), None).map_err(Status::from)?)
            .await
            .map_err(|source| Status::from(call_failed(machine, "list its Desk sessions", source)))?
            .into_inner()
            .sessions;
        let there: HashSet<String> = listed
            .into_iter()
            .filter_map(|shown| shown.session.map(|id| id.value))
            .collect();
        for (session, mut row) in self.desk_rows().await? {
            let gone = self.elsewhere(&session).as_deref() == Some(machine)
                && row.closed_unix_ms.is_some()
                && !there.contains(&session.id);
            if gone {
                row.deleted_unix_ms = Some(now_ms());
                row.held_work = String::new();
                self.save_row(&row).await?;
            }
        }
        Ok(())
    }
}

/// Whether `id` is one of the sessions `by_machine` names.
pub(super) fn is_remote(by_machine: &BTreeMap<String, Vec<String>>, id: &str) -> bool {
    by_machine.values().any(|ids| ids.iter().any(|held| held == id))
}
