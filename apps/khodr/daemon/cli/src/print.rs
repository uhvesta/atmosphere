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
                "{} {}{}",
                completed.request_id,
                if completed.approved {
                    "approved"
                } else {
                    "denied"
                },
                if completed.automatic {
                    " automatic"
                } else {
                    ""
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
        Some(Kind::Checkpoint(checkpoint)) => (
            "checkpoint",
            checkpoint
                .snapshots
                .iter()
                .map(|snapshot| format!("{}={}", snapshot.name, snapshot.commit))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        None => ("unknown", String::new()),
    };
    format!("{}\t{kind}\t{}", event.id, detail.replace('\n', "\\n"))
}

fn or_dash(text: &str) -> String {
    if text.is_empty() {
        "-".to_owned()
    } else {
        text.to_owned()
    }
}

/// `<id>\t<host>\t<state>\t<parent>\t<project>\t<approval mode>\t<backend>\t<model>\t<marks>\t<working directory>\t<name>`,
/// with `-` for this Mac, no parent, no project, the default model or no
/// marks. The marks are `archived` and `keeper`, joined by commas.
pub fn session(session: &pb::SessionInfo) -> String {
    let marks: Vec<&str> = [(session.archived, "archived"), (session.keeper, "keeper")]
        .into_iter()
        .filter_map(|(set, mark)| set.then_some(mark))
        .collect();
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        session.id,
        or_dash(&session.host),
        session.state().as_str_name(),
        or_dash(&session.parent_id),
        or_dash(&session.project_id),
        session.approval_mode().as_str_name(),
        session.backend().as_str_name(),
        or_dash(&session.model),
        or_dash(&marks.join(",")),
        or_dash(&session.working_directory),
        session.name
    )
}

/// `<id>\t<repositories>\t<name>`, each repository as `NAME=PATH`, joined
/// by commas.
pub fn project(project: &pb::Project) -> String {
    let repositories: Vec<String> = project
        .repositories
        .iter()
        .map(|repository| format!("{}={}", repository.name, repository.path))
        .collect();
    format!(
        "{}\t{}\t{}",
        project.id,
        repositories.join(","),
        project.name
    )
}

/// Each repository's diff; with several, each after a `# <name>` line.
pub fn changes(changes: &pb::Changes) -> String {
    match changes.repositories.as_slice() {
        [only] => only.patch.clone(),
        all => all
            .iter()
            .map(|repository| format!("# {}\n{}", repository.name, repository.patch))
            .collect(),
    }
}
