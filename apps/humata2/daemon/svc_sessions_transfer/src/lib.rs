//! Session transfer, move, rebuild, remote fork, continue-in, push, and remote
//! start/review handlers for `humatad2` (C3 Sibling 2; `SPLIT_PHASE3_DAEMON.md` §2.3).

pub mod continue_in;
pub mod fork_remote;
pub mod move_recover;
pub mod move_session;
pub mod ports;
pub mod push;
pub mod rebuild;
pub mod remote;
pub mod remote_review;
pub mod remote_start;
pub mod start_fresh;

use std::sync::Arc;

use humata2_daemon_ports::{Caller, PortError, SessionTransferPort};
use humata2_svc_sessions_common::{SessionCore, pb};

use crate::ports::TransferEnv;

/// Owns the transfer/roam/push/remote handlers and implements
/// [`SessionTransferPort`] for [`SessionCore`].
#[derive(Clone)]
pub struct SessionTransferSvc {
    pub core: SessionCore,
    pub env: Arc<dyn TransferEnv>,
}

impl SessionTransferSvc {
    pub fn new(core: SessionCore, env: Arc<dyn TransferEnv>) -> Self {
        Self { core, env }
    }
}

#[tonic::async_trait]
impl SessionTransferPort for SessionTransferSvc {
    async fn move_session(
        &self,
        caller: &Caller,
        req: pb::MoveSessionRequest,
    ) -> Result<pb::Operation, PortError> {
        self.handle_move_session(caller, req)
            .await
            .map_err(PortError::from)
    }

    async fn rebuild_session(
        &self,
        caller: &Caller,
        req: pb::RebuildSessionRequest,
    ) -> Result<pb::Operation, PortError> {
        self.handle_rebuild_session(caller, req)
            .await
            .map_err(PortError::from)
    }
}
