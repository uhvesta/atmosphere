//! `sessions_delete_many` (MASSDELETE, D876): one plan for the sessions an
//! agent names, one channel or one machine, then one delete with that plan.
//! The daemon makes every decision (its gate, the stale-plan check, never
//! the agent's own session, never all); this side carries the plan and
//! answers back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_mcp::{Call, ErrorKind, Reply, SessionToken, ToolError};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;
use tonic::metadata::{AsciiMetadataValue, MetadataValue};
use tonic::transport::Channel;
use tonic::{Request, Streaming};

use crate::ports::TOKEN_HEADER;

use super::{GrpcDaemon, applied, key, lock, text};

/// How often the watcher re-checks `AccessService.Explain` when no access
/// event arrived on the stream.
const RULE_POLL_INTERVAL: Duration = Duration::from_secs(5);

impl GrpcDaemon {
    /// `sessions_delete_many`: plans the scope, then deletes with that plan.
    /// Under an Ask rule the daemon answers approval-pending, which the call
    /// turns into a pending reply like any other.
    pub(super) async fn sessions_delete_many(&self, call: Call) -> Result<Reply, ToolError> {
        let scope = delete_scope(&call.arguments)?;
        let delete_work = delete_unpushed_work(&call.arguments);
        let plan = self
            .sessions()
            .plan_delete_sessions(self.request(pb::PlanDeleteSessionsRequest {
                scope: Some(pb::DeleteScope { scope: Some(scope) }),
            })?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        let response = self
            .sessions()
            .delete_sessions(self.request(pb::DeleteSessionsRequest {
                idempotency_key: Some(key(&call)),
                plan: plan.token.clone(),
                delete_unpushed_work: delete_work,
                typed_count: 0,
                check_only: false,
            })?)
            .await
            .map_err(|status| self.failed(&status))?
            .into_inner();
        match response.result {
            Some(pb::delete_sessions_response::Result::Deleted(done)) => Ok(applied(
                json!({
                    "deleted": done.sessions,
                    "trash_items": done
                        .trash_items
                        .into_iter()
                        .map(|item| item.value)
                        .collect::<Vec<_>>(),
                    "operation": done.operation.and_then(|operation| operation.id).map(|id| id.value),
                    "plan": delete_plan_json(&plan),
                }),
                None,
            )),
            Some(pb::delete_sessions_response::Result::Changed(changed)) => {
                Err(plan_changed(changed))
            }
            None => Err(ToolError {
                kind: ErrorKind::Internal,
                message: "the daemon answered neither a deletion nor the changed sessions, so it isn't known what was deleted".to_owned(),
                fix: "list the sessions with topology_get to see which are left, then call sessions_delete_many again if they still need deleting".to_owned(),
                retry: None,
                details: Map::new(),
            }),
        }
    }

    /// Keeps the caller's rule for `sessions_delete_many` as the daemon last
    /// gave it: the tool is listed only while the rule is Allow or Ask
    /// (D876). Anything else hides it, and a flip notifies the MCP server.
    #[cfg(test)]
    pub(super) fn learn_delete_many_rule(&self, decision: pb::Decision) -> bool {
        apply_delete_many_rule(
            &self.delete_many_offered,
            self.tool_list_changes.as_ref(),
            decision,
        )
    }

    /// Whether `tools/list` offers the tool, as last learned.
    pub(super) fn delete_many_offered(&self) -> bool {
        self.delete_many_offered.load(Ordering::Relaxed)
    }

    /// Asks the daemon now for the caller's rule (the agent itself, no
    /// target) and keeps it. A failed ask leaves the tool unlisted; the
    /// daemon refuses any call its rule doesn't allow anyway.
    pub(super) async fn ask_delete_many_rule(&self) {
        let explained = match self.request(pb::ExplainRequest {
            session: None,
            capability: pb::Capability::SessionsDeleteMany as i32,
            target: None,
        }) {
            Ok(request) => self
                .access()
                .explain(request)
                .await
                .ok()
                .map(|explained| explained.into_inner().decision()),
            Err(_) => None,
        };
        apply_delete_many_rule(
            &self.delete_many_offered,
            None,
            explained.unwrap_or(pb::Decision::Never),
        );
    }

    /// Starts watching for `sessions_delete_many` rule changes once `bind`
    /// succeeds, notifying the MCP server whenever the offered state flips.
    pub(super) fn watch_delete_many_rule(&self, token: SessionToken) {
        let Some(changed) = self.tool_list_changes.clone() else {
            return;
        };
        let mut watcher = lock(&self.rule_watcher);
        if watcher.is_some() {
            return;
        }
        let channel = self.channel.clone();
        let bundle = self.bundle.clone();
        let offered = Arc::clone(&self.delete_many_offered);
        *watcher = Some(tokio::spawn(watch_rule_loop(
            channel, token, bundle, offered, changed,
        )));
    }
}

/// Updates `offered` from `decision` and signals `changed` when the offered
/// state flips. Returns `true` if it flipped.
fn apply_delete_many_rule(
    offered: &AtomicBool,
    changed: Option<&mpsc::UnboundedSender<()>>,
    decision: pb::Decision,
) -> bool {
    let listed = matches!(decision, pb::Decision::Allow | pb::Decision::Ask);
    let previous = offered.swap(listed, Ordering::Relaxed);
    if previous != listed {
        if let Some(tx) = changed {
            let _ = tx.send(());
        }
        true
    } else {
        false
    }
}

fn signed_request<T>(
    message: T,
    token: &SessionToken,
    bundle: Option<&AsciiMetadataValue>,
) -> Option<Request<T>> {
    let mut request = Request::new(message);
    let mut value = MetadataValue::try_from(token.expose()).ok()?;
    value.set_sensitive(true);
    request.metadata_mut().insert(TOKEN_HEADER, value);
    if let Some(bundle) = bundle {
        request
            .metadata_mut()
            .insert(humata2_daemon_ports::release::BUNDLE_HEADER, bundle.clone());
    }
    Some(request)
}

async fn subscribe_access_events(
    channel: &Channel,
    token: &SessionToken,
    bundle: Option<&AsciiMetadataValue>,
) -> Option<Streaming<pb::Event>> {
    let request = signed_request(
        pb::SubscribeRequest {
            topics: vec![pb::Topic::Access as i32],
            ..Default::default()
        },
        token,
        bundle,
    )?;
    pb::event_service_client::EventServiceClient::new(channel.clone())
        .subscribe(request)
        .await
        .ok()
        .map(tonic::Response::into_inner)
}

async fn explain_delete_many(
    channel: &Channel,
    token: &SessionToken,
    bundle: Option<&AsciiMetadataValue>,
) -> Option<pb::Decision> {
    let request = signed_request(
        pb::ExplainRequest {
            session: None,
            capability: pb::Capability::SessionsDeleteMany as i32,
            target: None,
        },
        token,
        bundle,
    )?;
    pb::access_service_client::AccessServiceClient::new(channel.clone())
        .explain(request)
        .await
        .ok()
        .map(|explained| explained.into_inner().decision())
}

async fn next_access_event(stream: &mut Option<Streaming<pb::Event>>) -> bool {
    match stream {
        Some(events) => match events.message().await {
            Ok(Some(_)) => true,
            Ok(None) | Err(_) => {
                *stream = None;
                false
            }
        },
        None => std::future::pending().await,
    }
}

async fn watch_rule_loop(
    channel: Channel,
    token: SessionToken,
    bundle: Option<AsciiMetadataValue>,
    offered: Arc<AtomicBool>,
    changed: mpsc::UnboundedSender<()>,
) {
    let mut interval = tokio::time::interval(RULE_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // `bind` just asked; wait for the first interval or an access event.
    interval.tick().await;
    let mut stream = subscribe_access_events(&channel, &token, bundle.as_ref()).await;
    loop {
        let wake = tokio::select! {
            () = changed.closed() => return,
            _ = interval.tick() => {
                if stream.is_none() {
                    stream = subscribe_access_events(&channel, &token, bundle.as_ref()).await;
                }
                true
            }
            got = next_access_event(&mut stream) => got,
        };
        if wake
            && let Some(decision) = explain_delete_many(&channel, &token, bundle.as_ref()).await
        {
            apply_delete_many_rule(&offered, Some(&changed), decision);
        }
    }
}

/// Whether the caller explicitly asked to delete uncommitted or unpushed git
/// work. Defaults to false (D876).
fn delete_unpushed_work(arguments: &Map<String, Value>) -> bool {
    arguments
        .get("delete_unpushed_work")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The scope the call names: picked session ids, one channel, or one
/// machine. Exactly one, which the schema checks too. No call can name all.
fn delete_scope(arguments: &Map<String, Value>) -> Result<pb::delete_scope::Scope, ToolError> {
    let ids = arguments.get("session_ids").and_then(Value::as_array);
    let channel = text(arguments, "channel");
    let machine = text(arguments, "machine");
    match (ids, channel.is_empty(), machine.is_empty()) {
        (Some(ids), true, true) => Ok(pb::delete_scope::Scope::Sessions(
            pb::delete_scope::Picked {
                ids: ids
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|value| pb::SessionId {
                        value: value.to_owned(),
                    })
                    .collect(),
            },
        )),
        (None, false, true) => Ok(pb::delete_scope::Scope::Channel(pb::ChannelId {
            value: channel,
        })),
        (None, true, false) => Ok(pb::delete_scope::Scope::Machine(pb::MachineId {
            value: machine,
        })),
        _ => Err(ToolError {
            kind: ErrorKind::InvalidField,
            message: "name exactly one scope for sessions_delete_many: session_ids, channel or machine".to_owned(),
            fix: "pass session_ids (session ids from topology_get), or a channel id, or a machine id; never more than one of them, and no scope covers every session".to_owned(),
            retry: None,
            details: Map::new(),
        }),
    }
}

/// The daemon's Changed answer: nothing was deleted. The agent gets the new
/// plan and the changes in words.
fn plan_changed(changed: pb::delete_sessions_response::Changed) -> ToolError {
    let changes = changed.changes.join("; ");
    let mut details = Map::new();
    details.insert("changes".to_owned(), json!(changed.changes));
    details.insert(
        "plan".to_owned(),
        changed
            .plan
            .as_ref()
            .map_or(Value::Null, delete_plan_json),
    );
    ToolError {
        kind: ErrorKind::PlanStale,
        message: format!(
            "nothing was deleted: the sessions changed since the plan was made: {changes}"
        ),
        fix: "details.plan is the current plan; call sessions_delete_many again to delete it, or ask the user if the changes matter".to_owned(),
        retry: None,
        details,
    }
}

fn delete_plan_json(plan: &pb::DeleteSessionsPlan) -> Value {
    json!({
        "sessions": plan.sessions.iter().map(session_to_delete_json).collect::<Vec<_>>(),
        "left_alone": plan.left_alone.iter().map(|left| json!({
            "machine": left.machine.as_ref().map(|machine| machine.value.clone()),
            "sessions": left.sessions.len(),
        })).collect::<Vec<_>>(),
        "forks_staying": plan.forks_staying.len(),
        "desk_sessions_left_out": plan.desk_sessions_left_out,
        "needs_count": plan.needs_count,
    })
}

fn session_to_delete_json(found: &pb::SessionToDelete) -> Value {
    json!({
        "session": found.session.as_ref().map(|id| id.value.clone()),
        "title": found.title,
        "channel": found.channel.as_ref().map(|id| id.value.clone()),
        "machine": found.machine.as_ref().map(|id| id.value.clone()),
        "archived": found.archived,
        "unpushed_repos": found.unpushed.len(),
        "running_terminals": found.running_terminals,
        "goes_with": found.goes_with.as_ref().map(|id| id.value.clone()),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use humata2_mcp::{Daemon, ErrorKind, Operation};
    use serde_json::{Map, Value, json};

    use super::*;

    fn arguments(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap_or_default()
    }

    fn daemon() -> GrpcDaemon {
        GrpcDaemon::new(PathBuf::from("/nonexistent/humatad.sock"))
    }

    #[tokio::test]
    async fn a_new_connection_does_not_list_the_tool_before_it_learns_the_rule() {
        assert!(!daemon().serves(Operation::SessionsDeleteMany));
    }

    #[tokio::test]
    async fn the_tool_is_listed_under_ask_and_allow_and_hidden_under_never() {
        let daemon = daemon();
        for (decision, listed) in [
            (pb::Decision::Ask, true),
            (pb::Decision::Allow, true),
            (pb::Decision::Never, false),
            (pb::Decision::Unspecified, false),
        ] {
            daemon.learn_delete_many_rule(decision);
            assert_eq!(daemon.serves(Operation::SessionsDeleteMany), listed, "{decision:?}");
        }
    }

    #[tokio::test]
    async fn rule_flips_notify_tool_list_changes_only_when_offered_state_changes() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let daemon = GrpcDaemon::with_tool_list_changes(
            PathBuf::from("/nonexistent/humatad.sock"),
            tx,
        );

        assert!(!daemon.learn_delete_many_rule(pb::Decision::Never));
        assert!(rx.try_recv().is_err());

        assert!(daemon.learn_delete_many_rule(pb::Decision::Ask));
        assert!(rx.try_recv().is_ok());

        // Ask -> Allow keeps the tool listed, so no redundant notification.
        assert!(!daemon.learn_delete_many_rule(pb::Decision::Allow));
        assert!(rx.try_recv().is_err());

        assert!(daemon.learn_delete_many_rule(pb::Decision::Never));
        assert!(rx.try_recv().is_ok());
    }

    #[test]
    fn delete_unpushed_work_defaults_to_false_and_reads_explicit_flag() {
        assert!(!delete_unpushed_work(&arguments(json!({ "channel": "ch_1" }))));
        assert!(!delete_unpushed_work(&arguments(
            json!({ "channel": "ch_1", "delete_unpushed_work": false })
        )));
        assert!(delete_unpushed_work(&arguments(
            json!({ "channel": "ch_1", "delete_unpushed_work": true })
        )));
    }

    #[test]
    fn picked_ids_a_channel_or_a_machine_each_name_their_scope() {
        match delete_scope(&arguments(json!({ "session_ids": ["se_1", "se_2"] }))) {
            Ok(pb::delete_scope::Scope::Sessions(picked)) => {
                let ids: Vec<&str> = picked.ids.iter().map(|id| id.value.as_str()).collect();
                assert_eq!(ids, ["se_1", "se_2"]);
            }
            other => panic!("picked ids: {other:?}"),
        }
        match delete_scope(&arguments(json!({ "channel": "ch_1" }))) {
            Ok(pb::delete_scope::Scope::Channel(channel)) => assert_eq!(channel.value, "ch_1"),
            other => panic!("channel: {other:?}"),
        }
        match delete_scope(&arguments(json!({ "machine": "ma_1" }))) {
            Ok(pb::delete_scope::Scope::Machine(machine)) => assert_eq!(machine.value, "ma_1"),
            other => panic!("machine: {other:?}"),
        }
    }

    #[test]
    fn a_call_naming_no_scope_or_more_than_one_is_refused_and_all_is_never_a_scope() {
        for args in [
            json!({}),
            json!({ "channel": "ch_1", "machine": "ma_1" }),
            json!({ "session_ids": [], "channel": "ch_1" }),
            json!({ "all": true }),
        ] {
            let error = delete_scope(&arguments(args.clone())).expect_err("refused");
            assert_eq!(error.kind, ErrorKind::InvalidField, "{args}");
        }
    }

    #[test]
    fn a_changed_answer_is_refused_as_a_stale_plan_that_names_each_change() {
        let changed = pb::delete_sessions_response::Changed {
            plan: Some(pb::DeleteSessionsPlan::default()),
            changes: vec!["\"Ledger\" now has 2 unpushed commits".to_owned()],
        };

        let error = plan_changed(changed);

        assert_eq!(error.kind, ErrorKind::PlanStale);
        assert!(
            error.message.contains("\"Ledger\" now has 2 unpushed commits"),
            "{}",
            error.message
        );
        assert!(error.details.contains_key("plan"));
    }
}
