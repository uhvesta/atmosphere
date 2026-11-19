//! Sessions on other machines, as the host's SessionService handles them (SL-1, SL-3,
//! PER-11, CH-4, HO-5; D139, D144, D150).

use humata2_daemon_ports::Caller;
use humata2_store::SessionRecord;
use humata2_svc_sessions_common::pb;
use tonic::{Code, Status};

use crate::SessionTransferSvc;
use crate::ports::{clean_title, internal};

/// Whether `record` says its session lives on a machine other than `here`.
pub fn elsewhere(record: &SessionRecord, here: &str) -> bool {
    !record.machine.is_empty() && record.machine != here
}

/// The start the restart's report names as one it couldn't finish or remove:
/// what it says about `id`, which is what the person has to do. Only the
/// session's own line counts, not a shell tab of it.
pub fn stuck_start<'a>(last: &'a pb::LastStart, id: &str) -> Option<&'a str> {
    last.not_brought_back
        .iter()
        .find(|line| line.session == id && line.tab.is_empty())
        .map(|line| line.problem.as_str())
}

/// Error returned when a remote machine refuses or mishandles a session call.
#[derive(Debug, thiserror::Error)]
pub enum RemoteError {
    #[error("{machine} made session {got:?} instead of {asked:?}")]
    IdNotHonoured {
        machine: String,
        asked: String,
        got: String,
    },
    #[error("{machine} refused to {doing}: {reason}")]
    Machine {
        machine: String,
        doing: &'static str,
        reason: String,
        #[source]
        source: Status,
    },
}

impl From<RemoteError> for Status {
    fn from(error: RemoteError) -> Self {
        match error {
            RemoteError::IdNotHonoured { .. } => internal(error.to_string(), ""),
            RemoteError::Machine { source, .. } => source,
        }
    }
}

/// The status a machine's refusal of a start answers with. A machine that
/// made the session under another id than the host's has that session still:
/// nothing on the host points to it, so the answer names it and the machine
/// for the person to remove there, as a stop of it here could end someone
/// else's session of that id.
pub fn refusal(error: RemoteError) -> Status {
    match &error {
        RemoteError::IdNotHonoured { machine, got, .. } if !got.is_empty() => {
            let message = format!(
                "{error}; the session it made, {got:?}, is still on {machine} and nothing here points to it, so remove it on {machine} before you start again"
            );
            internal(message, "")
        }
        _ => Status::from(error),
    }
}

/// The failure an operation records for `status`.
pub fn failure_of(status: &Status) -> pb::Error {
    pb::Error {
        message: status.message().to_owned(),
        fix: String::new(),
        retry: None,
        kind: None,
    }
}

impl SessionTransferSvc {
    /// Whether `record`'s session lives on another machine.
    pub fn lives_elsewhere(&self, record: &SessionRecord) -> bool {
        elsewhere(record, &self.core.machine)
    }

    /// StartSession on another machine.
    pub async fn begin_remote(
        &self,
        _caller: &Caller,
        request: pb::StartSessionRequest,
        machine: &str,
    ) -> Result<pb::StartSessionResponse, Status> {
        let _title = clean_title(&request.title)?;
        Err(self.env.not_wired(&format!("begin_remote on {machine}")))
    }

    /// StopSession for a session on another machine.
    pub async fn stop_remote(&self, id: &str, machine: &str) -> Result<pb::Session, Status> {
        Err(self.env.not_wired(&format!("stop_remote {id} on {machine}")))
    }

    /// ResumeSession for a session on another machine.
    pub async fn resume_remote(
        &self,
        _caller: &Caller,
        _request: pb::ResumeSessionRequest,
        id: &str,
        machine: &str,
    ) -> Result<pb::Operation, Status> {
        Err(self.env.not_wired(&format!("resume_remote {id} on {machine}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(session: &str, tab: &str, problem: &str) -> pb::NotBroughtBack {
        pb::NotBroughtBack {
            session: session.to_owned(),
            tab: tab.to_owned(),
            problem: problem.to_owned(),
        }
    }

    /// A session the restart couldn't finish or remove is named stuck, with
    /// the restart's own words; one it didn't name, or no restart, is not.
    #[test]
    fn a_start_the_restart_left_stuck_is_found_by_its_session() {
        let last = pb::LastStart {
            not_brought_back: vec![
                line("", "", "the store is damaged"),
                line("se_other", "", "other problem"),
                line("se_1", "", "delete its folder by hand"),
            ],
            ..pb::LastStart::default()
        };
        assert_eq!(
            stuck_start(&last, "se_1"),
            Some("delete its folder by hand")
        );
        assert_eq!(stuck_start(&last, "se_2"), None);
        assert_eq!(stuck_start(&pb::LastStart::default(), "se_1"), None);
    }

    /// A shell tab that didn't come back is not its session's stuck start.
    #[test]
    fn a_shell_tab_that_did_not_come_back_is_not_a_stuck_start() {
        let last = pb::LastStart {
            not_brought_back: vec![line("se_1", "tab_1", "shell gone")],
            ..pb::LastStart::default()
        };
        assert_eq!(stuck_start(&last, "se_1"), None);
    }

    /// A machine that made the session under another id leaves that session
    /// on it, so the refusal names it and the machine for the person to remove.
    #[test]
    fn a_machine_that_ignored_the_id_is_refused_naming_the_session_it_made() {
        let error = RemoteError::IdNotHonoured {
            machine: "devbox".to_owned(),
            asked: "se_1".to_owned(),
            got: "se_other".to_owned(),
        };
        let status = refusal(error);
        assert_eq!(status.code(), Code::Internal);
        assert!(status.message().contains("se_other"));
        assert!(status.message().contains("devbox"));
    }

    /// Any other refusal goes to the caller as the machine said it.
    #[test]
    fn another_refusal_is_the_machines_own_answer() {
        let source = Status::failed_precondition("no clone");
        let error = RemoteError::Machine {
            machine: "devbox".to_owned(),
            doing: "start the session",
            reason: "no clone".to_owned(),
            source,
        };
        assert_eq!(refusal(error).code(), Code::FailedPrecondition);
    }
}
