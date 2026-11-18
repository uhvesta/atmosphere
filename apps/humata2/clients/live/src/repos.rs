//! The repo library and the one repo search on the real daemon, over its
//! RepoService (RL-1 to RL-12). The host keeps the library and the scope;
//! this client asks and converts.

use humata2_clients::DaemonError;
use humata2_clients::repos::{
    FoundRepo, LibraryRepo, RepoPlace, RepoScope, RepoSearch, RepoSearchProblem,
    RepoSearchProblemKind,
};
use humata2_grpc::Timestamp;
use humata2_grpc::humata::v1 as pb;
use humata2_grpc::humata::v1::repo_service_client::RepoServiceClient;
use pb::repo_search_problem::Kind as ProblemKind;

use crate::connection::{Shared, Wire};

const WHAT: &str = "The repo library";

fn client(shared: &Shared) -> RepoServiceClient<Wire> {
    RepoServiceClient::new(shared.wire.clone())
}

fn ms(at: Option<&Timestamp>) -> i64 {
    at.map_or(0, |at| {
        at.seconds
            .saturating_mul(1000)
            .saturating_add(i64::from(at.nanos / 1_000_000))
    })
}

fn timestamp(ms: i64) -> Timestamp {
    Timestamp {
        seconds: ms.div_euclid(1000),
        nanos: i32::try_from(ms.rem_euclid(1000)).unwrap_or(0) * 1_000_000,
    }
}

fn repo_text(repo: Option<&pb::RepoRef>) -> String {
    repo.map_or_else(String::new, |repo| format!("{}/{}", repo.owner, repo.name))
}

/// `owner/name` as the wire's pair; a text without both halves is refused.
fn repo_ref(text: &str) -> Result<pb::RepoRef, DaemonError> {
    match text.split_once('/') {
        Some((owner, name)) if !owner.is_empty() && !name.is_empty() => Ok(pb::RepoRef {
            owner: owner.to_owned(),
            name: name.to_owned(),
        }),
        _ => Err(DaemonError::Refused {
            what: "Removing a repo from the library".to_owned(),
            reason: format!("{text:?} is not owner/name. Use the repo as the library lists it"),
        }),
    }
}

fn scope_to_wire(scope: &RepoScope) -> pb::RepoScope {
    use pb::repo_scope::Kind;
    let kind = match scope {
        RepoScope::YourRepos => Kind::YourRepos(true),
        RepoScope::Owners { owners, include_me } => Kind::Owners(pb::RepoOwners {
            owners: owners.clone(),
            include_me: *include_me,
        }),
        RepoScope::AllOfGitHub => Kind::AllOfGithub(true),
    };
    pb::RepoScope { kind: Some(kind) }
}

fn scope_from_wire(scope: Option<&pb::RepoScope>) -> RepoScope {
    use pb::repo_scope::Kind;
    match scope.and_then(|scope| scope.kind.as_ref()) {
        Some(Kind::Owners(owners)) => RepoScope::Owners {
            owners: owners.owners.clone(),
            include_me: owners.include_me,
        },
        Some(Kind::AllOfGithub(true)) => RepoScope::AllOfGitHub,
        Some(Kind::YourRepos(_) | Kind::AllOfGithub(false)) | None => RepoScope::YourRepos,
    }
}

fn library_repo(wire: &pb::LibraryRepo) -> LibraryRepo {
    let last_used_in = match pb::RepoPlaceKind::try_from(wire.last_used_kind) {
        Ok(pb::RepoPlaceKind::Channel) => RepoPlace::Channel(
            wire.last_used_channel
                .as_ref()
                .map_or_else(String::new, |id| id.value.clone()),
        ),
        _ => RepoPlace::Desk,
    };
    LibraryRepo {
        repo: repo_text(wire.repo.as_ref()),
        remote: wire.remote.clone(),
        default_branch: wire.default_branch.clone(),
        added_at_ms: ms(wire.added_at.as_ref()),
        last_used_at_ms: ms(wire.last_used_at.as_ref()),
        last_used_in,
    }
}

