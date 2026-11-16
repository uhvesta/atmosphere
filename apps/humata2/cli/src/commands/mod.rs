//! Each command as calls on the daemon (CLI.md §5).

mod boards;
mod desk;
mod files;
mod hook;
mod hosts;
mod inbox;
mod login;
mod looking;
mod opencode;
mod search;
mod sessions;
mod settings;
mod streams;
mod turn;

use humata2_proto::humata::v1 as pb;
use prost::Message;
use serde_json::Value as Json;

use crate::args::*;
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, Rpc};
use crate::error::CliError;

/// Every page of a list, as one reply: the first page's fields with
/// `field`'s items from all of them, and no `page`.
async fn pages<D: Daemon, C: Console, Req: Message>(
    ctx: &Ctx<'_, D, C>,
    rpc: Rpc,
    field: &str,
    request: impl Fn(Option<pb::PageRequest>) -> Req,
) -> Result<Json, CliError> {
    let mut token = String::new();
    let mut all: Option<Json> = None;
    loop {
        let page = pb::PageRequest {
            page_size: 0,
            page_token: token.clone(),
        };
        let mut reply = ctx.json(rpc, &request(Some(page))).await?;
        let next = reply
            .get("page")
            .and_then(|page| page.get("next_page_token"))
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Json::Object(fields) = &mut reply {
            fields.remove("page");
        }
        match &mut all {
            None => all = Some(reply),
            Some(all) => {
                if let (Some(Json::Array(more)), Some(Json::Array(have))) =
                    (reply.get_mut(field).map(Json::take), all.get_mut(field))
                {
                    have.extend(more);
                }
            }
        }
        if next.is_empty() || next == token {
            break;
        }
        token = next;
    }
    Ok(all.unwrap_or(Json::Null))
}

/// One call, its reply as the result.
async fn reply<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    rpc: Rpc,
    request: &impl Message,
) -> Result<Done, CliError> {
    ctx.json(rpc, request).await.map(Done::reply)
}

/// A secret: typed with echo off in a terminal, else all of stdin less one
/// trailing newline, so it never has to be on the command line.
fn secret<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>, prompt: &str) -> Result<String, CliError> {
    if ctx.interactive() {
        return ctx
            .console
            .secret(prompt)
            .map_err(|source| CliError::Console { source });
    }
    let bytes = ctx
        .console
        .stdin()
        .map_err(|source| CliError::Console { source })?;
    let text = String::from_utf8(bytes).map_err(|_| CliError::BadValue {
        field: "stdin",
        value: "<not UTF-8>".into(),
        expected: "UTF-8 text",
    })?;
    let text = text.strip_suffix('\n').unwrap_or(&text);
    Ok(text.strip_suffix('\r').unwrap_or(text).to_owned())
}

