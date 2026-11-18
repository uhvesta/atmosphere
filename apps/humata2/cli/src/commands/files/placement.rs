//! `humata cp|mv <location>… <machine>` (PL-12): a send that names only the
//! machine lands where PL-3 says, as the Files page's Copy to and Move to
//! do. A channel's shared folder lands in that channel's shared folder; a
//! session's clone or folder lands in its channel's shared folder, under a
//! folder named for the session; both only on a machine of that channel.
//! Anything else lands in the machine's received place for the source
//! machine (PL-4).

use humata2_proto::humata::v1 as pb;
use pb::file_location::Root;

use crate::call::Ctx;
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

/// What the daemon knows that PL-3 reads: every session, and who is calling.
pub(super) struct Known {
    sessions: Vec<pb::Session>,
    me: Option<pb::AgentActor>,
    /// Each channel's machines, as read the first time a source of it
    /// asked, so every source of one channel is placed by the same answer.
    channels: std::sync::Mutex<Vec<(String, Vec<String>)>>,
}

impl Known {
    pub(super) async fn fetch<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<Self, CliError> {
        let mut sessions = Vec::new();
        let mut page = String::new();
        loop {
            let reply: pb::ListSessionsResponse = ctx
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
            sessions.extend(reply.sessions);
            page = reply.page.map(|page| page.next_page_token).unwrap_or_default();
            if page.is_empty() {
                break;
            }
        }
        let who: pb::WhoAmIResponse = ctx.call(rpc::daemon::WHO_AM_I, &pb::WhoAmIRequest {}).await?;
        let me = match who.actor.and_then(|actor| actor.actor) {
            Some(pb::actor::Actor::Agent(agent)) => Some(agent),
            _ => None,
        };
        Ok(Self {
            sessions,
            me,
            channels: std::sync::Mutex::new(Vec::new()),
        })
    }

    fn session(&self, id: &pb::SessionId) -> Option<&pb::Session> {
        self.sessions
            .iter()
            .find(|session| session.id.as_ref().is_some_and(|found| found.value == id.value))
    }
}

/// Whether `target` is one of `channel`'s machines: one the channel lists,
/// or one a live session of the channel runs on.
async fn in_channel<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    known: &Known,
    channel: &pb::ChannelId,
    target: &pb::MachineId,
) -> Result<bool, CliError> {
    let runs_there = known.sessions.iter().any(|session| {
        session.archive.is_none()
            && session.channel.as_ref().is_some_and(|found| found.value == channel.value)
            && session.machine.as_ref().is_some_and(|found| found.value == target.value)
    });
    if runs_there {
        return Ok(true);
    }
    let cached = |channels: &std::sync::Mutex<Vec<(String, Vec<String>)>>| {
        channels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .find(|(id, _)| *id == channel.value)
            .map(|(_, machines)| machines.contains(&target.value))
    };
    if let Some(found) = cached(&known.channels) {
        return Ok(found);
    }
    let found: pb::Channel = ctx
        .call(
            rpc::channel::GET_CHANNEL,
            &pb::GetChannelRequest {
                id: Some(channel.clone()),
            },
        )
        .await?;
    let machines: Vec<String> = found.machines.into_iter().map(|machine| machine.value).collect();
    let on = machines.contains(&target.value);
    known
        .channels
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((channel.value.clone(), machines));
    Ok(on)
}

/// Where `source` lands when it is sent to `target` by machine only.
pub(super) async fn landing<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    known: &Known,
    source: &pb::FileLocation,
    target: &pb::MachineId,
) -> Result<pb::FileLocation, CliError> {
    let shared = |channel: pb::ChannelId, path: String| pb::FileLocation {
        machine: Some(target.clone()),
        root: Some(Root::ChannelShared(pb::ChannelShared {
            channel: Some(channel),
        })),
        path,
    };
    match &source.root {
        Some(Root::ChannelShared(pb::ChannelShared {
            channel: Some(channel),
        })) => {
            if in_channel(ctx, known, channel, target).await? {
                return Ok(shared(channel.clone(), String::new()));
            }
        }
        Some(
            Root::SessionClone(pb::SessionClone { session, .. })
            | Root::SessionFolder(pb::SessionFolder { session }),
        ) => {
            // A bare path inside a session is the agent's own clone.
            let id = session
                .clone()
                .or_else(|| known.me.as_ref().and_then(|me| me.session.clone()));
            if let Some(record) = id.as_ref().and_then(|id| known.session(id))
                && let Some(channel) = record.channel.clone()
                && in_channel(ctx, known, &channel, target).await?
            {
                return Ok(shared(channel, session_slug(record)));
            }
        }
        _ => {}
    }
    let from = source
        .machine
        .clone()
        .or_else(|| known.me.as_ref().and_then(|me| me.machine.clone()))
        .ok_or(CliError::Missing {
            flag: "the source's machine: write it as <machine>:<location>",
        })?;
    Ok(pb::FileLocation {
        machine: Some(target.clone()),
        root: Some(Root::Received(pb::MachineReceived { from: Some(from) })),
        path: String::new(),
    })
}

/// The folder a session's files get under its channel's shared folder: the
/// last part of the session's own folder, so a renamed session keeps one
/// folder; else its title in lower case, every run of other characters one
/// `-`; else its id. The Files page names it the same way.
fn session_slug(session: &pb::Session) -> String {
    if let Some(name) = session
        .folder
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
    {
        return name.to_owned();
    }
    let mut slug = String::new();
    for c in session.title.chars() {
        if c.is_alphanumeric() {
            slug.extend(c.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        session.id.as_ref().map(|id| id.value.clone()).unwrap_or_default()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(folder: &str, title: &str) -> pb::Session {
        pb::Session {
            id: Some(pb::SessionId { value: "se_9".into() }),
            folder: folder.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_session_with_a_folder_is_named_for_its_folder() {
        assert_eq!(session_slug(&session("/h/payments/fix-login/", "Renamed")), "fix-login");
    }

    #[test]
    fn a_session_with_no_folder_is_named_for_its_title() {
        assert_eq!(session_slug(&session("", "Fix the Login!")), "fix-the-login");
    }

    #[test]
    fn a_session_whose_title_has_nothing_to_use_is_named_for_its_id() {
        assert_eq!(session_slug(&session("", "—")), "se_9");
    }
}
