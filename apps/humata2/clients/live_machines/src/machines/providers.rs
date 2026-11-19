//! Settings › Machines › Providers over the provider and machine services
//! (PROVIDERS): the list with each provider's commands, variables and
//! machines, a pasted file's import plan, and this Mac's host variables.
//!
//! A host variable's value is sent once and never read back, logged or kept.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    ProviderCommand, ProviderEntry, ProviderImportPlan, ProviderParam, ProviderProblem, ProviderVariable,
};
use humata2_clients_live_core::{LiveConnection, fresh_key};
use crate::humata2_grpc::humata::v1 as pb;

use super::{failure, machine_id, machine_service, provider_service, words};

/// The name a provider file is shown with on the daemon's side; the app
/// never reads a path, so the paste has none.
const PASTED: &str = "pasted provider";

fn id_of(provider: Option<&pb::ProviderId>) -> String {
    provider.map(|id| id.value.clone()).unwrap_or_default()
}

/// The actions a provider defines, in the order the add uses them.
fn actions_of(provider: &pb::Provider) -> Vec<String> {
    let mut actions = Vec::new();
    if provider.has_create {
        actions.push("create".to_owned());
    }
    for action in &provider.actions {
        let name = match pb::ProviderAction::try_from(*action) {
            Ok(pb::ProviderAction::Start) => "start",
            Ok(pb::ProviderAction::Stop) => "stop",
            Ok(pb::ProviderAction::Status) => "status",
            Ok(pb::ProviderAction::Delete) => "delete",
            Ok(pb::ProviderAction::Unspecified) | Err(_) => continue,
        };
        if !actions.iter().any(|kept| kept == name) {
            actions.push(name.to_owned());
        }
    }
    actions
}

/// One provider as the block shows it, from ListProviders: its commands,
/// the variables it reads (set or not, never a value) and its machines. A
/// plan's providers carry no variable states, so each reads as unset.
fn entry_of(provider: &pb::Provider) -> ProviderEntry {
    let variables = if provider.variables.is_empty() {
        let mut seen = BTreeSet::new();
        provider
            .env
            .iter()
            .filter(|name| seen.insert(name.as_str()))
            .map(|name| ProviderVariable { name: name.clone(), set: false })
            .collect()
    } else {
        provider
            .variables
            .iter()
            .map(|variable| ProviderVariable { name: variable.name.clone(), set: variable.set })
            .collect()
    };
    ProviderEntry {
        id: id_of(provider.id.as_ref()),
        label: provider.label.clone(),
        built_in: provider.built_in,
        actions: actions_of(provider),
        commands: provider
            .commands
            .iter()
            .map(|command| ProviderCommand { action: command.action.clone(), command: command.command.clone() })
            .collect(),
        params: provider
            .params
            .iter()
            .map(|param| ProviderParam {
                name: param.name.clone(),
                label: param.prompt.clone(),
                default: param.default_value.clone(),
                required: param.default_value.is_empty(),
            })
            .collect(),
        variables,
        machines: provider.machines.iter().map(|machine| machine.value.clone()).collect(),
    }
}

/// Each provider's commands in a plan, by provider id, in the plan's order.
fn commands_by_provider(commands: &[pb::provider_import_plan::Command]) -> BTreeMap<String, Vec<ProviderCommand>> {
    let mut by = BTreeMap::<String, Vec<ProviderCommand>>::new();
    for command in commands {
        by.entry(id_of(command.provider.as_ref())).or_default().push(ProviderCommand {
            action: command.action.clone(),
            command: command.command.clone(),
        });
    }
    by
}

fn problem_of(problem: &pb::ProviderConfigProblem) -> ProviderProblem {
    ProviderProblem {
        provider: id_of(problem.provider.as_ref()),
        field: problem.field.clone(),
        line: problem.line,
        message: problem.error.as_ref().map_or_else(|| "the daemon didn't say why".to_owned(), words),
    }
}

/// A daemon's import plan as the review shows it.
fn plan_of(plan: &pb::ProviderImportPlan) -> ProviderImportPlan {
    let mut commands = commands_by_provider(&plan.commands);
    ProviderImportPlan {
        providers: plan
            .providers
            .iter()
            .map(|provider| {
                let mut entry = entry_of(provider);
                if let Some(listed) = commands.remove(&entry.id) {
                    entry.commands = listed;
                }
                entry
            })
            .collect(),
        replaces: plan.replaces.iter().map(|id| id.value.clone()).collect(),
        problems: plan.problems.iter().map(problem_of).collect(),
        token: plan
            .token
            .as_ref()
            .map(|token| token.value.clone())
            .filter(|token| !token.is_empty()),
    }
}

pub(super) async fn list(shared: LiveConnection) -> Result<Vec<ProviderEntry>, DaemonError> {
    let reply = provider_service(&shared)
        .list_providers(pb::ListProvidersRequest {})
        .await
        .map_err(|status| failure(&shared, &status, "The providers"))?
        .into_inner();
    Ok(reply.providers.iter().map(entry_of).collect())
}

pub(super) async fn preview(shared: LiveConnection, text: String) -> Result<ProviderImportPlan, DaemonError> {
    let request = pb::PreviewProviderImportRequest { file: text.into_bytes(), file_name: PASTED.to_owned() };
    let plan = provider_service(&shared)
        .preview_provider_import(request)
        .await
        .map_err(|status| failure(&shared, &status, "Reviewing the provider file"))?
        .into_inner();
    Ok(plan_of(&plan))
}