pub async fn run<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>, command: Command) -> Result<Done, CliError> {
    match command {
        Command::Status => looking::status(ctx).await,
        Command::Whoami => looking::whoami(ctx).await,
        Command::Ls { all } => looking::ls(ctx, all).await,
        Command::Channels => looking::channels(ctx).await,
        Command::Channel(command) => looking::channel(ctx, command).await,
        Command::Topology => looking::topology(ctx).await,
        Command::Activity => looking::activity(ctx).await,
        Command::Events { after, topic } => looking::events(ctx, after, topic).await,
        Command::Audit => looking::audit(ctx).await,
        Command::Ops { id, command } => looking::ops(ctx, id, command).await,
        Command::Attach {
            session,
            tab,
            read_only,
        } => sessions::attach(ctx, &session, tab, read_only).await,
        Command::Session(command) => sessions::session(ctx, command).await,
        Command::Shell { machine } => sessions::shell(ctx, &machine).await,
        Command::Conversation { session } => sessions::conversation(ctx, &session).await,
        Command::Board {
            which,
            column,
            command,
        } => boards::board(ctx, which, column, command).await,
        Command::Views { session } => boards::views(ctx, session).await,
        Command::View { action, view } => boards::view(ctx, action, view).await,
        Command::Diff {
            session,
            repo,
            against,
            command,
        } => boards::diff(ctx, session, repo, against, command).await,
        Command::Comment(command) => boards::comment(ctx, command).await,
        Command::Review(command) => boards::review(ctx, command).await,
        Command::Inbox {
            all,
            session,
            with_forks,
            command,
        } => inbox::inbox(ctx, all, session, with_forks, command).await,
        Command::Access { command } => inbox::access(ctx, command).await,
        Command::Login { hours } => login::login(ctx, hours).await,
        Command::Logout => login::logout(ctx).await,
        Command::Logins => inbox::logins(ctx).await,
        Command::Token(command) => inbox::token(ctx, command).await,
        Command::Files(command) => files::files(ctx, command).await,
        Command::Cp(args) => files::transfer_start(ctx, args, false).await,
        Command::Mv(args) => files::transfer_start(ctx, args, true).await,
        Command::Transfers { active } => files::transfers(ctx, active).await,
        Command::Transfer(command) => files::transfer(ctx, command).await,
        Command::Trash { command } => files::trash(ctx, command).await,
        Command::Host(command) => hosts::host(ctx, command).await,
        Command::Provider(command) => hosts::provider(ctx, command).await,
        Command::Skills(command) => hosts::skills(ctx, command).await,
        Command::Bridge(command) => hosts::bridge(ctx, command).await,
        Command::Settings { command } => hosts::settings(ctx, command).await,
        Command::Agents(command) => hosts::agents(ctx, command).await,
        Command::Pin { thing } => hosts::mark(ctx, &thing, Some(true), None).await,
        Command::Unpin { thing } => hosts::mark(ctx, &thing, Some(false), None).await,
        Command::Vip { thing } => hosts::mark(ctx, &thing, None, Some(true)).await,
        Command::Unvip { thing } => hosts::mark(ctx, &thing, None, Some(false)).await,
        Command::Pinned { command } => hosts::pinned(ctx, command).await,
        Command::Sessions(command) => search::sessions(ctx, command).await,
        Command::Desk(command) => desk::desk(ctx, command).await,
        Command::Report(command) => hosts::report(ctx, command).await,
        Command::Turn { state } => turn::turn(ctx, state, std::time::SystemTime::now()).await,
        Command::Hook { agent, event } => hook::hook(ctx, &agent, event).await,
        Command::Opencode {
            conversation,
            state,
            program,
        } => opencode::opencode(ctx, conversation, state, program).await,
    }
}

