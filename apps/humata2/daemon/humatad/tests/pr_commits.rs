//! Reviewed commits per PR (D299), through a real `humatad2`.
//!
//! The user marks a PR's commits reviewed one by one, or all at once with
//! RV-6 Mark reviewed; the marks belong to the PR, outlive the session and
//! the daemon, follow a commit's patch through a force push, and keep an
//! append-only history. The agent only reads them. GitHub is the fake `gh`
//! of `common/pr_github.rs`; the rpcs and the MCP tool are in SEAMS.md.
//!
//! Every test opens acme/api#482 with three commits: the fixture's one and
//! "Keep cents exact" and "Test rounding" on top.
//!
//! The PR milestone's A9.

mod common;
#[path = "common/mcp_client.rs"]
mod mcp_client;
#[path = "common/pr_github.rs"]
mod pr_github;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use common::{error_of, finished, git, repo_ref, wait_running};
use humata2_grpc::humata::v1 as pb;
use pr_github::{API, Fixture, LEDGER, LOGIN, Pr, TWO_COMMITS, as_agent, commit, failed, ok, push_head, repo_of};
use serde_json::json;

use pb::commit_review_entry::Kind;
use pb::pr_commit::State;

fn now_ms() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap()
}

fn unix_ms(at: &Option<humata2_grpc::Timestamp>) -> i64 {
    let at = at.as_ref().expect("a mark says when it was made");
    at.seconds * 1000 + i64::from(at.nanos) / 1_000_000
}

/// `git patch-id --stable` of `sha`, as git computes it.
fn patch_id(work: &Path, sha: &str) -> String {
    let show = Command::new("git")
        .args(["-C", &work.display().to_string(), "show", sha])
        .output()
        .unwrap();
    assert!(show.status.success(), "{show:?}");
    let mut patch_id = Command::new("git")
        .args(["patch-id", "--stable"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(&mut patch_id.stdin.take().unwrap(), &show.stdout).unwrap();
    let out = patch_id.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap().split_whitespace().next().unwrap().to_owned()
}

/// The PR's commits on its head, oldest first.
fn shas(pr: &Pr) -> Vec<String> {
    pr.commits().into_iter().map(|(sha, _)| sha).collect()
}

/// A fixture whose acme/api#482 has three commits.
async fn three_commits() -> Fixture {
    let fixture = Fixture::new().await;
    fixture.move_head(API, &TWO_COMMITS);
    fixture
}

async fn list(fixture: &Fixture, session: &pb::Session, pr: &Pr) -> pb::PullRequestCommits {
    fixture
        .reviews()
        .await
        .list_pull_request_commits(pb::ListPullRequestCommitsRequest {
            session: session.id.clone(),
            pr: Some(pr.reference()),
        })
        .await
        .unwrap()
        .into_inner()
}

fn marking(session: &pb::Session, pr: &Pr, commits: &[String], reviewed: bool) -> pb::MarkCommitsReviewedRequest {
    pb::MarkCommitsReviewedRequest {
        idempotency_key: None,
        session: session.id.clone(),
        pr: Some(pr.reference()),
        commits: commits.to_vec(),
        reviewed,
        presence: None,
    }
}

fn keyed(request: pb::MarkCommitsReviewedRequest, key: &str) -> pb::MarkCommitsReviewedRequest {
    pb::MarkCommitsReviewedRequest {
        idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
        ..request
    }
}

async fn mark_keyed(
    fixture: &Fixture,
    request: pb::MarkCommitsReviewedRequest,
) -> pb::PullRequestCommits {
    fixture.reviews().await.mark_commits_reviewed(request).await.unwrap().into_inner()
}

async fn mark(
    fixture: &Fixture,
    session: &pb::Session,
    pr: &Pr,
    commits: &[String],
    reviewed: bool,
) -> pb::PullRequestCommits {
    fixture
        .reviews()
        .await
        .mark_commits_reviewed(marking(session, pr, commits, reviewed))
        .await
        .unwrap()
        .into_inner()
}

/// Updates the session's clone to the PR's head (PR-16) and waits for it.
async fn update_clone(fixture: &Fixture, session: &pb::Session) {
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
}

async fn diffs(fixture: &Fixture) -> pb::diff_service_client::DiffServiceClient<tonic::transport::Channel> {
    pb::diff_service_client::DiffServiceClient::new(fixture.daemon.channel().await)
}

/// The RV-6 snapshots acme/api keeps for `session`, oldest first.
async fn kept(fixture: &Fixture, session: &pb::Session) -> Vec<pb::ReviewedSnapshot> {
    diffs(fixture)
        .await
        .list_reviewed_snapshots(pb::ListReviewedSnapshotsRequest {
            session: session.id.clone(),
            repos: vec![repo_ref("acme", "api")],
        })
        .await
        .unwrap()
        .into_inner()
        .snapshots
}

fn commit_of<'a>(listed: &'a pb::PullRequestCommits, sha: &str) -> &'a pb::PrCommit {
    listed
        .commits
        .iter()
        .find(|commit| commit.sha == sha)
        .unwrap_or_else(|| panic!("{sha} is listed: {listed:#?}"))
}

