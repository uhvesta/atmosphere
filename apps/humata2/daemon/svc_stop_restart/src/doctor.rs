//! `DaemonService.RepairRestart` (PER-30, D242): the restart doctor's safe
//! repairs, the ones that can't lose anything.
//!
//! A session or a shell tab the last start didn't bring back is brought
//! back again the way the start (and ResumeSession) does it, under the
//! same access rule as ResumeSession: the user, or an agent the rule lets
//! resume sessions, which may be sent to Ask. The store is the user's: it
//! is checked as the next start checks it, a healthy one is refused, and a
//! damaged one makes this daemon answer and then leave, so that its next
//! start restores the last good copy (PER-31).

use std::sync::Arc;

use humata2_access::Capability;
use humata2_daemon_ports::{Caller, Gated};
use humata2_store::Store;
use tokio::sync::Notify;
use tonic::{Code, Request, Response, Status};

use crate::pb;
use crate::ports::{self, DoctorSessionsPort, StopInboxPort, answer_gated, failed};

/// Why a store repair was refused.
#[derive(Debug, thiserror::Error)]
enum StoreRefused {
    #[error("the store passed its check, so there is nothing to restore from a copy")]
    Healthy,
    #[error(
        "the store failed its check ({problem}), but no last good copy is kept beside it, so restarting humatad would leave it unable to start"
    )]
    NoCopy { problem: String },
}

impl StoreRefused {
    fn status(&self) -> Status {
        let fix = match self {
            Self::Healthy => {
                "nothing to repair in the store; if sessions weren't brought back, repair them one by one (humata doctor restarts lists them)"
            }
            Self::NoCopy { .. } => {
                "don't restart humatad yet: copy the state folder somewhere safe first, then ask for help restoring the database by hand"
            }
        };
        failed(Code::FailedPrecondition, self, fix, None)
    }
}

/// What serves RepairRestart on the daemon that brought its sessions back.
pub struct Doctor {
    sessions: Arc<dyn DoctorSessionsPort>,
    inbox: Arc<dyn StopInboxPort>,
    store: Arc<Store>,
    machine: String,
    /// Wakes the serve loop's stop, so a daemon whose store is damaged
    /// leaves once it has answered.
    leave: Arc<Notify>,
}

impl Doctor {
    pub fn new(
        sessions: Arc<dyn DoctorSessionsPort>,
        inbox: Arc<dyn StopInboxPort>,
        store: Arc<Store>,
        machine: String,
        leave: Arc<Notify>,
    ) -> Self {
        Self {
            sessions,
            inbox,
            store,
            machine,
            leave,
        }
    }

    pub async fn repair(
        &self,
        request: Request<pb::RepairRestartRequest>,
    ) -> Result<Response<pb::RepairRestartResponse>, Status> {
        use pb::repair_restart_request::Target;
        let caller = ports::caller(&request)?;
        let target = request
            .into_inner()
            .target
            .ok_or_else(|| ports::missing("target"))?;
        match target {
            Target::Session(id) => self.session(caller, id.value).await,
            Target::Tab(id) => self.tab(caller, id.value).await,
            Target::Store(_) => self.store(&caller).await,
        }
    }

    async fn session(
        &self,
        caller: Caller,
        id: String,
    ) -> Result<Response<pb::RepairRestartResponse>, Status> {
        let first = self
            .sessions
            .repair_session(&caller, id.clone(), false)
            .await
            .map(operation);
        let (sessions, who) = (self.sessions.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            "humata.v1.RepairRestartResponse",
            first,
            move || async move { sessions.repair_session(&who, id, true).await.map(operation) },
        )
        .await
    }

    async fn tab(
        &self,
        caller: Caller,
        id: String,
    ) -> Result<Response<pb::RepairRestartResponse>, Status> {
        let first = self
            .sessions
            .repair_tab(&caller, id.clone(), false)
            .await
            .map(done);
        let (sessions, who) = (self.sessions.clone(), caller.clone());
        answer_gated(
            &self.inbox,
            &caller,
            "humata.v1.RepairRestartResponse",
            first,
            move || async move { sessions.repair_tab(&who, id, true).await.map(done) },
        )
        .await
    }

    async fn store(&self, caller: &Caller) -> Result<Response<pb::RepairRestartResponse>, Status> {
        if !matches!(caller, Caller::User(_)) {
            return Err(ports::user_only_call(
                Capability::SessionStopOther,
                "restoring the store from its last good copy",
                "ask the user to run `humata doctor restarts` themselves",
            ));
        }
        let store = self.store.clone();
        let (damage, has_copy) =
            tokio::task::spawn_blocking(move || (store.damage(), store.has_last_good()))
                .await
                .map_err(|error| ports::internal(format!("the store check stopped: {error}"), ""))?;
        let damage =
            damage.map_err(|error| ports::store_error(error, &self.machine, "check the store"))?;
        let Some(problem) = damage else {
            return Err(StoreRefused::Healthy.status());
        };
        if !has_copy {
            return Err(StoreRefused::NoCopy { problem }.status());
        }
        humata2_rpc_kit::log::say!(
            "humatad2: the store is damaged ({problem}); leaving so the next start restores its last good copy"
        );
        // The stop waits for this answer to be sent.
        self.leave.notify_one();
        Ok(Response::new(pb::RepairRestartResponse {
            operation: None,
            exiting: true,
        }))
    }
}

fn operation(gated: Gated<pb::Operation>) -> Gated<pb::RepairRestartResponse> {
    match gated {
        Gated::Done(operation) => Gated::Done(pb::RepairRestartResponse {
            operation: Some(operation),
            exiting: false,
        }),
        Gated::Ask(asked) => Gated::Ask(asked),
    }
}

fn done(gated: Gated<()>) -> Gated<pb::RepairRestartResponse> {
    match gated {
        Gated::Done(()) => Gated::Done(pb::RepairRestartResponse::default()),
        Gated::Ask(asked) => Gated::Ask(asked),
    }
}