/// The operation id a command is, as MCP, audit and history name it.
pub fn operation(command: &Command) -> &'static str {
    match command {
        Command::Status => "daemon.status",
        Command::Whoami => "daemon.whoami",
        Command::Ls { .. } => "session.list",
        Command::Channels => "channel.list",
        Command::Channel(command) => match command {
            ChannelCommand::Create { .. } => "channel.create",
            ChannelCommand::Rename { .. } => "channel.rename",
            ChannelCommand::Repo(ChannelRepoCommand::Add { .. }) => "channel.add_repo",
            ChannelCommand::Repo(ChannelRepoCommand::Rm { .. }) => "channel.remove_repo",
            ChannelCommand::Machine(ChannelMachineCommand::Add { .. }) => "channel.add_machine",
            ChannelCommand::Machine(ChannelMachineCommand::Rm { .. }) => "channel.remove_machine",
            ChannelCommand::Grid { .. } => "channel.grid",
            ChannelCommand::Settings {
                command: Some(KeyCommand::Set { .. }),
                ..
            } => "channel_settings.update",
            ChannelCommand::Settings { .. } => "channel_settings.get",
            ChannelCommand::Delete { .. } => "channel.delete",
        },
        Command::Topology => "channel.topology",
        Command::Activity => "activity.list",
        Command::Events { .. } => "events.subscribe",
        Command::Audit => "audit.list",
        Command::Ops { command, id } => match (command, id) {
            (Some(OpsCommand::Cancel { .. }), _) => "operation.cancel",
            (Some(OpsCommand::Retry { .. }), _) => "operation.retry",
            (None, Some(_)) => "operation.get",
            (None, None) => "operation.list",
        },
        Command::Attach { read_only, .. } => {
            if *read_only {
                "terminal.watch"
            } else {
                "terminal.attach"
            }
        }
        Command::Session(command) => match command {
            SessionCommand::Queue { .. } => "terminal.list_queued",
            SessionCommand::SendNow { .. } => "terminal.send_queued_now",
            SessionCommand::Start { .. } => "session.start",
            SessionCommand::Fork { .. } => "session.fork",
            SessionCommand::Rename { .. } => "session.rename",
            SessionCommand::Stop { .. } => "session.stop",
            SessionCommand::Resume { .. } => "session.resume",
            SessionCommand::Archive { .. } => "session.archive",
            SessionCommand::Delete { .. } => "session.delete",
            SessionCommand::Restore { .. } => "session.restore",
            SessionCommand::Rebuild { .. } => "session.rebuild",
            SessionCommand::Move { .. } => "session.move",
            SessionCommand::Repo(SessionRepoCommand::Add { .. }) => "session.add_repo",
            SessionCommand::Repo(SessionRepoCommand::Drop { .. }) => "session.drop_repo",
            SessionCommand::OwnCopy { .. } => "session.own_copy",
            SessionCommand::Push { .. } => "session.push",
            SessionCommand::Note { .. } => "session.note",
            SessionCommand::Ask { .. } => "session.ask",
            SessionCommand::Type { .. } => "session.type",
            SessionCommand::Read { .. } => "session.read",
        },
        Command::Shell { .. } => "machine.shell",
        Command::Conversation { .. } => "session.conversation",
        Command::Board { command, .. } => match command {
            None => "board.items_list",
            Some(BoardCommand::Add { .. }) => "board.item_create",
            Some(BoardCommand::Edit { .. }) => "board.item_update",
            Some(BoardCommand::Move { .. }) => "board.item_move",
            Some(BoardCommand::Rm { .. }) => "board.item_delete",
            Some(BoardCommand::History { .. }) => "board.item_history",
        },
        Command::Views { .. } => "view.list",
        Command::View { .. } => "view.set_state",
        Command::Diff { command, .. } => match command {
            None => "diff.get",
            Some(DiffCommand::MarkReviewed { .. }) => "diff.mark_reviewed",
        },
        Command::Comment(command) => match command {
            CommentCommand::Add { .. } => "comment.create",
            CommentCommand::Send { .. } => "comment.send",
            CommentCommand::Ls { .. } => "comment.list",
            CommentCommand::Rm { .. } => "comment.drop",
        },
        Command::Review(command) => match command {
            ReviewCommand::Start { .. } => "review.start",
            ReviewCommand::Drafts { .. } => "review.list_drafts",
            ReviewCommand::Submit { .. } => "review.submit",
            ReviewCommand::AllowPush { .. } => "review.set_push_allowed",
        },
        Command::Inbox { command, .. } => match command {
            None => "inbox.list",
            Some(InboxCommand::Show { .. }) => "inbox.get",
            Some(
                InboxCommand::Approve { .. }
                | InboxCommand::Deny { .. }
                | InboxCommand::Reply { .. },
            ) => "inbox.answer",
            Some(InboxCommand::ApproveAll | InboxCommand::DenyAll) => "inbox.answer_all",
        },
        Command::Access { command } => match command {
            None => "access.get_rules",
            Some(AccessCommand::Set { .. }) => "access.set_rule",
            Some(AccessCommand::Away { .. }) => "access.set_away_rule",
            Some(AccessCommand::Grants) => "access.grants",
            Some(AccessCommand::Revoke { .. }) => "access.revoke_grant",
            Some(AccessCommand::Why { .. }) => "access.explain",
            Some(AccessCommand::Passphrase { .. }) => "access.set_passphrase",
            Some(AccessCommand::AskAhead { .. }) => "inbox.request",
        },
        Command::Login { .. } => "cli.login",
        Command::Logout => "cli.logout",
        Command::Logins => "cli.logins",
        Command::Token(command) => match command {
            TokenCommand::Create { .. } => "cli.token_create",
            TokenCommand::Ls => "cli.token_list",
            TokenCommand::Revoke { .. } => "cli.token_revoke",
        },
        Command::Files(command) => match command {
            FilesCommand::Ls { .. } => "file.list",
            FilesCommand::Cat { .. } => "file.read",
            FilesCommand::Find { .. } => "file.find",
            FilesCommand::Put { .. } => "file.write",
            FilesCommand::Mkdir { .. } => "file.folder_create",
            FilesCommand::Mv { .. } => "file.rename",
            FilesCommand::Rm { .. } => "file.trash",
        },
        Command::Cp(_) | Command::Mv(_) => "transfer.start",
        Command::Transfers { .. } => "transfer.list",
        Command::Transfer(command) => match command {
            TransferCommand::Pause { .. } => "transfer.pause",
            TransferCommand::Resume { .. } => "transfer.resume",
            TransferCommand::Cancel { .. } => "transfer.cancel",
        },
        Command::Trash { command } => match command {
            None => "trash.list",
            Some(TrashCommand::Restore { .. }) => "trash.restore",
            Some(TrashCommand::Empty) => "trash.empty",
        },
        Command::Host(command) => match command {
            HostCommand::Ls => "machine.list",
            HostCommand::Show { .. } => "machine.get",
            HostCommand::Add { .. } => "machine.add_ssh",
            HostCommand::Reconnect { .. } => "machine.reconnect",
            HostCommand::Remove { .. } => "machine.remove",
            HostCommand::Forget { .. } => "machine.forget",
            HostCommand::Root { .. } => "machine.change_root",
            HostCommand::VmTier { .. } => "machine.set_vm_tier",
            HostCommand::Storage { .. } => "machine.storage",
            HostCommand::ClearBuildCache { .. } => "machine.clear_build_cache",
            HostCommand::TrimLogs { .. } => "machine.trim_logs",
            HostCommand::Vars { .. } => "host.list_variables",
            HostCommand::Var(VarCommand::Set { .. }) => "host.set_variable",
            HostCommand::Var(VarCommand::Rm { .. }) => "host.remove_variable",
            HostCommand::TrustKey { .. } => "host.trust_new_key",
        },
        Command::Provider(command) => match command {
            ProviderCommand::Ls => "providers.list",
            ProviderCommand::Config(ProviderConfigCommand::Show) => "providers.config_get",
            ProviderCommand::Config(ProviderConfigCommand::Edit { .. }) => "providers.config_set",
            ProviderCommand::Import { .. } => "provider.import",
            ProviderCommand::Create { .. } => "provider.create",
            ProviderCommand::Start { .. } => "provider.start",
            ProviderCommand::Stop { .. } => "provider.stop",
            ProviderCommand::Status { .. } => "provider.status",
            ProviderCommand::Delete { .. } => "provider.delete",
            ProviderCommand::Login { .. } => "provider.login",
        },
        Command::Skills(SkillsCommand::Sync { .. }) => "skills.sync",
        Command::Bridge(command) => match command {
            BridgeCommand::Ls { .. } => "bridge.list",
            BridgeCommand::Close { .. } => "bridge.close",
        },
        Command::Settings { command } => match command {
            None | Some(SettingsCommand::Get { .. }) => "settings.get",
            Some(SettingsCommand::Set { .. }) => "settings.update",
            Some(SettingsCommand::Contain { .. }) => "settings.update",
        },
        Command::Agents(command) => match command {
            AgentsCommand::Ls => "agents.list",
            AgentsCommand::Add { .. } => "agents.add",
            AgentsCommand::Edit { .. } => "agents.update",
        },
        Command::Pin { .. } | Command::Unpin { .. } | Command::Vip { .. } | Command::Unvip { .. } => {
            "marks.set"
        }
        Command::Pinned { command } => match command {
            None => "pinned.list",
            Some(PinnedCommand::Reorder { .. }) => "pinned.reorder",
        },
        Command::Sessions(SessionsCommand::Search { .. }) => "session.search",
        Command::Desk(DeskCommand::Keep { .. }) => "desk.keep_folder",
        Command::Report(command) => match command {
            ReportCommand::Record { .. } => "report.record",
            ReportCommand::Stop => "report.stop",
            ReportCommand::Snapshot { .. } => "report.snapshot",
        },
        Command::Turn { .. } => "session.report_turn",
        Command::Hook { .. } => "session.hook",
        Command::Opencode { .. } => "session.opencode",
    }
}

