//! After a restart: the moves it cut short (MV-14, flow 26.6). Each journal
//! `<state>/carry/<operation>.json` names a session; from the moment the host
//! starts, that session shows Moving. Once the start's own recovery is done
//! and the links of the move's two machines are up, the move is undone (the
//! agent still runs on the old machine) or finished (it runs on the new
//! one), and the session shows what it is again.
//!
//! Nothing here polls: the start wakes its waiters when it's done, each
//! link wait is the link's own state change, and a machine that stays
//! offline keeps its move waiting.

use std::sync::Arc;

use humata2_grpc::humata::v1 as pb;
use tokio::sync::broadcast;

use super::SessionSvc;
use super::move_session::{self, Moving};
use crate::carry::{Carry, Journaled};
use crate::host_relay::{LinkState, Links};
use crate::peer_reports::ReportUpdate;
use crate::restart::OperationState;
use crate::status;

impl SessionSvc {
    /// Reads the journals of moves the last run left, marks their sessions
    /// Moving at once, and recovers each when the start is done and its
    /// machines' links are up. Call it once the carry is set and before
    /// anything is served.
    pub(crate) fn recover_moves_after(&self, started: Arc<crate::restart::Started>) {
        let Some(carry) = self.carry.get().cloned() else {
            return;
        };
        for journal in carry.journals() {
            // A move already undone, with only a machine's copy left to take
            // back, isn't moving: its session runs where it was.
            let marked = (journal.phase != "aborted")
                .then(|| Moving::begin(&journal.session, &journal.to, &journal.operation));
            self.recover_when_linked_after(Some(started.clone()), journal, marked);
        }
        self.recover_forks_after(started.clone());
        // A push on another machine the host was following when it stopped.
        self.recover_remote_pushes_after(started.clone());
        // D222: a remote archive's or restore's follow-up the last run left.
        self.recover_remote_archives_after(started.clone());
        // D554: a continue on another machine the last run journaled and
        // didn't record is asked again under its key once that machine links.
        self.recover_remote_continues_after(started.clone());
        // D432: a start the last run asked another machine for and heard
        // no answer to is asked again once that machine links.
        self.recover_remote_starts_after(started);
    }

    /// Finishes or undoes the move `journal` names once the links of its
    /// two machines are up, and drops `marked` then (D207).
    pub(super) fn recover_when_linked(&self, journal: Journaled, marked: Option<Moving>) {
        self.recover_when_linked_after(None, journal, marked);
    }

    fn recover_when_linked_after(
        &self,
        started: Option<Arc<crate::restart::Started>>,
        journal: Journaled,
        marked: Option<Moving>,
    ) {
        let (Some(carry), Some(remote)) = (self.carry.get().cloned(), self.remote.clone()) else {
            return;
        };
        let svc = self.clone();
        let operations = started.as_ref().map(|started| started.operations().clone());
        let note = move |state: OperationState, journal: &Journaled| {
            if let Some(operations) = &operations {
                operations.record(&journal.operation, "move", &journal.phase, state);
            }
        };
        note(
            OperationState::Pending {
                waits_for: format!("the start to finish, then the links of {} and {}", journal.from, journal.to),
            },
            &journal,
        );
        tokio::spawn(async move {
            if let Some(started) = started {
                started.wait().await;
            }
            let session = journal.session.clone();
            // A try that fails because a machine was cut off (its link still
            // read as up, or it flapped meanwhile) is made again once a link of
            // the move is up anew; the journal says what is still owed.
            let mut journal = journal;
            loop {
                let seen = match links_up(&carry, &journal).await {
                    Ok(seen) => seen,
                    Err(machine) => {
                        note(
                            OperationState::Stuck {
                                reason: format!("{machine} was forgotten before the move could be recovered"),
                            },
                            &journal,
                        );
                        crate::log::say!(
                            "humatad2: the move of session {} was cut short, and {machine} was forgotten before it could be recovered. Check the session on its machine; nothing was deleted",
                            journal.session
                        );
                        drop(marked);
                        return;
                    }
                };
                // Taken before the try, so a report that comes while it runs
                // isn't missed.
                let reports = remote.report_updates();
                let held = svc.lifecycle.lock_of(&journal.session).lock_owned().await;
                let outcome = move_session::recover(svc.clone(), carry.clone(), remote.clone(), journal.clone(), held).await;
                note(
                    match &outcome {
                        // The agent runs on the new machine from `resumed` on.
                        Ok(()) if matches!(journal.phase.as_str(), "resumed" | "switched") => OperationState::Finished,
                        Ok(()) => OperationState::Undone {
                            reason: format!("the agent still ran on {}", journal.from),
                        },
                        Err(error) => OperationState::Stuck {
                            reason: status::chain(error),
                        },
                    },
                    &journal,
                );
                if let Err(error) = &outcome {
                    crate::log::say!(
                        "humatad2: couldn't finish or undo the move of session {session}: {}. Nothing was deleted; Humata tries again when the session changes on a machine of the move, when one links again, or at the next start",
                        status::chain(error)
                    );
                }
                let again = match &outcome {
                    Ok(()) => false,
                    // A machine busy with the session, as when it was still
                    // resuming the agent the move asked it to, tells the host
                    // once that is done: the next try is then.
                    Err(error) if error.code() == tonic::Code::Aborted => {
                        tokio::select! {
                            relinked = relinked(&carry, &seen) => relinked.is_ok(),
                            reported = reported(reports, &journal) => reported.is_ok(),
                        }
                    }
                    Err(_) => relinked(&carry, &seen).await.is_ok(),
                };
                if !again {
                    break;
                }
                // A try may have moved the journal on (to `unpacked` or
                // `aborted`) or ended it: the next works from what it says now.
                let operation = journal.operation.clone();
                let reader = carry.clone();
                let kept = tokio::task::spawn_blocking(move || {
                    reader.journals().into_iter().find(|kept| kept.operation == operation)
                })
                .await;
                match kept {
                    Ok(Some(kept)) => journal = kept,
                    _ => break,
                }
            }
            drop(marked);
            svc.tell.session(&session, pb::Change::Updated, None).await;
        });
    }
}

