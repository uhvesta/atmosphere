//! `DaemonService.StopSessions` and `StartSessions` (D261, D265). Each
//! saves its record, answers, and then has the daemon start itself again:
//! the next run sweeps (stopped) or brings everything back (started).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use humata2_access::Refusal;
use humata2_grpc::humata::v1 as pb;
use tokio::sync::Notify;
use tonic::{Code, Request, Response, Status};

use super::boot::to_pb;
use super::{ByKind, Record, RecordError, StopFolder, StopState, StoppedBy, Via};
use crate::channel_service::failed;
use crate::identity::{self, Caller};
use crate::status;

/// How the serve loop is asked to leave and start itself again: the stop
/// signal's other way in, beside the doctor's.
#[derive(Clone)]
pub(crate) struct Relaunch {
    leave: Arc<Notify>,
    /// The fault point to reach just before starting again, set when one
    /// was asked for.
    point: Arc<Mutex<Option<&'static str>>>,
}

impl Relaunch {
    pub(crate) fn new(leave: Arc<Notify>) -> Self {
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
    pub(crate) fn take(&self) -> Option<&'static str> {
        self.point
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// What serves StopSessions and StartSessions.
pub(crate) struct StopSvc {
    folder: StopFolder,
    machine: String,
    relaunch: Relaunch,
}

impl StopSvc {
    pub(crate) fn new(folder: StopFolder, machine: String, relaunch: Relaunch) -> Self {
        Self {
            folder,
            machine,
            relaunch,
        }
    }

    pub(crate) async fn stop(
        &self,
        request: Request<pb::StopSessionsRequest>,
    ) -> Result<Response<pb::StopSessionsResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        let wanted = new_record(request.into_inner(), &self.machine);
        let folder = self.folder.clone();
        let (record, taken) = tokio::task::spawn_blocking(move || stop_now(&folder, wanted))
            .await
            .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
            .map_err(|error| record_status(&error))?;
        if taken {
            self.relaunch.request("stop.exec");
        }
        Ok(Response::new(pb::StopSessionsResponse {
            stopped: Some(to_pb(&record)),
        }))
    }

    pub(crate) async fn start(
        &self,
        request: Request<pb::StartSessionsRequest>,
    ) -> Result<Response<pb::StartSessionsResponse>, Status> {
        let caller = identity::caller(&request)?;
        user_only(&caller)?;
        let folder = self.folder.clone();
        tokio::task::spawn_blocking(move || folder.begin_start())
            .await
            .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))?
            .map_err(|error| record_status(&error))?;
        self.relaunch.request("start.exec");
        Ok(Response::new(pb::StartSessionsResponse {}))
    }
}

/// Saves `wanted` as the stop unless this machine is stopped already, in
/// which case that first stop is what comes back. True when `wanted` is the
/// one saved. Blocks.
fn stop_now(folder: &StopFolder, wanted: Record) -> Result<(Record, bool), RecordError> {
    match folder.read()? {
        StopState::Stopped(existing) => Ok((existing, false)),
        StopState::Running | StopState::Starting(_) => {
            folder.write_stopped(&wanted)?;
            Ok((wanted, true))
        }
    }
}

/// Only the user stops or starts a machine's sessions; an agent's own rule
/// comes with the access capability (D263).
fn user_only(caller: &Caller) -> Result<(), Status> {
    match caller {
        Caller::User(_) => Ok(()),
        Caller::Peer => Err(status::refused(&Refusal::PeerCannotCall)),
        Caller::Agent(_) | Caller::Script(_) => {
            Err(status::refused(&Refusal::UserOnly { capability: None }))
        }
    }
}

fn new_record(request: pb::StopSessionsRequest, machine: &str) -> Record {
    let via = match pb::StopVia::try_from(request.via).unwrap_or(pb::StopVia::Unspecified) {
        pb::StopVia::App => Via::App,
        pb::StopVia::Cli => Via::Cli,
        pb::StopVia::Ssh => Via::Ssh,
        pb::StopVia::Direct | pb::StopVia::Unspecified => Via::Direct,
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
            kind: ByKind::User,
            via,
            machine: machine.to_owned(),
            session: None,
        },
        reason: Some(request.reason).filter(|reason| !reason.is_empty()),
        ended: None,
    }
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// A record error as the status the caller gets. The errors already say
/// what to do, so the fix is left to them.
fn record_status(error: &RecordError) -> Status {
    let code = match error {
        RecordError::NotStopped { .. } | RecordError::Unreadable { .. } => {
            Code::FailedPrecondition
        }
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
            },
            "ma_1",
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
