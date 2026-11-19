//! Session archive & forget service (`humata2_svc_sessions_archive`),
//! implementing [`SessionArchivePort`] as a parallel sibling to
//! `svc_sessions_local` and `svc_sessions_transfer`.

pub mod ports;

use std::sync::Arc;

use humata2_daemon_ports::{Caller, PortError, SessionArchivePort};
use humata2_svc_sessions_common::{SessionCore, pb};

pub use self::ports::{ArchiveGuardPort, ArchiveHoldersPort, ArchiveRelayPort};
pub use humata2_svc_sessions_common::archive::{PlanKind, failure_of, session_id};
pub use humata2_svc_sessions_common::archive_remote::{
    finished, follow_to_end, machine_failed, next_link_up,
};

/// Archive/forget service implementation backed by [`SessionCore`] and narrow ports.
#[derive(Clone)]
pub struct SessionArchiveSvc {
    core: Arc<SessionCore>,
    guard: Arc<dyn ArchiveGuardPort>,
    relay: Arc<dyn ArchiveRelayPort>,
    holders: Arc<dyn ArchiveHoldersPort>,
}

impl SessionArchiveSvc {
    #[must_use]
    pub fn new(
        core: Arc<SessionCore>,
        guard: Arc<dyn ArchiveGuardPort>,
        relay: Arc<dyn ArchiveRelayPort>,
        holders: Arc<dyn ArchiveHoldersPort>,
    ) -> Self {
        Self {
            core,
            guard,
            relay,
            holders,
        }
    }

    #[must_use]
    pub fn core(&self) -> &Arc<SessionCore> {
        &self.core
    }

    #[must_use]
    pub fn guard(&self) -> &Arc<dyn ArchiveGuardPort> {
        &self.guard
    }

    #[must_use]
    pub fn relay(&self) -> &Arc<dyn ArchiveRelayPort> {
        &self.relay
    }

    #[must_use]
    pub fn holders(&self) -> &Arc<dyn ArchiveHoldersPort> {
        &self.holders
    }
}

#[tonic::async_trait]
impl SessionArchivePort for SessionArchiveSvc {
    async fn archive_session(
        &self,
        caller: &Caller,
        req: pb::ArchiveSessionRequest,
    ) -> Result<pb::Operation, PortError> {
        let sid = session_id(req.id.clone()).map_err(PortError::Status)?;
        self.guard
            .check_archive(caller, &sid)
            .map_err(PortError::Status)?;
        let machine = self
            .core
            .store
            .session(&sid)
            .map_err(|e| PortError::new(e.to_string()))?
            .map(|record| record.machine)
            .ok_or_else(|| PortError::Status(tonic::Status::not_found(format!("session {sid} not found"))))?;
        if machine != self.core.machine {
            return self.relay.archive_on_machine(&machine, req).await;
        }
        self.holders
            .end_session_holders(&sid)
            .map_err(PortError::new)?;
        self.core.tell.ended(&sid);
        Ok(pb::Operation {
            id: Some(pb::OperationId {
                value: format!("archive:{sid}"),
            }),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_kind_and_session_id_are_reexported() {
        let id = session_id(Some(pb::SessionId {
            value: "se_123".to_owned(),
        }))
        .expect("valid id");
        assert_eq!(id, "se_123");
    }
}
