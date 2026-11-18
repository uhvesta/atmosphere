//! GetGrid (CH-9, TP-1): the Repos and machines grid of a channel, read from
//! the host's store. One cell per repo and machine of the channel, repos in
//! the channel's order and, within each, machines in the channel's order.
//! A cell counts the live sessions on that machine that have a clone of that
//! repo; archived sessions and deleted Desk sessions count nothing. Its state
//! is the repo's main clone as the channel records it (CH-19), or
//! MACHINE_OFFLINE when the host can't reach the machine now (D764).

use std::collections::HashSet;

use humata2_access::{Capability, ChannelId, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, MainClone, Store, StoreError};
use tonic::Status;

use super::{ChannelSvc, Gated, blocking};
use crate::identity::Caller;
use crate::status;

/// A live session of the channel: the machine it is on and the repos it has
/// clones of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Placed {
    pub(super) machine: String,
    pub(super) repos: Vec<(String, String)>,
}

/// The grid of `channel`: `sessions` are its live sessions, `offline` the
/// machines the host can't reach now.
pub(super) fn grid(channel: &ChannelRecord, sessions: &[Placed], offline: &HashSet<String>) -> pb::ChannelGrid {
    let mut cells = Vec::with_capacity(channel.repos.len() * channel.machines.len());
    for repo in &channel.repos {
        for machine in &channel.machines {
            let count = sessions
                .iter()
                .filter(|session| &session.machine == machine)
                .filter(|session| {
                    session
                        .repos
                        .iter()
                        .any(|(owner, name)| owner == &repo.owner && name == &repo.name)
                })
                .count();
            let state = if offline.contains(machine) {
                pb::grid_cell::State::MachineOffline
            } else {
                match repo.main {
                    MainClone::Cloning => pb::grid_cell::State::Cloning,
                    MainClone::Ready => pb::grid_cell::State::Ready,
                    MainClone::Failed { .. } => pb::grid_cell::State::Failed,
                }
            };
            cells.push(pb::GridCell {
                repo: Some(pb::RepoRef {
                    owner: repo.owner.clone(),
                    name: repo.name.clone(),
                }),
                machine: Some(pb::MachineId { value: machine.clone() }),
                state: state as i32,
                sessions: u32::try_from(count).unwrap_or(u32::MAX),
            });
        }
    }
    pb::ChannelGrid {
        channel: Some(pb::ChannelId {
            value: channel.id.clone(),
        }),
        cells,
    }
}

/// The live sessions of `channel` in `store`, a session with no machine
/// placed on `this_machine`. Blocking.
fn placed(store: &Store, channel: &str, this_machine: &str) -> Result<Vec<Placed>, StoreError> {
    let gone: HashSet<String> = store
        .archives()?
        .into_iter()
        .map(|row| row.session_id)
        .chain(
            store
                .desks()?
                .into_iter()
                .filter(|row| row.deleted_unix_ms.is_some())
                .map(|row| row.session_id),
        )
        .collect();
    let mut placed = Vec::new();
    for session in store.sessions()? {
        if session.channel != channel || gone.contains(&session.id) {
            continue;
        }
        let repos = store
            .session_detail(&session.id)?
            .into_iter()
            .flat_map(|detail| detail.repos)
            .map(|repo| (repo.owner, repo.name))
            .collect();
        let machine = if session.machine.is_empty() {
            this_machine.to_owned()
        } else {
            session.machine
        };
        placed.push(Placed { machine, repos });
    }
    Ok(placed)
}

