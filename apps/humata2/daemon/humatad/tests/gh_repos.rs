//! CH-3: SearchGitHubRepos lists the repos the host's `gh` sees, fuzzy
//! matched and paged. `gh` is a fake script (admin builds read
//! `HUMATA2_GH`); it prints what `gh api --paginate user/repos --jq` would.

mod common;

use common::{Build, Daemon, World, folder};
use humata2_grpc::humata::v1 as pb;
use tonic::Code;
use tonic::transport::Channel;

/// A signed-in `gh` that sees four repos, most recently pushed first. While
/// a `broken` file sits beside it, listing fails as a signed-out `gh` does;
/// each listing adds a line to `calls`.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
case "$*" in
  "api user --jq .login"|"api user -q .login") echo "octo" ; exit 0 ;;
esac
case "$*" in
  *user/repos*)
    echo listed >> "$here/calls"
    if [ -f "$here/broken" ]; then
      echo "To get started with GitHub CLI, please run:  gh auth login" >&2
      exit 4
    fi
    echo '{"full_name":"acme/ledger","description":"The ledger","private":true,"default_branch":"main","pushed_at":"2026-11-15T10:00:00Z"}'
    echo '{"full_name":"acme/api","description":null,"private":false,"default_branch":"trunk","pushed_at":"2026-11-14T10:00:00Z"}'
    echo '{"full_name":"acme/docs","description":"","private":false,"default_branch":"main","pushed_at":"2026-11-13T10:00:00Z"}'
    echo '{"full_name":"octo/notes","description":"Notes","private":false,"default_branch":"main","pushed_at":"2026-11-12T10:00:00Z"}'
    ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

struct Gh {
    daemon: Daemon,
    dir: std::path::PathBuf,
}

async fn with_gh() -> Gh {
    let dir = folder();
    let world = World::new(dir.path());
    let gh_dir = dir.path().join("gh");
    std::fs::create_dir_all(&gh_dir).expect("the gh folder is made");
    let gh = gh_dir.join("gh");
    std::fs::write(&gh, FAKE_GH).expect("the fake gh is written");
    std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755))
        .expect("the fake gh runs");
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let gh_text = gh.display().to_string();
    let daemon =
        Daemon::start_with_env(Build::Admin, dir, &refs, &[("HUMATA2_GH", gh_text.as_str())])
            .await;
    Gh {
        daemon,
        dir: gh_dir,
    }
}

impl Gh {
    async fn search(
        &self,
        query: &str,
        page: Option<pb::PageRequest>,
    ) -> Result<pb::SearchGitHubReposResponse, tonic::Status> {
        pb::channel_service_client::ChannelServiceClient::<Channel>::new(
            self.daemon.channel().await,
        )
        .search_git_hub_repos(pb::SearchGitHubReposRequest {
            query: query.to_owned(),
            page,
        })
        .await
        .map(tonic::Response::into_inner)
    }

    /// How many times the daemon ran `gh` to list repos.
    fn listings(&self) -> usize {
        std::fs::read_to_string(self.dir.join("calls"))
            .map(|text| text.lines().count())
            .unwrap_or(0)
    }
}

fn names(response: &pb::SearchGitHubReposResponse) -> Vec<String> {
    response
        .repos
        .iter()
        .filter_map(|repo| repo.repo.as_ref())
        .map(|repo| format!("{}/{}", repo.owner, repo.name))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_query_lists_every_repo_gh_sees_most_recently_pushed_first() {
    let gh = with_gh().await;
    let listed = gh.search("", None).await.expect("the repos are listed");
    assert_eq!(names(&listed), ["acme/ledger", "acme/api", "acme/docs", "octo/notes"]);
    let ledger = &listed.repos[0];
    assert_eq!(ledger.description, "The ledger");
    assert!(ledger.is_private);
    assert_eq!(ledger.default_branch, "main");
    assert!(ledger.pushed_at.is_some());
    let api = &listed.repos[1];
    assert_eq!(api.description, "");
    assert!(!api.is_private);
    assert_eq!(api.default_branch, "trunk");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_query_keeps_the_fuzzy_matches_whole_ones_first() {
    let gh = with_gh().await;
    let listed = gh.search("LDGR", None).await.expect("the repos are listed");
    assert_eq!(names(&listed), ["acme/ledger"]);
    // "es" is whole in octo/notes and scattered in acme/docs.
    let listed = gh.search("es", None).await.expect("the repos are listed");
    assert_eq!(names(&listed), ["octo/notes", "acme/docs"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_follow_on_from_the_token() {
    let gh = with_gh().await;
    let first = gh
        .search(
            "",
            Some(pb::PageRequest {
                page_size: 3,
                page_token: String::new(),
            }),
        )
        .await
        .expect("the first page is listed");
    assert_eq!(names(&first), ["acme/ledger", "acme/api", "acme/docs"]);
    let token = first.page.map(|page| page.next_page_token).unwrap_or_default();
    assert!(!token.is_empty(), "a token leads to the rest");
    let rest = gh
        .search(
            "",
            Some(pb::PageRequest {
                page_size: 3,
                page_token: token,
            }),
        )
        .await
        .expect("the next page is listed");
    assert_eq!(names(&rest), ["octo/notes"]);
    assert!(rest.page.map_or(true, |page| page.next_page_token.is_empty()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listing_within_a_minute_does_not_run_gh_again() {
    let gh = with_gh().await;
    gh.search("", None).await.expect("the repos are listed");
    gh.search("acme", None).await.expect("the repos are listed");
    assert_eq!(gh.listings(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_out_gh_is_a_failed_precondition_and_is_not_remembered() {
    let gh = with_gh().await;
    std::fs::write(gh.dir.join("broken"), "").expect("gh is signed out");
    let failed = gh.search("", None).await.expect_err("listing fails");
    assert_eq!(failed.code(), Code::FailedPrecondition);
    std::fs::remove_file(gh.dir.join("broken")).expect("gh is signed in again");
    let listed = gh.search("", None).await.expect("the repos are listed");
    assert_eq!(listed.repos.len(), 4);
}
