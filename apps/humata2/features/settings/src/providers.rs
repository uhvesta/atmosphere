//! Settings › Machines › Providers (PROVIDERS): one row per provider with
//! its commands and the host variables they read, Remove for an imported
//! one, and Import provider… for a pasted provider file, reviewed first.
//!
//! A host variable's value is typed into the state, sent once, and gone from
//! the state as it is sent; it never comes back from the daemon.

use std::collections::BTreeMap;

use humata2_clients::machines::{ProviderEntry, ProviderImportPlan};
use humata2_clients::{Clients, DaemonError};
use tca::Effect;

/// Text that must not show in logs, recordings or test failures: its
/// `Debug` says only whether it is empty.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// `text`, kept secret.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Whether nothing is typed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The text, for the one call that sends it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() { "Secret(empty)" } else { "Secret(…)" })
    }
}

/// A pasted provider file, and its review once asked for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportDraft {
    /// What was pasted.
    pub text: String,
    /// What importing it would add, once reviewed with no problems.
    pub review: Option<ProviderImportPlan>,
    /// Why it can't be reviewed or imported, for the red line under the field.
    pub problem: Option<String>,
    /// A review or an import is under way.
    pub busy: bool,
}

/// The Providers block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProvidersState {
    /// Every provider, as the daemon last listed them; `None` until it has.
    pub list: Option<Vec<ProviderEntry>>,
    /// The provider whose Commands are open. One at a time.
    pub open: Option<String>,
    /// The provider whose Remove asks to confirm, or says why it can't.
    pub removing: Option<String>,
    /// Import provider…, while it is open.
    pub import: Option<ImportDraft>,
    /// The values typed for host variables, by name, until Set sends them.
    pub values: BTreeMap<String, Secret>,
    /// What went wrong last, outside the import field.
    pub error: Option<String>,
}

impl ProvidersState {
    /// The listed provider `id`.
    #[must_use]
    pub fn provider(&self, id: &str) -> Option<&ProviderEntry> {
        self.list.as_ref()?.iter().find(|provider| provider.id == id)
    }
}

/// What happens in the Providers block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvidersAction {
    /// Read the providers.
    Load,
    /// The daemon listed them, or couldn't.
    Loaded(Result<Vec<ProviderEntry>, DaemonError>),
    /// Open or close a provider's Commands, closing any other.
    ToggleCommands(String),
    /// Remove: ask to confirm, or say which machines still use it.
    Remove(String),
    /// Keep the provider.
    CancelRemove,
    /// Remove it, confirmed.
    ConfirmRemove(String),
    /// The daemon removed it, or refused.
    Removed(Result<(), DaemonError>),
    /// Open Import provider….
    OpenImport,
    /// Close it, dropping what was pasted.
    CancelImport,
    /// The paste field changed. A change drops a review made of the old text.
    ImportText(String),
    /// Review the pasted file.
    Review,
    /// The daemon planned the import, or couldn't.
    Reviewed(Result<ProviderImportPlan, DaemonError>),
    /// Import what the review showed.
    Import,
    /// The daemon imported it, or refused.
    Imported(Result<(), DaemonError>),
    /// A host variable's secure field changed.
    VariableValue(String, Secret),
    /// Set or Replace: send the typed value.
    SetVariable(String),
    /// Clear a host variable.
    ClearVariable(String),
    /// The daemon kept or cleared a variable, or refused.
    VariableSaved(Result<(), DaemonError>),
}

fn load(clients: &Clients) -> Effect<ProvidersAction> {
    let call = clients.machines.providers();
    Effect::future(async move { ProvidersAction::Loaded(call.await) })
}

/// Reads the providers, as Settings › Machines opens.
pub(crate) fn open(clients: &Clients) -> Effect<ProvidersAction> {
    load(clients)
}

/// The red line for a review with problems.
fn problems_line(plan: &ProviderImportPlan) -> String {
    let lines: Vec<String> = plan.problems.iter().map(ToString::to_string).collect();
    format!("This provider can't be imported. {}", lines.join(" "))
}

