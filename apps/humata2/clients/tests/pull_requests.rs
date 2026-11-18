//! The pull request client on its scripted fake: what a review asks of
//! GitHub and what comes back, as the app sees it (PR-1 to PR-17, A9,
//! D283 to D300).

use std::time::Duration;

use humata2_clients::DaemonError;
use humata2_clients::comments::{Anchor, Author, CommentId, Side};
use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::pull_requests::{
    Commit, CommitState, Draft, FakePullRequests, HeadChange, HistoryKind, LeftOutReason,
    PullRequestError, PullRequestInfo, PullRequestRef, PullRequestState, PullRequestsClient,
    ReviewStart, ReviewSubmission, Verdict,
};
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn api_482() -> PullRequestInfo {
    PullRequestInfo {
        pr: PullRequestRef::new("acme/api", 482),
        title: "Round refunds half a cent".to_owned(),
        author: "mira".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: "fix/refund-rounding".to_owned(),
        head_sha: "b2".to_owned(),
        files: 3,
        added: 42,
        removed: 7,
        state: PullRequestState::Open,
        checks: "3 passed, 1 failing".to_owned(),
    }
}

fn ledger_91() -> PullRequestInfo {
    PullRequestInfo {
        pr: PullRequestRef::new("acme/ledger", 91),
        title: "Ledger totals".to_owned(),
        head_branch: "totals".to_owned(),
        head_sha: "l2".to_owned(),
        ..api_482()
    }
}

fn this_mac() -> MachineId {
    MachineId::new("this-mac")
}

fn start_of(prs: Vec<PullRequestRef>) -> ReviewStart {
    ReviewStart {
        prs,
        machine: this_mac(),
        agent: Agent::Claude,
        model: String::new(),
        first_message: "Review these".to_owned(),
        push_allowed: false,
    }
}

/// A fake holding both pull requests, and one review of `api` started on it.
fn with_review(fake: &FakePullRequests) -> SessionId {
    fake.add_pull_request(api_482());
    fake.add_pull_request(ledger_91());
    block_on(fake.start_review(start_of(vec![PullRequestRef::new("acme/api", 482)])))
        .expect("the review starts")
}

fn anchor(repo: &str, path: &str, first: u32) -> Anchor {
    Anchor {
        repo: repo.to_owned(),
        path: path.to_owned(),
        side: Side::New,
        first,
        last: first,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    }
}

fn draft(id: &str, text: &str, outdated: bool) -> Draft {
    Draft {
        id: CommentId::new(id),
        anchor: Anchor {
            outdated,
            ..anchor("api", "src/refund.rs", 140)
        },
        text: text.to_owned(),
        author: Author::User,
        revision: 1,
    }
}

#[test]
fn a_link_or_a_name_parses_to_its_repository_and_number() {
    let expected = Some(PullRequestRef::new("acme/api", 482));
    assert_eq!(PullRequestRef::parse("https://github.com/acme/api/pull/482"), expected);
    assert_eq!(PullRequestRef::parse("https://github.com/acme/api/pull/482/files"), expected);
    assert_eq!(PullRequestRef::parse("acme/api#482"), expected);
    assert_eq!(PullRequestRef::parse("  acme/api#482  "), expected);
    assert_eq!(PullRequestRef::parse("https://github.com/acme/api"), None);
    assert_eq!(PullRequestRef::parse("https://example.com/acme/api/pull/482"), None);
    assert_eq!(PullRequestRef::parse("acme/api#0"), None);
    assert_eq!(PullRequestRef::parse("not a pull request"), None);
}

#[test]
fn resolving_a_pull_request_gives_its_title_branches_size_and_checks() {
    let fake = FakePullRequests::new();
    fake.add_pull_request(api_482());
    let info = block_on(fake.resolve(&PullRequestRef::new("acme/api", 482))).expect("it resolves");
    assert_eq!(info, api_482());
}

