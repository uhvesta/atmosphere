//! Channels, sessions and machines by name or id (CLI.md §4). An id is
//! used as it is; a name is looked up and must match exactly one, ignoring
//! case.

use humata2_proto::humata::v1 as pb;

use crate::call::Ctx;
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

/// Prefixes that mark an id rather than a name (provisional: the API
/// doesn't fix id formats).
const CHANNEL_ID: &str = "ch_";
const SESSION_ID: &str = "se_";
const MACHINE_ID: &str = "ma_";

impl<D: Daemon, C: Console> Ctx<'_, D, C> {
    pub async fn channel(&self, name: &str) -> Result<pb::ChannelId, CliError> {
        if name.starts_with(CHANNEL_ID) {
            return Ok(pb::ChannelId { value: name.into() });
        }
        let mut found = Vec::new();
        let mut page = String::new();
        loop {
            let reply: pb::ListChannelsResponse = self
                .call(
                    rpc::channel::LIST_CHANNELS,
                    &pb::ListChannelsRequest {
                        page: Some(pb::PageRequest {
                            page_size: 0,
                            page_token: page.clone(),
                        }),
                    },
                )
                .await?;
            for channel in reply.channels {
                let id = channel.id.map(|id| id.value).unwrap_or_default();
                found.push((id, channel.name));
            }
            page = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if page.is_empty() {
                break;
            }
        }
        pick("channel", name, found).map(|value| pb::ChannelId { value })
    }

    pub async fn session(&self, name: &str) -> Result<pb::SessionId, CliError> {
        if name.starts_with(SESSION_ID) {
            return Ok(pb::SessionId { value: name.into() });
        }
        let mut found = Vec::new();
        let mut page = String::new();
        loop {
            let reply: pb::ListSessionsResponse = self
                .call(
                    rpc::session::LIST_SESSIONS,
                    &pb::ListSessionsRequest {
                        include_archived: true,
                        page: Some(pb::PageRequest {
                            page_size: 0,
                            page_token: page.clone(),
                        }),
                        ..Default::default()
                    },
                )
                .await?;
            for session in reply.sessions {
                let id = session.id.map(|id| id.value).unwrap_or_default();
                found.push((id, session.title));
            }
            page = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if page.is_empty() {
                break;
            }
        }
        pick("session", name, found).map(|value| pb::SessionId { value })
    }

    pub async fn machine(&self, name: &str) -> Result<pb::MachineId, CliError> {
        if name.starts_with(MACHINE_ID) {
            return Ok(pb::MachineId { value: name.into() });
        }
        let mut found = Vec::new();
        let mut page = String::new();
        loop {
            let reply: pb::ListMachinesResponse = self
                .call(
                    rpc::machine::LIST_MACHINES,
                    &pb::ListMachinesRequest {
                        page: Some(pb::PageRequest {
                            page_size: 0,
                            page_token: page.clone(),
                        }),
                    },
                )
                .await?;
            for machine in reply.machines {
                let id = machine.id.map(|id| id.value).unwrap_or_default();
                found.push((id, machine.name));
            }
            page = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if page.is_empty() {
                break;
            }
        }
        pick("machine", name, found).map(|value| pb::MachineId { value })
    }

    /// `--channel`, when given.
    pub async fn channel_flag(&self) -> Result<Option<pb::ChannelId>, CliError> {
        match &self.flags.channel {
            Some(name) => Ok(Some(self.channel(name).await?)),
            None => Ok(None),
        }
    }

    /// `--channel`, or the agent's own channel, or a usage error.
    pub async fn this_channel(&self) -> Result<pb::ChannelId, CliError> {
        if let Some(channel) = self.channel_flag().await? {
            return Ok(channel);
        }
        let me: pb::WhoAmIResponse = self
            .call(rpc::daemon::WHO_AM_I, &pb::WhoAmIRequest {})
            .await?;
        me.channel
            .filter(|channel| !channel.value.is_empty())
            .ok_or(CliError::Missing {
                flag: "--channel <name|id>",
            })
    }

    /// The first `--machine`, when given.
    pub async fn machine_flag(&self) -> Result<Option<pb::MachineId>, CliError> {
        match self.flags.machine.first() {
            Some(name) => Ok(Some(self.machine(name).await?)),
            None => Ok(None),
        }
    }

    pub async fn needs_machine(&self) -> Result<pb::MachineId, CliError> {
        self.machine_flag().await?.ok_or(CliError::Missing {
            flag: "--machine <name|id>",
        })
    }

    /// Every `--machine`.
    pub async fn machines_flag(&self) -> Result<Vec<pb::MachineId>, CliError> {
        let mut machines = Vec::new();
        for name in &self.flags.machine {
            machines.push(self.machine(name).await?);
        }
        Ok(machines)
    }
}

/// The one id whose id or name is `name`.
fn pick(what: &'static str, name: &str, found: Vec<(String, String)>) -> Result<String, CliError> {
    if let Some((id, _)) = found.iter().find(|(id, _)| id == name) {
        return Ok(id.clone());
    }
    let mut matches: Vec<(String, String)> = found
        .into_iter()
        .filter(|(_, title)| title.eq_ignore_ascii_case(name))
        .collect();
    match matches.len() {
        0 => Err(CliError::NoMatch {
            what,
            name: name.into(),
        }),
        1 => Ok(matches.remove(0).0),
        _ => Err(CliError::Ambiguous {
            what,
            name: name.into(),
            matches: matches
                .into_iter()
                .map(|(id, title)| format!("{title} ({id})"))
                .collect(),
        }),
    }
}

/// `owner/name` as a repo.
pub fn repo(text: &str) -> Result<pb::RepoRef, CliError> {
    match text.split_once('/') {
        Some((owner, name)) if !owner.is_empty() && !name.is_empty() && !name.contains('/') => {
            Ok(pb::RepoRef {
                owner: owner.into(),
                name: name.into(),
            })
        }
        _ => Err(CliError::BadValue {
            field: "repo",
            value: text.into(),
            expected: "owner/name",
        }),
    }
}

/// A capability by its snake-case name, as the token example writes it.
pub fn capability(text: &str) -> Result<pb::Capability, CliError> {
    let name = format!("CAPABILITY_{}", text.to_ascii_uppercase().replace('-', "_"));
    pb::Capability::from_str_name(&name)
        .filter(|capability| *capability != pb::Capability::Unspecified)
        .ok_or(CliError::BadValue {
            field: "capability",
            value: text.into(),
            expected: "a capability such as channel_board_add (see `humata access`)",
        })
}
