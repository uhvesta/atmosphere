//! `humata machine script list|check|run|log` (CLI.md §5.6, MACHINESCRIPTS,
//! D687): what the rules file holds, whether it is valid, running a rule now
//! and the host's run log. Nothing here writes rules or scripts.

use humata2_proto::humata::v1 as pb;

use super::hosts::follow;
use super::reply;
use crate::args::ScriptCommand;
use crate::as_json;
use crate::call::{Ctx, Done};
use crate::console::Console;
use crate::daemon::{Daemon, rpc};
use crate::error::CliError;

pub(super) async fn script<D: Daemon, C: Console>(
    ctx: &Ctx<'_, D, C>,
    command: ScriptCommand,
) -> Result<Done, CliError> {
    match command {
        ScriptCommand::List { machine } => {
            let machine = match machine {
                Some(name) => Some(ctx.machine(&name).await?),
                None => None,
            };
            let request = pb::ListMachineScriptsRequest { machine };
            if ctx.flags.json {
                return reply(ctx, rpc::machine::LIST_MACHINE_SCRIPTS, &request).await;
            }
            let listed: pb::ListMachineScriptsResponse =
                ctx.call(rpc::machine::LIST_MACHINE_SCRIPTS, &request).await?;
            if let Some(problem) = &listed.problem {
                ctx.note(&problem.message);
                ctx.note(&problem.fix);
            }
            ctx.console.print(&list_text(&listed));
            Ok(Done::printed())
        }
        ScriptCommand::Check => {
            let checked: pb::CheckMachineScriptsResponse = ctx
                .call(
                    rpc::machine::CHECK_MACHINE_SCRIPTS,
                    &pb::CheckMachineScriptsRequest {},
                )
                .await?;
            if !checked.valid {
                return Err(CliError::ScriptsInvalid {
                    problems: checked.problems,
                    warnings: checked.warnings,
                });
            }
            for warning in &checked.warnings {
                ctx.note(warning);
            }
            Ok(Done::reply(as_json::typed(
                "humata.v1.CheckMachineScriptsResponse",
                &checked,
            )))
        }
        ScriptCommand::Run {
            rule,
            machine,
            session,
        } => {
            let session = match session {
                Some(session) => Some(ctx.session(&session).await?),
                None => None,
            };
            let request = pb::RunMachineScriptRequest {
                idempotency_key: ctx.key(),
                machine: Some(ctx.machine(&machine).await?),
                rule,
                session,
            };
            if ctx.flags.dry_run {
                return ctx.change(rpc::machine::RUN_MACHINE_SCRIPT, &request).await;
            }
            let started = ctx.raw(rpc::machine::RUN_MACHINE_SCRIPT, &request).await?;
            follow(ctx, rpc::machine::RUN_MACHINE_SCRIPT, &started)
                .await
                .map(Done::reply)
        }
        ScriptCommand::Log {
            machine,
            rule,
            limit,
        } => {
            let request = pb::ListMachineScriptRunsRequest {
                machine: Some(ctx.machine(&machine).await?),
                rule: rule.unwrap_or_default(),
                limit: limit.unwrap_or_default(),
            };
            if ctx.flags.json {
                return reply(ctx, rpc::machine::LIST_MACHINE_SCRIPT_RUNS, &request).await;
            }
            let logged: pb::ListMachineScriptRunsResponse =
                ctx.call(rpc::machine::LIST_MACHINE_SCRIPT_RUNS, &request).await?;
            ctx.console.print(&log_text(&logged.runs));
            Ok(Done::printed())
        }
    }
}

