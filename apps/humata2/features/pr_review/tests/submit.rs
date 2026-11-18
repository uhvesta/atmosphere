//! PR-9, PR-12 to PR-14, PR-16, D287: Submit review lists the drafts of each
//! pull request, edits or drops them, takes a verdict per pull request and a
//! summary, and posts one review per pull request. A post that fails leaves
//! that pull request's drafts in place; drafts the plan left out can be
//! re-attached or dropped.
use std::sync::Arc;

use humata2_clients::comments::{Anchor, Author, CommentId, CommentsClient, FakeComments, Side, Which};
use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_clients::pull_requests::fake::Call;
use humata2_clients::pull_requests::{
    Draft, FakePullRequests, LeftOutReason, PullRequestError, PullRequestInfo, PullRequestRef,
    PullRequestState, PullRequestsClient, ReviewStart, Verdict,
};
use humata2_pr_review::submit::{Delegate, Outcome, Step, Submit, SubmitAction as A, SubmitState};
use tca::TestStore;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn api() -> PullRequestRef {
    PullRequestRef::new("acme/api", 482)
}

fn web() -> PullRequestRef {
    PullRequestRef::new("acme/web", 77)
}

fn info(pr: &PullRequestRef) -> PullRequestInfo {
    PullRequestInfo {
        pr: pr.clone(),
        title: format!("Change {}", pr.number),
        author: "mara".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: format!("change-{}", pr.number),
        head_sha: "abc123".to_owned(),
        files: 2,
        added: 10,
        removed: 1,
        state: PullRequestState::Open,
        checks: String::new(),
    }
}

fn anchor(repo: &str, line: u32, outdated: bool) -> Anchor {
    Anchor {
        repo: repo.to_owned(),
        path: "src/refund.rs".to_owned(),
        side: Side::New,
        first: line,
        last: line,
        quoted: vec!["let rounded = cents - fee;".to_owned()],
        base: None,
        detached: false,
        outdated,
    }
}

struct Rig {
    github: FakePullRequests,
    comments: FakeComments,
    session: SessionId,
    store: TestStore<Submit>,
}

impl Rig {
    /// A review of both pull requests, no drafts yet.
    fn new() -> Self {
        let github = FakePullRequests::new();
        github.add_pull_request(info(&api()));
        github.add_pull_request(info(&web()));
        let session = block_on(github.start_review(ReviewStart {
            prs: vec![api(), web()],
            machine: MachineId::new("this-mac"),
            agent: Agent::Claude,
            model: String::new(),
            first_message: String::new(),
            push_allowed: false,
        }))
        .expect("the review starts");
        let comments = FakeComments::new();
        let store = TestStore::new(
            SubmitState::default(),
            Submit::new(Arc::new(github.clone()), Arc::new(comments.clone())),
        )
        .non_exhaustive();
        Self {
            github,
            comments,
            session,
            store,
        }
    }

    /// A draft on `pr`, as the person or the agent wrote it, held by the
    /// comment ledger and shown by the plan.
    fn draft(&self, pr: &PullRequestRef, line: u32, text: &str, outdated: bool) -> CommentId {
        let at = anchor(&pr.repo, line, outdated);
        let comment = self.comments.seed_pull_request_draft(
            &self.session,
            at.clone(),
            text,
            Author::Agent("claude".to_owned()),
            &pr.repo,
            pr.number,
        );
        self.github.add_draft(
            &self.session,
            pr,
            Draft {
                id: comment.id.clone(),
                anchor: at,
                text: text.to_owned(),
                author: Author::Agent("claude".to_owned()),
                revision: comment.revision,
            },
        );
        comment.id
    }

    /// Submit review opened and planned.
    fn open(&mut self) {
        self.store.send(A::Open(self.session.clone()), |_| {});
        self.planned();
    }

    fn planned(&mut self) {
        self.store.receive_matching(
            "the daemon planned the submit",
            |action| matches!(action, A::Planned(_)),
            |_| {},
        );
    }

    fn submitted(&mut self) {
        self.store.receive_matching(
            "the daemon answered Submit",
            |action| matches!(action, A::Submitted(_)),
            |_| {},
        );
    }

