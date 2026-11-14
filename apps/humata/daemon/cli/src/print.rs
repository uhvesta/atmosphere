//! One line per thing, for people and for scripts that split on tabs.

use humata_wire::pb;

/// `<event id>\t<kind>\t<detail>`. A subagent's step is indented two
/// spaces and ends with `\t<call that started it>`.
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
        Some(Kind::MessageQueued(queued)) => {
            ("queued", format!("{} {}", queued.queue_id, queued.text))
        }
        Some(Kind::Dequeued(dequeued)) => ("dequeued", dequeued.queue_id.clone()),
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
        Some(Kind::Waiting(waiting)) => ("waiting", waiting.reason.clone()),
        Some(Kind::Notice(notice)) => ("notice", notice.text.clone()),
        Some(Kind::Rewound(rewound)) => (
            "rewound",
            if rewound.files {
                format!("{} (and its files)", rewound.text)
            } else {
                rewound.text.clone()
            },
        ),
        Some(Kind::RewindUndone(_)) => ("rewind-undone", String::new()),
        Some(Kind::Compacted(_)) => ("compacted", String::new()),
        Some(Kind::Compacting(compacting)) => ("compacting", compacting.focus.clone()),
        Some(Kind::Subagent(subagent)) => (
            "subagent",
            format!(
                "{} {} tokens={} tools={} ms={} {}",
                or_dash(&subagent.agent_type),
                subagent_status(subagent.status()),
                subagent.total_tokens,
                subagent.tool_uses,
                subagent.duration_ms,
                subagent.description
            ),
        ),
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
        Some(Kind::Saved(saved)) => (
            "saved",
            saved
                .snapshots
                .iter()
                .map(|snapshot| format!("{}={}", snapshot.name, snapshot.commit))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        None => ("unknown", String::new()),
    };
    let line = format!("{}\t{kind}\t{}", event.id, detail.replace('\n', "\\n"));
    if event.parent_call_id.is_empty() {
        line
    } else {
        format!("  {line}\t{}", event.parent_call_id)
    }
}

fn subagent_status(status: pb::SubagentStatus) -> &'static str {
    match status {
        pb::SubagentStatus::Unspecified => "unknown",
        pb::SubagentStatus::Running => "running",
        pb::SubagentStatus::Completed => "completed",
        pb::SubagentStatus::Failed => "failed",
        pb::SubagentStatus::Stopped => "stopped",
    }
}

/// The subagents in `events`, each as last reported, in the order they
/// started: `<call id>\t<status>\t<type>\t<tokens>\t<tool calls>\t<ms>`
/// then `\t<description>`.
pub fn subagents(events: &[pb::SessionEvent]) -> Vec<String> {
    use pb::session_event::Kind;
    let mut latest: Vec<(&str, &pb::Subagent)> = Vec::new();
    for event in events {
        let Some(Kind::Subagent(subagent)) = &event.kind else {
            continue;
        };
        let call = event.parent_call_id.as_str();
        match latest.iter_mut().find(|(at, _)| *at == call) {
            Some(known) => known.1 = subagent,
            None => latest.push((call, subagent)),
        }
    }
    latest
        .into_iter()
        .map(|(call, subagent)| {
            format!(
                "{call}\t{}\t{}\t{}\t{}\t{}\t{}",
                subagent_status(subagent.status()),
                or_dash(&subagent.agent_type),
                subagent.total_tokens,
                subagent.tool_uses,
                subagent.duration_ms,
                or_dash(&subagent.description).replace('\n', " ")
            )
        })
        .collect()
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
/// marks. The marks are `archived`, `keeper` and `offline` (its host can't
/// be reached, so this is as it last was), joined by commas.
pub fn session(session: &pb::SessionInfo) -> String {
    let marks: Vec<&str> = [
        (session.archived, "archived"),
        (session.keeper, "keeper"),
        (session.host_offline, "offline"),
    ]
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

/// `<thread id>\t<repository>/<path>:<start>-<end>\t<state>`, then a line
/// `\t<comment id>\t<you|agent>\t<turn>\t<text>` for each comment.
pub fn files(files: &pb::Files) -> String {
    let mut out: String = files
        .files
        .iter()
        .map(|file| format!("{:>10}  {}\n", file.size, file.path))
        .collect();
    if files.truncated {
        out.push_str("(more files not listed)\n");
    }
    out
}

/// One line per entry: a folder's name ends in "/", a file's has its size.
pub fn folder(folder: &pb::Folder) -> String {
    let mut out: String = folder
        .entries
        .iter()
        .map(|entry| {
            if entry.is_directory {
                format!("{:>10}  {}/\n", "", entry.name)
            } else {
                format!("{:>10}  {}\n", entry.size, entry.name)
            }
        })
        .collect();
    if folder.truncated {
        out.push_str("(more entries not listed)\n");
    }
    out
}

pub fn thread(thread: &pb::ReviewThread) -> String {
    let file = thread
        .file
        .as_ref()
        .map(|file| format!("{}/{}", file.repository, file.path))
        .unwrap_or_default();
    let mut state = if thread.resolved { "resolved" } else { "open" }.to_owned();
    if thread.outdated {
        state.push_str(",outdated");
    }
    let mut text = format!(
        "{}\t{file}:{}-{}\t{state}\n",
        thread.id, thread.start_line, thread.end_line
    );
    for comment in &thread.comments {
        let author = match comment.author() {
            pb::CommentAuthor::You => "you".to_owned(),
            pb::CommentAuthor::Agent => "agent".to_owned(),
            pb::CommentAuthor::Github => format!("@{} on GitHub", comment.login),
        };
        text.push_str(&format!(
            "\t{}\t{author}\t{}\t{}\n",
            comment.id, comment.turn, comment.text
        ));
    }
    text
}

fn status(status: pb::ItemStatus) -> &'static str {
    match status {
        pb::ItemStatus::Unspecified | pb::ItemStatus::Todo => "todo",
        pb::ItemStatus::InProgress => "in_progress",
        pb::ItemStatus::WaitingOnYou => "waiting_on_you",
        pb::ItemStatus::InReview => "in_review",
        pb::ItemStatus::Done => "done",
        pb::ItemStatus::WontDo => "wont_do",
    }
}

fn priority(priority: pb::Priority) -> &'static str {
    match priority {
        pb::Priority::None => "none",
        pb::Priority::Low => "low",
        pb::Priority::Medium => "medium",
        pb::Priority::High => "high",
        pb::Priority::Urgent => "urgent",
    }
}

