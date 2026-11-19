//! PR-10: a pull request review's Diff compares each repository with its
//! pull request's base, through a real `humatad2`. Here acme/ledger#91
//! merges into release/2.4, which has a commit main doesn't, so the base
//! decides what the Diff lists.
//!
//! The base is fetched into the clone the first time, once however many
//! Diffs ask at once, and again when the pull request's head moves or
//! GitHub says it now merges somewhere else.
//!
//! GitHub is the fake `gh` of `common/pr_github.rs`.

use humatad_test_common as common;
use humatad_test_common::mcp_client;
use humatad_test_common::pr_github;

use common::{create_channel, error_of, finished, git, repo_ref, start_session, wait_running};
use humata2_grpc::humata::v1 as pb;
use pr_github::{API, Fixture, LEDGER, Pr, push_head};

const RELEASE: &str = "release/2.4";

/// Commits `file` on release/2.4 and pushes it, cutting the branch from
/// main first when `cut`. The PR's own branch is checked out after.
fn commit_on_release(pr: &Pr, file: &str, cut: bool) {
    let work = pr.work.display().to_string();
    if cut {
        git(&["-C", &work, "fetch", "-q", "origin"]);
        git(&["-C", &work, "checkout", "-q", "-b", RELEASE, "origin/main"]);
    } else {
        git(&["-C", &work, "checkout", "-q", RELEASE]);
    }
    std::fs::write(pr.work.join(file), format!("// {file} is on {RELEASE} only\n")).unwrap();
    git(&["-C", &work, "add", "."]);
    git(&["-C", &work, "commit", "-q", "-m", &format!("Add {file} on {RELEASE}")]);
    git(&["-C", &work, "push", "-q", "origin", RELEASE]);
    git(&["-C", &work, "checkout", "-q", pr.head_branch]);
}

/// Rebases the PR's head onto release/2.4 and force-pushes it.
fn rebase_onto_release(pr: &Pr) {
    let work = pr.work.display().to_string();
    git(&["-C", &work, "rebase", "-q", RELEASE]);
    push_head(pr, true);
}

/// Cuts release/2.4 from main with a commit of its own (release.rs), and
/// moves acme/ledger#91 onto it: GitHub says the pull request merges into
/// release/2.4 now.
fn retarget_ledger_to_release(fixture: &mut Fixture) {
    commit_on_release(&fixture.prs[LEDGER], "release.rs", true);
    rebase_onto_release(&fixture.prs[LEDGER]);
    fixture.prs[LEDGER].base = RELEASE;
    fixture.gh.publish(&fixture.prs);
}

/// A session of acme/ledger that reviews no pull request.
async fn plain_session(fixture: &Fixture) -> pb::Session {
    let (channel, _) = create_channel(&fixture.daemon, "Ledger", &[("acme", "ledger")]).await;
    let (id, _) = start_session(&fixture.daemon, &channel, "Ledger work").await;
    wait_running(&fixture.daemon, &id).await
}

fn diff_base(base: pb::diff_base::Base) -> Option<pb::DiffBase> {
    Some(pb::DiffBase { base: Some(base) })
}

fn pull_request_base() -> Option<pb::DiffBase> {
    diff_base(pb::diff_base::Base::PullRequestBase(pb::PullRequestBase {}))
}

async fn diff(
    fixture: &Fixture,
    session: &pb::Session,
    base: Option<pb::DiffBase>,
) -> Result<pb::GetDiffResponse, tonic::Status> {
    pb::diff_service_client::DiffServiceClient::new(fixture.daemon.channel().await)
        .get_diff(pb::GetDiffRequest {
            session: session.id.clone(),
            base,
            repos: Vec::new(),
        })
        .await
        .map(tonic::Response::into_inner)
}

