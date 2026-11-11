//! One line per thing, for people and for scripts that split on tabs.

use khodr_wire::pb;

/// `<event id>\t<kind>\t<detail>`.
pub fn event(event: &pb::SessionEvent) -> String {
    use pb::session_event::Kind;
    let (kind, detail) = match &event.kind {
        Some(Kind::UserMessage(message)) => ("user", message.text.clone()),
        Some(Kind::TurnStarted(_)) => ("turn_started", String::new()),
        Some(Kind::AssistantDelta(delta)) => ("delta", delta.text.clone()),
        Some(Kind::AssistantMessage(message)) => ("assistant", message.text.clone()),
        Some(Kind::Reasoning(reasoning)) => ("reasoning", reasoning.text.clone()),
        Some(Kind::ToolStarted(tool)) => (
            "tool_started",
            format!("{} {} {}", tool.call_id, tool.tool, tool.arguments_json),
        ),
        Some(Kind::ToolCompleted(tool)) => (
            "tool_completed",
            format!(
                "{} {} {}",
                tool.call_id,
                if tool.success { "ok" } else { "failed" },
                tool.output
            ),
        ),
        Some(Kind::PermissionRequested(request)) => (
            "permission_requested",
            format!(
                "{} {} {}",
                request.request_id,
                request.kind().as_str_name(),
                request.summary
            ),
        ),
        Some(Kind::PermissionCompleted(completed)) => (
            "permission_completed",
            format!(
                "{} {}",
                completed.request_id,
                if completed.approved {
                    "approved"
                } else {
                    "denied"
                }
            ),
        ),
        Some(Kind::TurnEnded(ended)) => (
            "turn_ended",
            if ended.aborted {
                "aborted".to_owned()
            } else {
                String::new()
            },
        ),
        Some(Kind::Idle(_)) => ("idle", String::new()),
        Some(Kind::Failed(failed)) => ("failed", failed.message.clone()),
        Some(Kind::Usage(usage)) => (
            "usage",
            format!(
                "{} in={} out={} premium={}",
                usage.model, usage.input_tokens, usage.output_tokens, usage.premium_requests
            ),
        ),
        None => ("unknown", String::new()),
    };
    format!("{}\t{kind}\t{}", event.id, detail.replace('\n', "\\n"))
}

/// `<id>\t<host>\t<state>\t<parent>\t<name>`, with `-` for this Mac or no parent.
pub fn session(session: &pb::SessionInfo) -> String {
    let or_dash = |text: &str| {
        if text.is_empty() {
            "-".to_owned()
        } else {
            text.to_owned()
        }
    };
    format!(
        "{}\t{}\t{}\t{}\t{}",
        session.id,
        or_dash(&session.host),
        session.state().as_str_name(),
        or_dash(&session.parent_id),
        session.name
    )
}