fn problem(wire: &pb::RepoSearchProblem) -> RepoSearchProblem {
    let kind = match ProblemKind::try_from(wire.kind) {
        Ok(ProblemKind::GhMissing) => RepoSearchProblemKind::GhMissing,
        Ok(ProblemKind::GhSignedOut) => RepoSearchProblemKind::GhSignedOut,
        Ok(ProblemKind::RateLimited) => RepoSearchProblemKind::RateLimited {
            retry_at_ms: wire.retry_at.as_ref().map(|at| ms(Some(at))),
        },
        Ok(ProblemKind::TooShort) => RepoSearchProblemKind::TooShort,
        Ok(ProblemKind::GithubUnreachable | ProblemKind::Unspecified) | Err(_) => {
            RepoSearchProblemKind::GitHubUnreachable
        }
    };
    RepoSearchProblem {
        kind,
        message: wire.message.clone(),
    }
}

fn search(wire: &pb::SearchReposResponse) -> RepoSearch {
    RepoSearch {
        library: wire.library.iter().map(library_repo).collect(),
        github: wire
            .github
            .iter()
            .map(|found| FoundRepo {
                repo: repo_text(found.repo.as_ref()),
                description: found.description.clone(),
                is_private: found.is_private,
            })
            .collect(),
        scope: scope_from_wire(wire.scope.as_ref()),
        problem: wire.problem.as_ref().map(problem),
        unknown_owners: wire.unknown_owners.clone(),
    }
}

fn library(wire: &pb::ListRepoLibraryResponse) -> Vec<LibraryRepo> {
    wire.repos.iter().map(library_repo).collect()
}

impl Shared {
    pub(crate) async fn search_repos(
        &self,
        query: String,
        scope: Option<RepoScope>,
    ) -> Result<RepoSearch, DaemonError> {
        let answer = client(self)
            .search_repos(pb::SearchReposRequest {
                query,
                scope: scope.as_ref().map(scope_to_wire),
                page: None,
            })
            .await
            .map_err(|status| self.error(&status, "The repo search"))?
            .into_inner();
        Ok(search(&answer))
    }