/// `<id>\t<parent>\t<status>\t<given status>\t<priority>\t<sessions>\t<title>`,
/// with `-` for a task's parent and no sessions; the sessions joined by
/// commas.
pub fn item(item: &pb::Item) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}\t{}",
        item.id,
        or_dash(&item.parent_id),
        status(item.status()),
        status(item.given_status()),
        priority(item.priority()),
        or_dash(&item.session_ids.join(",")),
        item.title
    )
}

fn fields(fields: Option<&pb::ItemFields>) -> String {
    fields.map_or_else(
        || "-".to_owned(),
        |fields| {
            format!(
                "{} {} {:?}",
                status(fields.status()),
                priority(fields.priority()),
                fields.title
            )
        },
    )
}

/// `<change>\t<item>\t<reverts>\t<before>\t<after>\t<by>`, `-` for
/// nothing reverted, before an add and after a removal. `<by>` is where it
/// was made from (`app`, `command_line`, `mcp` or `humata`), or
/// `session:<id>` for a session's proposal or request; `-` when unknown.
pub fn change(change: &pb::BoardChange) -> String {
    let by = if change.session_id.is_empty() {
        match change.surface() {
            pb::ChangeSurface::App => "app".to_owned(),
            pb::ChangeSurface::CommandLine => "command_line".to_owned(),
            pb::ChangeSurface::Mcp => "mcp".to_owned(),
            pb::ChangeSurface::Humata => "humata".to_owned(),
            pb::ChangeSurface::Unspecified => "-".to_owned(),
        }
    } else {
        format!("session:{}", change.session_id)
    };
    format!(
        "{}\t{}\t{}\t{}\t{}\t{by}",
        change.id,
        change.item_id,
        if change.reverts == 0 {
            "-".to_owned()
        } else {
            change.reverts.to_string()
        },
        fields(change.before.as_ref()),
        fields(change.after.as_ref())
    )
}

