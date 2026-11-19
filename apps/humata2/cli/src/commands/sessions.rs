//! Sessions (CLI.md §5.2): their terminals, life cycle, repos and pushes,
//! and talking to their agents.

use humata2_proto::humata::v1 as pb;

use super::{pages, reply, streams, values};
use crate::args::{SessionCommand, SessionRepoCommand};
use crate::call::{Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;
use crate::resolve;

pub(super) async fn attach<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: Option<String>,
    tab: Option<String>,
    read_only: bool,
) -> Result<Done, CliError> {
    streams::no_json(ctx)?;
    if ctx.flags.dry_run {
        return Err(CliError::AttachDryRun);
    }
    let Some(session) = session else {
        // A terminal no session holds: the tab alone names it.
        let tab = tab.ok_or(CliError::Missing {
            flag: "<session> or --tab <tab>",
        })?;
        let open = pb::AttachOpen {
            session: None,
            tab: Some(pb::TabId { value: tab }),
            read_only,
            ..Default::default()
        };
        return streams::attach(ctx, open).await;
    };
    let session = ctx.session(&session).await?;
    let tab = match tab {
        Some(value) => pb::TabId { value },
        None => {
            let request = pb::ListTabsRequest {
                session: Some(session.clone()),
            };
            let tabs: pb::ListTabsResponse = ctx.call(rpc::terminal::LIST_TABS, &request).await?;
            default_tab(&session.value, &tabs.tabs)?
        }
    };
    let open = pb::AttachOpen {
        session: Some(session),
        tab: Some(tab),
        read_only,
        ..Default::default()
    };
    streams::attach(ctx, open).await
}

/// The tab `attach` opens without `--tab`: the agent's tab, or the only tab
/// when the session has no agent tab. Anything else needs `--tab`.
fn default_tab(session: &str, tabs: &[pb::Tab]) -> Result<pb::TabId, CliError> {
    let agent = tabs.iter().find(|tab| tab.kind == pb::tab::Kind::Agent as i32);
    let chosen = match (agent, tabs) {
        (Some(tab), _) => Some(tab),
        (None, [only]) => Some(only),
        (None, _) => None,
    };
    chosen
        .and_then(|tab| tab.id.clone())
        .ok_or_else(|| CliError::NoAgentTab {
            session: session.to_owned(),
            tabs: tabs
                .iter()
                .filter_map(|tab| tab.id.as_ref().map(|id| id.value.clone()))
                .collect(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, kind: pb::tab::Kind) -> pb::Tab {
        pb::Tab {
            id: Some(pb::TabId { value: id.into() }),
            kind: kind as i32,
            ..Default::default()
        }
    }

    #[test]
    fn the_agent_tab_is_chosen_over_shells() {
        let tabs = [tab("tb_shell", pb::tab::Kind::Shell), tab("tb_agent", pb::tab::Kind::Agent)];
        let chosen = default_tab("se_1", &tabs).expect("the agent tab is chosen");
        assert_eq!(chosen.value, "tb_agent");
    }

    #[test]
    fn the_only_tab_is_chosen_when_there_is_no_agent_tab() {
        let tabs = [tab("tb_shell", pb::tab::Kind::Shell)];
        let chosen = default_tab("se_1", &tabs).expect("the only tab is chosen");
        assert_eq!(chosen.value, "tb_shell");
    }

    #[test]
    fn several_shells_and_no_agent_tab_is_an_error_naming_the_tabs() {
        let tabs = [tab("tb_a", pb::tab::Kind::Shell), tab("tb_b", pb::tab::Kind::Shell)];
        assert!(matches!(
            default_tab("se_1", &tabs),
            Err(CliError::NoAgentTab { tabs, .. }) if tabs == ["tb_a", "tb_b"]
        ));
    }

    #[test]
    fn no_tabs_at_all_is_an_error() {
        assert!(matches!(
            default_tab("se_1", &[]),
            Err(CliError::NoAgentTab { tabs, .. }) if tabs.is_empty()
        ));
    }
}

/// Opens a shell on a machine and attaches to it; with `--json`, prints
/// the new tab instead, for a script to attach to later.
pub(super) async fn shell<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    machine: Option<String>,
    session: Option<String>,
) -> Result<Done, CliError> {
    let tab: pb::Tab = match (machine, session) {
        (_, Some(session)) => {
            let request = pb::OpenShellTabRequest {
                idempotency_key: ctx.key(),
                session: Some(ctx.session(&session).await?),
                ..Default::default()
            };
            if ctx.flags.json || ctx.flags.dry_run {
                return ctx.change(rpc::terminal::OPEN_SHELL_TAB, &request).await;
            }
            ctx.call(rpc::terminal::OPEN_SHELL_TAB, &request).await?
        }
        (Some(machine), None) => {
            let request = pb::OpenMachineShellRequest {
                idempotency_key: ctx.key(),
                machine: Some(ctx.machine(&machine).await?),
            };
            if ctx.flags.json || ctx.flags.dry_run {
                return ctx.change(rpc::terminal::OPEN_MACHINE_SHELL, &request).await;
            }
            ctx.call(rpc::terminal::OPEN_MACHINE_SHELL, &request).await?
        }
        (None, None) => {
            return Err(CliError::Missing {
                flag: "<machine> or --session <session>",
            });
        }
    };
    // HO-6: a shell on another machine is attached through the host by its machine.
    let open = pb::AttachOpen {
        session: tab.session,
        tab: tab.id,
        machine: tab.machine,
        ..Default::default()
    };
    streams::attach(ctx, open).await
}

pub(super) async fn conversation<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: &str,
) -> Result<Done, CliError> {
    let id = ctx.session(session).await?;
    pages(ctx, rpc::session::READ_CONVERSATION, "entries", |page| {
        pb::ReadConversationRequest {
            id: Some(id.clone()),
            page,
        }
    })
    .await
    .map(Done::reply)
}