/// The remote machines of a move and the generation of each one's link
/// when a try began (none when it had dropped again by then).
type Seen = Vec<(String, Option<u64>)>;

/// Waits until the links of both machines of the move are up. The host
/// itself always is. Err names a machine that was forgotten meanwhile.
async fn links_up(carry: &Carry, journal: &Journaled) -> Result<Seen, String> {
    let mut seen = Vec::new();
    for machine in [&journal.from, &journal.to] {
        if carry.is_host(machine) {
            continue;
        }
        wait_up(carry.links(), machine).await.map_err(|()| machine.clone())?;
        let state = watch_provider(carry.links(), machine);
        let now = *state.borrow();
        let generation = match now {
            LinkState::Up { generation } => Some(generation),
            LinkState::Down => None,
        };
        seen.push((machine.clone(), generation));
    }
    Ok(seen)
}

/// Waits until the link of one of `seen`'s machines is up anew, which is
/// when a try that failed for a machine cut off may succeed. Err when a
/// machine was forgotten, or none of the move's machines has a link.
async fn relinked(carry: &Carry, seen: &Seen) -> Result<(), ()> {
    let mut waits = tokio::task::JoinSet::new();
    for (machine, generation) in seen {
        let (links, machine, generation) = (carry.links().clone(), machine.clone(), *generation);
        waits.spawn(async move { came_back(&links, &machine, generation).await });
    }
    match waits.join_next().await {
        Some(Ok(outcome)) => outcome,
        _ => Err(()),
    }
}

/// Waits until a remote machine of the move reports on its session, or
/// stops holding it. Err once no more reports can come.
async fn reported(mut reports: broadcast::Receiver<Arc<ReportUpdate>>, journal: &Journaled) -> Result<(), ()> {
    loop {
        match reports.recv().await {
            Ok(update) => match &*update {
                ReportUpdate::Session { machine, session, .. } | ReportUpdate::Gone { machine, session }
                    if session == &journal.session && (machine == &journal.from || machine == &journal.to) =>
                {
                    return Ok(());
                }
                _ => {}
            },
            // Reports were missed, and one may have been about the session.
            Err(broadcast::error::RecvError::Lagged(_)) => return Ok(()),
            Err(broadcast::error::RecvError::Closed) => return Err(()),
        }
    }
}

pub(super) fn watch_provider(
    links: &Arc<dyn humata2_svc_transfers::ports::LinkProvider>,
    machine: &str,
) -> tokio::sync::watch::Receiver<LinkState> {
    let initial = if links.channel(machine).is_ok() {
        LinkState::Up { generation: 1 }
    } else {
        LinkState::Down
    };
    let (tx, rx) = tokio::sync::watch::channel(initial);
    let mut all = links.watch_all();
    let target = machine.to_owned();
    tokio::spawn(async move {
        loop {
            match all.recv().await {
                Ok((name, state)) if name == target => {
                    let mapped = match state {
                        humata2_svc_transfers::ports::LinkState::Up { generation } => {
                            LinkState::Up { generation }
                        }
                        humata2_svc_transfers::ports::LinkState::Down => LinkState::Down,
                    };
                    if tx.send(mapped).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    rx
}

pub(super) fn relayed_provider(
    links: &Arc<dyn humata2_svc_transfers::ports::LinkProvider>,
    machine: &str,
) -> Result<crate::host_relay::Relayed, crate::host_relay::RelayError> {
    links
        .channel(machine)
        .map(|channel| crate::host_relay::Relayed {
            channel,
            generation: 0,
        })
        .map_err(|_| crate::host_relay::RelayError::Offline {
            machine: machine.to_owned(),
        })
}

/// Waits until `machine`'s link is up with a generation other than `seen`.
async fn came_back(
    links: &Arc<dyn humata2_svc_transfers::ports::LinkProvider>,
    machine: &str,
    seen: Option<u64>,
) -> Result<(), ()> {
    let mut state = watch_provider(links, machine);
    state
        .wait_for(|now| matches!(now, LinkState::Up { generation } if Some(*generation) != seen))
        .await
        .map(drop)
        .map_err(drop)
}

pub(super) async fn wait_up(
    links: &Arc<dyn humata2_svc_transfers::ports::LinkProvider>,
    machine: &str,
) -> Result<(), ()> {
    let mut state = watch_provider(links, machine);
    state
        .wait_for(|now| matches!(now, LinkState::Up { .. }))
        .await
        .map(drop)
        .map_err(drop)
}