/// `<session>\t<reason>\t<project>\t<item>\t<name>`, `-` for none; for
/// what an agent asks, then `\t<request id>\t<status asked for>\t<text>`.
pub fn inbox_entry(entry: &pb::InboxEntry) -> String {
    let reason = match entry.reason() {
        pb::InboxReason::Approval => "approval",
        pb::InboxReason::Reply => "reply",
        pb::InboxReason::Failed => "failed",
        pb::InboxReason::Question => "question",
        pb::InboxReason::Move => "move",
        pb::InboxReason::Unspecified => "-",
    };
    // What an agent asks to do to another session waits as a question does.
    let reason = match entry.request.as_ref().map(pb::AgentRequest::kind) {
        Some(pb::AgentRequestKind::StartSession) => "start_session",
        Some(pb::AgentRequestKind::SendMessage) => "send_message",
        _ => reason,
    };
    let asked = entry.request.as_ref().map_or_else(String::new, |request| {
        let status = match request.kind() {
            pb::AgentRequestKind::Move => status(request.status()),
            _ => "-",
        };
        format!(
            "\t{}\t{status}\t{}",
            request.id,
            request.text.replace('\n', " ")
        )
    });
    let session = entry.session.as_ref();
    format!(
        "{}\t{reason}\t{}\t{}\t{}",
        session
            .map(|session| session.id.as_str())
            .unwrap_or_default(),
        or_dash(&entry.project_name),
        or_dash(&entry.item_title),
        session
            .map(|session| session.name.as_str())
            .unwrap_or_default(),
    ) + &asked
}

/// A recovery copy: its session, name, host, the turns it has of the
/// session's, and whether its host lost the session.
pub fn terminal(terminal: &pb::Terminal) -> String {
    let state = if terminal.exited {
        format!("exited {}", terminal.exit_code)
    } else {
        "running".to_owned()
    };
    format!(
        "{}\t{}\t{}\t{}\t{state}",
        terminal.id,
        or_dash(&terminal.host),
        or_dash(&terminal.session_id),
        terminal.folder
    )
}

/// A proposal: its id, kind, state, title, and the item it made or
/// changes.
pub fn proposal(proposal: &pb::Proposal) -> String {
    let kind = match proposal.kind() {
        pb::ProposalKind::SpecEdit => "spec edit",
        _ => "task",
    };
    let state = match proposal.state() {
        pb::ProposalState::Applied => "applied",
        pb::ProposalState::Declined => "declined",
        _ => "pending",
    };
    let item = if proposal.applied_item_id.is_empty() {
        &proposal.item_id
    } else {
        &proposal.applied_item_id
    };
    format!(
        "{}\t{kind}\t{state}\t{}\t{}",
        proposal.id,
        proposal.title,
        or_dash(item)
    )
}

/// A forward: its id, host, the port there, the port here, and session.
pub fn forward(forward: &pb::Forward) -> String {
    format!(
        "{}\t{}\t{}\t127.0.0.1:{}\t{}",
        forward.id,
        or_dash(&forward.host),
        forward.port,
        forward.local_port,
        or_dash(&forward.session_id)
    )
}

pub fn recovery(recovery: &pb::Recovery) -> String {
    let state = if recovery.on_host { "on host" } else { "lost" };
    format!(
        "{}\t{}\t{}\t{}/{} turns\t{state}",
        recovery.session_id,
        or_dash(&recovery.name),
        or_dash(&recovery.host),
        recovery.backed_up_turns,
        recovery.turns
    )
}

pub fn host(host: &pb::HostInfo) -> String {
    let env = if host.env_names.is_empty() {
        "-".to_owned()
    } else {
        host.env_names.join(",")
    };
    let setup = if host.setup_script.trim().is_empty() {
        "no setup"
    } else {
        "setup"
    };
    format!(
        "{}\t{}\t{env}\t{setup}",
        host.name,
        or_dash(&host.ssh_destination)
    )
}

/// `<id>\t<used or github>\t<owner>\t<name>\t<location>`, with `-` for
/// one a project used, which has no owner.
pub fn saved_repository(repository: &pb::SavedRepository) -> String {
    let source = match repository.source() {
        pb::RepositorySource::Github => "github",
        pb::RepositorySource::Used | pb::RepositorySource::Unspecified => "used",
    };
    format!(
        "{}\t{source}\t{}\t{}\t{}",
        repository.id,
        or_dash(&repository.owner),
        repository.name,
        repository.location
    )
}

/// `<login>\t<repositories>\t<listed or never>\t<why the last listing
/// failed>`, with `-` when it didn't.
pub fn repository_owner(owner: &pb::RepositoryOwner) -> String {
    let listed = if owner.refreshed_unix_ms == 0 {
        "never listed"
    } else {
        "listed"
    };
    format!(
        "{}\t{} repositories\t{listed}\t{}",
        owner.login,
        owner.repositories,
        or_dash(&owner.error.replace('\n', " "))
    )
}

fn setup(setup: Option<&pb::SetupResult>) -> String {
    match setup {
        Some(setup) if setup.ran && setup.succeeded => "setup ok".to_owned(),
        Some(setup) if setup.ran => format!(
            "setup failed: {}",
            setup.output.lines().last().unwrap_or_default()
        ),
        _ => "no setup".to_owned(),
    }
}

