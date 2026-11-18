//! Settings › General › Repositories (ST-21, RL-4, RL-7, RL-8): the repo
//! library with Remove per repo and Forget all… behind a confirm, and the
//! default scope as owners plus an Include my repos switch. The host keeps
//! both; this reducer reads them when the section opens and writes each
//! change through the daemon.

use humata2_clients::repos::{LibraryRepo, RepoScope};
use humata2_clients::{Clients, DaemonError};
use tca::Effect;

/// What the Repositories section shows and waits on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepositoriesState {
    /// The library, most recently used first; `None` until the host says.
    pub library: Option<Vec<LibraryRepo>>,
    /// The saved default scope; `None` until the host says.
    pub scope: Option<RepoScope>,
    /// Forget all… asked, and waiting for its confirm.
    pub confirming_forget: bool,
    /// A change is out to the host.
    pub busy: bool,
    /// What went wrong last, and what to do.
    pub error: Option<String>,
    /// The orgs the user's GitHub login belongs to, as the host listed them.
    pub github_owners: Vec<String>,
    /// The time the host gave when the library was last read, in
    /// milliseconds since the epoch; "used ... ago" is worked out from it.
    pub now_ms: i64,
}

impl RepositoriesState {
    /// How many repos Forget all… would forget.
    #[must_use]
    pub fn library_len(&self) -> usize {
        self.library.as_ref().map_or(0, Vec::len)
    }

    /// The owners of the default scope, in order.
    #[must_use]
    pub fn owners(&self) -> &[String] {
        match &self.scope {
            Some(RepoScope::Owners { owners, .. }) => owners,
            _ => &[],
        }
    }

    /// Whether the default scope includes the user's own repos (the switch).
    #[must_use]
    pub fn include_me(&self) -> bool {
        match &self.scope {
            Some(RepoScope::Owners { include_me, .. }) => *include_me,
            Some(RepoScope::YourRepos) => true,
            _ => false,
        }
    }

    /// Owners worth offering after **+ Org**: the library's owners that the
    /// default scope does not list yet.
    #[must_use]
    pub fn owner_suggestions(&self) -> Vec<String> {
        let listed = self.owners();
        let library = self.library.as_deref().unwrap_or_default();
        humata2_clients::repos::suggest_owners(library, &self.github_owners, listed)
    }
}

/// What happens in the Repositories section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepositoriesAction {
    /// The section opened: read the library and the scope.
    Open,
    /// The host listed the library, or could not.
    LibraryShown(Result<Vec<LibraryRepo>, DaemonError>),
    /// The host said the default scope, or could not.
    ScopeShown(Result<RepoScope, DaemonError>),
    /// Remove one repo (`owner/name`) from the library.
    Remove(String),
    /// The host removed it, or could not; the library it holds now.
    Removed(Result<Vec<LibraryRepo>, DaemonError>),
    /// **Forget all…**: ask for the confirm.
    ForgetAsked,
    /// The confirm was dismissed.
    ForgetCancelled,
    /// The confirm was accepted.
    ForgetConfirmed,
    /// The host forgot the library (how many repos it held), or could not.
    Forgotten(Result<u32, DaemonError>),
    /// An owner chip's ✕ in the default scope.
    OwnerRemoved(String),
    /// An owner joins the default scope.
    OwnerAdded(String),
    /// The Include my repos switch.
    ToggleIncludeMe,
    /// The host saved the default scope, or refused.
    ScopeSaved(Result<RepoScope, DaemonError>),
    /// The host listed the user's GitHub orgs, or could not.
    OwnersListed(Result<Vec<String>, DaemonError>),
}

/// The scope with owners and the switch applied: no owners with the switch
/// on is Your repos (RL-7).
fn scope_of(owners: Vec<String>, include_me: bool) -> RepoScope {
    if owners.is_empty() && include_me {
        RepoScope::YourRepos
    } else {
        RepoScope::Owners { owners, include_me }
    }
}

