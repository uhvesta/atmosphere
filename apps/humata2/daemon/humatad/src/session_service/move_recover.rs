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

use super::SessionSvc;
use super::move_session::{self, Moving};
use crate::carry::{Carry, Journaled};
use crate::host_relay::{LinkState, Links};
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
            let marked = (journal.phase != "aborted").then(|| Moving::begin(&journal.session));
            self.recover_when_linked_after(Some(started.clone()), journal, marked);
        }
        self.recover_forks_after(started);
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
        tokio::spawn(async move {
            if let Some(started) = started {
                started.wait().await;
            }
            if let Err(machine) = links_up(&carry, &journal).await {
                crate::log::say!(
                    "humatad2: the move of session {} was cut short, and {machine} was forgotten before it could be recovered. Check the session on its machine; nothing was deleted",
                    journal.session
                );
                drop(marked);
                return;
            }
            let held = svc.lifecycle.lock_of(&journal.session).lock_owned().await;
            let session = journal.session.clone();
            if let Err(error) = move_session::recover(svc.clone(), carry, remote, journal, held).await {
                crate::log::say!(
                    "humatad2: couldn't finish or undo the move of session {session}: {}. Nothing was deleted; the next start tries again, or look at the session on both machines",
                    status::chain(&error)
                );
            }
            drop(marked);
            svc.tell.session(&session, pb::Change::Updated, None).await;
        });
    }
}

/// Waits until the links of both machines of the move are up. The host
/// itself always is. Err names a machine that was forgotten meanwhile.
async fn links_up(carry: &Carry, journal: &Journaled) -> Result<(), String> {
    for machine in [&journal.from, &journal.to] {
        if carry.is_host(machine) {
            continue;
        }
        wait_up(carry.links(), machine).await.map_err(|()| machine.clone())?;
    }
    Ok(())
}

pub(super) async fn wait_up(links: &Links, machine: &str) -> Result<(), ()> {
    let mut state = links.watch(machine);
    state
        .wait_for(|now| matches!(now, LinkState::Up { .. }))
        .await
        .map(drop)
        .map_err(drop)
}
