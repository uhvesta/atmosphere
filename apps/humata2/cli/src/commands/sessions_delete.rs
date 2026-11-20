//! `humata session delete` for many sessions (MASSDELETE): the plan for the
//! one scope the flags name, a confirmation that says what goes, then one
//! delete against that plan's token.

use std::collections::BTreeMap;

use humata2_proto::humata::v1 as pb;

use crate::as_json;
use crate::call::{self, Ctx, Done, Outcome};
use crate::console::Console;
use crate::daemon::{Daemon, DaemonError, Rpc, rpc};
use crate::error::CliError;

/// What `session delete` was given, as its arguments name it.
pub struct Bulk {
    pub sessions: Vec<String>,
    pub channel: Option<String>,
    pub machine: Option<String>,
    pub all: bool,
    pub delete_work: bool,
    pub count: Option<u32>,
}

/// The one scope a delete covers.
#[derive(Debug, PartialEq, Eq)]
enum Scope {
    Sessions(Vec<String>),
    Channel(String),
    Machine(String),
    All,
}

pub(super) async fn run<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>, bulk: Bulk) -> Result<Done, CliError> {
    let scope = pick_scope(bulk.sessions, bulk.channel, bulk.machine, bulk.all)?;
    let all = scope == Scope::All;
    if let (false, Some(count)) = (all, bulk.count) {
        return Err(CliError::BadValue {
            field: "--count",
            value: count.to_string(),
            expected: "--all, since only --all deletes by count",
        });
    }
    if let Some(plan) = ctx.flags.plan.clone() {
        // A plan printed earlier: its sessions aren't listed again here, so
        // an --all count is only what was passed, and the daemon checks it.
        let typed_count = if all {
            settle_count(ctx, None, bulk.count)?
        } else {
            0
        };
        return execute(ctx, Some(pb::PlanToken { value: plan }), bulk.delete_work, typed_count).await;
    }

    let method = rpc::session::PLAN_DELETE_SESSIONS;
    let request = pb::PlanDeleteSessionsRequest {
        scope: Some(scope_request(ctx, &scope).await?),
    };
    let bytes = ctx.raw(method, &request).await?;
    let plan: pb::DeleteSessionsPlan = call::decode(method, &bytes)?;
    if !ctx.flags.json {
        ctx.console.print(&render_plan(&plan));
    }
    if ctx.flags.dry_run {
        if ctx.flags.json {
            return Ok(Done {
                result: as_json::encoded(method.response, &bytes),
                outcome: Outcome::Planned,
            });
        }
        if let Some(token) = &plan.token {
            ctx.note(&format!(
                "Nothing changed. Run it again with --plan {} to do this.",
                token.value
            ));
        }
        return Ok(Done::printed());
    }

    let titles = unpushed_titles(&plan);
    if !bulk.delete_work && !titles.is_empty() {
        return Err(refusal(
            rpc::session::DELETE_SESSIONS,
            pb::error::Kind::UnpushedWork(pb::UnpushedWork::default()),
            format!(
                "{} session(s) have unpushed work that deleting would lose: {}. Nothing was deleted.",
                titles.len(),
                titles.join(", ")
            ),
            "Push that work first, or add --delete-work to delete it with the sessions.".to_owned(),
        ));
    }
    let typed_count = if all {
        let planned = u32::try_from(plan.sessions.len()).unwrap_or(u32::MAX);
        settle_count(ctx, Some(planned), bulk.count)?
    } else {
        confirm_plain(ctx)?;
        0
    };
    execute(ctx, plan.token.clone(), bulk.delete_work, typed_count).await
}

/// The scope the flags name: session ids, `--channel`, `--machine` or
/// `--all`, and exactly one of them.
fn pick_scope(
    sessions: Vec<String>,
    channel: Option<String>,
    machine: Option<String>,
    all: bool,
) -> Result<Scope, CliError> {
    let mut given = Vec::new();
    if !sessions.is_empty() {
        given.push("session ids");
    }
    if channel.is_some() {
        given.push("--channel");
    }
    if machine.is_some() {
        given.push("--machine");
    }
    if all {
        given.push("--all");
    }
    match given.as_slice() {
        [] => Err(CliError::Missing {
            flag: "session ids, --channel, --machine or --all",
        }),
        [_, _, ..] => Err(CliError::BadValue {
            field: "scope",
            value: given.join(" and "),
            expected: "exactly one of session ids, --channel, --machine or --all",
        }),
        [_] => Ok(match (channel, machine) {
            (Some(channel), _) => Scope::Channel(channel),
            (None, Some(machine)) => Scope::Machine(machine),
            (None, None) if all => Scope::All,
            (None, None) => Scope::Sessions(sessions),
        }),
    }
}