pub(super) async fn import(shared: LiveConnection, token: Option<String>) -> Result<(), DaemonError> {
    const WHAT: &str = "Importing the provider file";
    let Some(token) = token else {
        return Err(DaemonError::Refused {
            what: WHAT.to_owned(),
            reason: "the file has problems, so nothing was added. Fix them and review it again".to_owned(),
        });
    };
    let reply = provider_service(&shared)
        .import_provider(pb::ImportProviderRequest {
            idempotency_key: fresh_key(),
            plan: Some(pb::PlanToken { value: token }),
            // The app is the person: no presence proof is sent.
            presence: None,
        })
        .await
        .map_err(|status| failure(&shared, &status, WHAT))?
        .into_inner();
    match reply.problems.first() {
        None => Ok(()),
        Some(problem) => Err(DaemonError::Refused {
            what: WHAT.to_owned(),
            reason: format!("nothing was added: {}. Fix it and review the file again", problem_of(problem)),
        }),
    }
}

pub(super) async fn remove(shared: LiveConnection, provider: String) -> Result<(), DaemonError> {
    let request = pb::RemoveProviderRequest {
        idempotency_key: fresh_key(),
        provider: Some(pb::ProviderId { value: provider }),
        // The app is the person: no presence proof is sent.
        presence: None,
    };
    provider_service(&shared)
        .remove_provider(request)
        .await
        .map_err(|status| failure(&shared, &status, "Removing the provider"))?;
    Ok(())
}

pub(super) async fn set_variable(shared: LiveConnection, name: String, value: String) -> Result<(), DaemonError> {
    let request = pb::SetHostVariableRequest {
        idempotency_key: fresh_key(),
        id: machine_id(shared.machine()),
        name,
        value,
        // The app is the person: no presence proof is sent.
        presence: None,
    };
    machine_service(&shared)
        .set_host_variable(request)
        .await
        .map_err(|status| failure(&shared, &status, "Setting a host variable"))?;
    Ok(())
}

pub(super) async fn clear_variable(shared: LiveConnection, name: String) -> Result<(), DaemonError> {
    let request = pb::RemoveHostVariableRequest {
        idempotency_key: fresh_key(),
        id: machine_id(shared.machine()),
        name,
        // The app is the person: no presence proof is sent.
        presence: None,
    };
    machine_service(&shared)
        .remove_host_variable(request)
        .await
        .map_err(|status| failure(&shared, &status, "Clearing a host variable"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire_provider() -> pb::Provider {
        pb::Provider {
            id: Some(pb::ProviderId { value: "hcloud".to_owned() }),
            label: "Hetzner Cloud".to_owned(),
            env: vec!["HCLOUD_TOKEN".to_owned(), "HCLOUD_TOKEN".to_owned(), "SSH_KEY".to_owned()],
            has_create: true,
            actions: vec![pb::ProviderAction::Delete as i32, pb::ProviderAction::Start as i32],
            ..pb::Provider::default()
        }
    }

    #[test]
    fn a_listed_provider_brings_its_commands_variables_and_machines() {
        let listed = pb::Provider {
            commands: vec![pb::ProviderCommand { action: "create".to_owned(), command: "hcloud server create".to_owned() }],
            variables: vec![
                pb::ProviderVariable { name: "HCLOUD_TOKEN".to_owned(), set: false },
                pb::ProviderVariable { name: "SSH_KEY".to_owned(), set: true },
            ],
            machines: vec![pb::MachineId { value: "box-1".to_owned() }],
            ..wire_provider()
        };
        let entry = entry_of(&listed);
        assert_eq!(entry.actions, ["create", "delete", "start"]);
        assert_eq!(entry.commands[0].command, "hcloud server create");
        assert_eq!(entry.unset().collect::<Vec<_>>(), ["HCLOUD_TOKEN"]);
        assert_eq!(entry.variables.len(), 2);
        assert_eq!(entry.machines, ["box-1"]);
    }

    #[test]
    fn a_provider_without_variable_states_reads_each_once_as_unset() {
        let entry = entry_of(&wire_provider());
        assert_eq!(entry.unset().collect::<Vec<_>>(), ["HCLOUD_TOKEN", "SSH_KEY"]);
    }

    #[test]
    fn a_plan_gives_each_provider_its_own_commands_and_no_token_with_problems() {
        let plan = pb::ProviderImportPlan {
            providers: vec![wire_provider()],
            commands: vec![
                pb::provider_import_plan::Command {
                    provider: Some(pb::ProviderId { value: "hcloud".to_owned() }),
                    action: "create".to_owned(),
                    command: "hcloud server create".to_owned(),
                },
                pb::provider_import_plan::Command {
                    provider: Some(pb::ProviderId { value: "other".to_owned() }),
                    action: "create".to_owned(),
                    command: "other create".to_owned(),
                },
            ],
            problems: vec![pb::ProviderConfigProblem { line: 3, ..pb::ProviderConfigProblem::default() }],
            token: Some(pb::PlanToken { value: String::new() }),
            ..pb::ProviderImportPlan::default()
        };
        let shown = plan_of(&plan);
        assert_eq!(shown.providers[0].commands.len(), 1);
        assert_eq!(shown.problems[0].line, 3);
        assert_eq!(shown.token, None);
    }
}