#[test]
fn a_pull_request_github_cannot_find_is_not_found() {
    let fake = FakePullRequests::new();
    let missing = PullRequestRef::new("acme/api", 999);
    let error = block_on(fake.resolve(&missing)).expect_err("nothing is seeded");
    assert_eq!(error, PullRequestError::NotFound { pr: missing });
}

#[test]
fn a_signed_out_gh_refuses_every_pull_request_call() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    fake.sign_out(true);
    let resolve = block_on(fake.resolve(&PullRequestRef::new("acme/api", 482)));
    assert!(matches!(resolve, Err(PullRequestError::SignedOut { .. })));
    let heads = block_on(fake.check_heads(&session));
    assert!(matches!(heads, Err(PullRequestError::SignedOut { .. })));
    fake.sign_out(false);
    assert!(block_on(fake.resolve(&PullRequestRef::new("acme/api", 482))).is_ok());
}

#[test]
fn a_rate_limit_says_how_long_to_wait() {
    let fake = FakePullRequests::new();
    fake.add_pull_request(api_482());
    fake.set_rate_limit(Some(Duration::from_secs(600)));
    let error = block_on(fake.resolve(&PullRequestRef::new("acme/api", 482)))
        .expect_err("the limit stops the call");
    assert_eq!(
        error,
        PullRequestError::RateLimited {
            machine: this_mac(),
            retry_after: Some(Duration::from_secs(600)),
        }
    );
}

#[test]
fn a_scripted_failure_answers_only_its_own_call() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let refused = PullRequestError::Daemon(DaemonError::Refused {
        what: "The clone".to_owned(),
        reason: "a test made this fail".to_owned(),
    });
    fake.fail("update_clone", refused.clone());
    assert_eq!(block_on(fake.update_clone(&session)), Err(refused));
    assert!(block_on(fake.check_heads(&session)).is_ok());
    fake.clear_failure("update_clone");
    assert!(block_on(fake.update_clone(&session)).is_ok());
}

#[test]
fn starting_a_review_makes_a_session_that_checks_its_heads() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let check = block_on(fake.check_heads(&session)).expect("the review's heads are checked");
    assert!(check.changes.is_empty());
}

#[test]
fn a_review_of_a_pull_request_not_seen_is_not_started() {
    let fake = FakePullRequests::new();
    fake.add_pull_request(api_482());
    let missing = PullRequestRef::new("acme/ledger", 91);
    let error = block_on(fake.start_review(start_of(vec![missing.clone()])))
        .expect_err("the pull request isn't seeded");
    assert_eq!(error, PullRequestError::NotFound { pr: missing });
}

#[test]
fn a_new_commit_reaches_the_feed_and_the_next_check_lists_it_once() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let mut heads = block_on(fake.watch_heads()).expect("the feed is followed");
    let change = HeadChange {
        pr: PullRequestRef::new("acme/api", 482),
        new_commits: 2,
        force_pushed: false,
        state: PullRequestState::Open,
        base_branch: None,
        stack: None,
    };
    fake.emit_head_change(&session, change.clone());
    let pushed = heads.try_recv().expect("the change is pushed, not polled for");
    assert_eq!(pushed.session, session);
    assert_eq!(pushed.changes, vec![change.clone()]);
    let first = block_on(fake.check_heads(&session)).expect("checked");
    assert_eq!(first.changes, vec![change]);
    let second = block_on(fake.check_heads(&session)).expect("checked again");
    assert!(second.changes.is_empty());
}

#[test]
fn updating_a_session_that_holds_no_review_is_not_found() {
    let fake = FakePullRequests::new();
    let stranger = SessionId::new("se_nobody");
    let error = block_on(fake.update_clone(&stranger)).expect_err("no review there");
    assert!(matches!(error, PullRequestError::Daemon(DaemonError::NotFound { .. })));
}