/// How an `--all` delete's count is settled.
#[derive(Debug, PartialEq, Eq)]
enum Count {
    /// The count passed is the plan's, or no plan is here to check it and
    /// the daemon will.
    Matches,
    /// Passed, and not the plan's.
    Differs { planned: u32, given: u32 },
    /// Not passed, and a person is at a terminal to type it.
    Typed { planned: u32 },
    /// Not passed, and nobody can type it.
    Needed { planned: Option<u32> },
}

fn count_rule(planned: Option<u32>, given: Option<u32>, can_type: bool) -> Count {
    match (planned, given) {
        (Some(planned), Some(given)) if given != planned => Count::Differs { planned, given },
        (_, Some(_)) => Count::Matches,
        (Some(planned), None) if can_type => Count::Typed { planned },
        (planned, None) => Count::Needed { planned },
    }
}

/// The count an `--all` delete sends: the one passed, or the one typed at a
/// terminal, which must be the plan's. `--yes` doesn't skip it.
fn settle_count<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    planned: Option<u32>,
    given: Option<u32>,
) -> Result<u32, CliError> {
    match count_rule(planned, given, ctx.interactive()) {
        Count::Matches => Ok(given.unwrap_or_default()),
        Count::Differs { planned, given } => Err(count_differs(planned, &given.to_string())),
        Count::Needed { planned } => Err(count_needed(planned)),
        Count::Typed { planned } => {
            let typed = ctx
                .console
                .line("Type the number of sessions to delete: ")
                .map_err(|source| CliError::Console { source })?;
            let typed = typed.trim();
            if typed.parse::<u32>().ok() == Some(planned) {
                Ok(planned)
            } else {
                Err(count_differs(planned, typed))
            }
        }
    }
}

/// The "Continue?" after a plan: skipped by `--yes`, and refused without a
/// terminal to ask at.
fn confirm_plain<D: Daemon, C: Console>(ctx: &Ctx<'_, D, C>) -> Result<(), CliError> {
    if ctx.flags.yes {
        return Ok(());
    }
    if !ctx.interactive() {
        return Err(CliError::NeedsConfirmation);
    }
    let go = ctx
        .console
        .confirm("Continue?")
        .map_err(|source| CliError::Console { source })?;
    if go {
        Ok(())
    } else {
        Err(CliError::NotConfirmed)
    }
}

async fn scope_request<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    scope: &Scope,
) -> Result<pb::DeleteScope, CliError> {
    let scope = match scope {
        Scope::Sessions(names) => {
            let mut ids = Vec::with_capacity(names.len());
            for name in names {
                ids.push(ctx.session(name).await?);
            }
            pb::delete_scope::Scope::Sessions(pb::delete_scope::Picked { ids })
        }
        Scope::Channel(name) => pb::delete_scope::Scope::Channel(ctx.channel(name).await?),
        Scope::Machine(name) => pb::delete_scope::Scope::Machine(ctx.machine(name).await?),
        Scope::All => pb::delete_scope::Scope::All(true),
    };
    Ok(pb::DeleteScope { scope: Some(scope) })
}

async fn execute<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    plan: Option<pb::PlanToken>,
    delete_unpushed_work: bool,
    typed_count: u32,
) -> Result<Done, CliError> {
    let request = pb::DeleteSessionsRequest {
        idempotency_key: ctx.key(),
        plan,
        delete_unpushed_work,
        typed_count,
        check_only: false,
    };
    let method = rpc::session::DELETE_SESSIONS;
    let bytes = ctx.raw(method, &request).await?;
    let reply: pb::DeleteSessionsResponse = call::decode(method, &bytes)?;
    match reply.result {
        Some(pb::delete_sessions_response::Result::Changed(changed)) => {
            Err(changed_refusal(ctx, changed))
        }
        _ => Ok(Done::reply(as_json::encoded(method.response, &bytes))),
    }
}

/// A `changed` answer: nothing was deleted, and the changes are said.
fn changed_refusal<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    changed: pb::delete_sessions_response::Changed,
) -> CliError {
    let pb::delete_sessions_response::Changed { changes, .. } = changed;
    if !changes.is_empty() {
        ctx.note(&format!("Changed since the plan:\n{}", changes.join("\n")));
    }
    let fix = if ctx.flags.plan.is_some() {
        "Run it again without --plan to see the new plan, then confirm it."
    } else {
        "Run the same command again to see the new plan, then confirm it."
    };
    refusal(
        rpc::session::DELETE_SESSIONS,
        pb::error::Kind::PlanStale(pb::PlanStale { changes }),
        "the sessions changed since the plan; nothing was deleted.".to_owned(),
        fix.to_owned(),
    )
}