pub(super) async fn session<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: SessionCommand,
) -> Result<Done, CliError> {
    match command {
        SessionCommand::Queue { session } => {
            let request = pb::ListQueuedRequest {
                session: Some(ctx.session(&session).await?),
            };
            reply(ctx, rpc::terminal::LIST_QUEUED, &request).await
        }
        SessionCommand::Tabs { session } => {
            let request = pb::ListTabsRequest {
                session: Some(ctx.session(&session).await?),
            };
            reply(ctx, rpc::terminal::LIST_TABS, &request).await
        }
        SessionCommand::SendNow { session, ids } => {
            let request = pb::SendQueuedNowRequest {
                idempotency_key: ctx.key(),
                session: Some(ctx.session(&session).await?),
                ids: ids
                    .into_iter()
                    .map(|value| pb::QueuedTypingId { value })
                    .collect(),
            };
            ctx.change(rpc::terminal::SEND_QUEUED_NOW, &request).await
        }
        SessionCommand::Start {
            agent,
            model,
            repo,
            title,
            message,
        } => {
            let repos: Vec<pb::RepoChoice> = repo
                .iter()
                .map(|repo| {
                    resolve::repo(repo).map(|repo| pb::RepoChoice {
                        repo: Some(repo),
                        include: true,
                    })
                })
                .collect::<Result<_, CliError>>()?;
            let request = pb::StartSessionRequest {
                idempotency_key: ctx.key(),
                channel: Some(ctx.this_channel().await?),
                title: title.unwrap_or_default(),
                agent: agent.map(|value| pb::AgentId { value }),
                model: model.unwrap_or_default(),
                machine: Some(ctx.machine_or_this().await?),
                repos,
                first_message: message.unwrap_or_default(),
                board_item: None,
                branch: String::new(),
                rebuild: None,
                review: None,
            };
            ctx.change(rpc::session::START_SESSION, &request).await
        }
        SessionCommand::Fork {
            session,
            kind,
            agent,
            before_turn,
            title,
            branch,
        } => {
            let parent = ctx.session(&session).await?;
            refuse_another_channel(ctx, &session, &parent).await?;
            let request = pb::ForkSessionRequest {
                idempotency_key: ctx.key(),
                parent: Some(parent),
                kind: values::fork_kind(kind) as i32,
                before_turn: before_turn.unwrap_or(0),
                title: title.unwrap_or_default(),
                agent: agent.map(|value| pb::AgentId { value }),
                model: String::new(),
                machine: ctx.machine_flag().await?,
                branch: branch.unwrap_or_default(),
                reason: String::new(),
            };
            if ctx.flags.dry_run {
                let preview = ctx.json(rpc::session::PREVIEW_FORK, &request).await?;
                return Ok(Done {
                    result: preview,
                    outcome: Outcome::Planned,
                });
            }
            ctx.change(rpc::session::FORK_SESSION, &request).await
        }
        SessionCommand::Read { session, from, to } => super::search::read(ctx, &session, from, to).await,
        SessionCommand::Rename { session, title } => {
            let request = pb::RenameSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                title,
                expected_revision: None,
                rename_branch: false,
            };
            ctx.change(rpc::session::RENAME_SESSION, &request).await
        }
        SessionCommand::Stop { tab: Some(tab), .. } => {
            let request = pb::StopRunningHolderRequest {
                machine: Some(ctx.machine_or_this().await?),
                tab: Some(pb::TabId { value: tab }),
            };
            ctx.change(rpc::machine::STOP_RUNNING_HOLDER, &request).await
        }
        SessionCommand::Stop { session, tab: None } => {
            let request = pb::StopSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&named(session)?).await?),
                for_move: false,
            };
            ctx.change(rpc::session::STOP_SESSION, &request).await
        }
        SessionCommand::Resume { tab: Some(tab), .. } => {
            let request = pb::ResumeRunningHolderRequest {
                machine: Some(ctx.machine_or_this().await?),
                tab: Some(pb::TabId { value: tab }),
            };
            ctx.change(rpc::machine::RESUME_RUNNING_HOLDER, &request).await
        }
        SessionCommand::End { tab: Some(tab), .. } => {
            let request = pb::EndRunningHolderRequest {
                machine: Some(ctx.machine_or_this().await?),
                tab: Some(pb::TabId { value: tab }),
            };
            ctx.change(rpc::machine::END_RUNNING_HOLDER, &request).await
        }
        // End for good of a session is Archive (D784, Fable's ruling).
        // A Desk session has no archive (D182.2): it ends by the Desk's Close.
        SessionCommand::End { session, tab: None } => end_for_good(ctx, &named(session)?).await,
        SessionCommand::Running => {
            let request = pb::ListRunningHoldersRequest {
                machine: Some(ctx.machine_or_this().await?),
            };
            reply(ctx, rpc::machine::LIST_RUNNING_HOLDERS, &request).await
        }
        SessionCommand::Adopt { tab } => {
            let request = pb::AdoptRunningHolderRequest {
                machine: Some(ctx.machine_or_this().await?),
                tab: Some(pb::TabId { value: tab }),
                channel: ctx.channel_flag().await?,
            };
            ctx.change(rpc::machine::ADOPT_RUNNING_HOLDER, &request).await
        }
        SessionCommand::Resume { session, fresh, tab: None } => {
            use pb::resume_session_request::Mode;
            let session = named(session)?;
            let mode = if fresh {
                Mode::StartFreshWithSummary
            } else {
                Mode::Resume
            };
            let request = pb::ResumeSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                mode: mode as i32,
            };
            ctx.change(rpc::session::RESUME_SESSION, &request).await
        }
        SessionCommand::Archive {
            session,
            push_first,
            delete_work,
            end,
        } => {
            let choice = match (push_first, delete_work) {
                (true, _) => pb::ArchiveChoice::CommitPushPrThenArchive,
                (false, true) => pb::ArchiveChoice::ArchiveAndDeleteWork,
                (false, false) => pb::ArchiveChoice::Archive,
            };
            archive(ctx, &session, choice, end).await
        }
        SessionCommand::Delete { session } => {
            let id = Some(ctx.session(&session).await?);
            let delete = pb::DeleteSessionRequest {
                idempotency_key: ctx.key(),
                id: id.clone(),
                plan: None,
            };
            ctx.planned(
                rpc::session::PLAN_DELETE,
                &pb::PlanDeleteRequest { id },
                |plan: &pb::DeletePlan| plan.token.clone(),
                move |plan| async move {
                    ctx.json(
                        rpc::session::DELETE_SESSION,
                        &pb::DeleteSessionRequest { plan, ..delete },
                    )
                    .await
                },
            )
            .await
        }
        SessionCommand::Restore { session, stopped } => {
            let request = pb::RestoreSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                machine: ctx.machine_flag().await?,
                stopped,
            };
            ctx.change(rpc::session::RESTORE_SESSION, &request).await
        }
        SessionCommand::Rebuild { session } => {
            let request = pb::RebuildSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                machine: ctx.machine_flag().await?,
            };
            ctx.change(rpc::session::REBUILD_SESSION, &request).await
        }
        SessionCommand::Move { session, to } => {
            let request = pb::MoveSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                to: Some(ctx.machine(&to).await?),
                ..Default::default()
            };
            ctx.change(rpc::session::MOVE_SESSION, &request).await
        }
        SessionCommand::Repo(SessionRepoCommand::Add { session, repo }) => {
            let request = pb::AddRepoToSessionRequest {
                idempotency_key: ctx.key(),
                id: Some(ctx.session(&session).await?),
                repo: Some(resolve::repo(&repo)?),
            };
            ctx.change(rpc::session::ADD_REPO_TO_SESSION, &request).await
        }
        SessionCommand::Repo(SessionRepoCommand::Drop { session, repo }) => {
            let id = Some(ctx.session(&session).await?);
            let repo = Some(resolve::repo(&repo)?);
            let drop = pb::DropRepoRequest {
                idempotency_key: ctx.key(),
                id: id.clone(),
                repo: repo.clone(),
                plan: None,
            };
            ctx.planned(
                rpc::session::PLAN_DROP_REPO,
                &pb::PlanDropRepoRequest { id, repo },
                |plan: &pb::DropRepoPlan| plan.token.clone(),
                move |plan| async move {
                    ctx.json(rpc::session::DROP_REPO, &pb::DropRepoRequest { plan, ..drop })
                        .await
                },
            )
            .await
        }
        SessionCommand::OwnCopy { session } => {
            // The daemon copies every clone the fork shares with its parent.
            let request = pb::GiveOwnCopyRequest {
                idempotency_key: ctx.key(),
                fork: Some(ctx.session(&session).await?),
            };
            ctx.change(rpc::session::GIVE_OWN_COPY, &request).await
        }
        SessionCommand::Continue { session, before_turn } => {
            // The target is the global --channel, never the agent's own: the
            // daemon refuses the session's own channel and the Desk.
            let channel = ctx.channel_flag().await?.ok_or(CliError::Missing {
                flag: "--channel <name|id>",
            })?;
            let request = pb::ContinueInChannelRequest {
                idempotency_key: ctx.key(),
                session: Some(ctx.session(&session).await?),
                channel: Some(channel),
                before_turn: before_turn.unwrap_or(0),
            };
            ctx.change(rpc::session::CONTINUE_IN_CHANNEL, &request).await
        }
        SessionCommand::Push {
            session,
            repo,
            pr,
            from_view,
            message,
        } => push(ctx, &session, &repo, pr, from_view, message).await,
        SessionCommand::Note { session, text } => {
            let request = pb::SendNoteRequest {
                idempotency_key: ctx.key(),
                to: Some(ctx.session(&session).await?),
                text,
                reason: String::new(),
            };
            ctx.change(rpc::session::SEND_NOTE, &request).await
        }
        SessionCommand::Ask { session, text } => {
            let request = pb::AskSessionRequest {
                idempotency_key: ctx.key(),
                to: Some(ctx.session(&session).await?),
                question: text,
                reason: String::new(),
                timeout_seconds: ctx.wait_seconds(),
            };
            ctx.note(&format!("Waiting for {session} to answer…"));
            ctx.change(rpc::session::ASK_SESSION, &request).await
        }
        SessionCommand::Answer {
            question,
            answer,
            decline,
        } => {
            let request = pb::AnswerQuestionRequest {
                idempotency_key: ctx.key(),
                question: Some(pb::QuestionId { value: question }),
                answer: answer.unwrap_or_default(),
                decline,
            };
            ctx.change(rpc::session::ANSWER_QUESTION, &request).await
        }
        SessionCommand::Type {
            session,
            text,
            no_submit,
            at_turn_end,
            tab,
        } => {
            use pb::type_request::When;
            let when = if at_turn_end { When::TurnEnds } else { When::Now };
            let request = pb::TypeRequest {
                idempotency_key: ctx.key(),
                session: Some(ctx.session(&session).await?),
                tab: tab.map(|value| pb::TabId { value }),
                text: if no_submit { text } else { submitted(text) },
                when: when as i32,
            };
            if ctx.flags.json || ctx.flags.dry_run {
                return ctx.change(rpc::terminal::TYPE, &request).await;
            }
            let reply: pb::TypeResponse = ctx.call(rpc::terminal::TYPE, &request).await?;
            ctx.console.print(&typed_text(&reply));
            Ok(Done::printed())
        }
    }
}