#[test]
fn a_plan_leaves_out_an_outdated_draft_and_says_why() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let pr = PullRequestRef::new("acme/api", 482);
    fake.add_draft(&session, &pr, draft("c-1", "Round the other way?", false));
    fake.add_draft(&session, &pr, draft("c-2", "Old remark", true));
    let plan = block_on(fake.plan_submit(&session)).expect("the plan is made");
    let mine = plan.pull_requests.iter().find(|each| each.pr == pr).expect("planned");
    let posted: Vec<_> = mine.drafts.iter().map(|draft| draft.id.clone()).collect();
    assert_eq!(posted, vec![CommentId::new("c-1")]);
    assert_eq!(mine.left_out.len(), 1);
    assert_eq!(mine.left_out[0].draft.id, CommentId::new("c-2"));
    assert_eq!(mine.left_out[0].reason, LeftOutReason::Outdated);
}

#[test]
fn submitting_posts_one_review_per_pull_request_and_keeps_the_rest_as_drafts() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let pr = PullRequestRef::new("acme/api", 482);
    fake.add_draft(&session, &pr, draft("c-1", "Posted", false));
    fake.add_draft(&session, &pr, draft("c-2", "Stays a draft", false));
    let plan = block_on(fake.plan_submit(&session)).expect("planned");
    let reviews = vec![ReviewSubmission {
        pr: pr.clone(),
        verdict: Verdict::RequestChanges,
        summary: "Two things".to_owned(),
        drafts: vec![CommentId::new("c-1")],
    }];
    let results = block_on(fake.submit_review(&session, &plan.token, reviews)).expect("submitted");
    assert_eq!(results.len(), 1);
    assert!(results[0].outcome.is_ok());
    assert_eq!(fake.posted().len(), 1);
    let again = block_on(fake.plan_submit(&session)).expect("planned again");
    let left: Vec<_> = again.pull_requests[0].drafts.iter().map(|draft| draft.id.clone()).collect();
    assert_eq!(left, vec![CommentId::new("c-2")]);
}

#[test]
fn a_post_that_fails_keeps_its_drafts_and_the_other_reviews_still_post() {
    let fake = FakePullRequests::new();
    fake.add_pull_request(api_482());
    fake.add_pull_request(ledger_91());
    let api = PullRequestRef::new("acme/api", 482);
    let ledger = PullRequestRef::new("acme/ledger", 91);
    let session = block_on(fake.start_review(start_of(vec![api.clone(), ledger.clone()])))
        .expect("the review of both starts");
    fake.add_draft(&session, &api, draft("c-1", "On api", false));
    fake.add_draft(&session, &ledger, draft("c-2", "On ledger", false));
    let refused = PullRequestError::Daemon(DaemonError::Refused {
        what: "The review".to_owned(),
        reason: "GitHub refused the review".to_owned(),
    });
    fake.fail_post(&ledger, refused.clone());
    let plan = block_on(fake.plan_submit(&session)).expect("planned");
    let reviews = vec![
        ReviewSubmission {
            pr: api.clone(),
            verdict: Verdict::Comment,
            summary: String::new(),
            drafts: vec![CommentId::new("c-1")],
        },
        ReviewSubmission {
            pr: ledger.clone(),
            verdict: Verdict::Comment,
            summary: String::new(),
            drafts: vec![CommentId::new("c-2")],
        },
    ];
    let results = block_on(fake.submit_review(&session, &plan.token, reviews)).expect("submitted");
    let api_result = results.iter().find(|each| each.pr == api).expect("api answered");
    let ledger_result = results.iter().find(|each| each.pr == ledger).expect("ledger answered");
    assert!(api_result.outcome.is_ok());
    assert_eq!(ledger_result.outcome, Err(refused));
    let again = block_on(fake.plan_submit(&session)).expect("planned again");
    let kept = again
        .pull_requests
        .iter()
        .find(|each| each.pr == ledger)
        .expect("ledger planned");
    assert_eq!(kept.drafts.len(), 1);
}

