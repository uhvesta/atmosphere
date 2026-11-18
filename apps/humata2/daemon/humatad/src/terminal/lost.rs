//! Input to a session whose machine is gone (D783). Removing or forgetting
//! a machine archives its sessions as lost with it, but the session's record
//! still names the machine, and the host knows machines by name (D61,
//! D776): once another machine is added under that name, a call relayed
//! there would be answered by a daemon that never had the session, as
//! `not_found`. So nothing is relayed for a session whose machine was
//! removed or forgotten after it ran there (its archive is marked
//! `machine_removed`, or `machine_lost`); the caller hears that the machine
//! is gone and what still works.

use humata2_grpc::humata::v1 as pb;
use humata2_store::SessionRecord;
use prost::Message;
use tonic::{Code, Status};

use super::{Core, blocking};
use crate::status;

#[derive(Debug, thiserror::Error)]
enum InputRefused {
    #[error(
        "session {session} ran on machine {machine}, which was removed from Humata or forgotten since, so nothing runs the session any more"
    )]
    MachineGone { session: String, machine: String },
}

impl Core {
    /// Refuses input to, or an attach of, `record` when its machine was
    /// removed or forgotten after it ran there: archived then, or before,
    /// and marked so (or lost with a machine forgotten before the mark
    /// existed). Any other session goes on.
    pub(super) async fn refuse_if_machine_gone(&self, record: &SessionRecord) -> Result<(), Status> {
        let (store, id) = (self.config.store.clone(), record.id.clone());
        let row = blocking(move || store.archive(&id))
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "check whether the session's machine is gone"))?;
        let lost = row
            .and_then(|row| pb::ArchiveSummary::decode(row.summary.as_slice()).ok())
            .is_some_and(|summary| summary.machine_removed || summary.machine_lost);
        if !lost {
            return Ok(());
        }
        let refused = InputRefused::MachineGone {
            session: record.id.clone(),
            machine: record.machine.clone(),
        };
        Err(status::failure(
            Code::FailedPrecondition,
            pb::Error {
                message: refused.to_string(),
                fix: format!(
                    "its history still reads with `humata session read {}`. To carry on its work, start a new session on a machine you have now; a machine added under the name {} is another machine and doesn't have this session",
                    record.id, record.machine
                ),
                retry: None,
                kind: None,
            },
        ))
    }
}