    fn state(&self) -> &SubmitState {
        self.store.state()
    }

    fn review(&self, pr: &PullRequestRef) -> &humata2_pr_review::submit::PullRequestReview {
        self.state()
            .reviews
            .iter()
            .find(|review| &review.pr == pr)
            .expect("the review lists this pull request")
    }

    fn submits_made(&self) -> usize {
        self.github
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::SubmitReview(..)))
            .count()
    }
}

#[test]
fn it_lists_the_drafts_per_pull_request_with_the_agents_summary() {
    let mut rig = Rig::new();
    rig.draft(&api(), 140, "Rounds down", false);
    rig.draft(&api(), 150, "Missing a test", false);
    rig.draft(&web(), 12, "Typo", false);
    block_on(rig.github.draft_summary(&rig.session, &api(), "Mostly fine; rounding needs a look"))
        .expect("the agent drafted a summary");
    rig.open();

    assert_eq!(rig.state().step, Step::Reviewing);
    assert_eq!(rig.review(&api()).drafts.len(), 2);
    assert_eq!(rig.review(&web()).drafts.len(), 1);
    assert_eq!(rig.review(&api()).summary, "Mostly fine; rounding needs a look");
    assert_eq!(rig.review(&api()).verdict, Verdict::Comment);
    assert!(rig.state().can_submit());
}

#[test]
fn a_plan_that_cannot_be_made_says_why_and_posts_nothing() {
    let mut rig = Rig::new();
    rig.github.sign_out(true);
    rig.open();
    assert!(rig.state().error.is_some());
    assert!(rig.state().reviews.is_empty());
    assert!(!rig.state().can_submit());
    rig.store.send(A::Submit, |_| {});
    assert_eq!(rig.submits_made(), 0);
}

#[test]
fn a_draft_can_be_edited() {
    let mut rig = Rig::new();
    let id = rig.draft(&api(), 140, "Rounds down", false);
    rig.open();
    rig.store.send(
        A::EditDraft {
            id: id.clone(),
            text: "Rounds down, losing a cent".to_owned(),
        },
        |_| {},
    );
    rig.store.receive_matching("edited", |action| matches!(action, A::Edited(Ok(_))), |_| {});
    let shown = &rig.review(&api()).drafts[0];
    assert_eq!(shown.text, "Rounds down, losing a cent");
    assert!(shown.revision > 1);
    let held = block_on(rig.comments.list(&rig.session, Which::Open)).expect("listed");
    assert_eq!(held.comments[0].text, "Rounds down, losing a cent");
}

#[test]
fn an_edit_made_against_an_old_revision_is_refused_and_changes_nothing() {
    let mut rig = Rig::new();
    let id = rig.draft(&api(), 140, "Rounds down", false);
    rig.open();
    block_on(rig.comments.edit(&id, "Changed in another window", 1)).expect("another window edits");
    rig.store.send(
        A::EditDraft {
            id,
            text: "My edit".to_owned(),
        },
        |_| {},
    );
    rig.store.receive_matching("refused", |action| matches!(action, A::Edited(Err(_))), |_| {});
    assert!(rig.state().error.is_some());
    assert_eq!(rig.review(&api()).drafts[0].text, "Rounds down");
}

#[test]
fn a_draft_can_be_dropped() {
    let mut rig = Rig::new();
    let keep = rig.draft(&api(), 140, "Rounds down", false);
    let drop = rig.draft(&api(), 150, "Missing a test", false);
    rig.open();
    rig.store.send(A::DropDraft(drop), |_| {});
    rig.store.receive_matching("dropped", |action| matches!(action, A::Dropped { result: Ok(()), .. }), |_| {});
    let ids: Vec<_> = rig.review(&api()).drafts.iter().map(|draft| draft.id.clone()).collect();
    assert_eq!(ids, vec![keep]);
    let held = block_on(rig.comments.list(&rig.session, Which::Open)).expect("listed");
    assert_eq!(held.comments.len(), 1);
}