/// Enum values the protocol takes for the CLI's own value names.
pub(crate) mod values {
    use humata2_proto::humata::v1 as pb;

    use crate::args;

    pub fn column(column: args::Column) -> pb::BoardColumn {
        match column {
            args::Column::Backlog => pb::BoardColumn::Backlog,
            args::Column::ToDo => pb::BoardColumn::ToDo,
            args::Column::InProgress => pb::BoardColumn::InProgress,
            args::Column::InReview => pb::BoardColumn::InReview,
            args::Column::Done => pb::BoardColumn::Done,
        }
    }

    pub fn on(state: args::OnOff) -> bool {
        state == args::OnOff::On
    }

    pub fn fork_kind(kind: args::ForkKind) -> pb::ForkKind {
        match kind {
            args::ForkKind::CodeAndContext => pb::ForkKind::CodeAndContext,
            args::ForkKind::CodeOnly => pb::ForkKind::CodeOnly,
            args::ForkKind::ContextOnly => pb::ForkKind::ContextOnly,
        }
    }

    pub fn view_state(action: args::ViewAction) -> pb::ViewState {
        match action {
            args::ViewAction::Open => pb::ViewState::Opened,
            args::ViewAction::Later => pb::ViewState::Later,
            args::ViewAction::NotNow => pb::ViewState::NotNow,
            args::ViewAction::Revert => pb::ViewState::Reverted,
            args::ViewAction::Keep => pb::ViewState::Kept,
        }
    }