    pub(crate) async fn repo_library(&self) -> Result<Vec<LibraryRepo>, DaemonError> {
        let answer = client(self)
            .list_repo_library(pb::ListRepoLibraryRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(library(&answer))
    }

    pub(crate) async fn remove_from_repo_library(
        &self,
        repo: &str,
    ) -> Result<Vec<LibraryRepo>, DaemonError> {
        let repo = repo_ref(repo)?;
        let answer = client(self)
            .remove_from_repo_library(pb::RemoveFromRepoLibraryRequest { repo: Some(repo) })
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(library(&answer))
    }

    pub(crate) async fn forget_repo_library(&self) -> Result<u32, DaemonError> {
        let answer = client(self)
            .forget_repo_library(pb::ForgetRepoLibraryRequest {})
            .await
            .map_err(|status| self.error(&status, WHAT))?
            .into_inner();
        Ok(answer.forgotten)
    }

    /// The orgs the host's `gh` user belongs to. A `gh` that cannot list them
    /// answers a problem and no orgs; suggestions are optional, and the search
    /// itself already says what is wrong with `gh`, so the orgs are just none.
    pub(crate) async fn repo_owners(&self) -> Result<Vec<String>, DaemonError> {
        let answer = client(self)
            .list_repo_owners(pb::ListRepoOwnersRequest {})
            .await
            .map_err(|status| self.error(&status, "The orgs your GitHub login belongs to"))?
            .into_inner();
        Ok(answer.owners)
    }

    pub(crate) async fn repo_scope(&self) -> Result<RepoScope, DaemonError> {
        let answer = client(self)
            .get_repo_scope(pb::GetRepoScopeRequest {})
            .await
            .map_err(|status| self.error(&status, "The repo search scope"))?
            .into_inner();
        Ok(scope_from_wire(Some(&answer)))
    }

    pub(crate) async fn save_repo_scope(&self, scope: RepoScope) -> Result<RepoScope, DaemonError> {
        let answer = client(self)
            .update_repo_scope(pb::UpdateRepoScopeRequest {
                scope: Some(scope_to_wire(&scope)),
            })
            .await
            .map_err(|status| self.error(&status, "The repo search scope"))?
            .into_inner();
        Ok(scope_from_wire(Some(&answer)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scope_survives_the_wire() {
        for scope in [
            RepoScope::YourRepos,
            RepoScope::Owners { owners: vec!["acme".to_owned(), "octo".to_owned()], include_me: true },
            RepoScope::Owners { owners: Vec::new(), include_me: false },
            RepoScope::AllOfGitHub,
        ] {
            assert_eq!(scope_from_wire(Some(&scope_to_wire(&scope))), scope);
        }
    }

    #[test]
    fn a_missing_scope_is_your_repos() {
        assert_eq!(scope_from_wire(None), RepoScope::YourRepos);
        assert_eq!(scope_from_wire(Some(&pb::RepoScope { kind: None })), RepoScope::YourRepos);
    }

    #[test]
    fn a_library_repo_reads_its_place_and_times_in_milliseconds() {
        let wire = pb::LibraryRepo {
            repo: Some(pb::RepoRef { owner: "acme".to_owned(), name: "ledger".to_owned() }),
            remote: "git@github.com:acme/ledger.git".to_owned(),
            default_branch: "main".to_owned(),
            added_at: Some(Timestamp { seconds: 10, nanos: 500_000_000 }),
            last_used_at: Some(Timestamp { seconds: 20, nanos: 0 }),
            last_used_kind: pb::RepoPlaceKind::Channel as i32,
            last_used_channel: Some(pb::ChannelId { value: "ch_1".to_owned() }),
        };
        let repo = library_repo(&wire);
        assert_eq!(repo.repo, "acme/ledger");
        assert_eq!(repo.added_at_ms, 10_500);
        assert_eq!(repo.last_used_at_ms, 20_000);
        assert_eq!(repo.last_used_in, RepoPlace::Channel("ch_1".to_owned()));

        let on_desk = pb::LibraryRepo { last_used_kind: pb::RepoPlaceKind::Desk as i32, ..wire };
        assert_eq!(library_repo(&on_desk).last_used_in, RepoPlace::Desk);
    }

    #[test]
    fn timestamps_round_trip_through_milliseconds() {
        for value in [0, 1, 999, 1_000, 1_234_567_890_123] {
            assert_eq!(ms(Some(&timestamp(value))), value);
        }
        assert_eq!(ms(None), 0);
    }

    #[test]
    fn problems_read_their_kind_and_retry_time() {
        let wire = |kind: ProblemKind, retry_at: Option<Timestamp>| pb::RepoSearchProblem {
            kind: kind as i32,
            message: "m".to_owned(),
            retry_at,
        };
        assert_eq!(problem(&wire(ProblemKind::GhMissing, None)).kind, RepoSearchProblemKind::GhMissing);
        assert_eq!(problem(&wire(ProblemKind::GhSignedOut, None)).kind, RepoSearchProblemKind::GhSignedOut);
        assert_eq!(problem(&wire(ProblemKind::TooShort, None)).kind, RepoSearchProblemKind::TooShort);
        assert_eq!(
            problem(&wire(ProblemKind::GithubUnreachable, None)).kind,
            RepoSearchProblemKind::GitHubUnreachable
        );
        assert_eq!(
            problem(&wire(ProblemKind::RateLimited, Some(Timestamp { seconds: 5, nanos: 0 }))).kind,
            RepoSearchProblemKind::RateLimited { retry_at_ms: Some(5_000) }
        );
        assert_eq!(
            problem(&wire(ProblemKind::RateLimited, None)).kind,
            RepoSearchProblemKind::RateLimited { retry_at_ms: None }
        );
    }

    #[test]
    fn a_repo_to_remove_needs_an_owner_and_a_name() {
        assert!(repo_ref("acme/ledger").is_ok());
        for bad in ["ledger", "/ledger", "acme/", ""] {
            assert!(matches!(repo_ref(bad), Err(DaemonError::Refused { .. })), "{bad:?}");
        }
    }

    #[test]
    fn a_search_answer_keeps_both_groups_and_the_problem() {
        let wire = pb::SearchReposResponse {
            library: vec![pb::LibraryRepo::default()],
            github: vec![pb::FoundRepo {
                repo: Some(pb::RepoRef { owner: "o".to_owned(), name: "n".to_owned() }),
                description: "d".to_owned(),
                is_private: true,
            }],
            scope: Some(scope_to_wire(&RepoScope::AllOfGitHub)),
            problem: Some(pb::RepoSearchProblem {
                kind: ProblemKind::TooShort as i32,
                message: String::new(),
                retry_at: None,
            }),
            unknown_owners: vec!["ghost".to_owned()],
            page: None,
        };
        let answer = search(&wire);
        assert_eq!(answer.library.len(), 1);
        assert_eq!(answer.github[0].repo, "o/n");
        assert!(answer.github[0].is_private);
        assert_eq!(answer.scope, RepoScope::AllOfGitHub);
        assert!(answer.problem.is_some());
        assert_eq!(answer.unknown_owners, ["ghost"]);
    }
}
