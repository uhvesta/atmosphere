//! `topology_get` (TP-1, D772): the agent's channel's machines, this one
//! first, then the channel's in its order, then any other machine one of its
//! sessions is on; each with whether the host can reach it and the channel's
//! live sessions there. WhoAmI names this machine and the channel,
//! GetChannel its machines, ListSessions the sessions, and GetGrid the
//! reachability: all are CHANNEL_READ on the agent's own channel, so the tool
//! needs no machine-list access the agent doesn't have.

use crate::pb;
use humata2_mcp::{Reply, ToolError};
use serde_json::{Value, json};

use super::{GrpcDaemon, applied, lower};

/// `topology_get` stops after this many pages of sessions, so a token that
/// never ends can't keep it asking.
const SESSION_PAGES_MAX: usize = 100;

impl GrpcDaemon {
    /// `topology_get`.
    pub(super) async fn topology_get(&self) -> Result<Reply, ToolError> {
        let (agent, channel) = self.agent().await?;
        let here = agent.machine.map(|id| id.value).unwrap_or_default();
        let channel = channel.unwrap_or_default();
        let about = self
            .channels()
            .get_channel(self.request(pb::GetChannelRequest {
                id: Some(channel.clone()),
            })?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let grid = self
            .channels()
            .get_grid(self.request(pb::GetGridRequest {
                channel: Some(channel.clone()),
            })?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let mut sessions = Vec::new();
        let mut page_token = String::new();
        for _ in 0..SESSION_PAGES_MAX {
            let listed = self
                .sessions()
                .list_sessions(self.request(pb::ListSessionsRequest {
                    channel: Some(channel.clone()),
                    machine: None,
                    include_archived: false,
                    page: Some(pb::PageRequest { page_size: 0, page_token }),
                })?)
                .await
                .map_err(|status| self.failed(&status))?
                .into_inner();
            sessions.extend(listed.sessions);
            page_token = listed.page.map(|page| page.next_page_token).unwrap_or_default();
            if page_token.is_empty() {
                break;
            }
        }
        Ok(applied(topology_json(&here, &about, &grid, &sessions), None))
    }
}

/// The tool's result from what the daemon answered.
fn topology_json(here: &str, channel: &pb::Channel, grid: &pb::ChannelGrid, sessions: &[pb::Session]) -> Value {
    let mut order: Vec<&str> = Vec::new();
    let listed = channel.machines.iter().map(|id| id.value.as_str());
    let placed = sessions.iter().filter_map(|session| session.machine.as_ref()).map(|id| id.value.as_str());
    for machine in std::iter::once(here).chain(listed).chain(placed) {
        if !machine.is_empty() && !order.contains(&machine) {
            order.push(machine);
        }
    }
    let machines: Vec<Value> = order
        .into_iter()
        .map(|machine| {
            let on: Vec<Value> = sessions
                .iter()
                .filter(|session| session.machine.as_ref().is_some_and(|id| id.value == machine))
                .map(|session| {
                    json!({
                        "id": session.id.as_ref().map(|id| id.value.clone()),
                        "title": session.title,
                        "agent": session.agent.as_ref().map(|id| id.value.clone()),
                        "status": lower(session.status().as_str_name(), "SESSION_STATUS_"),
                    })
                })
                .collect();
            json!({
                "id": machine,
                "name": machine,
                "reachability": reachability(machine, here, grid),
                "sessions": on,
            })
        })
        .collect();
    json!({ "this_machine": here, "machines": machines })
}

/// `online` for this machine. Another is `offline` when the grid marks it
/// MACHINE_OFFLINE, `online` when its cells show a clone state, and
/// `unknown` when it has no cells (a channel with no repos, or a machine
/// the channel doesn't list).
fn reachability(machine: &str, here: &str, grid: &pb::ChannelGrid) -> &'static str {
    if machine == here {
        return "online";
    }
    let mut cells = grid
        .cells
        .iter()
        .filter(|cell| cell.machine.as_ref().is_some_and(|id| id.value == machine))
        .peekable();
    if cells.peek().is_none() {
        "unknown"
    } else if cells.any(|cell| cell.state() == pb::grid_cell::State::MachineOffline) {
        "offline"
    } else {
        "online"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, machine: &str) -> pb::Session {
        pb::Session {
            id: Some(pb::SessionId { value: id.to_owned() }),
            title: id.to_owned(),
            machine: Some(pb::MachineId { value: machine.to_owned() }),
            ..pb::Session::default()
        }
    }

    fn channel(machines: &[&str]) -> pb::Channel {
        pb::Channel {
            machines: machines
                .iter()
                .map(|machine| pb::MachineId { value: (*machine).to_owned() })
                .collect(),
            ..pb::Channel::default()
        }
    }

    fn cell(machine: &str, state: pb::grid_cell::State) -> pb::GridCell {
        pb::GridCell {
            machine: Some(pb::MachineId { value: machine.to_owned() }),
            state: state as i32,
            ..pb::GridCell::default()
        }
    }

    /// (id, reachability, session ids) of each machine, in order.
    fn shown(topology: &Value) -> Vec<(String, String, Vec<String>)> {
        topology["machines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|machine| {
                (
                    machine["id"].as_str().unwrap().to_owned(),
                    machine["reachability"].as_str().unwrap().to_owned(),
                    machine["sessions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|session| session["id"].as_str().unwrap().to_owned())
                        .collect(),
                )
            })
            .collect()
    }

    fn owned(id: &str, reach: &str, sessions: &[&str]) -> (String, String, Vec<String>) {
        (id.to_owned(), reach.to_owned(), sessions.iter().map(|s| (*s).to_owned()).collect())
    }

    #[test]
    fn this_machine_comes_first_then_the_channels_then_any_other_a_session_is_on() {
        let sessions = [session("se_1", "devbox"), session("se_2", "mac"), session("se_3", "old-box")];
        let grid = pb::ChannelGrid {
            cells: vec![cell("mac", pb::grid_cell::State::Ready), cell("devbox", pb::grid_cell::State::Ready)],
            ..pb::ChannelGrid::default()
        };

        let topology = topology_json("mac", &channel(&["devbox", "mac"]), &grid, &sessions);

        assert_eq!(topology["this_machine"], "mac");
        assert_eq!(
            shown(&topology),
            [
                owned("mac", "online", &["se_2"]),
                owned("devbox", "online", &["se_1"]),
                owned("old-box", "unknown", &["se_3"]),
            ]
        );
    }

    #[test]
    fn a_machine_the_grid_marks_offline_is_offline_and_one_with_no_cells_is_unknown() {
        let grid = pb::ChannelGrid {
            cells: vec![
                cell("gpu", pb::grid_cell::State::MachineOffline),
                cell("devbox", pb::grid_cell::State::Cloning),
            ],
            ..pb::ChannelGrid::default()
        };

        let topology = topology_json("mac", &channel(&["mac", "gpu", "devbox", "spare"]), &grid, &[]);

        assert_eq!(
            shown(&topology),
            [
                owned("mac", "online", &[]),
                owned("gpu", "offline", &[]),
                owned("devbox", "online", &[]),
                owned("spare", "unknown", &[]),
            ]
        );
    }
}