pub fn health(health: &pb::HostHealth) -> String {
    let Some(status) = health.status.as_ref().filter(|_| health.reachable) else {
        return format!("{}\tunreachable\t{}", health.name, health.problem);
    };
    let copilot = if status.copilot_version.is_empty() {
        format!("no copilot: {}", status.copilot_problem)
    } else {
        format!("copilot {}", status.copilot_version)
    };
    let token = if status.github_token {
        "token"
    } else {
        "no token"
    };
    format!(
        "{}\treachable\thumatad {}\t{}\t{} GB free\t{copilot}\t{token}\t{}",
        health.name,
        status.program_version,
        status.platform,
        status.free_disk_bytes / 1_000_000_000,
        setup(status.setup.as_ref()),
    )
}

/// `<backend>\t<id>\t<updated, ms since 1970>\t<working directory>\t<summary>`,
/// one line per session that can be imported.
pub fn importable(session: &pb::ImportableSession) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{}",
        session.backend().as_str_name(),
        session.id,
        session.updated_unix_ms,
        session.working_directory,
        session.summary
    )
}

/// `<name>\t<state>\t<folder>` per command, then what to add to the shell's
/// startup file when the folder isn't on its PATH.
pub fn command_links(linked: &pb::CommandsLinked) -> String {
    let mut printed = String::new();
    for link in &linked.links {
        let state = match link.state() {
            pb::CommandLinkState::Linked => "linked",
            pb::CommandLinkState::Stale => "linked to another copy of Humata",
            pb::CommandLinkState::Other => "something else of that name is there, left alone",
            pb::CommandLinkState::Missing | pb::CommandLinkState::Unspecified => "not linked",
        };
        printed.push_str(&format!("{}\t{state}\t{}\n", link.name, linked.folder));
    }
    if !linked.on_path {
        printed.push_str(&format!(
            "{} isn't on your login shell's PATH; add `export PATH=\"{}:$PATH\"` to its startup file, such as ~/.zprofile\n",
            linked.folder, linked.folder
        ));
    }
    printed
}

/// `<kind>\t<session>\t<turn>\t<event>\t<project>\t<item>\t<related>\t<snippet>`,
/// `-` for none, with the words found between `**` and the snippet on one
/// line.
pub fn search_hit(hit: &pb::SearchHit) -> String {
    let kind = match hit.kind() {
        pb::SearchHitKind::Message | pb::SearchHitKind::Unspecified => "message",
        pb::SearchHitKind::Reply => "reply",
        pb::SearchHitKind::ToolCall => "tool_call",
        pb::SearchHitKind::ToolOutput => "tool_output",
        pb::SearchHitKind::Comment => "comment",
        pb::SearchHitKind::Spec => "spec",
    };
    let snippet = hit
        .snippet
        .replace(['\u{E000}', '\u{E001}'], "**")
        .replace(['\n', '\t'], " ");
    format!(
        "{kind}\t{}\t{}\t{}\t{}\t{}\t{}\t{snippet}",
        or_dash(&hit.session_id),
        if hit.turn == 0 {
            "-".to_owned()
        } else {
            hit.turn.to_string()
        },
        or_dash(&hit.event_id),
        or_dash(&hit.project_id),
        or_dash(&hit.item_id),
        if hit.related { "related" } else { "-" },
    )
}

pub fn landed(landed: &pb::SessionLanded) -> String {
    let mut out = String::new();
    for repository in &landed.repositories {
        let commit = repository.commit.get(..8).unwrap_or(&repository.commit);
        let pushed = if repository.pushed { ", pushed" } else { "" };
        out.push_str(&format!(
            "{}\t{} at {commit} onto {}{pushed}\n",
            repository.name, repository.branch, repository.base
        ));
    }
    for pull in &landed.pull_requests {
        out.push_str(&pull_request(pull));
        out.push('\n');
    }
    out
}

pub fn pull_request(pull: &pb::PullRequest) -> String {
    let state = match pull.state() {
        pb::PullRequestState::Merged => "merged",
        pb::PullRequestState::Closed => "closed",
        _ => "open",
    };
    let review = match pull.review() {
        pb::ReviewStanding::Approved => "approved",
        pb::ReviewStanding::ChangesRequested => "changes requested",
        pb::ReviewStanding::Commented => "commented",
        _ => "no review",
    };
    let checks = match pull.checks() {
        pb::ChecksState::Passing => "checks pass",
        pb::ChecksState::Failing => "checks fail",
        pb::ChecksState::Pending => "checks running",
        _ => "no checks",
    };
    format!(
        "{}#{}\t{state}, {review}, {checks}\t{}\t{}",
        pull.github_repository, pull.number, pull.title, pull.url
    )
}