fn count_needed(planned: Option<u32>) -> CliError {
    let (message, fix) = match planned {
        Some(count) => (
            format!("deleting all {count} sessions needs --count {count} to confirm; nothing was deleted."),
            format!("Run it again with --count {count}, or with --dry-run to see the plan first."),
        ),
        None => (
            "deleting every session in the plan needs --count with the number that plan showed; nothing was deleted."
                .to_owned(),
            "Run it again with --count N, where N is the number the plan showed.".to_owned(),
        ),
    };
    refusal(
        rpc::session::DELETE_SESSIONS,
        invalid_field("--count"),
        message,
        fix,
    )
}

fn count_differs(planned: u32, typed: &str) -> CliError {
    refusal(
        rpc::session::DELETE_SESSIONS,
        invalid_field("--count"),
        format!("the plan lists {planned} sessions, not {typed}; nothing was deleted."),
        format!("Pass --count {planned} to delete them all, or run with --dry-run to see the plan again."),
    )
}

fn invalid_field(field: &str) -> pb::error::Kind {
    pb::error::Kind::InvalidField(pb::InvalidField {
        field: field.to_owned(),
        ..Default::default()
    })
}

/// A refusal the CLI makes itself, in the daemon's shape, so it exits and
/// prints as any refused call does: `kind` gives the name and the exit code.
fn refusal(rpc: Rpc, kind: pb::error::Kind, message: String, fix: String) -> CliError {
    CliError::Daemon(DaemonError::Refused {
        rpc: rpc.path,
        code: tonic::Code::FailedPrecondition as i32,
        error: Box::new(pb::Error {
            message,
            fix,
            kind: Some(kind),
            ..Default::default()
        }),
    })
}

/// The sessions whose unpushed work a delete would lose.
fn unpushed_titles(plan: &pb::DeleteSessionsPlan) -> Vec<&str> {
    plan.sessions
        .iter()
        .filter(|session| !session.unpushed.is_empty())
        .map(|session| session.title.as_str())
        .collect()
}

/// The plan for a person: the sessions grouped by channel, then what goes
/// along, what stays, what is left alone and what the Desk keeps.
fn render_plan(plan: &pb::DeleteSessionsPlan) -> String {
    let mut lines = vec![format!("{} session(s) to delete:", plan.sessions.len())];
    let mut by_channel: BTreeMap<&str, Vec<&pb::SessionToDelete>> = BTreeMap::new();
    for session in &plan.sessions {
        by_channel
            .entry(channel_id(session.channel.as_ref()))
            .or_default()
            .push(session);
    }
    for (channel, sessions) in &by_channel {
        lines.push(format!("Channel {channel}"));
        for session in sessions {
            lines.push(format!(
                "  {} ({}): {}",
                session.title,
                session_id(session.session.as_ref()),
                terminals(session.running_terminals)
            ));
            for repo in &session.unpushed {
                lines.push(format!("    would lose unpushed work in {}", unpushed_repo(repo)));
            }
        }
    }

    let going: Vec<&pb::SessionToDelete> = plan
        .sessions
        .iter()
        .filter(|session| session.goes_with.is_some())
        .collect();
    if !going.is_empty() {
        lines.push("Context forks that go along (they work in a session's folder):".to_owned());
        for session in going {
            lines.push(format!(
                "  {} ({}) goes with {}",
                session.title,
                session_id(session.session.as_ref()),
                session_id(session.goes_with.as_ref())
            ));
        }
    }
    if !plan.forks_staying.is_empty() {
        lines.push("Code forks that stay (they keep their folder and run on):".to_owned());
        for work in &plan.forks_staying {
            lines.push(format!(
                "  {} ({}): {}",
                work.title,
                session_id(work.session.as_ref()),
                work_summary(work.dirty_files, work.unpushed_commits)
            ));
        }
    }
    if !plan.left_alone.is_empty() {
        lines.push("Machines left alone (offline, nothing on them is deleted):".to_owned());
        for machine in &plan.left_alone {
            lines.push(format!(
                "  {}: {} session(s) not deleted",
                machine_id(machine.machine.as_ref()),
                machine.sessions.len()
            ));
        }
    }
    if plan.desk_sessions_left_out > 0 {
        lines.push(format!(
            "Desk sessions left out: {} (the Desk's Clean up deletes them)",
            plan.desk_sessions_left_out
        ));
    }
    lines.join("\n")
}

fn session_id(id: Option<&pb::SessionId>) -> &str {
    id.map_or("", |id| id.value.as_str())
}

fn channel_id(id: Option<&pb::ChannelId>) -> &str {
    id.map_or("", |id| id.value.as_str())
}

fn machine_id(id: Option<&pb::MachineId>) -> &str {
    id.map_or("", |id| id.value.as_str())
}