    pub fn verdict(verdict: args::Verdict) -> pb::Verdict {
        match verdict {
            args::Verdict::Comment => pb::Verdict::Comment,
            args::Verdict::Approve => pb::Verdict::Approve,
            args::Verdict::RequestChanges => pb::Verdict::RequestChanges,
        }
    }

    pub fn decision(decision: args::Decision) -> pb::Decision {
        match decision {
            args::Decision::Allow => pb::Decision::Allow,
            args::Decision::Ask => pb::Decision::Ask,
            args::Decision::Never => pb::Decision::Never,
        }
    }

    pub fn away(away: args::Away) -> pb::AwayMode {
        match away {
            args::Away::Wait => pb::AwayMode::Wait,
            args::Away::Skip => pb::AwayMode::Skip,
            args::Away::LowRisk => pb::AwayMode::AllowLowRisk,
        }
    }

    pub fn on_existing(choice: args::OnExisting) -> pb::OnExisting {
        match choice {
            args::OnExisting::Fail => pb::OnExisting::Fail,
            args::OnExisting::Replace => pb::OnExisting::Replace,
            args::OnExisting::KeepBoth => pb::OnExisting::KeepBoth,
        }
    }

    pub fn topic(topic: args::Topic) -> pb::Topic {
        match topic {
            args::Topic::Sessions => pb::Topic::Sessions,
            args::Topic::Channels => pb::Topic::Channels,
            args::Topic::Machines => pb::Topic::Machines,
            args::Topic::Boards => pb::Topic::Boards,
            args::Topic::Inbox => pb::Topic::Inbox,
            args::Topic::Access => pb::Topic::Access,
            args::Topic::Comments => pb::Topic::Comments,
            args::Topic::Views => pb::Topic::Views,
            args::Topic::Operations => pb::Topic::Operations,
            args::Topic::Transfers => pb::Topic::Transfers,
            args::Topic::Activity => pb::Topic::Activity,
            args::Topic::Marks => pb::Topic::Marks,
            args::Topic::Settings => pb::Topic::Settings,
            args::Topic::Journal => pb::Topic::Journal,
        }
    }
}