/// `text` with Enter pressed after it (D671: a trailing newline is the
/// submit), unless it already ends in one.
/// The session a command names; clap asks for it unless `--tab` is given.
fn named(session: Option<String>) -> Result<String, CliError> {
    session.ok_or(CliError::Missing {
        flag: "<session> or --tab <tab>",
    })
}

/// Archive `session` by its plan, as `choice` says.
/// The channel id every Desk session carries (DK-2).
const DESK_CHANNEL: &str = "desk";

/// Ends `session` for good: Archive for a channel session, the Desk's Close
/// for a Desk session, which keeps its folder for Reopen (DK-11).
async fn end_for_good<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>, session: &str) -> Result<Done, CliError> {
    let id = ctx.session(session).await?;
    let found: pb::Session = ctx
        .call(rpc::session::GET_SESSION, &pb::GetSessionRequest { id: Some(id.clone()) })
        .await?;
    if found.channel.as_ref().is_some_and(|channel| channel.value == DESK_CHANNEL) {
        let request = pb::CloseDeskSessionRequest {
            idempotency_key: ctx.key(),
            session: Some(id),
            ..Default::default()
        };
        return ctx.change(rpc::desk::CLOSE_DESK_SESSION, &request).await;
    }
    archive(ctx, &id.value, pb::ArchiveChoice::Archive, true).await
}

