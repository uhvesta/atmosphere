//! `DaemonService.StopSessions` and `StartSessions` (D261, D265). Each
//! saves its record, answers, and then has the daemon start itself again:
//! the next run sweeps (stopped) or brings everything back (started).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::{
    AccessRequest, Call, Capability, MachineAction, MachineActionKind, MachineId, Refusal, Target,
};
use humata2_daemon_ports::{Caller, Gated};
use tokio::sync::Notify;
use tonic::{Code, Request, Response, Status};

use super::boot::to_pb;
use super::{ByKind, Record, RecordError, StopFolder, StopState, StoppedBy, Via};
use crate::pb;
use crate::ports::{self, StopGuardPort, StopInboxPort, answer_gated, failed};

/// How the serve loop is asked to leave and start itself again: the stop
/// signal's other way in, beside the doctor's.
#[derive(Clone)]
pub struct Relaunch {
    leave: Arc<Notify>,
    /// The fault point to reach just before starting again, set when one
    /// was asked for.
    point: Arc<Mutex<Option<&'static str>>>,
}

impl Relaunch {
    pub fn new(leave: Arc<Notify>) -> Self {
        Self {
            leave,
            point: Arc::default(),
        }
    }

    /// Asks the daemon to start itself again once it has answered what is
    /// in flight, reaching `point` first.
    fn request(&self, point: &'static str) {
        *self.point.lock().unwrap_or_else(PoisonError::into_inner) = Some(point);
        self.leave.notify_one();
    }

    /// The fault point of a start again that was asked for, if one was.
    pub fn take(&self) -> Option<&'static str> {
        self.point
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// Who a stop is, once the rules let it through.
struct Admitted {
    kind: ByKind,
    /// The session whose agent asked, for an agent.
    session: Option<String>,
}

impl Admitted {
    fn of(kind: ByKind, session: Option<String>) -> Self {
        Self { kind, session }
    }
}

/// What serves StopSessions and StartSessions.
#[derive(Clone)]
pub struct StopSvc {
    folder: StopFolder,
    machine: String,
    relaunch: Relaunch,
    guard: Arc<dyn StopGuardPort>,
    /// Where an agent's stop or start waits for the user when its rule
    /// says Ask me (D263).
    inbox: Arc<dyn StopInboxPort>,
}

impl StopSvc {
    pub fn new(
        folder: StopFolder,
        machine: String,
        relaunch: Relaunch,
        guard: Arc<dyn StopGuardPort>,
        inbox: Arc<dyn StopInboxPort>,
    ) -> Self {
        Self {
            folder,
            machine,
            relaunch,
            guard,
            inbox,
        }
    }

    pub async fn stop(
        &self,
        request: Request<pb::StopSessionsRequest>,
    ) -> Result<Response<pb::StopSessionsResponse>, Status> {
        let caller = ports::caller_if_any(&request)?;
        let outside = ports::outside_every_session(&request);
        let wanted = request.into_inner();
        let first = self
            .stop_as(caller.as_ref(), outside, wanted.clone(), false)
            .await;
        let Some(agent) = caller.filter(|caller| matches!(caller, Caller::Agent(_))) else {
            return first.map(|gated| match gated {
                Gated::Done(done) => Response::new(done),
                // Only an agent's call is ever sent to Ask me.
                Gated::Ask(_) => Response::new(pb::StopSessionsResponse::default()),
            });
        };
        let me = self.clone();
        let who = agent.clone();
        answer_gated(
            &self.inbox,
            &agent,
            "humata.v1.StopSessionsResponse",
            first,
            move || async move { me.stop_as(Some(&who), false, wanted, true).await },
        )
        .await
    }

