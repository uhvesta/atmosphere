//! RepoService (RL-1 to RL-10) through a real `humatad2`: the repo
//! library, the one repo search with its GitHub group, and the default
//! scope in config/repos.toml. `gh` is a fake script (admin builds read
//! `HUMATA2_GH`); it records what it was asked in files beside it.

use humatad_test_common as common;

use std::path::PathBuf;

use common::{Build, Daemon, folder, invalid_field, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_repo_library::{Library, Place, RepoName, Use};
use pb::repo_scope::Kind;
use pb::repo_search_problem::Kind as ProblemKind;
use pb::repo_service_client::RepoServiceClient;
use tonic::Code;
use tonic::transport::Channel;

/// A signed-in `gh` as `octo`. A repository search adds a line to
/// `searches` and finds acme/ledger and acme/api; a listing adds a line to
/// `listings` and sees acme/ledger and octo/notes. A `signed_out` or
/// `limited` file beside it makes searching fail that way, and a search
/// that names the owner `ghost` fails as GitHub does for an owner it
/// doesn't have.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
case "$*" in
  "api user --jq .login") echo octo; exit 0 ;;
  "api users/ghost --jq .login") echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
  "api users/"*" --jq .login") echo "${2#users/}"; exit 0 ;;
esac
case "$*" in
  *search/repositories*)
    echo "$*" >> "$here/searches"
    if [ -f "$here/signed_out" ]; then
      echo "To get started with GitHub CLI, please run:  gh auth login" >&2
      exit 4
    fi
    if [ -f "$here/limited" ]; then
      echo "gh: API rate limit exceeded for user ID 42. (HTTP 403)" >&2
      exit 1
    fi
    case "$*" in
      *user:ghost*) echo "gh: Validation Failed (HTTP 422)" >&2; exit 1 ;;
    esac
    echo '{"full_name":"acme/ledger","description":"The ledger","private":true}'
    echo '{"full_name":"acme/api","description":null,"private":false}'
    ;;
  *user/repos*)
    echo "$*" >> "$here/listings"
    echo '{"full_name":"acme/ledger","description":"The ledger","private":true,"default_branch":"main","pushed_at":"2026-11-15T10:00:00Z"}'
    echo '{"full_name":"octo/notes","description":"Notes","private":false,"default_branch":"main","pushed_at":"2026-11-12T10:00:00Z"}'
    ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

struct Rig {
    daemon: Daemon,
    gh: PathBuf,
}

/// A daemon with the fake `gh`, `scope_file` as config/repos.toml when given
/// and `library` already holding those repos.
async fn start(scope_file: Option<&str>, library: &[&str]) -> Rig {
    let dir = folder();
    let gh_dir = dir.path().join("gh");
    std::fs::create_dir_all(&gh_dir).unwrap();
    let gh = gh_dir.join("gh");
    std::fs::write(&gh, FAKE_GH).unwrap();
    std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    if let Some(text) = scope_file {
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/repos.toml"), text).unwrap();
    }
    if !library.is_empty() {
        let seeded = Library::open(&state_folder(dir.path()).join("repos.db")).unwrap();
        for (n, full) in library.iter().enumerate() {
            seeded
                .record(&Use {
                    repo: RepoName::parse(full).unwrap(),
                    remote: format!("https://github.com/{full}.git"),
                    default_branch: "main".to_owned(),
                    at: 1_000 + i64::try_from(n).unwrap(),
                    place: Place::Desk,
                    canonical: false,
                })
                .unwrap();
        }
    }
    let gh_text = gh.display().to_string();
    let daemon =
        Daemon::start_with_env(Build::Admin, dir, &[], &[("HUMATA2_GH", gh_text.as_str())]).await;
    Rig {
        daemon,
        gh: gh_dir,
    }
}

impl Rig {
    async fn client(&self) -> RepoServiceClient<Channel> {
        RepoServiceClient::new(self.daemon.channel().await)
    }