async fn archive<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: &str,
    choice: pb::ArchiveChoice,
    end: bool,
) -> Result<Done, CliError> {
    let id = Some(ctx.session(session).await?);
    let archive = pb::ArchiveSessionRequest {
        idempotency_key: ctx.key(),
        id: id.clone(),
        choice: choice as i32,
        plan: None,
        end: end.then_some(true),
    };
    ctx.planned(
        rpc::session::PLAN_ARCHIVE,
        &pb::PlanArchiveRequest { id },
        |plan: &pb::ArchivePlan| plan.token.clone(),
        move |plan| async move {
            ctx.json(
                rpc::session::ARCHIVE_SESSION,
                &pb::ArchiveSessionRequest { plan, ..archive },
            )
            .await
        },
    )
    .await
}

fn submitted(mut text: String) -> String {
    if !text.ends_with(['\n', '\r']) {
        text.push('\n');
    }
    text
}

/// What a person is told a Type did. Text that is only typed hasn't been
/// sent. Text written mid-turn may run after the turn: the agent decides.
fn typed_text(reply: &pb::TypeResponse) -> String {
    let what = if reply.queued.is_some() {
        "queued; typed and submitted when the turn ends"
    } else if reply.submitted {
        "typed and submitted"
    } else {
        "typed, not submitted (press Enter or run again without --no-submit)"
    };
    if reply.agent_busy {
        format!("{what}\nSent while the agent was busy; it may run after the current turn")
    } else {
        what.to_owned()
    }
}

