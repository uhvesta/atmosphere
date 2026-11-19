//! CopyDeskDocument (DK-13, D182.2): copies a file of a Desk session's folder
//! into a channel's `shared/` folder on this machine. The Desk keeps its own
//! copy, and a channel file of the same name is never overwritten.

use std::sync::PoisonError;

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::copy_document;
use tonic::Status;

use super::{DeskSvc, desk_status, failed_precondition, session_id};
use crate::ports::blocking;
use humata2_daemon_ports::Caller;
use crate::ports as status;

impl DeskSvc {
    /// Copies the file, or replays the first answer for the same idempotency
    /// key (as Clean up does), so a retry copies nothing again.
    pub(super) async fn copy_document(
        &self,
        _caller: &Caller,
        request: pb::CopyDeskDocumentRequest,
    ) -> Result<pb::CopyDeskDocumentResponse, Status> {
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone())
            .filter(|key| !key.is_empty());
        let Some(key) = key else {
            return self.copy_once(request).await;
        };
        // One call per key at a time; a retry answers the first call.
        let _held = self.locks.hold(&format!("copy {key}")).await;
        let first = self
            .copied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&key)
            .cloned();
        if let Some(first) = first {
            return Ok(first);
        }
        let answer = self.copy_once(request).await?;
        self.copied
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, answer.clone());
        Ok(answer)
    }

    async fn copy_once(
        &self,
        request: pb::CopyDeskDocumentRequest,
    ) -> Result<pb::CopyDeskDocumentResponse, Status> {
        let id = session_id(request.session)?;
        if let Some(machine) = request
            .machine
            .as_ref()
            .map(|machine| machine.value.as_str())
            .filter(|machine| !machine.is_empty())
        {
            if machine != self.machine {
                return Err(failed_precondition(
                    format!("machine {machine} isn't this one, and a Desk document is copied on the machine that holds its session; nothing was copied"),
                    "leave machine unset, or name this machine, and copy again",
                ));
            }
        }
        let channel_id = request
            .channel
            .map(|channel| channel.value)
            .filter(|channel| !channel.is_empty())
            .ok_or_else(|| status::missing("channel"))?;
        let (session, _) = self.load(&id).await?;
        let (store, key) = (self.starter.store.clone(), channel_id.clone());
        let channel = blocking(move || store.channel(&key))
            .await?
            .map_err(|error| crate::ports::store_error(error, &self.machine, "look up the channel"))?;
        let Some(channel) = channel else {
            return Err(status::not_found(
                pb::EntityKind::Channel,
                &channel_id,
                format!("no channel {channel_id} is on this machine"),
                "pick a channel from the list",
            ));
        };
        let shared = self.starter.layout.shared(&channel.folder);
        let (folder, path) = (session.folder.clone(), request.path.clone());
        let name = blocking(move || copy_document(&folder, &path, &shared))
            .await?
            .map_err(|error| desk_status(&error))?;
        Ok(pb::CopyDeskDocumentResponse {
            copy: Some(pb::FileLocation {
                machine: Some(pb::MachineId {
                    value: self.machine.clone(),
                }),
                root: Some(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                    channel: Some(pb::ChannelId { value: channel.id }),
                })),
                path: name,
            }),
        })
    }
}