/// Applies `action`, calling the daemon through `clients`.
pub(crate) fn reduce(clients: &Clients, state: &mut ProvidersState, action: ProvidersAction) -> Effect<ProvidersAction> {
    match action {
        ProvidersAction::Load => return load(clients),
        ProvidersAction::Loaded(result) => match result {
            Ok(list) => {
                if state.open.as_ref().is_some_and(|open| !list.iter().any(|p| &p.id == open)) {
                    state.open = None;
                }
                if state.removing.as_ref().is_some_and(|gone| !list.iter().any(|p| &p.id == gone)) {
                    state.removing = None;
                }
                state.list = Some(list);
                state.error = None;
            }
            // Keep what was listed: only this read failed.
            Err(error) => state.error = Some(error.to_string()),
        },
        ProvidersAction::ToggleCommands(id) => {
            // A value typed in the row that closes is dropped, never kept.
            state.values.clear();
            state.open = if state.open.as_ref() == Some(&id) { None } else { Some(id) };
        }
        ProvidersAction::Remove(id) => {
            if state.provider(&id).is_some_and(|provider| !provider.built_in) {
                state.removing = Some(id);
            }
        }
        ProvidersAction::CancelRemove => state.removing = None,
        ProvidersAction::ConfirmRemove(id) => {
            // Only the row that asked, imported, with no machines left on it.
            let removable = state.removing.as_ref() == Some(&id)
                && state.provider(&id).is_some_and(|provider| !provider.built_in && provider.machines.is_empty());
            if !removable {
                return Effect::none();
            }
            let call = clients.machines.remove_provider(&id);
            return Effect::future(async move { ProvidersAction::Removed(call.await) });
        }
        ProvidersAction::Removed(result) => {
            state.removing = None;
            match result {
                Ok(()) => {
                    state.error = None;
                    return load(clients);
                }
                Err(error) => state.error = Some(error.to_string()),
            }
        }
        ProvidersAction::OpenImport => {
            if state.import.is_none() {
                state.import = Some(ImportDraft::default());
            }
        }
        ProvidersAction::CancelImport => {
            if !state.import.as_ref().is_some_and(|draft| draft.busy) {
                state.import = None;
            }
        }
        ProvidersAction::ImportText(text) => {
            if let Some(draft) = &mut state.import
                && !draft.busy
                && draft.text != text
            {
                draft.text = text;
                draft.review = None;
                draft.problem = None;
            }
        }
        ProvidersAction::Review => {
            let Some(draft) = &mut state.import else {
                return Effect::none();
            };
            if draft.busy {
                return Effect::none();
            }
            if draft.text.trim().is_empty() {
                draft.problem = Some("Paste the provider's TOML first, then Review.".to_owned());
                return Effect::none();
            }
            draft.busy = true;
            draft.problem = None;
            draft.review = None;
            let call = clients.machines.preview_provider_import(&draft.text);
            return Effect::future(async move { ProvidersAction::Reviewed(call.await) });
        }
        ProvidersAction::Reviewed(result) => {
            let Some(draft) = &mut state.import else {
                return Effect::none();
            };
            draft.busy = false;
            match result {
                Ok(plan) if plan.problems.is_empty() && plan.token.is_some() => draft.review = Some(plan),
                Ok(plan) if plan.problems.is_empty() => {
                    draft.problem = Some(
                        "Humata couldn't plan this import, so nothing can be added from it. Review it again; if this keeps happening, restart Humata and try again.".to_owned(),
                    );
                }
                Ok(plan) => draft.problem = Some(problems_line(&plan)),
                Err(error) => draft.problem = Some(error.to_string()),
            }
        }
        ProvidersAction::Import => {
            let Some(draft) = &mut state.import else {
                return Effect::none();
            };
            let Some(plan) = draft.review.as_ref().filter(|_| !draft.busy) else {
                return Effect::none();
            };
            let call = clients.machines.import_provider(plan);
            draft.busy = true;
            return Effect::future(async move { ProvidersAction::Imported(call.await) });
        }
        ProvidersAction::Imported(result) => {
            let Some(draft) = &mut state.import else {
                return Effect::none();
            };
            draft.busy = false;
            match result {
                Ok(()) => {
                    state.import = None;
                    return load(clients);
                }
                // A stale plan can't be retried: the person reviews again.
                Err(error) => {
                    draft.review = None;
                    draft.problem = Some(error.to_string());
                }
            }
        }
        ProvidersAction::VariableValue(name, value) => {
            if value.is_empty() {
                state.values.remove(&name);
            } else {
                state.values.insert(name, value);
            }
        }
        ProvidersAction::SetVariable(name) => {
            // The value leaves the state as it is sent (HO-13).
            let Some(value) = state.values.remove(&name) else {
                return Effect::none();
            };
            let call = clients.machines.set_provider_variable(&name, value.expose());
            return Effect::future(async move { ProvidersAction::VariableSaved(call.await) });
        }
        ProvidersAction::ClearVariable(name) => {
            state.values.remove(&name);
            let call = clients.machines.clear_provider_variable(&name);
            return Effect::future(async move { ProvidersAction::VariableSaved(call.await) });
        }
        ProvidersAction::VariableSaved(result) => match result {
            Ok(()) => {
                state.error = None;
                return load(clients);
            }
            Err(error) => state.error = Some(error.to_string()),
        },
    }
    Effect::none()
}
