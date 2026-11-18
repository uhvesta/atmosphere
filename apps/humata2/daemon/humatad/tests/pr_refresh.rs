//! Check a PR for new commits now (D298), through a real `humatad2`.
//!
//! Refresh is `CheckHeads` asked by the user rather than by the D286 poll.
//! Every daemon here polls once an hour, so whatever a test sees came from
//! its own check. GitHub is the fake `gh` of `common/pr_github.rs`; the
//! contract is in SEAMS.md.
//!
//! The PR milestone's A8.

mod common;
// The shared fixture's agent speaks MCP; these tests never start one.
#[path = "common/mcp_client.rs"]
#[allow(dead_code)]
mod mcp_client;
#[path = "common/pr_github.rs"]
mod pr_github;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{get_session, git};
use humata2_grpc::humata::v1 as pb;
use pr_github::{API, Fixture, LEDGER, SLOW, TWO_COMMITS, change_of, clone_head, repo_of};

/// A poll interval no test outlives: only the tests' own checks run.
const NEVER: &str = "3600000";

fn now_ms() -> i64 {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()).unwrap()
}

fn unix_ms(at: &Option<humata2_grpc::Timestamp>) -> i64 {
    let at = at.as_ref().expect("the check says when it ran");
    at.seconds * 1000 + i64::from(at.nanos) / 1_000_000
}