/// Text from the command line as the protocol's values.
pub(crate) mod parse {
    use humata2_proto::humata::v1 as pb;

    use crate::error::CliError;

    /// `owner/repo#12` or `https://github.com/owner/repo/pull/12`.
    pub fn pull_request(text: &str) -> Result<pb::PullRequestRef, CliError> {
        let bad = || CliError::BadValue {
            field: "pull request",
            value: text.into(),
            expected: "owner/repo#12 or https://github.com/owner/repo/pull/12",
        };
        let (repo, number) = if let Some(path) = text
            .strip_prefix("https://github.com/")
            .or_else(|| text.strip_prefix("http://github.com/"))
        {
            let mut parts = path.split('/');
            let owner = parts.next().unwrap_or_default();
            let name = parts.next().unwrap_or_default();
            if parts.next() != Some("pull") {
                return Err(bad());
            }
            let number = parts.next().unwrap_or_default();
            (format!("{owner}/{name}"), number.to_owned())
        } else {
            let (repo, number) = text.split_once('#').ok_or_else(bad)?;
            (repo.to_owned(), number.to_owned())
        };
        let repo = crate::resolve::repo(&repo).map_err(|_| bad())?;
        let number = number.parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(bad)?;
        Ok(pb::PullRequestRef {
            repo: Some(repo),
            number,
        })
    }

    /// `owner/repo:path:12-20`, or `:12` for one line.
    pub fn anchor(text: &str) -> Result<pb::CommentAnchor, CliError> {
        let bad = || CliError::BadValue {
            field: "comment location",
            value: text.into(),
            expected: "<owner/repo>:<path>:<first>-<last>",
        };
        let (rest, lines) = text.rsplit_once(':').ok_or_else(bad)?;
        let (repo, path) = rest.split_once(':').ok_or_else(bad)?;
        let (first, last) = lines.split_once('-').unwrap_or((lines, lines));
        let first = first.parse::<u32>().map_err(|_| bad())?;
        let last = last.parse::<u32>().map_err(|_| bad())?;
        if path.is_empty() || first == 0 || last < first {
            return Err(bad());
        }
        Ok(pb::CommentAnchor {
            file: Some(pb::RepoPath {
                repo: Some(crate::resolve::repo(repo)?),
                path: path.into(),
            }),
            lines: Some(pb::LineRange { first, last }),
            side: pb::DiffSide::New as i32,
            ..Default::default()
        })
    }

    /// `key=value`.
    pub fn pair(text: &str) -> Result<(String, String), CliError> {
        match text.split_once('=') {
            Some((key, value)) if !key.is_empty() => Ok((key.into(), value.into())),
            _ => Err(CliError::BadValue {
                field: "--param",
                value: text.into(),
                expected: "key=value",
            }),
        }
    }

    pub fn on_off(field: &'static str, text: &str) -> Result<bool, CliError> {
        match text {
            "on" | "true" | "yes" => Ok(true),
            "off" | "false" | "no" => Ok(false),
            _ => Err(CliError::BadValue {
                field,
                value: text.into(),
                expected: "on or off",
            }),
        }
    }

    pub fn number<T: std::str::FromStr>(field: &'static str, text: &str) -> Result<T, CliError> {
        text.parse().map_err(|_| CliError::BadValue {
            field,
            value: text.into(),
            expected: "a number",
        })
    }
}