/// Refuses `session fork --channel <c>` when the parent isn't in channel
/// `c`: a fork stays in its parent's channel, and `session continue
/// --channel` is what carries a session into another (CV-8). No fork is
/// asked for then.
async fn refuse_another_channel<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    named: &str,
    parent: &pb::SessionId,
) -> Result<(), CliError> {
    let (Some(typed), Some(channel)) = (ctx.flags.channel.clone(), ctx.channel_flag().await?) else {
        return Ok(());
    };
    let mut page = String::new();
    loop {
        let reply: pb::ListSessionsResponse = ctx
            .call(
                rpc::session::LIST_SESSIONS,
                &pb::ListSessionsRequest {
                    channel: Some(channel.clone()),
                    include_archived: true,
                    page: Some(pb::PageRequest {
                        page_size: 0,
                        page_token: page.clone(),
                    }),
                    ..Default::default()
                },
            )
            .await?;
        if reply.sessions.iter().any(|session| session.id.as_ref() == Some(parent)) {
            return Ok(());
        }
        let next = reply.page.map(|page| page.next_page_token).unwrap_or_default();
        if next.is_empty() || next == page {
            return Err(CliError::ForkElsewhere {
                session: named.to_owned(),
                channel: typed,
            });
        }
        page = next;
    }
}

