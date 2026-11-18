//! Clean up and the sweep (DK-11, D182.2, D182.4): deleting closed Desk
//! sessions. A scratch folder goes to this machine's Bin and its row is
//! tombstoned; a picked folder never moves, only its row is tombstoned. The
//! sweep deletes the scratch folders whose 7 days are up, unless git work
//! would be lost, which it writes to the row instead.

use std::collections::HashMap;
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{DESK_CHANNEL, folder_bytes, held_work, scratch_child};
use humata2_store::{DeskFolderKind, DeskRow, SessionRecord, StoreError};
use humata2_transfers::NewItem;
use sha2::{Digest, Sha256};
use tonic::Status;

use super::{DeskSvc, desk_status, failed_precondition, now_ms, remote_close, session_id, view};
use crate::channel_service::blocking;
use crate::identity::{self, Caller};
use crate::status;
use crate::trash_service::caller_data;

/// The longest the sweep waits before it looks again (D182.4).
const HOUR_MS: i64 = 60 * 60 * 1000;

/// What deleting one closed session took out.
struct Removed {
    trash_item: Option<String>,
    bytes: u64,
}

/// Starts the sweep: one pass now, then one after each Close (its wake),
/// when the next closed scratch folder comes due, and hourly while a folder
/// holds git work. With none of those it arms no timer at all. Each pass returns when the next closed scratch folder
/// comes due, so the wait is set by the wall clock and not by one sleep to
/// the deadline: a sleep that the machine slept through is checked again.
pub(super) fn spawn_sweep(svc: DeskSvc) {
    tokio::spawn(async move {
        loop {
            // With nothing coming due, no timer is armed: only a Close
            // wakes the sweep, so an idle daemon sleeps.
            match svc.sweep().await {
                Some(due) => {
                    let wait = (due - now_ms()).clamp(0, HOUR_MS);
                    let wait = Duration::from_millis(u64::try_from(wait).unwrap_or(0));
                    let _ = tokio::time::timeout(wait, svc.wake.notified()).await;
                }
                None => svc.wake.notified().await,
            }
        }
    });
}

impl DeskSvc {
    /// PlanDeskCleanUp (DK-11, MV-9, D182.2): the closed sessions, least
    /// recently used first, with what deleting each frees and the git work
    /// it would lose, and a token naming the sessions and that work. Sizes
    /// are not part of the token.
    pub(super) async fn plan(&self) -> Result<pb::DeskCleanUpPlan, Status> {
        let closed: Vec<(SessionRecord, DeskRow)> = self
            .desk_rows()
            .await?
            .into_iter()
            .filter(|(_, row)| row.closed_unix_ms.is_some())
            .collect();
        let mut rows = Vec::with_capacity(closed.len());
        let mut digest = Sha256::new();
        let mut theirs: HashMap<String, Option<HashMap<String, (Option<u64>, String)>>> = HashMap::new();
        for (session, row) in closed {
            let (size_bytes, unpushed) = if let Some(machine) = self.elsewhere(&session) {
                // Its folder is the machine's: the machine says what is in it.
                if !theirs.contains_key(&machine) {
                    let asked = self.plan_there(&machine).await;
                    theirs.insert(machine.clone(), asked);
                }
                theirs
                    .get(&machine)
                    .and_then(Option::as_ref)
                    .and_then(|rows| rows.get(&session.id))
                    .cloned()
                    .unwrap_or_default()
            } else {
                match row.kind {
                    DeskFolderKind::Scratch { .. } => {
                        let folder = session.folder.clone();
                        let (bytes, work) =
                            blocking(move || (folder_bytes(&folder), held_work(&folder))).await?;
                        (Some(bytes), work)
                    }
                    DeskFolderKind::Picked => (None, String::new()),
                }
            };
            digest.update(session.id.as_bytes());
            digest.update([0u8]);
            digest.update(unpushed.as_bytes());
            digest.update([0u8]);
            rows.push(pb::DeskCleanUpRow {
                session: Some(pb::SessionId { value: session.id }),
                size_bytes,
                unpushed,
            });
        }
        let token: String = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(pb::DeskCleanUpPlan {
            rows,
            token: Some(pb::PlanToken { value: token }),
        })
    }