/// Applies `action`, calling the daemon through `clients`.
pub(crate) fn reduce(
    clients: &Clients,
    state: &mut RepositoriesState,
    action: RepositoriesAction,
) -> Effect<RepositoriesAction> {
    use RepositoriesAction as A;
    match action {
        A::Open => {
            state.error = None;
            state.confirming_forget = false;
            let library = clients.daemon.clone();
            let scope = clients.daemon.clone();
            let owners = clients.daemon.clone();
            Effect::merge([
                Effect::future(async move { A::LibraryShown(library.repo_library().await) }),
                Effect::future(async move { A::ScopeShown(scope.repo_scope().await) }),
                Effect::future(async move { A::OwnersListed(owners.repo_owners().await) }),
            ])
        }
        A::OwnersListed(Ok(owners)) => {
            state.github_owners = owners;
            Effect::none()
        }
        // Suggestions are optional; the library's owners are still offered.
        A::OwnersListed(Err(_)) => Effect::none(),
        A::LibraryShown(Ok(library)) | A::Removed(Ok(library)) => {
            state.busy = false;
            state.now_ms = clients.daemon.now_ms();
            state.library = Some(library);
            Effect::none()
        }
        A::ScopeShown(Ok(scope)) | A::ScopeSaved(Ok(scope)) => {
            state.busy = false;
            state.scope = Some(scope);
            Effect::none()
        }
        A::LibraryShown(Err(error))
        | A::ScopeShown(Err(error))
        | A::Removed(Err(error))
        | A::Forgotten(Err(error))
        | A::ScopeSaved(Err(error)) => {
            state.busy = false;
            state.error = Some(error.to_string());
            Effect::none()
        }
        A::Remove(repo) => {
            if state.busy {
                return Effect::none();
            }
            state.busy = true;
            state.error = None;
            let daemon = clients.daemon.clone();
            Effect::future(async move { A::Removed(daemon.remove_from_repo_library(&repo).await) })
        }
        A::ForgetAsked => {
            if state.library_len() > 0 {
                state.confirming_forget = true;
            }
            Effect::none()
        }
        A::ForgetCancelled => {
            state.confirming_forget = false;
            Effect::none()
        }
        A::ForgetConfirmed => {
            if !state.confirming_forget || state.busy {
                return Effect::none();
            }
            state.confirming_forget = false;
            state.busy = true;
            state.error = None;
            let daemon = clients.daemon.clone();
            Effect::future(async move { A::Forgotten(daemon.forget_repo_library().await) })
        }
        A::Forgotten(Ok(_)) => {
            state.busy = false;
            state.library = Some(Vec::new());
            Effect::none()
        }
        A::OwnerRemoved(owner) => {
            let mut owners = state.owners().to_vec();
            owners.retain(|listed| !listed.eq_ignore_ascii_case(&owner));
            let scope = scope_of(owners, state.include_me());
            save(clients, state, scope)
        }
        A::OwnerAdded(owner) => {
            let owner = owner.trim().trim_start_matches('@').to_owned();
            let mut owners = state.owners().to_vec();
            if owner.is_empty() || owners.iter().any(|listed| listed.eq_ignore_ascii_case(&owner)) {
                return Effect::none();
            }
            owners.push(owner);
            let scope = scope_of(owners, state.include_me());
            save(clients, state, scope)
        }
        A::ToggleIncludeMe => {
            let include_me = !state.include_me();
            let scope = scope_of(state.owners().to_vec(), include_me);
            save(clients, state, scope)
        }
    }
}

/// Writes `scope` as the default; the section shows what the host kept.
fn save(clients: &Clients, state: &mut RepositoriesState, scope: RepoScope) -> Effect<RepositoriesAction> {
    if state.busy {
        return Effect::none();
    }
    state.busy = true;
    state.error = None;
    let daemon = clients.daemon.clone();
    Effect::future(async move { RepositoriesAction::ScopeSaved(daemon.save_repo_scope(scope).await) })
}