fn terminals(count: u32) -> String {
    match count {
        0 => "no terminals end".to_owned(),
        1 => "1 terminal ends".to_owned(),
        count => format!("{count} terminals end"),
    }
}

fn unpushed_repo(repo: &pb::UnpushedRepo) -> String {
    let name = repo
        .repo
        .as_ref()
        .map_or_else(String::new, |repo| format!("{}/{}", repo.owner, repo.name));
    let state = if repo.never_pushed {
        "never pushed".to_owned()
    } else {
        format!("{} unpushed commit(s)", repo.unpushed_commits)
    };
    let mut parts = vec![state];
    if repo.dirty_files > 0 {
        parts.push(format!("{} uncommitted file(s)", repo.dirty_files));
    }
    format!("{name} on {}: {}", repo.branch, parts.join(", "))
}

fn work_summary(dirty_files: u32, unpushed_commits: u32) -> String {
    format!("{unpushed_commits} unpushed commit(s), {dirty_files} uncommitted file(s)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, title: &str, channel: &str) -> pb::SessionToDelete {
        pb::SessionToDelete {
            session: Some(pb::SessionId { value: id.into() }),
            title: title.into(),
            channel: Some(pb::ChannelId {
                value: channel.into(),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn each_scope_flag_picks_its_scope() {
        assert_eq!(
            pick_scope(vec!["se_1".into()], None, None, false).ok(),
            Some(Scope::Sessions(vec!["se_1".into()]))
        );
        assert_eq!(
            pick_scope(Vec::new(), Some("ch_1".into()), None, false).ok(),
            Some(Scope::Channel("ch_1".into()))
        );
        assert_eq!(
            pick_scope(Vec::new(), None, Some("ma_1".into()), false).ok(),
            Some(Scope::Machine("ma_1".into()))
        );
        assert_eq!(pick_scope(Vec::new(), None, None, true).ok(), Some(Scope::All));
    }

    #[test]
    fn no_scope_is_missing() {
        assert!(matches!(
            pick_scope(Vec::new(), None, None, false),
            Err(CliError::Missing { .. })
        ));
    }

    #[test]
    fn two_scopes_at_once_are_refused() {
        assert!(matches!(
            pick_scope(vec!["se_1".into()], Some("ch_1".into()), None, false),
            Err(CliError::BadValue { .. })
        ));
        assert!(matches!(
            pick_scope(Vec::new(), Some("ch_1".into()), None, true),
            Err(CliError::BadValue { .. })
        ));
    }

    #[test]
    fn an_all_delete_with_a_matching_count_needs_no_typing() {
        assert_eq!(count_rule(Some(3), Some(3), false), Count::Matches);
    }

    #[test]
    fn a_count_that_is_not_the_plans_is_refused() {
        assert_eq!(
            count_rule(Some(3), Some(2), false),
            Count::Differs { planned: 3, given: 2 }
        );
    }

    #[test]
    fn a_non_interactive_all_without_a_count_needs_one() {
        assert_eq!(
            count_rule(Some(3), None, false),
            Count::Needed { planned: Some(3) }
        );
    }

    #[test]
    fn at_a_terminal_without_a_count_the_count_is_typed() {
        assert_eq!(count_rule(Some(3), None, true), Count::Typed { planned: 3 });
    }

    #[test]
    fn with_no_plan_here_the_passed_count_goes_to_the_daemon() {
        assert_eq!(count_rule(None, Some(3), false), Count::Matches);
        assert_eq!(count_rule(None, None, false), Count::Needed { planned: None });
    }

    #[test]
    fn sessions_with_unpushed_work_are_listed_for_the_refusal() {
        let mut risky = session("se_2", "Add tests", "ch_1");
        risky.unpushed = vec![pb::UnpushedRepo {
            unpushed_commits: 2,
            ..Default::default()
        }];
        let plan = pb::DeleteSessionsPlan {
            sessions: vec![session("se_1", "Fix login", "ch_1"), risky],
            ..Default::default()
        };
        assert_eq!(unpushed_titles(&plan), vec!["Add tests"]);
    }

    #[test]
    fn the_plan_names_each_session_and_the_machine_left_alone() {
        let plan = pb::DeleteSessionsPlan {
            sessions: vec![
                session("se_1", "Fix login", "ch_1"),
                session("se_2", "Add tests", "ch_2"),
            ],
            left_alone: vec![pb::LeftAlone {
                machine: Some(pb::MachineId {
                    value: "ma_offline".into(),
                }),
                sessions: Vec::new(),
            }],
            ..Default::default()
        };
        let text = render_plan(&plan);
        assert!(text.contains("Fix login"), "{text}");
        assert!(text.contains("Add tests"), "{text}");
        assert!(text.contains("ma_offline"), "{text}");
    }
}