pub fn base_updated(updated: &pb::BaseUpdated) -> String {
    let mut out = String::new();
    for repository in &updated.repositories {
        let outcome = match repository.outcome() {
            pb::MergeOutcome::UpToDate => "up to date".to_owned(),
            pb::MergeOutcome::Conflicts => {
                format!("conflicts in {}", repository.conflicts.join(", "))
            }
            _ => "merged".to_owned(),
        };
        out.push_str(&format!(
            "{}\t{} {outcome}\n",
            repository.name, repository.upstream
        ));
    }
    if updated.sent_to_agent {
        out.push_str("the agent was asked to resolve the conflicts\n");
    }
    out
}

fn scope(scope: pb::ApprovalScope) -> &'static str {
    match scope {
        pb::ApprovalScope::Proposals => "proposals",
        pb::ApprovalScope::Board => "board",
        pb::ApprovalScope::Destructive => "destructive",
        pb::ApprovalScope::Unspecified => "-",
    }
}

/// An admin action asked for: its id, state, scope, the client that asked
/// and what it does.
pub fn approval(approval: &pb::Approval) -> String {
    let state = match approval.state() {
        pb::ApprovalState::Approved => "approved",
        pb::ApprovalState::Denied => "denied",
        _ => "pending",
    };
    format!(
        "{}\t{state}\t{}\t{}\t{}",
        approval.id,
        scope(approval.scope()),
        or_dash(&approval.client),
        approval.summary
    )
}

/// A grant in force: its id, scopes, when it ends, and how many actions
/// ran under it.
pub fn grant(grant: &pb::Grant) -> String {
    let scopes: Vec<&str> = grant.scopes().map(scope).collect();
    format!(
        "{}\t{}\tuntil {}\t{} used",
        grant.id,
        scopes.join(","),
        grant.expires_unix_ms,
        grant.used
    )
}

/// A skill: where it is, its name and hash, then what it's for.
pub fn skill(skill: &pb::FoundSkill) -> String {
    let host = if skill.host.is_empty() {
        "mac"
    } else {
        &skill.host
    };
    let origin = match skill.origin() {
        pb::SkillOrigin::ClaudeUser => "claude",
        pb::SkillOrigin::CopilotUser => "copilot",
        pb::SkillOrigin::ClaudeRepository => "repo-claude",
        pb::SkillOrigin::GithubRepository => "repo-github",
        pb::SkillOrigin::Project => "project",
        pb::SkillOrigin::Unspecified => "unknown",
    };
    let at = if skill.path.is_empty() {
        format!("project {}", skill.project_id)
    } else {
        skill.path.clone()
    };
    format!(
        "skill\t{}\t{host}\t{origin}\t{}\t{at}\t{}",
        skill.name, skill.hash, skill.description
    )
}

/// A clash: its name, the winner's hash or "unpicked", then every file's.
pub fn skill_clash(clash: &pb::SkillClash) -> String {
    let winner = if clash.winner.is_empty() {
        "unpicked"
    } else {
        &clash.winner
    };
    let hashes: Vec<&str> = clash.candidates.iter().map(|c| c.hash.as_str()).collect();
    format!("clash\t{}\t{winner}\t{}", clash.name, hashes.join(" "))
}

/// A plugin: its agent, name and host, whether its CLI has it on, its
/// version, what it adds, then what it's for.
pub fn plugin(plugin: &pb::FoundPlugin) -> String {
    let agent = match plugin.agent() {
        pb::PluginAgent::Claude => "claude",
        pb::PluginAgent::Copilot => "copilot",
        pb::PluginAgent::Unspecified => "-",
    };
    let host = if plugin.host.is_empty() {
        "mac"
    } else {
        &plugin.host
    };
    let state = if plugin.enabled { "on" } else { "off" };
    let mut adds = Vec::new();
    for (count, what) in [
        (plugin.skills.len(), "skills"),
        (plugin.commands.len(), "commands"),
        (plugin.agents.len(), "agents"),
        (plugin.mcp_servers.len(), "mcp"),
    ] {
        if count > 0 {
            adds.push(format!("{count} {what}"));
        }
    }
    if plugin.hooks {
        adds.push("hooks".to_owned());
    }
    if plugin.lsp_servers {
        adds.push("lsp".to_owned());
    }
    format!(
        "plugin\t{agent}\t{}\t{host}\t{state}\t{}\t{}\t{}",
        plugin.id,
        or_dash(&plugin.version),
        or_dash(&adds.join(", ")),
        plugin.description
    )
}