    /// The body of StopSessions; `approved` once the user said yes to it in
    /// the Inbox.
    async fn stop_as(
        &self,
        caller: Option<&Caller>,
        outside: bool,
        wanted: pb::StopSessionsRequest,
        approved: bool,
    ) -> Result<Gated<pb::StopSessionsResponse>, Status> {
        let admitted = match self.admit_stop(caller, outside, approved)? {
            Gated::Ask(asked) => return Ok(Gated::Ask(asked)),
            Gated::Done(admitted) => admitted,
        };
        let wanted = new_record(wanted, &self.machine, admitted);
        let folder = self.folder.clone();
        let (record, taken) = tokio::task::spawn_blocking(move || stop_now(&folder, wanted))
            .await
            .map_err(|error| ports::internal(format!("a background task failed: {error}"), ""))?
            .map_err(|error| record_status(&error))?;
        if taken {
            self.relaunch.request("stop.exec");
        }
        Ok(Gated::Done(pb::StopSessionsResponse {
            stopped: Some(to_pb(&record)),
        }))
    }

    /// Who may stop this machine's sessions (D263): the user always; a
    /// person typing `humata stop` on the machine with no credential,
    /// outside every session; an agent or script only as its rule or token
    /// allows.
    fn admit_stop(
        &self,
        caller: Option<&Caller>,
        outside: bool,
        approved: bool,
    ) -> Result<Gated<Admitted>, Status> {
        let Some(caller) = caller else {
            return if outside {
                Ok(Gated::Done(Admitted::of(ByKind::Terminal, None)))
            } else {
                Err(ports::unauthenticated())
            };
        };
        match caller {
            Caller::Peer => Err(ports::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) => Ok(Gated::Done(Admitted::of(ByKind::User, None))),
            Caller::Agent(agent) => Ok(
                match self.gate(caller, MachineActionKind::StopSessions, approved)? {
                    Some(asked) => Gated::Ask(asked),
                    None => Gated::Done(Admitted::of(
                        ByKind::Agent,
                        Some(agent.session.id.to_string()),
                    )),
                },
            ),
            Caller::Script(_) => {
                if let Some(asked) = self.gate(caller, MachineActionKind::StopSessions, approved)? {
                    return Err(ports::needs_approval(asked.capability));
                }
                Ok(Gated::Done(Admitted::of(ByKind::Script, None)))
            }
        }
    }

    /// Checks `action` on this machine for `caller` against the rule;
    /// `Some` is the request a rule of Ask me puts in the Inbox.
    fn gate(
        &self,
        caller: &Caller,
        action: MachineActionKind,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        ports::gate(
            &self.guard,
            caller,
            &Call::Use {
                capability: action.capability(),
                target: Target::Machine(MachineAction {
                    machine: MachineId::new(&self.machine),
                    action,
                }),
            },
            approved,
        )
    }

    pub async fn start(
        &self,
        request: Request<pb::StartSessionsRequest>,
    ) -> Result<Response<pb::StartSessionsResponse>, Status> {
        let caller = ports::caller(&request)?;
        let first = self.start_as(&caller, false).await;
        let me = self.clone();
        let who = caller.clone();
        answer_gated(
            &self.inbox,
            &caller,
            "humata.v1.StartSessionsResponse",
            first,
            move || async move { me.start_as(&who, true).await },
        )
        .await
    }

    /// The body of StartSessions. The user always may. An agent or script
    /// never starts what the user stopped, whatever its rule says; it
    /// starts a stop an agent or a terminal made only as its rule allows.
    async fn start_as(
        &self,
        caller: &Caller,
        approved: bool,
    ) -> Result<Gated<pb::StartSessionsResponse>, Status> {
        match caller {
            Caller::Peer => return Err(ports::refused(&Refusal::PeerCannotCall)),
            Caller::User(_) => {}
            Caller::Agent(_) | Caller::Script(_) => {
                if self.stopped_by_user().await? {
                    return Err(ports::refused(&Refusal::UserOnly {
                        capability: Some(Capability::MachineSessionsStop),
                    }));
                }
                if let Some(asked) = self.gate(caller, MachineActionKind::StartSessions, approved)?
                {
                    return if matches!(caller, Caller::Agent(_)) {
                        Ok(Gated::Ask(asked))
                    } else {
                        Err(ports::needs_approval(asked.capability))
                    };
                }
            }
        }
        let folder = self.folder.clone();
        tokio::task::spawn_blocking(move || folder.begin_start())
            .await
            .map_err(|error| ports::internal(format!("a background task failed: {error}"), ""))?
            .map_err(|error| record_status(&error))?;
        self.relaunch.request("start.exec");
        Ok(Gated::Done(pb::StartSessionsResponse {}))
    }