/// The paths the Diff lists, sorted.
fn paths(diff: &pb::GetDiffResponse) -> Vec<String> {
    let mut paths: Vec<String> = diff
        .repos
        .iter()
        .flat_map(|repo| repo.files.iter().map(|file| file.path.clone()))
        .collect();
    paths.sort();
    paths
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pull_request_into_a_release_branch_shows_its_diff_against_that_branch() {
    let mut fixture = Fixture::new().await;
    retarget_ledger_to_release(&mut fixture);
    let session = fixture.start_review(&[LEDGER]).await;

    let against_base = diff(&fixture, &session, pull_request_base()).await.unwrap();
    assert_eq!(
        paths(&against_base),
        ["ledger.rs"],
        "against its base the review shows only what the pull request changes: {against_base:#?}"
    );
    assert!(
        against_base.repos.iter().all(|repo| repo.label.contains(RELEASE)),
        "the Diff says what it compares with: {against_base:#?}"
    );

    let origin_main = diff_base(pb::diff_base::Base::OriginMain(pb::OriginMain {}));
    let against_main = diff(&fixture, &session, origin_main).await.unwrap();
    assert_eq!(
        paths(&against_main),
        ["ledger.rs", "release.rs"],
        "origin/main stays available, and differs here: {against_main:#?}"
    );

    let file = pb::diff_service_client::DiffServiceClient::new(fixture.daemon.channel().await)
        .get_file_diff(pb::GetFileDiffRequest {
            session: session.id.clone(),
            base: pull_request_base(),
            file: Some(pb::RepoPath {
                repo: Some(repo_ref("acme", "ledger")),
                path: "ledger.rs".to_owned(),
            }),
            ..Default::default()
        })
        .await;
    assert!(file.is_ok(), "a file's hunks come against the same base: {file:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_that_reviews_no_pull_request_has_no_pull_request_base() {
    let fixture = Fixture::new().await;
    let review = fixture.start_review(&[LEDGER]).await;
    let plain = plain_session(&fixture).await;

    assert!(diff(&fixture, &review, pull_request_base()).await.is_ok());
    let refused = diff(&fixture, &plain, pull_request_base()).await.unwrap_err();
    let error = error_of(&refused);
    assert!(
        matches!(error.kind, Some(pb::error::Kind::InvalidField(_))),
        "the base is refused, with the field named: {refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn diffs_asked_at_once_fetch_a_new_base_once_and_all_succeed() {
    let mut fixture = Fixture::new().await;
    retarget_ledger_to_release(&mut fixture);
    let session = fixture.start_review(&[LEDGER]).await;

    // The clone doesn't have origin/release/2.4 yet: every one of these
    // needs it at the same moment.
    let (a, b, c, d) = tokio::join!(
        diff(&fixture, &session, pull_request_base()),
        diff(&fixture, &session, pull_request_base()),
        diff(&fixture, &session, pull_request_base()),
        diff(&fixture, &session, pull_request_base()),
    );
    for each in [a, b, c, d] {
        let each = each.expect("no Diff trips over another's fetch");
        assert_eq!(paths(&each), ["ledger.rs"], "{each:#?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_base_that_moved_is_fetched_again_when_the_clone_follows_the_head() {
    let mut fixture = Fixture::new().await;
    retarget_ledger_to_release(&mut fixture);
    let session = fixture.start_review(&[LEDGER]).await;
    let first = diff(&fixture, &session, pull_request_base()).await.unwrap();
    assert_eq!(paths(&first), ["ledger.rs"], "{first:#?}");

    // release/2.4 gains notes.rs, and #91 is rebased onto it.
    commit_on_release(&fixture.prs[LEDGER], "notes.rs", false);
    rebase_onto_release(&fixture.prs[LEDGER]);
    fixture.gh.publish(&fixture.prs);
    let operation = fixture
        .reviews()
        .await
        .update_clone(pb::UpdateCloneRequest {
            idempotency_key: None,
            session: session.id.clone(),
        })
        .await
        .unwrap()
        .into_inner();
    let done = finished(&fixture.daemon, &operation).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");

    let after = diff(&fixture, &session, pull_request_base()).await.unwrap();
    assert_eq!(
        paths(&after),
        ["ledger.rs"],
        "notes.rs is the base's, not the pull request's: {after:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pull_request_retargeted_after_the_start_diffs_against_its_new_base_once_checked() {
    let mut fixture = Fixture::new().await;
    let session = fixture.start_review(&[LEDGER]).await;
    let first = diff(&fixture, &session, pull_request_base()).await.unwrap();
    assert!(
        first.repos.iter().all(|repo| !repo.label.contains(RELEASE)),
        "{first:#?}"
    );

    // GitHub retargets #91 onto a new release/2.4; its head stays.
    commit_on_release(&fixture.prs[LEDGER], "release.rs", true);
    fixture.prs[LEDGER].base = RELEASE;
    fixture.gh.publish(&fixture.prs);
    fixture.check_heads(&session).await;

    let after = diff(&fixture, &session, pull_request_base()).await.unwrap();
    assert!(
        after.repos.iter().all(|repo| repo.label.contains(RELEASE)),
        "the check saw the new base, and the Diff compares with it: {after:#?}"
    );
    assert_eq!(paths(&after), ["ledger.rs"], "{after:#?}");
}

fn repo_on(owner: &str, name: &str, base: pb::diff_spec_base::Base) -> pb::RepoDiffSpec {
    pb::RepoDiffSpec {
        repo: Some(repo_ref(owner, name)),
        base: Some(pb::DiffSpecBase { base: Some(base) }),
        ..pb::RepoDiffSpec::default()
    }
}

fn spec_base(repos: Vec<pb::RepoDiffSpec>) -> Option<pb::DiffBase> {
    diff_base(pb::diff_base::Base::Spec(pb::DiffSpec { repos }))
}

fn paths_in(diff: &pb::GetDiffResponse, name: &str) -> Vec<String> {
    let mut paths: Vec<String> = diff
        .repos
        .iter()
        .filter(|repo| repo.repo.as_ref().is_some_and(|each| each.name == name))
        .flat_map(|repo| repo.files.iter().map(|file| file.path.clone()))
        .collect();
    paths.sort();
    paths
}

#[tokio::test(flavor = "multi_thread")]
async fn one_repo_on_its_pull_requests_base_and_another_on_a_rev_each_keep_their_own() {
    let mut fixture = Fixture::new().await;
    retarget_ledger_to_release(&mut fixture);
    let session = fixture.start_review(&[API, LEDGER]).await;
    let origin_main = diff_base(pb::diff_base::Base::OriginMain(pb::OriginMain {}));
    let against_main = diff(&fixture, &session, origin_main).await.unwrap();

    let on_pr = pb::diff_spec_base::Base::PullRequestBase(pb::PullRequestBaseRef {});
    let on_rev = pb::diff_spec_base::Base::Rev("origin/main".to_owned());
    let mixed = spec_base(vec![repo_on("acme", "ledger", on_pr), repo_on("acme", "api", on_rev)]);
    let both = diff(&fixture, &session, mixed).await.unwrap();
    assert_eq!(
        paths_in(&both, "ledger"),
        ["ledger.rs"],
        "ledger is compared with release/2.4, which has release.rs already: {both:#?}"
    );
    assert_eq!(
        paths_in(&both, "api"),
        paths_in(&against_main, "api"),
        "api is compared with the rev it was given: {both:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repo_that_is_no_pull_requests_is_refused_the_pull_request_base() {
    let fixture = Fixture::new().await;
    let plain = plain_session(&fixture).await;
    let on_pr = pb::diff_spec_base::Base::PullRequestBase(pb::PullRequestBaseRef {});
    let refused = diff(&fixture, &plain, spec_base(vec![repo_on("acme", "ledger", on_pr)]))
        .await
        .unwrap_err();
    let error = error_of(&refused);
    assert!(
        matches!(error.kind, Some(pb::error::Kind::InvalidField(_))),
        "the base is refused, with the field named: {refused:?}"
    );
}
