//! A renamed channel's folder follows on every machine the channel lists
//! (MV-16, D314). Each machine does what this host does in rename_folder.rs
//! through its own `RenameChannel`: it derives the folder from the name,
//! renames the folder once and rewrites its records. Its agents and shells
//! keep running under the old name, which stays a link to the new folder
//! while they do (D801 D); only a folder on another disk than the Humata
//! folder stops its agents and resumes them. A machine that is offline is renamed when it next
//! links: nothing is queued, the host compares each channel's name and
//! folder with the machine's at every link, so a host restart loses
//! nothing. Until then the host leaves its records of that machine's
//! sessions as they are, so a machine not yet renamed is never shown as if
//! it were.

use std::path::PathBuf;

use humata2_store::{ChannelRecord, Placement, StoreError};
use tonic::Status;

use super::rename_ask::refusal_of;
use super::{ChannelSvc, blocking};
use crate::host_relay::LinkState;
use crate::remote_work::RemoteError;
use crate::status;
use crate::trash_service::now_ms;

impl ChannelSvc {
    /// Makes each linked machine of `channel` carry the name and folder the
    /// host has. A machine whose link is down is skipped: it is renamed when
    /// it links. A machine that is up and refuses answers its own refusal,
    /// which says what to do; the host's rename stands, and asking again
    /// renames the machine.
    pub(super) async fn rename_on_machines(&self, channel: &ChannelRecord, confirmed: bool) -> Result<(), Status> {
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        for machine in channel.machines.iter().filter(|machine| **machine != self.machine) {
            match remote
                .rename_channel_on(machine, &channel.id, &channel.name, &channel.folder, confirmed)
                .await
            {
                Ok(desks) => {
                    self.refit_desks(machine, channel, desks).await;
                    self.settled(&channel.id, machine).await;
                }
                Err(RemoteError::Relay(_)) => crate::log::say!(
                    "humatad2: {machine} is offline, so channel {}'s folder there is renamed when it next links; until then it stays as it was",
                    channel.name
                ),
                // D348: a machine that can't rename by itself asks in the Inbox.
                Err(error) => match refusal_of(&error) {
                    Some(why) => self.ask_about(machine, channel, why).await,
                    None => return Err(Status::from(error)),
                },
            }
        }
        Ok(())
    }

    /// Catches machines up for the daemon's life: each machine already up,
    /// then each that links, is renamed to what the host's channels say.
    pub(super) async fn follow_machines(&self) {
        let Some(remote) = self.remote.clone() else {
            return;
        };
        let mut links = remote.watch_links();
        for machine in remote.machines_up() {
            self.catch_up(machine);
        }
        while let Some((machine, state)) = links.next().await {
            if matches!(state, LinkState::Up { .. }) {
                self.catch_up(machine);
            }
        }
    }

    fn catch_up(&self, machine: String) {
        let me = self.clone();
        tokio::spawn(async move { me.rename_machine(&machine).await });
    }

    /// Renames every channel of `machine` whose name or folder there isn't
    /// the host's. One that can't be is logged, and tried again at the
    /// machine's next link or the channel's next rename.
    async fn rename_machine(&self, machine: &str) {
        let Some(remote) = &self.remote else {
            return;
        };
        let store = self.starter.store.clone();
        let channels = match blocking(move || store.channels()).await {
            Ok(Ok(channels)) => channels,
            Ok(Err(error)) => {
                crate::log::say!("humatad2: couldn't read the channels to catch {machine} up: {}", status::chain(&error));
                return;
            }
            Err(error) => {
                crate::log::say!("humatad2: couldn't read the channels to catch {machine} up: {}", error.message());
                return;
            }
        };
        for channel in channels.iter().filter(|channel| channel.machines.iter().any(|held| held == machine)) {
            match remote
                .rename_channel_on(machine, &channel.id, &channel.name, &channel.folder, false)
                .await
            {
                Ok(desks) => {
                    self.refit_desks(machine, channel, desks).await;
                    self.settled(&channel.id, machine).await;
                }
                Err(error) => match refusal_of(&error) {
                    Some(why) => self.ask_about(machine, channel, why).await,
                    None => crate::log::say!(
                        "humatad2: couldn't rename channel {} on {machine}: {}; rename the channel again from the host once that is sorted out",
                        channel.name,
                        status::chain(&error)
                    ),
                },
            }
        }
    }

    /// Points the host's records of `machine`'s channel Desk sessions at the
    /// folders the machine answered, now that its channel's folder moved.
    /// Its other sessions of `channel` follow the channel's folder too.
    pub(super) async fn refit_desks(&self, machine: &str, channel: &ChannelRecord, desks: Vec<(String, PathBuf)>) {
        self.refit_sessions(machine, channel).await;
        if desks.is_empty() {
            return;
        }
        let (store, on) = (self.starter.store.clone(), machine.to_owned());
        let saved = blocking(move || -> Result<(), StoreError> {
            for (id, folder) in desks {
                let Some(mut record) = store.session(&id)? else {
                    continue;
                };
                if record.machine == on && record.folder != folder {
                    record.folder = folder;
                    store.put_session(&record)?;
                }
            }
            Ok(())
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::log::say!(
                "humatad2: couldn't save where {machine}'s Desk sessions work after the rename: {}",
                status::chain(&error)
            ),
            Err(error) => crate::log::say!(
                "humatad2: couldn't save where {machine}'s Desk sessions work after the rename: {}",
                error.message()
            ),
        }
    }

    /// D348: points the host's records of `machine`'s sessions of `channel`
    /// at the channel's folder, now that the machine renamed it. The host
    /// records them in its own layout, so each one still under another
    /// folder of the channels root moves to the channel's; the rest stay.
    async fn refit_sessions(&self, machine: &str, channel: &ChannelRecord) {
        let (store, layout, on, channel) =
            (self.starter.store.clone(), self.starter.layout.clone(), machine.to_owned(), channel.clone());
        let saved = blocking(move || -> Result<(), StoreError> {
            let new_path = layout.channel(&channel.folder);
            let mut olds = std::collections::BTreeSet::new();
            for record in store.sessions()? {
                if record.channel != channel.id || record.machine != on {
                    continue;
                }
                let Some((base, first)) = std::iter::once(layout.root()).chain(layout.older_roots()).find_map(|base| {
                    let first = record.folder.strip_prefix(&base).ok()?.components().next()?;
                    Some((base, first))
                }) else {
                    continue;
                };
                let old_path = base.join(first);
                if old_path != new_path {
                    olds.insert(old_path);
                }
            }
            for old_path in olds {
                let placement = Placement {
                    name: channel.name.clone(),
                    folder: channel.folder.clone(),
                    old_path,
                    new_path: new_path.clone(),
                    now_unix_ms: now_ms(),
                };
                store.move_machine_sessions(&channel.id, &on, &placement)?;
            }
            Ok(())
        })
        .await;
        match saved {
            Ok(Ok(())) => {}
            Ok(Err(error)) => crate::log::say!(
                "humatad2: couldn't save where {machine}'s sessions work after the rename: {}; rename the channel again from the host to retry",
                status::chain(&error)
            ),
            Err(error) => crate::log::say!(
                "humatad2: couldn't save where {machine}'s sessions work after the rename: {}; rename the channel again from the host to retry",
                error.message()
            ),
        }
    }
}
