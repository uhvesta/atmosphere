//! RL-8: RepoService.ListRepoOwners names the orgs the host's `gh` user
//! belongs to, for **+ Org**'s suggestions. `gh` is a fake script (admin
//! builds read `HUMATA2_GH`). A `gh` that cannot list them is a problem in
//! the answer, not a failed call, and a peer may not ask.

use humatad_test_common as common;

use common::{Build, Daemon, folder};
use humata2_grpc::humata::v1 as pb;
use pb::repo_search_problem::Kind as ProblemKind;
use pb::repo_service_client::RepoServiceClient;

/// A signed-in `gh` as `octo` in the orgs acme and bazelbuild (one listed
/// twice, as paging can), or signed out when a `signed_out` file is beside it.
const FAKE_GH: &str = r#"#!/bin/sh
here=$(dirname "$0")
case "$*" in
  *user/orgs*)
    if [ -f "$here/signed_out" ]; then
      echo "To get started with GitHub CLI, please run:  gh auth login" >&2
      exit 4
    fi
    printf 'acme\nbazelbuild\n\nacme\n'
    ;;
  *) echo "fake gh: unexpected $*" >&2; exit 1 ;;
esac
"#;

async fn start(signed_out: bool) -> Daemon {
    let dir = folder();
    let gh_dir = dir.path().join("gh");
    std::fs::create_dir_all(&gh_dir).unwrap();
    let gh = gh_dir.join("gh");
    std::fs::write(&gh, FAKE_GH).unwrap();
    std::fs::set_permissions(&gh, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    if signed_out {
        std::fs::write(gh_dir.join("signed_out"), "").unwrap();
    }
    let gh_text = gh.display().to_string();
    Daemon::start_with_env(Build::Admin, dir, &[], &[("HUMATA2_GH", gh_text.as_str())]).await
}

async fn owners(daemon: &Daemon) -> pb::ListRepoOwnersResponse {
    RepoServiceClient::new(daemon.channel().await)
        .list_repo_owners(pb::ListRepoOwnersRequest {})
        .await
        .unwrap()
        .into_inner()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_orgs_are_listed_once_each_in_the_order_gh_gives_them() {
    let daemon = start(false).await;
    let answer = owners(&daemon).await;
    assert_eq!(answer.owners, vec!["acme", "bazelbuild"]);
    assert!(answer.problem.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_out_gh_is_a_problem_with_no_orgs_not_a_failed_call() {
    let daemon = start(true).await;
    let answer = owners(&daemon).await;
    assert!(answer.owners.is_empty());
    let kind = answer.problem.map(|problem| problem.kind);
    assert_eq!(kind, Some(ProblemKind::GhSignedOut as i32));
}