    /// CleanUpDesk (DK-11, D182.2, NV-27): deletes the named closed sessions,
    /// when the list still holds as the plan said. When it doesn't, nothing
    /// is deleted and the new plan is the answer.
    pub(super) async fn clean_up(
        &self,
        caller: &Caller,
        request: pb::CleanUpDeskRequest,
    ) -> Result<pb::CleanUpDeskResponse, Status> {
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone())
            .filter(|key| !key.is_empty());
        let Some(key) = key else {
            return self.clean_up_once(caller, request).await;
        };
        // One call per key at a time; a retry answers the first call.
        let _held = self.locks.hold(&format!("clean-up {key}")).await;
        if let Some(first) = self.cleaned_answer(&key) {
            return Ok(first);
        }
        let answer = self.clean_up_once(caller, request).await?;
        self.cleaned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, answer.clone());
        Ok(answer)
    }

    fn cleaned_answer(&self, key: &str) -> Option<pb::CleanUpDeskResponse> {
        self.cleaned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    /// Clean up itself, for a call not seen before.
    async fn clean_up_once(
        &self,
        caller: &Caller,
        request: pb::CleanUpDeskRequest,
    ) -> Result<pb::CleanUpDeskResponse, Status> {
        let ids = request
            .delete
            .into_iter()
            .map(|id| session_id(Some(id)))
            .collect::<Result<Vec<_>, _>>()?;
        // An open session is refused before anything is judged or deleted.
        for id in &ids {
            let (_, row) = self.load(id).await?;
            if row.closed_unix_ms.is_none() {
                return Err(failed_precondition(
                    format!("Desk session {id} is open, so it can't be cleaned up; nothing was deleted"),
                    "close the session first; Clean up deletes only closed sessions",
                ));
            }
        }
        let plan = self.plan().await?;
        let current = plan.token.as_ref().map(|token| token.value.clone());
        if request.plan.map(|token| token.value) != current {
            return Ok(answer(pb::clean_up_desk_response::Result::Changed(plan)));
        }
        // A session on another machine is deleted by that machine, into its
        // own Bin; a machine that can't be reached refuses all of it first.
        let remote = self.remote_ids(&ids).await?;
        let (mut sessions, mut freed_bytes, mut trash_items) = (0u32, 0u64, Vec::new());
        for (machine, theirs) in &remote {
            let gone = self.clean_up_there(machine, theirs).await?;
            sessions += gone.sessions;
            freed_bytes += gone.freed_bytes;
            trash_items.extend(gone.trash_items);
        }
        for id in ids.iter().filter(|id| !remote_close::is_remote(&remote, id)) {
            let _held = self.locks.hold(id).await;
            let (session, row) = self.load(id).await?;
            // Reopened since the check: that session is no longer closed.
            if row.closed_unix_ms.is_none() {
                continue;
            }
            let removed = self.delete_closed(&session, &row, Some(caller)).await?;
            sessions += 1;
            freed_bytes += removed.bytes;
            trash_items.extend(removed.trash_item.map(|value| pb::TrashItemId { value }));
        }
        Ok(answer(pb::clean_up_desk_response::Result::Deleted(
            pb::clean_up_desk_response::Deleted {
                sessions,
                freed_bytes,
                trash_items,
            },
        )))
    }

    /// One pass of the sweep (D182.4): every closed scratch folder past its
    /// days goes, unless it still holds git work, which is written to its row
    /// for the next pass. Returns when the next folder not yet due comes due.
    pub(super) async fn sweep(&self) -> Option<i64> {
        let candidates: Vec<String> = match self.desk_rows().await {
            Ok(rows) => rows
                .into_iter()
                .filter(|(_, row)| view::expires_at(row).is_some())
                .map(|(session, _)| session.id)
                .collect(),
            Err(error) => {
                crate::log::say!(
                    "humatad2: the Desk sweep couldn't list the Desk sessions: {}",
                    error.message()
                );
                // Try again within the hour rather than never.
                return Some(now_ms() + HOUR_MS);
            }
        };
        let mut next: Option<i64> = None;
        for id in candidates {
            let _held = self.locks.hold(&id).await;
            let Ok((session, row)) = self.load(&id).await else {
                continue;
            };
            let Some(due) = view::expires_at(&row) else {
                continue;
            };
            if due > now_ms() {
                next = Some(next.map_or(due, |soonest| soonest.min(due)));
                continue;
            }
            if let Some(machine) = self.elsewhere(&session) {
                // The machine runs this sweep on its own folders; the host
                // only notes which of its rows the machine has deleted.
                if let Err(error) = self.settle_remote(&machine).await {
                    crate::log::say!(
                        "humatad2: the Desk sweep couldn't check {machine}'s Desk sessions: {}",
                        error.message()
                    );
                }
                let again = now_ms() + HOUR_MS;
                next = Some(next.map_or(again, |soonest| soonest.min(again)));
                continue;
            }
            // A version keeps the folder for good, even one saved after Close
            // (DV-4, D451); a folder that can't be checked is kept too.
            if self.holds_version(&session.folder).await {
                let mut row = row;
                row.kind = DeskFolderKind::Scratch { kept: true };
                if let Err(error) = self.save_row(&row).await {
                    crate::log::say!(
                        "humatad2: the Desk sweep couldn't keep session {id}'s folder: {}",
                        error.message()
                    );
                    let again = now_ms() + HOUR_MS;
                    next = Some(next.map_or(again, |soonest| soonest.min(again)));
                }
                continue;
            }
            let folder = session.folder.clone();
            let work = match blocking(move || held_work(&folder)).await {
                Ok(work) => work,
                Err(error) => {
                    crate::log::say!(
                        "humatad2: the Desk sweep couldn't check session {id}'s folder: {}",
                        error.message()
                    );
                    continue;
                }
            };
            if !work.is_empty() {
                // Held work is looked at again within the hour (D182.4).
                let again = now_ms() + HOUR_MS;
                next = Some(next.map_or(again, |soonest| soonest.min(again)));
                if row.held_work != work {
                    let mut row = row;
                    row.held_work = work;
                    if let Err(error) = self.save_row(&row).await {
                        crate::log::say!(
                            "humatad2: the Desk sweep couldn't save session {id}'s held work: {}",
                            error.message()
                        );
                    }
                }
                continue;
            }
            if let Err(error) = self.delete_closed(&session, &row, None).await {
                crate::log::say!(
                    "humatad2: the Desk sweep couldn't delete session {id}'s folder: {}",
                    error.message()
                );
            }
        }
        next
    }

    /// Deletes a closed Desk session's folder and tombstones its row (D182.2,
    /// D182.4). A scratch folder still there goes to this machine's Bin, after
    /// checking it is a direct child of the Desk folder and not a symlink; a
    /// picked folder stays where it is. The caller holds the session's lock.
    ///
    /// The folder goes before the row is marked deleted, and the Desk event
    /// that drops the row is published after that commit: whoever hears the
    /// row go finds the folder gone (D440). Keep that order.
    async fn delete_closed(
        &self,
        session: &SessionRecord,
        row: &DeskRow,
        by: Option<&Caller>,
    ) -> Result<Removed, Status> {
        let mut removed = Removed {
            trash_item: None,
            bytes: 0,
        };
        let scratch = matches!(row.kind, DeskFolderKind::Scratch { .. });
        if scratch && std::fs::symlink_metadata(&session.folder).is_ok() {
            let desk = self.starter.layout.root().join(DESK_CHANNEL);
            let folder = session.folder.clone();
            let was = pb::EntityRef {
                entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                    value: session.id.clone(),
                })),
            };
            let by_data = by.map_or_else(String::new, |caller| {
                caller_data(&identity::actor(caller), &was)
            });
            let item = NewItem {
                description: format!("Desk session {}", session.title),
                caller_data: by_data,
                now_unix_ms: now_ms(),
            };
            let bin = self.bin.clone();
            let item = blocking(move || -> Result<humata2_transfers::TrashItem, Status> {
                let folder = scratch_child(&desk, &folder).map_err(|error| desk_status(&error))?;
                let trash = bin.get()?;
                trash.put(&folder, item).map_err(|error| {
                    status::internal(
                        format!(
                            "couldn't move the Desk folder {} to the Bin: {}; check that the Bin's folder can be written to, then clean up again",
                            folder.display(),
                            status::chain(&error)
                        ),
                        "",
                    )
                })
            })
            .await??;
            removed.bytes = item.bytes;
            removed.trash_item = Some(item.id);
        }
        let mut row = row.clone();
        row.deleted_unix_ms = Some(now_ms());
        row.held_work = String::new();
        self.save_row(&row).await?;
        // After the commit, so whoever hears the row go finds the folder gone.
        super::follow::tell_gone(self, &session.id);
        Ok(removed)
    }

    /// Every Desk session that isn't deleted, with its row.
    pub(super) async fn desk_rows(&self) -> Result<Vec<(SessionRecord, DeskRow)>, Status> {
        let store = self.starter.store.clone();
        let (sessions, rows) = blocking(move || -> Result<_, StoreError> {
            Ok((store.sessions()?, store.desks()?))
        })
        .await?
        .map_err(|error| status::store(error, &self.machine, "list the Desk's sessions"))?;
        let mut rows: HashMap<String, DeskRow> = rows
            .into_iter()
            .map(|row| (row.session_id.clone(), row))
            .collect();
        Ok(sessions
            .into_iter()
            .filter(|session| session.channel == DESK_CHANNEL)
            .filter_map(|session| {
                let row = rows.remove(&session.id)?;
                row.deleted_unix_ms.is_none().then_some((session, row))
            })
            .collect())
    }
}

fn answer(result: pb::clean_up_desk_response::Result) -> pb::CleanUpDeskResponse {
    pb::CleanUpDeskResponse {
        result: Some(result),
    }
}