#[test]
fn drafts_the_plan_left_out_are_listed_and_do_not_post() {
    let mut rig = Rig::new();
    rig.draft(&api(), 140, "Rounds down", false);
    let old = rig.draft(&api(), 90, "About lines that changed", true);
    rig.open();
    let review = rig.review(&api());
    assert_eq!(review.drafts.len(), 1);
    assert_eq!(review.left_out.len(), 1);
    assert_eq!(review.left_out[0].draft.id, old);
    assert_eq!(review.left_out[0].reason, LeftOutReason::Outdated);

    rig.store.send(A::Submit, |_| {});
    rig.submitted();
    let posted = rig.github.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].1.drafts.len(), 1);
    assert!(!posted[0].1.drafts.contains(&old));
}

#[test]
fn a_left_out_draft_can_be_re_attached_and_then_posts() {
    let mut rig = Rig::new();
    let old = rig.draft(&api(), 90, "About lines that changed", true);
    rig.open();
    rig.store.send(
        A::Reattach {
            id: old.clone(),
            anchor: anchor("acme/api", 95, false),
        },
        |_| {},
    );
    rig.store.receive_matching("re-attached", |action| matches!(action, A::Reattached(Ok(_))), |_| {});
    let review = rig.review(&api());
    assert!(review.left_out.is_empty());
    assert_eq!(review.drafts[0].id, old);
    assert_eq!(review.drafts[0].anchor.first, 95);
    assert!(!review.drafts[0].anchor.outdated);
}

#[test]
fn a_left_out_draft_can_be_dropped() {
    let mut rig = Rig::new();
    let old = rig.draft(&api(), 90, "About lines that changed", true);
    rig.open();
    rig.store.send(A::DropDraft(old), |_| {});
    rig.store.receive_matching("dropped", |action| matches!(action, A::Dropped { result: Ok(()), .. }), |_| {});
    assert!(rig.review(&api()).left_out.is_empty());
}

#[test]
fn each_pull_request_posts_with_its_own_verdict_and_summary() {
    let mut rig = Rig::new();
    let a = rig.draft(&api(), 140, "Rounds down", false);
    let w = rig.draft(&web(), 12, "Typo", false);
    rig.open();
    rig.store.send(A::SetVerdict { pr: api(), verdict: Verdict::RequestChanges }, |_| {});
    rig.store.send(A::SetSummary { pr: api(), summary: "Fix the rounding".to_owned() }, |_| {});
    rig.store.send(A::SetVerdict { pr: web(), verdict: Verdict::Approve }, |_| {});

    rig.store.send(A::Submit, |_| {});
    assert_eq!(rig.state().step, Step::Posting);
    rig.submitted();

    let posted = rig.github.posted();
    assert_eq!(posted.len(), 2);
    let on_api = &posted.iter().find(|(pr, _)| *pr == api()).expect("api posted").1;
    assert_eq!(on_api.verdict, Verdict::RequestChanges);
    assert_eq!(on_api.summary, "Fix the rounding");
    assert_eq!(on_api.drafts, vec![a]);
    let on_web = &posted.iter().find(|(pr, _)| *pr == web()).expect("web posted").1;
    assert_eq!(on_web.verdict, Verdict::Approve);
    assert_eq!(on_web.drafts, vec![w]);
}

#[test]
fn a_pull_request_with_nothing_to_say_is_not_posted() {
    let mut rig = Rig::new();
    rig.draft(&api(), 140, "Rounds down", false);
    rig.open();
    rig.store.send(A::Submit, |_| {});
    rig.submitted();
    let posted = rig.github.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].0, api());
}

#[test]
fn when_every_review_posts_the_parent_is_told_and_each_has_its_link() {
    let mut rig = Rig::new();
    rig.draft(&api(), 140, "Rounds down", false);
    rig.draft(&web(), 12, "Typo", false);
    rig.open();
    rig.store.send(A::Submit, |_| {});
    rig.submitted();
    assert!(matches!(rig.review(&api()).outcome, Some(Outcome::Posted(_))));
    assert!(matches!(rig.review(&web()).outcome, Some(Outcome::Posted(_))));
    rig.store.receive(A::Delegate(Delegate::Posted(rig.session.clone())), |_| {});
    assert!(!rig.state().can_submit());
}