    /// Whether the stop in force was the user's own. A machine that is not
    /// stopped has none.
    async fn stopped_by_user(&self) -> Result<bool, Status> {
        let folder = self.folder.clone();
        let state = tokio::task::spawn_blocking(move || folder.read())
            .await
            .map_err(|error| ports::internal(format!("a background task failed: {error}"), ""))?
            .map_err(|error| record_status(&error))?;
        Ok(matches!(
            state,
            StopState::Stopped(record) if record.by.kind == ByKind::User
        ))
    }
}

/// Saves `wanted` as the stop unless this machine is stopped already, in
/// which case that first stop is what comes back. True when `wanted` is the
/// one saved. A stop this machine was already started from is refused
/// (`AlreadyApplied`), so one that arrives late changes nothing (D262).
/// Blocks.
fn stop_now(folder: &StopFolder, wanted: Record) -> Result<(Record, bool), RecordError> {
    match folder.read()? {
        StopState::Stopped(existing) => Ok((existing, false)),
        StopState::Starting(starting) if starting.stop_id == wanted.stop_id => {
            Err(RecordError::AlreadyApplied {
                stop_id: wanted.stop_id,
            })
        }
        StopState::Running | StopState::Starting(_) => {
            folder.refuse_applied(&wanted.stop_id)?;
            folder.write_stopped(&wanted)?;
            Ok((wanted, true))
        }
    }
}

fn new_record(request: pb::StopSessionsRequest, machine: &str, admitted: Admitted) -> Record {
    let via = match pb::StopVia::try_from(request.via).unwrap_or(pb::StopVia::Unspecified) {
        pb::StopVia::App => Via::App,
        pb::StopVia::Cli => Via::Cli,
        pb::StopVia::Ssh => Via::Ssh,
        pb::StopVia::Direct | pb::StopVia::Unspecified => Via::Direct,
    };
    // A person typing on the machine came by the CLI, whatever they sent.
    let via = if admitted.kind == ByKind::Terminal {
        Via::Cli
    } else {
        via
    };
    let stop_id = if request.stop_id.is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        request.stop_id
    };
    Record {
        stop_id,
        stopped_at_unix_ms: unix_ms(),
        by: StoppedBy {
            kind: admitted.kind,
            via,
            // The host stopping a box is the one who asked (D262).
            machine: Some(request.by_machine)
                .filter(|asked| !asked.is_empty())
                .unwrap_or_else(|| machine.to_owned()),
            session: admitted.session,
        },
        reason: Some(request.reason).filter(|reason| !reason.is_empty()),
        ended: None,
    }
}