/// The rules, then each rule's last run per machine, then the script names.
fn list_text(listed: &pb::ListMachineScriptsResponse) -> String {
    let mut rules = vec![row(&["RULE", "ON", "MACHINES", "ONCE", "STEPS"])];
    for rule in &listed.rules {
        let machines = if rule.machines.is_empty() {
            "all".to_owned()
        } else {
            rule.machines.join(",")
        };
        let steps: Vec<String> = rule
            .steps
            .iter()
            .map(|step| format!("{} {}", step.kind, step.label))
            .collect();
        rules.push(vec![
            rule.name.clone(),
            rule.triggers.join(","),
            machines,
            if rule.once { "yes" } else { "no" }.to_owned(),
            steps.join("; "),
        ]);
    }
    let mut out = table(&rules);
    if !listed.last_runs.is_empty() {
        let mut runs = vec![row(&["RULE", "MACHINE", "LAST RUN", "STARTED", "TOOK"])];
        for run in &listed.last_runs {
            runs.push(vec![
                run.rule.clone(),
                machine_text(run),
                outcome_text(run.outcome).to_owned(),
                started_text(run),
                took_text(run.took_ms),
            ]);
        }
        out.push_str("\n\n");
        out.push_str(&table(&runs));
    }
    if !listed.scripts.is_empty() {
        out.push_str("\n\nscripts: ");
        out.push_str(&listed.scripts.join(", "));
    }
    out
}

/// One row a run, then what each step that didn't pass said.
fn log_text(runs: &[pb::MachineScriptRun]) -> String {
    let mut rows = vec![row(&["RUN", "RULE", "TRIGGER", "STARTED", "OUTCOME", "TOOK"])];
    for run in runs {
        rows.push(vec![
            run.id.clone(),
            run.rule.clone(),
            run.trigger.clone(),
            started_text(run),
            outcome_text(run.outcome).to_owned(),
            took_text(run.took_ms),
        ]);
    }
    let mut out = table(&rows);
    for run in runs {
        for step in &run.steps {
            let passed = matches!(
                pb::MachineScriptOutcome::try_from(step.outcome),
                Ok(pb::MachineScriptOutcome::Passed)
            );
            if passed {
                continue;
            }
            out.push_str(&format!(
                "\n{} step {} ({} {}): {}",
                run.id,
                step.index,
                step.kind,
                step.label,
                outcome_text(step.outcome)
            ));
            if let Some(exit) = step.exit {
                out.push_str(&format!(", exit {exit}"));
            }
            if !step.error.is_empty() {
                out.push_str(&format!(": {}", step.error));
            }
            for line in &step.tail {
                out.push_str(&format!("\n    {line}"));
            }
        }
    }
    out
}

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|cell| (*cell).to_owned()).collect()
}

/// Left-aligned columns, two spaces apart, no trailing spaces.
fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|at| {
            rows.iter()
                .filter_map(|row| row.get(at))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let mut line = String::new();
            for (at, cell) in row.iter().enumerate() {
                line.push_str(cell);
                if at + 1 < row.len() {
                    let width = widths.get(at).copied().unwrap_or(0);
                    let pad = width.saturating_sub(cell.chars().count());
                    line.push_str(&" ".repeat(pad + 2));
                }
            }
            line.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn machine_text(run: &pb::MachineScriptRun) -> String {
    run.machine
        .as_ref()
        .map(|machine| machine.value.clone())
        .unwrap_or_default()
}

fn outcome_text(outcome: i32) -> &'static str {
    match pb::MachineScriptOutcome::try_from(outcome) {
        Ok(pb::MachineScriptOutcome::Passed) => "passed",
        Ok(pb::MachineScriptOutcome::Failed) => "failed",
        Ok(pb::MachineScriptOutcome::TimedOut) => "timed out",
        Ok(pb::MachineScriptOutcome::Skipped) => "skipped",
        Ok(pb::MachineScriptOutcome::Running) => "running",
        Ok(pb::MachineScriptOutcome::Unspecified) | Err(_) => "unknown",
    }
}

fn started_text(run: &pb::MachineScriptRun) -> String {
    match &run.started_at {
        Some(at) => as_json::rfc3339(at.seconds, at.nanos)
            .chars()
            .take(19)
            .collect::<String>()
            .replace('T', " "),
        None => String::new(),
    }
}

fn took_text(took_ms: u64) -> String {
    format!("{}.{}s", took_ms / 1000, took_ms % 1000 / 100)
}