    async fn search(
        &self,
        query: &str,
        scope: Option<pb::RepoScope>,
    ) -> Result<pb::SearchReposResponse, tonic::Status> {
        self.client()
            .await
            .search_repos(pb::SearchReposRequest {
                query: query.to_owned(),
                scope,
                page: None,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// The lines `gh` recorded in `file`: one per run of that kind.
    fn recorded(&self, file: &str) -> Vec<String> {
        std::fs::read_to_string(self.gh.join(file))
            .map(|text| text.lines().map(str::to_owned).collect())
            .unwrap_or_default()
    }

    fn config(&self) -> PathBuf {
        self.daemon.dir.path().join("config/repos.toml")
    }
}

fn owners(names: &[&str], include_me: bool) -> pb::RepoScope {
    pb::RepoScope {
        kind: Some(Kind::Owners(pb::RepoOwners {
            owners: names.iter().map(|name| (*name).to_owned()).collect(),
            include_me,
        })),
    }
}

fn all_of_github() -> pb::RepoScope {
    pb::RepoScope {
        kind: Some(Kind::AllOfGithub(true)),
    }
}

fn repo_ref(full: &str) -> pb::RepoRef {
    let (owner, name) = full.split_once('/').unwrap();
    pb::RepoRef {
        owner: owner.to_owned(),
        name: name.to_owned(),
    }
}

fn library_names(repos: &[pb::LibraryRepo]) -> Vec<String> {
    repos
        .iter()
        .filter_map(|repo| repo.repo.as_ref())
        .map(|repo| format!("{}/{}", repo.owner, repo.name))
        .collect()
}

fn github_names(response: &pb::SearchReposResponse) -> Vec<String> {
    response
        .github
        .iter()
        .filter_map(|found| found.repo.as_ref())
        .map(|repo| format!("{}/{}", repo.owner, repo.name))
        .collect()
}

fn problem_kind(response: &pb::SearchReposResponse) -> Option<ProblemKind> {
    response.problem.as_ref().map(|problem| problem.kind())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_library_lists_nothing_and_removing_an_unknown_repo_changes_nothing() {
    let rig = start(None, &[]).await;
    let mut client = rig.client().await;
    let listed = client
        .list_repo_library(pb::ListRepoLibraryRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(listed.repos.is_empty());

    let removed = client
        .remove_from_repo_library(pb::RemoveFromRepoLibraryRequest {
            repo: Some(repo_ref("acme/nothing")),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(removed.repos.is_empty());
    assert_eq!(removed.revision, listed.revision);
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_takes_one_repo_out_and_a_bad_repo_is_refused() {
    let rig = start(None, &["acme/api", "acme/docs"]).await;
    let mut client = rig.client().await;
    let removed = client
        .remove_from_repo_library(pb::RemoveFromRepoLibraryRequest {
            repo: Some(repo_ref("ACME/api")),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(library_names(&removed.repos), ["acme/docs"]);

    let refused = client
        .remove_from_repo_library(pb::RemoveFromRepoLibraryRequest {
            repo: Some(repo_ref("not a/repo")),
        })
        .await
        .unwrap_err();
    assert_eq!(invalid_field(&refused).0, "repo");
}

#[tokio::test(flavor = "multi_thread")]
async fn forgetting_the_library_says_how_many_repos_it_forgot() {
    let rig = start(None, &["acme/api", "acme/docs"]).await;
    let mut client = rig.client().await;
    let forgot = client
        .forget_repo_library(pb::ForgetRepoLibraryRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(forgot.forgotten, 2);
    let listed = client
        .list_repo_library(pb::ListRepoLibraryRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(listed.repos.is_empty());
    let again = client
        .forget_repo_library(pb::ForgetRepoLibraryRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(again.forgotten, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_default_owners_qualify_the_github_search_and_library_repos_are_left_out_of_it() {
    let rig = start(Some("# my scope\nowners = [\"acme\"]\n"), &["acme/api"]).await;
    let found = rig.search("ac", None).await.unwrap();
    assert_eq!(library_names(&found.library), ["acme/api"]);
    assert_eq!(github_names(&found), ["acme/ledger"]);
    assert_eq!(found.github[0].description, "The ledger");
    assert!(found.github[0].is_private);
    assert!(found.problem.is_none());
    assert_eq!(found.scope, Some(owners(&["acme"], true)));

    let searches = rig.recorded("searches");
    assert_eq!(searches.len(), 1);
    assert!(searches[0].contains("search/repositories"), "{}", searches[0]);
    assert!(searches[0].contains("user:acme"), "{}", searches[0]);
    // include_me is on by default, so the signed-in login is searched too.
    assert!(searches[0].contains("user:octo"), "{}", searches[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scope_for_one_search_is_used_once_and_never_saved() {
    let rig = start(Some("owners = [\"acme\"]\n"), &[]).await;
    let found = rig.search("ledger", Some(all_of_github())).await.unwrap();
    assert_eq!(found.scope, Some(all_of_github()));
    assert_eq!(github_names(&found), ["acme/ledger", "acme/api"]);
    let searches = rig.recorded("searches");
    assert_eq!(searches.len(), 1);
    assert!(!searches[0].contains("user:"), "{}", searches[0]);

    let saved = rig
        .client()
        .await
        .get_repo_scope(pb::GetRepoScopeRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(saved, owners(&["acme"], true));
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_owners_keeps_the_files_comments_and_all_of_github_is_refused() {
    let rig = start(Some("# kept\nowners = [\"acme\"]\n"), &[]).await;
    let mut client = rig.client().await;
    let saved = client
        .update_repo_scope(pb::UpdateRepoScopeRequest {
            scope: Some(owners(&["acme", "bazelbuild"], false)),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(saved, owners(&["acme", "bazelbuild"], false));
    let text = std::fs::read_to_string(rig.config()).unwrap();
    assert!(text.contains("# kept"), "{text}");
    let read_back = client
        .get_repo_scope(pb::GetRepoScopeRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(read_back, owners(&["acme", "bazelbuild"], false));

    let refused = client
        .update_repo_scope(pb::UpdateRepoScopeRequest {
            scope: Some(all_of_github()),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert_eq!(std::fs::read_to_string(rig.config()).unwrap(), text);
}

#[tokio::test(flavor = "multi_thread")]
async fn saving_a_scope_makes_the_config_folder_when_there_is_none() {
    let rig = start(None, &[]).await;
    let mut client = rig.client().await;
    client
        .update_repo_scope(pb::UpdateRepoScopeRequest {
            scope: Some(owners(&["acme"], true)),
        })
        .await
        .unwrap();
    let text = std::fs::read_to_string(rig.config()).unwrap();
    assert!(text.contains("acme"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_file_the_scope_is_your_repos_matched_from_the_listing() {
    let rig = start(None, &[]).await;
    let found = rig.search("led", None).await.unwrap();
    assert_eq!(found.scope.clone().and_then(|scope| scope.kind), Some(Kind::YourRepos(true)));
    assert_eq!(github_names(&found), ["acme/ledger"]);
    assert_eq!(rig.recorded("listings").len(), 1);
    assert!(rig.recorded("searches").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_shorter_than_the_scope_needs_asks_github_nothing() {
    let rig = start(Some("owners = [\"acme\"]\n"), &["acme/api"]).await;
    let found = rig.search("a", None).await.unwrap();
    assert_eq!(problem_kind(&found), Some(ProblemKind::TooShort));
    assert!(found.github.is_empty());
    assert_eq!(library_names(&found.library), ["acme/api"]);
    assert!(rig.recorded("searches").is_empty());

    // All of GitHub needs three.
    let found = rig.search("ab", Some(all_of_github())).await.unwrap();
    assert_eq!(problem_kind(&found), Some(ProblemKind::TooShort));
    assert!(rig.recorded("searches").is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_out_gh_and_a_rate_limit_are_problems_and_the_library_still_answers() {
    let rig = start(Some("owners = [\"acme\"]\n"), &["acme/api"]).await;
    std::fs::write(rig.gh.join("signed_out"), "").unwrap();
    let found = rig.search("ap", None).await.unwrap();
    assert_eq!(problem_kind(&found), Some(ProblemKind::GhSignedOut));
    assert_eq!(library_names(&found.library), ["acme/api"]);
    assert!(found.github.is_empty());

    std::fs::remove_file(rig.gh.join("signed_out")).unwrap();
    std::fs::write(rig.gh.join("limited"), "").unwrap();
    let found = rig.search("ap", None).await.unwrap();
    assert_eq!(problem_kind(&found), Some(ProblemKind::RateLimited));
    assert_eq!(library_names(&found.library), ["acme/api"]);

    // A failure is not remembered: once gh answers again, so does the search.
    std::fs::remove_file(rig.gh.join("limited")).unwrap();
    let found = rig.search("ap", None).await.unwrap();
    assert!(found.problem.is_none());
    assert_eq!(github_names(&found), ["acme/ledger"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_gh_that_cannot_be_run_is_a_problem_too() {
    let dir = folder();
    let missing = dir.path().join("no-such-gh");
    std::fs::create_dir_all(dir.path().join("config")).unwrap();
    std::fs::write(dir.path().join("config/repos.toml"), "owners = [\"acme\"]\ninclude_me = false\n").unwrap();
    let missing_text = missing.display().to_string();
    let daemon =
        Daemon::start_with_env(Build::Admin, dir, &[], &[("HUMATA2_GH", missing_text.as_str())]).await;
    let found = RepoServiceClient::new(daemon.channel().await)
        .search_repos(pb::SearchReposRequest {
            query: "ledger".to_owned(),
            scope: None,
            page: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(problem_kind(&found), Some(ProblemKind::GhMissing));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_same_query_and_scope_within_a_minute_asks_github_once() {
    let rig = start(Some("owners = [\"acme\"]\n"), &[]).await;
    rig.search("ledger", None).await.unwrap();
    rig.search("ledger", None).await.unwrap();
    assert_eq!(rig.recorded("searches").len(), 1);
    // Another query, or another scope, is another search.
    rig.search("api", None).await.unwrap();
    assert_eq!(rig.recorded("searches").len(), 2);
    rig.search("ledger", Some(all_of_github())).await.unwrap();
    assert_eq!(rig.recorded("searches").len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_github_does_not_know_is_named_and_the_rest_of_the_scope_still_searches() {
    let rig = start(Some("owners = [\"ghost\", \"acme\"]\n"), &[]).await;
    let found = rig.search("ledger", None).await.unwrap();
    assert_eq!(found.unknown_owners, ["ghost"]);
    assert!(found.problem.is_none());
    assert_eq!(github_names(&found), ["acme/ledger", "acme/api"]);
    let searches = rig.recorded("searches");
    let last = searches.last().unwrap();
    assert!(!last.contains("user:ghost"), "{last}");
    assert!(last.contains("user:acme"), "{last}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_scope_file_that_is_not_valid_fails_the_search_with_nothing_to_search() {
    let rig = start(Some("owners = [\n"), &["acme/api"]).await;
    let failed = rig.search("ap", None).await.unwrap_err();
    assert_eq!(failed.code(), Code::FailedPrecondition);
    // The library is still there to read.
    let listed = rig
        .client()
        .await
        .list_repo_library(pb::ListRepoLibraryRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(library_names(&listed.repos), ["acme/api"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_library_group_ignores_the_scope_and_pages_cover_the_github_group() {
    let rig = start(Some("owners = [\"bazelbuild\"]\n"), &["acme/api"]).await;
    let page = |token: String| pb::SearchReposRequest {
        query: "ledger".to_owned(),
        scope: Some(all_of_github()),
        page: Some(pb::PageRequest {
            page_size: 1,
            page_token: token,
        }),
    };
    let first = rig
        .client()
        .await
        .search_repos(page(String::new()))
        .await
        .unwrap()
        .into_inner();
    // acme/api doesn't match "ledger", so the library doesn't show it and
    // GitHub's group keeps both hits: one a page.
    assert_eq!(github_names(&first), ["acme/ledger"]);
    let token = first.page.unwrap().next_page_token;
    assert!(!token.is_empty());
    let second = rig
        .client()
        .await
        .search_repos(page(token))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(github_names(&second), ["acme/api"]);
    assert!(second.page.unwrap().next_page_token.is_empty());
    // A scope naming other owners doesn't hide the library's own repo.
    let found = rig.search("acme/api", None).await.unwrap();
    assert_eq!(library_names(&found.library), ["acme/api"]);
}