pub(super) fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// A record error as the status the caller gets. The errors already say
/// what to do, so the fix is left to them.
fn record_status(error: &RecordError) -> Status {
    let code = match error {
        RecordError::NotStopped { .. }
        | RecordError::Unreadable { .. }
        | RecordError::AlreadyApplied { .. } => Code::FailedPrecondition,
        _ => Code::Internal,
    };
    failed(code, error, "", None)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn record(stop_id: &str, reason: &str) -> Record {
        new_record(
            pb::StopSessionsRequest {
                stop_id: stop_id.to_owned(),
                reason: reason.to_owned(),
                via: pb::StopVia::Cli as i32,
                ..Default::default()
            },
            "ma_1",
            Admitted::of(ByKind::User, None),
        )
    }

    #[test]
    fn a_stop_is_saved_and_reads_back_as_stopped() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        let (saved, taken) = stop_now(&folder, record("s1", "runaway")).expect("saved");
        assert!(taken);
        assert_eq!(saved.stop_id, "s1");
        assert_eq!(folder.read().expect("readable"), StopState::Stopped(saved));
    }

    #[test]
    fn a_second_stop_gets_the_first_back_and_changes_nothing() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        let (first, _) = stop_now(&folder, record("s1", "first")).expect("saved");
        let (again, taken) = stop_now(&folder, record("s2", "second")).expect("answered");
        assert!(!taken);
        assert_eq!(again, first);
        assert_eq!(folder.read().expect("readable"), StopState::Stopped(first));
    }

    #[test]
    fn a_stop_with_no_id_is_given_one_and_each_is_different() {
        assert!(!record("", "").stop_id.is_empty());
        assert_ne!(record("", "").stop_id, record("", "").stop_id);
    }

    #[test]
    fn a_stop_the_host_asked_for_names_the_host_as_who_stopped_it() {
        let asked = new_record(
            pb::StopSessionsRequest {
                stop_id: "s1".to_owned(),
                via: pb::StopVia::Ssh as i32,
                by_machine: "host-mac".to_owned(),
                ..Default::default()
            },
            "ma_1",
            Admitted::of(ByKind::User, None),
        );
        assert_eq!(asked.by.machine, "host-mac");
        assert_eq!(record("s1", "").by.machine, "ma_1");
    }

    #[test]
    fn a_stop_already_applied_and_started_from_is_refused_and_leaves_the_machine_running() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        stop_now(&folder, record("s1", "")).expect("saved");
        folder.begin_start().expect("started");
        let error = stop_now(&folder, record("s1", "")).expect_err("a stop under way is applied");
        assert_eq!(record_status(&error).code(), Code::FailedPrecondition);
        folder.finish_start().expect("finished");
        let error = stop_now(&folder, record("s1", "")).expect_err("the stop was applied");
        assert_eq!(record_status(&error).code(), Code::FailedPrecondition);
        assert_eq!(folder.read().expect("readable"), StopState::Running);
        let (_, taken) = stop_now(&folder, record("s2", "")).expect("a new stop is taken");
        assert!(taken);
    }

    #[test]
    fn a_stop_typed_on_the_machine_is_recorded_as_from_the_cli_and_names_no_session() {
        let made = new_record(
            pb::StopSessionsRequest {
                stop_id: "s1".to_owned(),
                reason: String::new(),
                via: pb::StopVia::App as i32,
                ..Default::default()
            },
            "ma_1",
            Admitted::of(ByKind::Terminal, None),
        );
        assert_eq!(made.by.kind, ByKind::Terminal);
        assert_eq!(made.by.via, Via::Cli);
        assert_eq!(made.by.session, None);
    }

    #[test]
    fn an_agents_stop_names_its_session() {
        let made = new_record(
            pb::StopSessionsRequest::default(),
            "ma_1",
            Admitted::of(ByKind::Agent, Some("se_7".to_owned())),
        );
        assert_eq!(made.by.kind, ByKind::Agent);
        assert_eq!(made.by.session.as_deref(), Some("se_7"));
    }

    #[test]
    fn a_stop_with_no_reason_has_none() {
        assert_eq!(record("s1", "").reason, None);
    }

    #[test]
    fn starting_what_is_not_stopped_is_a_failed_precondition() {
        let dir = TempDir::new().expect("a temp dir");
        let folder = StopFolder::new(dir.path());
        let error = folder.begin_start().expect_err("nothing is stopped");
        assert_eq!(record_status(&error).code(), Code::FailedPrecondition);
    }

    #[tokio::test]
    async fn a_relaunch_asked_for_wakes_the_serve_loop_and_names_its_point_once() {
        let leave = Arc::new(Notify::new());
        let relaunch = Relaunch::new(leave.clone());
        assert_eq!(relaunch.take(), None);
        relaunch.request("stop.exec");
        leave.notified().await;
        assert_eq!(relaunch.take(), Some("stop.exec"));
        assert_eq!(relaunch.take(), None);
    }
}