impl ChannelSvc {
    /// The body of GetGrid; `approved` once the user said yes to it.
    pub(super) async fn grid_of(
        &self,
        caller: &Caller,
        request: pb::GetGridRequest,
        approved: bool,
    ) -> Result<Gated<pb::ChannelGrid>, Status> {
        let id = request.channel.map(|id| id.value).filter(|id| !id.is_empty());
        let id = id.ok_or_else(|| status::missing("channel"))?;
        if let Some(asked) = self.gate(
            caller,
            Capability::ChannelRead,
            Target::Channel(ChannelId::new(id.as_str())),
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        let channel = self.find(&id).await?;
        let (store, key, machine) = (self.starter.store.clone(), id.clone(), self.machine.clone());
        let sessions = blocking(move || placed(&store, &key, &machine))
            .await?
            .map_err(|error| status::store(error, &self.machine, "read the channel's sessions"))?;
        let (known, machines, this) = (self.known.clone(), channel.machines.clone(), self.machine.clone());
        let offline = blocking(move || offline(known.as_deref(), &machines, &this)).await?;
        Ok(Gated::Done(grid(&channel, &sessions, &offline)))
    }
}

/// The machines of `machines` the host can't reach now: every one but this
/// machine whose last seen reachability isn't ONLINE. A daemon with no
/// machine list (a machine's own) reaches none but itself. Blocking: reads
/// the machine list.
fn offline(known: Option<&crate::machine_service::Work>, machines: &[String], this_machine: &str) -> HashSet<String> {
    machines
        .iter()
        .filter(|machine| machine.as_str() != this_machine)
        .filter(|machine| {
            let Some(work) = known else {
                return true;
            };
            match work.machines.get(machine) {
                Ok(entry) => work.machine(&entry).reachability != pb::Reachability::Online as i32,
                Err(_) => true,
            }
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_store::ChannelRepo;

    fn channel(repos: &[(&str, MainClone)], machines: &[&str]) -> ChannelRecord {
        ChannelRecord {
            id: "ch_1".to_owned(),
            name: "Payments".to_owned(),
            folder: "payments".to_owned(),
            branch_prefix: None,
            default_agent: "claude".to_owned(),
            repos: repos
                .iter()
                .map(|(name, main)| ChannelRepo {
                    owner: "acme".to_owned(),
                    name: (*name).to_owned(),
                    main: main.clone(),
                })
                .collect(),
            machines: machines.iter().map(|m| (*m).to_owned()).collect(),
            revision: 1,
            created_unix_ms: 0,
        }
    }

    fn on(machine: &str, repos: &[&str]) -> Placed {
        Placed {
            machine: machine.to_owned(),
            repos: repos.iter().map(|name| ("acme".to_owned(), (*name).to_owned())).collect(),
        }
    }

    /// (repo name, machine, state, sessions) of each cell, in order.
    fn shown(grid: &pb::ChannelGrid) -> Vec<(String, String, pb::grid_cell::State, u32)> {
        grid.cells
            .iter()
            .map(|cell| {
                (
                    cell.repo.clone().unwrap().name,
                    cell.machine.clone().unwrap().value,
                    cell.state(),
                    cell.sessions,
                )
            })
            .collect()
    }

    #[test]
    fn each_cell_counts_the_sessions_with_a_clone_of_its_repo_on_its_machine() {
        let channel = channel(&[("api", MainClone::Ready), ("web", MainClone::Ready)], &["mac", "devbox"]);
        let sessions = [on("mac", &["api", "web"]), on("mac", &["api"]), on("devbox", &["web"])];

        let grid = grid(&channel, &sessions, &HashSet::new());

        use pb::grid_cell::State::Ready;
        assert_eq!(grid.channel.as_ref().unwrap().value, "ch_1");
        assert_eq!(
            shown(&grid),
            [
                ("api".to_owned(), "mac".to_owned(), Ready, 2),
                ("api".to_owned(), "devbox".to_owned(), Ready, 0),
                ("web".to_owned(), "mac".to_owned(), Ready, 1),
                ("web".to_owned(), "devbox".to_owned(), Ready, 1),
            ]
        );
    }

    #[test]
    fn a_cell_shows_its_repos_main_clone_or_that_its_machine_is_offline() {
        let channel = channel(
            &[
                ("api", MainClone::Cloning),
                ("web", MainClone::Failed { reason: "no access".to_owned() }),
            ],
            &["mac", "gpu"],
        );
        let offline = HashSet::from(["gpu".to_owned()]);

        let grid = grid(&channel, &[on("gpu", &["api"])], &offline);

        use pb::grid_cell::State::{Cloning, Failed, MachineOffline};
        assert_eq!(
            shown(&grid),
            [
                ("api".to_owned(), "mac".to_owned(), Cloning, 0),
                ("api".to_owned(), "gpu".to_owned(), MachineOffline, 1),
                ("web".to_owned(), "mac".to_owned(), Failed, 0),
                ("web".to_owned(), "gpu".to_owned(), MachineOffline, 0),
            ]
        );
    }

    #[test]
    fn sessions_on_machines_or_repos_the_channel_no_longer_has_count_nothing() {
        let channel = channel(&[("api", MainClone::Ready)], &["mac"]);

        let grid = grid(&channel, &[on("old-box", &["api"]), on("mac", &["gone"])], &HashSet::new());

        assert_eq!(shown(&grid), [("api".to_owned(), "mac".to_owned(), pb::grid_cell::State::Ready, 0)]);
    }

    #[test]
    fn a_daemon_with_no_machine_list_reaches_only_itself() {
        let machines = ["mac".to_owned(), "devbox".to_owned()];

        assert_eq!(offline(None, &machines, "mac"), HashSet::from(["devbox".to_owned()]));
    }
}