/// Pushes the named repos, or every repo the push plan lists.
async fn push<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    session: &str,
    repos: &[String],
    pr: bool,
    from_view: Option<String>,
    message: Option<String>,
) -> Result<Done, CliError> {
    let id = Some(ctx.session(session).await?);
    let named: Vec<pb::RepoRef> = repos
        .iter()
        .map(|repo| resolve::repo(repo))
        .collect::<Result<_, CliError>>()?;
    let choice = move |repo: pb::RepoRef| pb::PushChoice {
        repo: Some(repo),
        commit_message: message.clone().unwrap_or_default(),
        open_pull_request: pr,
        pr_title: String::new(),
        pr_body: String::new(),
        base_branch: String::new(),
    };
    let plan_request = pb::PlanPushRequest {
        id: id.clone(),
        spec: None,
    };
    let again = plan_request.clone();
    let push = pb::PushAndOpenPullRequestsRequest {
        idempotency_key: ctx.key(),
        id,
        repos: Vec::new(),
        plan: None,
        from_view: from_view.map(|value| pb::ViewId { value }),
    };
    ctx.planned(
        rpc::session::PLAN_PUSH,
        &plan_request,
        |plan: &pb::PushPlan| plan.token.clone(),
        |plan| async move {
            let repos = if named.is_empty() {
                let listed: pb::PushPlan = ctx.call(rpc::session::PLAN_PUSH, &again).await?;
                listed.repos.into_iter().filter_map(|repo| repo.repo).collect()
            } else {
                named
            };
            let request = pb::PushAndOpenPullRequestsRequest {
                repos: repos.into_iter().map(choice).collect(),
                plan,
                ..push
            };
            ctx.json(rpc::session::PUSH_AND_OPEN_PULL_REQUESTS, &request)
                .await
        },
    )
    .await
}
