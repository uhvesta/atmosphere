//! A machine that can't take a channel's rename by itself asks in the
//! Inbox (D348, MV-16). The host renames its own folder first, then relays
//! the rename to each machine that lists the channel. A machine whose
//! agents may be in a turn, or that has something at the new folder, renames
//! nothing and answers why. The host then puts one request in its Inbox,
//! "Rename <channel> on <machine>", naming the sessions that would stop and
//! what to do. Approving it renames the machine as `confirm_running` does,
//! and the machine's agents resume in the new folder. Until then the host's
//! records of that machine keep its old paths.

use std::collections::HashSet;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, Store, StoreError};
use tonic::Status;

use super::{ChannelSvc, blocking};
use crate::inbox::{Deferred, Run};
use crate::remote_work::RemoteError;
use crate::status;

/// Why a machine didn't rename its folder.
pub(super) enum Why {
    /// Agents there may be in a turn, or are still starting.
    Busy,
    /// Something is already at the new folder there.
    Taken,
}

/// What the machine's refusal says about itself, when it is one of the two
/// a request in the Inbox can settle.
pub(super) fn refusal_of(error: &RemoteError) -> Option<Why> {
    let RemoteError::Machine { source, .. } = error else {
        return None;
    };
    match status::error_of(source)?.kind? {
        pb::error::Kind::SessionBusy(_) => Some(Why::Busy),
        pb::error::Kind::NameTaken(_) => Some(Why::Taken),
        _ => None,
    }
}

/// The key one request for `channel` on `machine` is held under.
fn ask_key(channel: &str, machine: &str) -> String {
    format!("rename/{channel}/{machine}")
}

/// The id and title of each session of `channel` on `machine`, its open
/// Desk sessions included: what a rename there stops.
fn at_risk(store: &Store, channel: &str, machine: &str) -> Result<Vec<(String, String)>, StoreError> {
    let desks: HashSet<String> = store
        .channel_desks()?
        .into_iter()
        .filter(|row| row.channel == channel && row.closed_unix_ms.is_none())
        .map(|row| row.session_id)
        .collect();
    Ok(store
        .sessions()?
        .into_iter()
        .filter(|session| session.machine == machine && (session.channel == channel || desks.contains(&session.id)))
        .map(|session| (session.id, session.title))
        .collect())
}

impl ChannelSvc {
    /// Puts the request for `channel` on `machine` in the Inbox, saying
    /// `why`, or brings the one already there up to date.
    pub(super) async fn ask_about(&self, machine: &str, channel: &ChannelRecord, why: Why) {
        let (store, id, on) = (self.starter.store.clone(), channel.id.clone(), machine.to_owned());
        let sessions = match blocking(move || at_risk(&store, &id, &on)).await {
            Ok(Ok(found)) => found,
            Ok(Err(error)) => {
                crate::log::say!(
                    "humatad2: couldn't read which sessions of channel {} run on {machine}, so the request to rename it there names none: {}",
                    channel.name,
                    status::chain(&error)
                );
                Vec::new()
            }
            Err(error) => {
                crate::log::say!(
                    "humatad2: couldn't read which sessions of channel {} run on {machine}, so the request to rename it there names none: {}",
                    channel.name,
                    error.message()
                );
                Vec::new()
            }
        };
        let named: Vec<String> = sessions.iter().map(|(id, title)| format!("{title} ({id})")).collect();
        let listing = if named.is_empty() {
            "no session of the channel".to_owned()
        } else {
            named.join(", ")
        };
        let (name, folder) = (&channel.name, &channel.folder);
        let reason = match why {
            Why::Busy => format!(
                "Channel {name} was renamed here, but {machine} didn't rename its folder to {folder}: agents there may be in a turn. Approving this stops {listing} on {machine}, renames the folder and resumes them in it, cutting off any turn in progress. Until then {machine} keeps its old folder, and so do this host's records of its sessions."
            ),
            Why::Taken => format!(
                "Channel {name} was renamed here, but {machine} has something at the folder {folder} in Humata's channels folder, so it left its folder as it was and picked no other name. Move or delete what is at {folder} on {machine}, then approve this to try again: approving stops {listing} on {machine}, renames the folder and resumes them in it."
            ),
        };
        let card = pb::InboxRequest {
            channel: Some(pb::ChannelId { value: channel.id.clone() }),
            action_title: format!("Rename {name} on {machine}"),
            reason,
            preview: Some(pb::RequestPreview {
                preview: Some(pb::request_preview::Preview::Command(pb::CommandPreview {
                    machine: Some(pb::MachineId { value: machine.to_owned() }),
                    command: format!("rename the channel's folder to {folder}"),
                    notes: listing,
                })),
            }),
            operation: "channel.rename".to_owned(),
            ..Default::default()
        };
        let (me, id, on) = (self.clone(), channel.id.clone(), machine.to_owned());
        let run: Run = Box::new(move |_| {
            Box::pin(async move {
                me.rename_asked(&on, &id).await.map(|()| Deferred {
                    result: Vec::new(),
                    result_type: String::new(),
                })
            })
        });
        if let Err(error) = self.inbox.post_host_ask(&ask_key(&channel.id, machine), card, run).await {
            crate::log::say!(
                "humatad2: couldn't ask in the Inbox to rename channel {name} on {machine}: {}; rename the channel again once that is sorted out",
                error.message()
            );
        }
    }

    /// `machine` now holds the channel's name and folder: the request that
    /// asked for it, if one is pending, has nothing left to ask.
    pub(super) async fn settled(&self, channel: &str, machine: &str) {
        self.inbox
            .withdraw_host_ask(&ask_key(channel, machine), pb::WithdrawReason::Resolved)
            .await;
    }

    /// What approving the request does: renames `channel` on `machine` as
    /// the host has it now, stopping its agents there if they are busy.
    async fn rename_asked(&self, machine: &str, channel: &str) -> Result<(), Status> {
        let Some(remote) = &self.remote else {
            return Err(status::internal(
                "this daemon has no way to reach other machines, so the channel was not renamed there; approve the request from the host",
                "",
            ));
        };
        let (store, id) = (self.starter.store.clone(), channel.to_owned());
        let found = blocking(move || store.channel(&id))
            .await?
            .map_err(|error| status::store(error, &self.machine, "read the channel to rename it on a machine"))?;
        // A channel deleted meanwhile has nothing left to rename.
        let Some(channel) = found else {
            return Ok(());
        };
        let desks = remote
            .rename_channel_on(machine, &channel.id, &channel.name, &channel.folder, true)
            .await
            .map_err(Status::from)?;
        self.refit_desks(machine, &channel, desks).await;
        Ok(())
    }
}