/// Waits until `gh api graphql` has been run `calls` times in all.
async fn graphql_calls_reach(fixture: &Fixture, calls: usize) {
    let deadline = Instant::now() + SLOW;
    while fixture.gh.graphql_calls() < calls {
        assert!(Instant::now() < deadline, "GitHub was never asked for the heads:\n{}", fixture.gh.log());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn refreshing_now_finds_new_commits_before_the_next_poll() {
    let fixture = Fixture::with_env(&[("HUMATA2_PR_POLL_MS", NEVER)]).await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    let reviewed = clone_head(repo_of(&session, &api));

    fixture.move_head(API, &TWO_COMMITS);
    let heads = fixture.check_heads(&session).await;
    let change = change_of(&heads, &api).expect("the check finds api#482's new commits");
    assert_eq!(change.new_commits, 2, "{change:?}");
    assert!(!change.force_pushed, "{change:?}");
    assert_eq!(change.state, pb::PullRequestState::Open as i32);

    // A check never moves the clone or tells the agent; Update the clone does.
    let still = get_session(&fixture.daemon, session.id.as_ref().unwrap()).await;
    assert_eq!(clone_head(repo_of(&still, &api)), reviewed);
    let said = std::fs::read_to_string(fixture.world.conversation(&session.conversation_id))
        .unwrap_or_default();
    for (subject, _) in TWO_COMMITS {
        assert!(!said.contains(subject), "the agent is not told of {subject} by a check: {said}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn refreshing_an_unchanged_pr_says_when_it_was_checked_and_moves_nothing() {
    let fixture = Fixture::with_env(&[("HUMATA2_PR_POLL_MS", NEVER)]).await;
    let session = fixture.start_review(&[API, LEDGER]).await;
    let api = fixture.prs[API].clone();
    let reviewed = clone_head(repo_of(&session, &api));
    let asked = fixture.gh.graphql_calls();

    let before = now_ms();
    let heads = fixture.check_heads(&session).await;
    let after = now_ms();
    assert!(heads.changes.is_empty(), "nothing moved, so nothing changed: {heads:?}");
    let at = unix_ms(&heads.checked_at);
    // A timestamp may be cut to the second below the clock's millisecond.
    assert!(before - 1000 <= at && at <= after, "checked at {at}, asked between {before} and {after}");
    assert!(fixture.gh.graphql_calls() > asked, "an unchanged answer still came from GitHub");

    let still = get_session(&fixture.daemon, session.id.as_ref().unwrap()).await;
    let repo = repo_of(&still, &api);
    assert_eq!(clone_head(repo), reviewed);
    assert_eq!(repo.branch, api.head_branch);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_refreshes_at_once_ask_github_once() {
    // A check within the reuse window (15 s in D298) answers from the last;
    // the window is kept long here so the third check below falls in it.
    let fixture = Fixture::with_env(&[
        ("HUMATA2_PR_POLL_MS", NEVER),
        ("HUMATA2_PR_CHECK_REUSE_MS", "600000"),
    ])
    .await;
    let session = fixture.start_review(&[API]).await;
    let api = fixture.prs[API].clone();
    fixture.move_head(API, &TWO_COMMITS);
    let asked = fixture.gh.graphql_calls();

    // GitHub is slow to answer the first check; the second comes while the
    // first is still waiting on it.
    fixture.gh.hold_graphql(true);
    let first = fixture.check_heads(&session);
    let second = async {
        graphql_calls_reach(&fixture, asked + 1).await;
        fixture.check_heads(&session).await
    };
    let release = async {
        graphql_calls_reach(&fixture, asked + 1).await;
        let quiet = Instant::now() + Duration::from_millis(500);
        while Instant::now() < quiet {
            assert_eq!(
                fixture.gh.graphql_calls(),
                asked + 1,
                "a check while one runs joins it:\n{}",
                fixture.gh.log()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        fixture.gh.hold_graphql(false);
    };
    let (first, second, ()) = tokio::join!(first, second, release);

    for heads in [&first, &second] {
        let change = change_of(heads, &api).expect("both checks see api#482 move");
        assert_eq!(change.new_commits, 2, "{heads:?}");
    }
    assert_eq!(first.checked_at, second.checked_at, "one check, one answer");

    // Right after, the last answer stands without asking GitHub again.
    let third = fixture.check_heads(&session).await;
    assert_eq!(change_of(&third, &api).map(|change| change.new_commits), Some(2), "{third:?}");
    assert_eq!(third.checked_at, first.checked_at);
    assert_eq!(fixture.gh.graphql_calls(), asked + 1, "{}", fixture.gh.log());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refresh_is_published_on_the_feed_like_a_poll() {
    let fixture = Fixture::with_env(&[("HUMATA2_PR_POLL_MS", NEVER)]).await;
    let session = fixture.start_review(&[API]).await;
    let mut events = pb::event_service_client::EventServiceClient::new(fixture.daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            sessions: vec![session.id.clone().unwrap()],
            topics: vec![pb::Topic::Sessions as i32],
            ..pb::SubscribeRequest::default()
        })
        .await
        .unwrap()
        .into_inner();

    fixture.move_head(API, &TWO_COMMITS);
    let checked = fixture.check_heads(&session).await;
    assert_eq!(change_of(&checked, &fixture.prs[API]).map(|change| change.new_commits), Some(2));

    let api = fixture.prs[API].reference();
    let deadline = Instant::now() + SLOW;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = tokio::time::timeout(left, events.message())
            .await
            .expect("the check's news never came on the feed")
            .unwrap()
            .expect("the feed ended");
        if let Some(pb::event::Payload::PullRequestHeads(heads)) = event.payload {
            assert_eq!(heads.session, session.id);
            if heads
                .changes
                .iter()
                .any(|change| change.pr == Some(api.clone()) && change.new_commits == 2)
            {
                break;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_retargeted_base_shows_as_a_head_change() {
    let mut fixture = Fixture::with_env(&[("HUMATA2_PR_POLL_MS", NEVER)]).await;
    let session = fixture.start_review(&[API, LEDGER]).await;

    // acme/api gains a release branch, and #482 is retargeted onto it.
    let main = fixture.world.root.join("main-acme-api").display().to_string();
    git(&["-C", &main, "push", "-q", "origin", "main:refs/heads/release"]);
    fixture.prs[API].base = "release";
    fixture.gh.publish(&fixture.prs);

    let heads = fixture.check_heads(&session).await;
    let api = change_of(&heads, &fixture.prs[API]).expect("a retarget is a change");
    assert_eq!(api.base_branch, "release", "{api:?}");
    assert_eq!(api.new_commits, 0, "{api:?}");
    assert!(!api.force_pushed, "{api:?}");
    assert!(
        change_of(&heads, &fixture.prs[LEDGER]).is_none(),
        "ledger#91 kept its base and head: {heads:?}"
    );
}