#[test]
fn a_failed_post_keeps_that_pull_requests_drafts_and_the_others_still_post() {
    let mut rig = Rig::new();
    rig.draft(&api(), 140, "Rounds down", false);
    let kept = rig.draft(&web(), 12, "Typo", false);
    let refused = PullRequestError::NotVisible { repo: "acme/web".to_owned() };
    rig.github.fail_post(&web(), refused.clone());
    rig.open();
    rig.store.send(A::Submit, |_| {});
    rig.submitted();
    // The spent plan is made again so what didn't post shows as it is now.
    rig.planned();

    assert!(matches!(rig.review(&api()).outcome, Some(Outcome::Posted(_))));
    assert_eq!(rig.review(&web()).outcome, Some(Outcome::Failed(refused)));
    assert_eq!(rig.review(&web()).drafts.len(), 1);
    assert_eq!(rig.review(&web()).drafts[0].id, kept);
    assert!(rig.review(&api()).drafts.is_empty(), "what posted is no longer a draft");
    assert_eq!(rig.github.posted().len(), 1);
    assert!(rig.state().can_submit(), "the unposted review can be tried again");

    // Trying again never posts the pull request that posted.
    rig.store.send(A::Submit, |_| {});
    rig.submitted();
    rig.planned();
    assert_eq!(rig.github.posted().len(), 1);
    assert_eq!(rig.review(&web()).drafts.len(), 1);
}

#[test]
fn what_the_person_chose_survives_the_plan_being_made_again() {
    let mut rig = Rig::new();
    rig.draft(&web(), 12, "Typo", false);
    rig.github.fail_post(&web(), PullRequestError::NotVisible { repo: "acme/web".to_owned() });
    rig.open();
    rig.store.send(A::SetVerdict { pr: web(), verdict: Verdict::RequestChanges }, |_| {});
    rig.store.send(A::SetSummary { pr: web(), summary: "My words".to_owned() }, |_| {});
    rig.store.send(A::Submit, |_| {});
    rig.submitted();
    rig.planned();
    assert_eq!(rig.review(&web()).verdict, Verdict::RequestChanges);
    assert_eq!(rig.review(&web()).summary, "My words");
}

#[test]
fn a_plan_that_went_stale_is_refused_and_planned_again() {
    let mut rig = Rig::new();
    rig.draft(&api(), 140, "Rounds down", false);
    rig.open();
    // Another window planned the submit, so this plan's token is spent.
    block_on(rig.github.plan_submit(&rig.session)).expect("another plan");
    rig.store.send(A::Submit, |_| {});
    rig.store.receive_matching("refused", |action| matches!(action, A::Submitted(Err(_))), |_| {});
    assert!(rig.state().error.is_some());
    assert!(rig.github.posted().is_empty());
    rig.planned();
    assert!(rig.state().can_submit());

    rig.store.send(A::Submit, |_| {});
    rig.submitted();
    assert_eq!(rig.github.posted().len(), 1);
}

#[test]
fn submit_with_nothing_to_post_asks_nobody() {
    let mut rig = Rig::new();
    rig.open();
    assert!(!rig.state().can_submit());
    rig.store.send(A::Submit, |_| {});
    assert_eq!(rig.submits_made(), 0);
}

#[test]
fn closing_forgets_the_plan() {
    let mut rig = Rig::new();
    rig.draft(&api(), 140, "Rounds down", false);
    rig.open();
    rig.store.send(A::Close, |_| {});
    assert_eq!(rig.state(), &SubmitState::default());
}

#[test]
fn a_plan_that_answers_after_submit_review_closed_does_not_reopen_it() {
    let mut rig = Rig::new();
    rig.draft(&api(), 140, "Rounds down", false);
    rig.store.send(A::Open(rig.session.clone()), |_| {});
    rig.store.send(A::Close, |_| {});
    rig.planned();
    assert_eq!(rig.state().step, Step::Closed);
    assert!(rig.state().reviews.is_empty());
}