#[test]
fn a_token_no_plan_issued_is_refused() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let error = block_on(fake.submit_review(&session, "plan-made-up", Vec::new()))
        .expect_err("the token is unknown");
    assert!(matches!(error, PullRequestError::Daemon(DaemonError::Refused { .. })));
}

#[test]
fn a_drafted_summary_is_in_the_plan_for_its_pull_request() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let pr = PullRequestRef::new("acme/api", 482);
    block_on(fake.draft_summary(&session, &pr, "Rounding is right; one question")).expect("drafted");
    let plan = block_on(fake.plan_submit(&session)).expect("planned");
    assert_eq!(plan.pull_requests[0].drafted_summary, "Rounding is right; one question");
}

fn seeded_commits(fake: &FakePullRequests) -> PullRequestRef {
    let pr = PullRequestRef::new("acme/api", 482);
    fake.seed_commits(&pr, &[("a1", "Round half up"), ("b2", "Test the rounding")]);
    pr
}

#[test]
fn marking_a_commit_reviewed_records_who_when_and_which_head() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let pr = seeded_commits(&fake);
    let marked = block_on(fake.mark_commits_reviewed(&session, &pr, vec!["a1".to_owned()], true))
        .expect("marked");
    let first: &Commit = &marked.commits[0];
    assert_eq!(first.sha, "a1");
    assert_eq!(first.state, CommitState::Reviewed);
    assert_eq!(first.reviewed_head.as_deref(), Some("b2"));
    assert!(first.reviewed_at_unix_ms.is_some());
    assert!(first.by.is_some());
    assert_eq!(marked.commits[1].state, CommitState::Unreviewed);
    assert_eq!(marked.history.len(), 1);
    assert_eq!(marked.history[0].kind, HistoryKind::Marked);
    assert_eq!(marked.history[0].commits, vec!["a1".to_owned()]);
    assert_eq!(marked.history[0].head_sha, "b2");
}

#[test]
fn unmarking_adds_to_the_history_and_keeps_what_came_before() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    let pr = seeded_commits(&fake);
    block_on(fake.mark_commits_reviewed(&session, &pr, vec!["a1".to_owned()], true)).expect("marked");
    let unmarked = block_on(fake.mark_commits_reviewed(&session, &pr, vec!["a1".to_owned()], false))
        .expect("unmarked");
    assert_eq!(unmarked.commits[0].state, CommitState::Unreviewed);
    let kinds: Vec<HistoryKind> = unmarked.history.iter().map(|entry| entry.kind).collect();
    assert_eq!(kinds, vec![HistoryKind::Marked, HistoryKind::Unmarked]);
    let seqs: Vec<u32> = unmarked.history.iter().map(|entry| entry.seq).collect();
    assert_eq!(seqs, vec![1, 2]);
}

#[test]
fn every_review_of_a_pull_request_sees_its_marks() {
    let fake = FakePullRequests::new();
    let first = with_review(&fake);
    let pr = seeded_commits(&fake);
    let second = block_on(fake.start_review(start_of(vec![pr.clone()]))).expect("a second review");
    block_on(fake.mark_commits_reviewed(&first, &pr, vec!["b2".to_owned()], true)).expect("marked");
    let seen = block_on(fake.list_commits(&second, &pr)).expect("listed");
    assert_eq!(seen.commits[1].state, CommitState::Reviewed);
}

#[test]
fn the_calls_made_are_recorded_in_order() {
    let fake = FakePullRequests::new();
    let session = with_review(&fake);
    block_on(fake.check_heads(&session)).expect("checked");
    let calls = fake.calls();
    assert!(matches!(calls.first(), Some(humata2_clients::pull_requests::Call::StartReview(_))));
    assert_eq!(
        calls.last(),
        Some(&humata2_clients::pull_requests::Call::CheckHeads(session))
    );
}
