//! `session_start` (D910): an agent starts a session in a channel, by
//! default its own. A Desk session belongs to no channel, so its agent
//! names one. The daemon's StartSession runs the access check and asks the
//! user first when the rule says so.

use crate::pb;
use humata2_mcp::{Call, ErrorKind, Reply, ToolError};
use serde_json::{Map, Value, json};

use super::session_repo::parse_repo;
use super::{GrpcDaemon, applied, key, text};

/// The channel id every Desk session carries (DK-2); it is no channel.
const DESK_CHANNEL: &str = "desk";

/// The channel a start goes to: the one named, else the caller's own.
fn start_channel(named: &str, own: Option<&pb::ChannelId>) -> Result<pb::ChannelId, ToolError> {
    if !named.trim().is_empty() {
        return Ok(pb::ChannelId {
            value: named.trim().to_owned(),
        });
    }
    match own.filter(|own| !own.value.is_empty() && own.value != DESK_CHANNEL) {
        Some(own) => Ok(own.clone()),
        None => {
            let mut details = Map::new();
            details.insert("field".to_owned(), json!("channel"));
            details.insert("problem".to_owned(), json!("missing"));
            Err(ToolError {
                kind: ErrorKind::InvalidField,
                message: "this session belongs to no channel, so session_start can't default to one"
                    .to_owned(),
                fix: "send channel with a channel id from channels_list".to_owned(),
                retry: None,
                details,
            })
        }
    }
}

/// The repos a start names, each included; none means the channel's
/// default repos.
fn start_repos(arguments: &Map<String, Value>) -> Result<Vec<pb::RepoChoice>, ToolError> {
    arguments
        .get("repos")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|repo| {
            parse_repo(repo).map(|repo| pb::RepoChoice {
                repo: Some(repo),
                include: true,
            })
        })
        .collect()
}

/// The StartSession request a `session_start` call makes.
fn start_request(
    call: &Call,
    own_channel: Option<&pb::ChannelId>,
) -> Result<pb::StartSessionRequest, ToolError> {
    let arguments = &call.arguments;
    let channel = start_channel(&text(arguments, "channel"), own_channel)?;
    let repos = start_repos(arguments)?;
    let machine = text(arguments, "machine");
    let agent = text(arguments, "agent");
    Ok(pb::StartSessionRequest {
        idempotency_key: Some(key(call)),
        channel: Some(channel),
        title: text(arguments, "title"),
        agent: (!agent.is_empty()).then_some(pb::AgentId { value: agent }),
        model: text(arguments, "model"),
        machine: (!machine.is_empty()).then_some(pb::MachineId { value: machine }),
        repos,
        first_message: text(arguments, "first_message"),
        ..Default::default()
    })
}

impl GrpcDaemon {
    /// `session_start`.
    pub(super) async fn session_start(&self, call: Call) -> Result<Reply, ToolError> {
        let (_, own_channel) = self.agent().await?;
        let request = self.request(start_request(&call, own_channel.as_ref())?)?;
        let started = self
            .sessions()
            .start_session(request)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let operation = started.operation.and_then(|operation| operation.id);
        Ok(applied(
            json!({
                "session": started.session.map(|id| id.value),
                "operation": operation.map(|id| id.value),
            }),
            None,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(value: &str) -> pb::ChannelId {
        pb::ChannelId {
            value: value.to_owned(),
        }
    }

    fn arguments(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn a_named_channel_is_the_start_channel() {
        let chosen = start_channel("payments", Some(&channel("search"))).unwrap();
        assert_eq!(chosen, channel("payments"));
        let from_desk = start_channel("payments", Some(&channel(DESK_CHANNEL))).unwrap();
        assert_eq!(from_desk, channel("payments"));
    }

    #[test]
    fn with_no_channel_named_a_channel_session_starts_in_its_own() {
        let chosen = start_channel("", Some(&channel("search"))).unwrap();
        assert_eq!(chosen, channel("search"));
    }

    #[test]
    fn a_desk_session_must_name_a_channel() {
        for own in [Some(channel(DESK_CHANNEL)), Some(channel("")), None] {
            let refused = start_channel("", own.as_ref()).unwrap_err();
            assert_eq!(refused.kind, ErrorKind::InvalidField, "{own:?}");
        }
    }

    #[test]
    fn repos_are_each_included_and_a_bad_one_is_refused() {
        let repos = start_repos(&arguments(json!({ "repos": ["acme/api", "acme/web"] }))).unwrap();
        assert_eq!(repos.len(), 2);
        assert!(repos.iter().all(|choice| choice.include));
        assert!(start_repos(&arguments(json!({}))).unwrap().is_empty());
        assert!(start_repos(&arguments(json!({ "repos": ["acme"] }))).is_err());
    }
}