fn state_of(listed: &pb::PullRequestCommits, sha: &str) -> i32 {
    commit_of(listed, sha).state
}

fn by_the_user(by: &Option<pb::ReviewedBy>) -> bool {
    by.as_ref().is_some_and(|by| {
        by.gh_login == LOGIN
            && matches!(
                by.actor.as_ref().and_then(|actor| actor.actor.as_ref()),
                Some(pb::actor::Actor::User(_))
            )
    })
}

fn set(commits: &[String]) -> BTreeSet<String> {
    commits.iter().cloned().collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn marked_commits_stay_reviewed_across_a_restart() {
    let mut fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let shas = shas(&api);
    assert_eq!(shas.len(), 3);

    let marked = mark(&fixture, &session, &api, &shas[..2], true).await;
    fixture.restart().await;
    wait_running(&fixture.daemon, session.id.as_ref().unwrap()).await;

    let back = list(&fixture, &session, &api).await;
    assert_eq!(back, marked, "the store keeps the marks and their history");
    assert_eq!(state_of(&back, &shas[0]), State::Reviewed as i32);
    assert_eq!(state_of(&back, &shas[1]), State::Reviewed as i32);
    assert_eq!(state_of(&back, &shas[2]), State::Unreviewed as i32);
}

#[tokio::test(flavor = "multi_thread")]
async fn marking_records_when_which_head_and_who() {
    let fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let commits = api.commits();
    let shas = shas(&api);
    let head = api.head();

    let untouched = list(&fixture, &session, &api).await;
    assert_eq!(untouched.head_sha, head);
    assert_eq!(untouched.commits.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(), shas, "oldest first");
    assert!(untouched.commits.iter().all(|c| c.state == State::Unreviewed as i32), "{untouched:#?}");
    assert!(untouched.history.is_empty(), "{untouched:#?}");

    let before = now_ms();
    let marked = mark(&fixture, &session, &api, &shas[..1], true).await;
    let after = now_ms();

    let first = commit_of(&marked, &shas[0]);
    assert_eq!(first.state, State::Reviewed as i32);
    assert_eq!(first.subject, commits[0].1);
    assert_eq!(first.patch_id, patch_id(&api.work, &shas[0]));
    assert_eq!(first.reviewed_head_sha, head);
    assert!(by_the_user(&first.by), "{first:?}");
    let at = unix_ms(&first.reviewed_at);
    assert!(before - 1000 <= at && at <= after, "marked at {at}, between {before} and {after}");

    for sha in &shas[1..] {
        let other = commit_of(&marked, sha);
        assert_eq!(other.state, State::Unreviewed as i32, "{other:?}");
        assert!(other.reviewed_at.is_none() && other.by.is_none(), "{other:?}");
    }

    let [entry] = marked.history.as_slice() else {
        panic!("one mark, one history entry: {marked:#?}");
    };
    assert_eq!(entry.kind, Kind::Marked as i32);
    assert_eq!(entry.commits, shas[..1]);
    assert_eq!(entry.head_sha, head);
    assert_eq!(entry.snapshot_n, None, "marking one commit takes no snapshot");
    assert!(by_the_user(&entry.by), "{entry:?}");
    assert_eq!(unix_ms(&entry.at), at);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_force_push_carries_a_mark_to_the_same_patch() {
    let fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let old = shas(&api);
    let marked = mark(&fixture, &session, &api, &old, true).await;

    // main moves under the PR and the PR is rebased onto it: every sha
    // changes, no patch does.
    let main = fixture.world.root.join("main-acme-api");
    commit(&main, "Add a changelog", &[("CHANGELOG.md", 0, "Refunds round half-even.")]);
    git(&["-C", &main.display().to_string(), "push", "-q", "origin", "main"]);
    let work = api.work.display().to_string();
    git(&["-C", &work, "fetch", "-q", "origin"]);
    git(&["-C", &work, "rebase", "-q", "origin/main"]);
    push_head(&api, true);
    fixture.gh.publish(&fixture.prs);
    update_clone(&fixture, &session).await;

    let new = shas(&api);
    assert_eq!(new.len(), 3);
    let after = list(&fixture, &session, &api).await;
    assert_eq!(after.head_sha, api.head());
    assert_eq!(after.commits.iter().map(|c| c.sha.clone()).collect::<Vec<_>>(), new);
    for (was, now) in old.iter().zip(&new) {
        assert_ne!(was, now);
        let carried = commit_of(&after, now);
        assert_eq!(carried.state, State::Carried as i32, "{carried:?}");
        assert_eq!(&carried.reviewed_as_sha, was, "{carried:?}");
        assert_eq!(carried.patch_id, patch_id(&api.work, now));
    }

    // The history keeps the marks and adds the carry.
    assert_eq!(after.history[..marked.history.len()], marked.history[..]);
    let carry = after.history.last().unwrap();
    assert_eq!(carry.kind, Kind::Carried as i32, "{after:#?}");
    assert_eq!(set(&carry.commits), set(&new));
    assert_eq!(carry.head_sha, api.head());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_changed_patch_after_a_force_push_is_unreviewed() {
    let fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let old = shas(&api);
    mark(&fixture, &session, &api, &old, true).await;

    // The second commit is rewritten with a different change; the third is
    // recommitted as it was, on top of it.
    let work = api.work.display().to_string();
    git(&["-C", &work, "reset", "-q", "--hard", "HEAD~2"]);
    commit(
        &api.work,
        TWO_COMMITS[0].0,
        &[("refund.rs", 3, "    let rounded = round_half_even_exact(cents - fee - tax);")],
    );
    commit(&api.work, TWO_COMMITS[1].0, TWO_COMMITS[1].1);
    push_head(&api, true);
    fixture.gh.publish(&fixture.prs);
    update_clone(&fixture, &session).await;

    let new = shas(&api);
    assert_eq!(new[0], old[0], "the first commit was kept");
    assert_ne!(patch_id(&api.work, &new[1]), patch_id(&api.work, &old[1]), "a changed patch");
    assert_eq!(patch_id(&api.work, &new[2]), patch_id(&api.work, &old[2]), "the same patch");

    let after = list(&fixture, &session, &api).await;
    assert_eq!(state_of(&after, &new[0]), State::Reviewed as i32);
    let changed = commit_of(&after, &new[1]);
    assert_eq!(changed.state, State::Unreviewed as i32, "{changed:?}");
    assert!(changed.reviewed_as_sha.is_empty(), "{changed:?}");
    let same = commit_of(&after, &new[2]);
    assert_eq!(same.state, State::Carried as i32, "{same:?}");
    assert_eq!(same.reviewed_as_sha, old[2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn unmarking_adds_history_and_keeps_earlier_entries() {
    let fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let shas = shas(&api);

    let marked = mark(&fixture, &session, &api, &shas[..2], true).await;
    let unmarked = mark(&fixture, &session, &api, &shas[1..2], false).await;

    assert_eq!(state_of(&unmarked, &shas[0]), State::Reviewed as i32);
    assert_eq!(state_of(&unmarked, &shas[1]), State::Unreviewed as i32);
    assert_eq!(unmarked.history.len(), marked.history.len() + 1, "{unmarked:#?}");
    assert_eq!(unmarked.history[..marked.history.len()], marked.history[..], "earlier entries stay");
    let last = unmarked.history.last().unwrap();
    assert_eq!(last.kind, Kind::Unmarked as i32);
    assert_eq!(last.commits, shas[1..2]);
    assert!(by_the_user(&last.by), "{last:?}");
    assert!(
        unmarked.history.windows(2).all(|pair| pair[0].seq < pair[1].seq),
        "entries in the order they were made: {unmarked:#?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_reviewed_marks_every_commit_up_to_head_and_pins_its_snapshot() {
    let fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let shas = shas(&api);
    let head = api.head();
    // One commit by hand first: that takes no snapshot.
    mark(&fixture, &session, &api, &shas[..1], true).await;
    assert!(kept(&fixture, &session).await.is_empty(), "marking a commit takes no snapshot");

    let marked = diffs(&fixture)
        .await
        .mark_reviewed(pb::MarkReviewedRequest {
            session: session.id.clone(),
            ..pb::MarkReviewedRequest::default()
        })
        .await
        .unwrap()
        .into_inner();
    let snapshot = marked
        .snapshots
        .iter()
        .find(|snapshot| snapshot.repo == Some(repo_ref("acme", "api")))
        .expect("acme/api is snapshotted");

    let listed = list(&fixture, &session, &api).await;
    for sha in &shas {
        let reviewed = commit_of(&listed, sha);
        assert_eq!(reviewed.state, State::Reviewed as i32, "{reviewed:?}");
    }
    for sha in &shas[1..] {
        assert_eq!(commit_of(&listed, sha).reviewed_head_sha, head);
    }
    let whole = listed.history.last().unwrap();
    assert_eq!(whole.kind, Kind::WholePr as i32, "{listed:#?}");
    assert_eq!(set(&whole.commits), set(&shas), "every commit up to the head");
    assert_eq!(whole.head_sha, head);
    assert_eq!(whole.snapshot_n, Some(snapshot.n));
    assert!(by_the_user(&whole.by), "{whole:?}");

    // The snapshot stays past RV-30's last 20, and its ref is in the clone.
    let pinned = kept(&fixture, &session).await;
    let kept_one = pinned.iter().find(|kept| kept.n == snapshot.n).expect("the snapshot is kept");
    assert!(kept_one.pinned, "{kept_one:?}");
    let refs = git(&[
        "-C",
        &repo_of(&session, &api).path,
        "for-each-ref",
        "--format=%(objectname)",
        "refs/humata/reviewed",
    ]);
    assert!(refs.lines().any(|sha| sha == snapshot.commit), "{refs}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_review_of_the_pr_sees_its_marks() {
    let fixture = three_commits().await;
    let first = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let shas = shas(&api);
    let marked = mark(&fixture, &first, &api, &shas[..2], true).await;

    let second = fixture.start_review(&[API]).await;
    assert_ne!(first.id, second.id);
    let seen = list(&fixture, &second, &api).await;
    assert_eq!(state_of(&seen, &shas[0]), State::Reviewed as i32);
    assert_eq!(state_of(&seen, &shas[1]), State::Reviewed as i32);
    assert_eq!(state_of(&seen, &shas[2]), State::Unreviewed as i32);
    assert_eq!(seen.history, marked.history, "the history is the PR's, not the session's");
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_user_marks_commits_the_agent_reads_them() {
    let fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let mut agent = fixture.agent(&session).await;
    let api = fixture.prs[API].clone();
    let shas = shas(&api);

    let refused = fixture
        .reviews()
        .await
        .mark_commits_reviewed(as_agent(marking(&session, &api, &shas[..1], true), &agent.token))
        .await
        .unwrap_err();
    let error = error_of(&refused);
    assert!(matches!(error.kind, Some(pb::error::Kind::UserOnly(_))), "{error:?}");
    let untouched = list(&fixture, &session, &api).await;
    assert!(untouched.history.is_empty(), "the agent marked nothing: {untouched:#?}");

    mark(&fixture, &session, &api, &shas[..1], true).await;
    let reply = agent.call("review_commits", json!({ "pr": api.slug() })).await;
    let result = ok(&reply);
    assert_eq!(result["head_sha"], api.head(), "{reply}");
    let commits = result["commits"].as_array().expect("the commits are listed");
    let listed: Vec<&str> = commits.iter().map(|c| c["sha"].as_str().unwrap_or_default()).collect();
    assert_eq!(listed, shas.iter().map(String::as_str).collect::<Vec<_>>(), "{reply}");
    assert_eq!(commits[0]["state"], "reviewed", "{reply}");
    assert_eq!(commits[1]["state"], "unreviewed", "{reply}");
    assert_eq!(result["history"][0]["kind"], "marked", "{reply}");

    // A PR outside the review is not the agent's to read.
    let outside = agent
        .call("review_commits", json!({ "pr": fixture.prs[LEDGER].slug() }))
        .await;
    failed(&outside);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_mark_returns_its_first_answer_and_marks_once() {
    let mut fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let shas = shas(&api);
    let first_call = keyed(marking(&session, &api, &shas[..1], true), "mark-1");

    let first = mark_keyed(&fixture, first_call.clone()).await;
    let second_call = keyed(marking(&session, &api, &shas[1..2], true), "mark-2");
    let other = mark_keyed(&fixture, second_call).await;
    assert_eq!(other.history.len(), 2, "{other:#?}");

    let repeat = mark_keyed(&fixture, first_call.clone()).await;
    assert_eq!(repeat, first, "a repeat answers what the first call did");
    fixture.restart().await;
    wait_running(&fixture.daemon, session.id.as_ref().unwrap()).await;
    let after_restart = mark_keyed(&fixture, first_call).await;
    assert_eq!(after_restart, first, "the answer outlives the daemon");

    let listed = list(&fixture, &session, &api).await;
    assert_eq!(listed.history, other.history, "neither repeat marked again: {listed:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_key_does_not_let_the_agent_past_the_user_only_check() {
    let fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let agent = fixture.agent(&session).await;
    let api = fixture.prs[API].clone();
    let shas = shas(&api);
    let call = keyed(marking(&session, &api, &shas[..1], true), "mark-1");
    mark_keyed(&fixture, call.clone()).await;

    let refused = fixture
        .reviews()
        .await
        .mark_commits_reviewed(as_agent(call, &agent.token))
        .await
        .unwrap_err();
    let error = error_of(&refused);
    assert!(matches!(error.kind, Some(pb::error::Kind::UserOnly(_))), "{error:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn update_clone_carries_marks_when_it_moves_the_clone() {
    let fixture = three_commits().await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let old = shas(&api);
    mark(&fixture, &session, &api, &old, true).await;

    let main = fixture.world.root.join("main-acme-api");
    commit(&main, "Add a changelog", &[("CHANGELOG.md", 0, "Refunds round half-even.")]);
    git(&["-C", &main.display().to_string(), "push", "-q", "origin", "main"]);
    let work = api.work.display().to_string();
    git(&["-C", &work, "fetch", "-q", "origin"]);
    git(&["-C", &work, "rebase", "-q", "origin/main"]);
    push_head(&api, true);
    fixture.gh.publish(&fixture.prs);
    let before = now_ms();
    update_clone(&fixture, &session).await;
    let updated = now_ms();
    // A carry worked out on the next read would be dated after this.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let after = list(&fixture, &session, &api).await;
    let carry = after.history.last().unwrap();
    assert_eq!(carry.kind, Kind::Carried as i32, "{after:#?}");
    assert_eq!(set(&carry.commits), set(&shas(&api)));
    let at = unix_ms(&carry.at);
    assert!(
        before <= at && at <= updated,
        "carried at {at}, updated between {before} and {updated}"
    );
    for now in shas(&api) {
        let carried = commit_of(&after, &now);
        assert_eq!(carried.state, State::Carried as i32, "{carried:?}");
    }
}
