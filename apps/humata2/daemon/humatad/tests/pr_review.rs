//! Review a pull request with an agent beside me (PR-2–PR-17, D283–D288),
//! through a real `humatad2`.
//!
//! GitHub is the fake `gh` of `common/pr_github.rs` (admin builds read
//! `HUMATA2_GH`), which keeps its state in files beside it: the pull
//! requests it knows, the reviews posted to it and the reviews it lists.
//! The remotes are the world's bare repositories, each pull request's head
//! pushed to its branch and to `refs/pull/<n>/head` (a fork's only to the
//! latter), as GitHub keeps them. The `gh` contract these tests pin is in
//! SEAMS.md: `gh pr view <n> --repo <owner>/<name> --json …` for one pull
//! request, `gh api graphql` with one alias per pull request for the heads,
//! and `gh api repos/<o>/<r>/pulls/<n>/reviews` with `--input -` to post and
//! without to list.
//!
//! The PR milestone's daemon batch (A1-A6).

use humatad_test_common as common;
use humatad_test_common::mcp_client;
use humatad_test_common::pr_github;

use std::time::Instant;

use common::{error_of, finished, get_session, git, repo_ref, wait_running};
use humata2_grpc::humata::v1 as pb;
use pr_github::{
    API, CONTEXT, FORK, Fixture, LEDGER, Pr, SLOW, TWO_COMMITS, as_agent, change_of, clone_head, edit,
    failed, ok, push_head, repo_of,
};
use serde_json::{Value, json};
use tonic::Code;

fn comment_id(id: &str) -> pb::CommentId {
    pb::CommentId { value: id.to_owned() }
}

fn id_of(comment: &pb::Comment) -> &str {
    comment.id.as_ref().map_or("", |id| id.value.as_str())
}

fn find<'a>(comments: &'a [pb::Comment], id: &str) -> &'a pb::Comment {
    comments
        .iter()
        .find(|comment| id_of(comment) == id)
        .unwrap_or_else(|| panic!("comment {id} is listed: {comments:#?}"))
}

fn by_agent(comment: &pb::Comment) -> bool {
    matches!(
        comment.author.as_ref().and_then(|author| author.actor.as_ref()),
        Some(pb::actor::Actor::Agent(_))
    )
}

fn by_user(comment: &pb::Comment) -> bool {
    matches!(
        comment.author.as_ref().and_then(|author| author.actor.as_ref()),
        Some(pb::actor::Actor::User(_))
    )
}

fn destined_for(comment: &pb::Comment, pr: &Pr) -> bool {
    comment.destination.as_ref().and_then(|d| d.destination.as_ref())
        == Some(&pb::comment_destination::Destination::PullRequest(pr.reference()))
}

fn is(comment: &pb::Comment, status: pb::CommentStatus) -> bool {
    comment.status == status as i32
}

fn per_pr<'a>(plan: &'a pb::SubmitPlan, pr: &Pr) -> &'a pb::submit_plan::PerPullRequest {
    plan.pull_requests
        .iter()
        .find(|per| per.pr == Some(pr.reference()))
        .unwrap_or_else(|| panic!("the plan has {}: {plan:#?}", pr.slug()))
}

fn ids(comments: &[pb::Comment]) -> Vec<String> {
    let mut ids: Vec<String> = comments.iter().map(|c| id_of(c).to_owned()).collect();
    ids.sort();
    ids
}


// ---------------------------------------------------------------------------
// A1: what a pull request is (PR-3).

#[tokio::test(flavor = "multi_thread")]
async fn a_pull_request_resolves_from_its_url_and_from_owner_repo_number() {
    let fixture = Fixture::new().await;
    let pr = &fixture.prs[API];
    let by_url = fixture
        .resolve(pb::resolve_pull_request_request::Ref::Url(pr.url()))
        .await
        .unwrap();
    let by_ref = fixture
        .resolve(pb::resolve_pull_request_request::Ref::Pr(pr.reference()))
        .await
        .unwrap();
    assert_eq!(by_url, by_ref);
    assert_eq!(by_url.pr, Some(pr.reference()));
    assert_eq!(by_url.title, "Round refunds half-even");
    assert_eq!(by_url.author, "mira");
    assert_eq!(by_url.base_branch, "main");
    assert_eq!(by_url.head_branch, "uhvesta/fix-refund-rounding");
    assert_eq!(by_url.head_sha, pr.head());
    // refund.rs changed a line, rounding.rs is new.
    assert_eq!((by_url.files, by_url.added, by_url.removed), (2, 2, 1));
    assert_eq!(by_url.state, pb::PullRequestState::Open as i32);
    assert!(!by_url.checks.is_empty(), "{by_url:?}");

    let fork = fixture
        .resolve(pb::resolve_pull_request_request::Ref::Pr(fixture.prs[FORK].reference()))
        .await
        .unwrap();
    assert_eq!(fork.head_branch, "patch-1");
    assert_eq!(fork.head_sha, fixture.prs[FORK].head());
    fixture.gh.assert_token_stayed_with_gh(fixture.daemon.dir.path());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pull_request_that_cant_be_fetched_says_why_and_what_to_do() {
    let fixture = Fixture::new().await;
    let pr = |owner: &str, name: &str, number| {
        pb::resolve_pull_request_request::Ref::Pr(pb::PullRequestRef {
            repo: Some(repo_ref(owner, name)),
            number,
            ..pb::PullRequestRef::default()
        })
    };

    let missing = error_of(&fixture.resolve(pr("acme", "api", 999)).await.unwrap_err());
    assert!(
        matches!(&missing.kind, Some(pb::error::Kind::NotFound(found))
            if found.kind == pb::EntityKind::PullRequest as i32),
        "{missing:?}"
    );
    assert!(!missing.fix.is_empty());

    let hidden = error_of(&fixture.resolve(pr("acme", "secret", 1)).await.unwrap_err());
    assert!(
        matches!(&hidden.kind, Some(pb::error::Kind::RepoNotVisible(repo))
            if repo.repo == Some(repo_ref("acme", "secret"))),
        "{hidden:?}"
    );
    assert!(!hidden.fix.is_empty());

    fixture.gh.fail(Some("signed-out"));
    let signed_out = error_of(&fixture.resolve(pr("acme", "api", 482)).await.unwrap_err());
    assert!(
        matches!(&signed_out.kind, Some(pb::error::Kind::GithubCli(problem))
            if problem.problem == pb::git_hub_cli_problem::Problem::NotSignedIn as i32),
        "{signed_out:?}"
    );
    assert!(!signed_out.fix.is_empty());
    fixture.gh.assert_token_stayed_with_gh(fixture.daemon.dir.path());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rate_limited_lookup_says_to_wait_until_the_limit_resets() {
    let fixture = Fixture::new().await;
    fixture.gh.fail(Some("rate-limited"));
    let status = fixture
        .resolve(pb::resolve_pull_request_request::Ref::Url(fixture.prs[API].url()))
        .await
        .unwrap_err();
    let error = error_of(&status);
    let Some(pb::error::Kind::RateLimited(limited)) = &error.kind else {
        panic!("a rate limit is told as one: {error:?}");
    };
    assert_eq!(limited.service, pb::rate_limited::Service::Github as i32);
    let waits = limited.resets_at.is_some()
        || error.retry.as_ref().is_some_and(|retry| retry.after.is_some());
    assert!(waits, "a rate limit says how long to wait: {error:?}");
    assert!(!error.fix.is_empty());

    // Once the limit is gone, the same lookup works.
    fixture.gh.fail(None);
    fixture
        .resolve(pb::resolve_pull_request_request::Ref::Url(fixture.prs[API].url()))
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// A2: starting a review (PR-2, PR-5, PR-6).

#[tokio::test(flavor = "multi_thread")]
async fn starting_a_review_makes_the_pr_reviews_channel_and_one_session_on_every_prs_head() {
    let fixture = Fixture::new().await;
    assert!(fixture.pr_reviews_channels().await.is_empty());

    let mut events = pb::event_service_client::EventServiceClient::new(fixture.daemon.channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Channels as i32],
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();

    let session = fixture.start_review(&[API, LEDGER]).await;

    let channels = fixture.pr_reviews_channels().await;
    assert_eq!(channels.len(), 1, "{channels:#?}");
    let channel = &channels[0];
    assert_eq!(session.channel, channel.id);
    assert_eq!(channel.folder, "pr-reviews");

    // The first PR review publishes the newly created PR Reviews channel on the
    // event feed so live clients immediately show it in the sidebar and header.
    let told = tokio::time::timeout(common::WAIT, async {
        loop {
            let event = events.message().await.unwrap().expect("the feed ended");
            if let Some(pb::event::Payload::Channel(told)) = event.payload {
                break (event.change, told);
            }
        }
    })
    .await
    .expect("first PR review publishes a Channel event");
    assert_eq!(told.0, pb::Change::Created as i32);
    assert_eq!(told.1.id, channel.id);
    assert_eq!(told.1.folder, "pr-reviews");
    assert!(told.1.is_pr_reviews);
    let mut repos = channel.repos.clone();
    repos.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(repos, [repo_ref("acme", "api"), repo_ref("acme", "ledger")]);

    let binding = session.pull_request.clone().expect("a review session is bound to its PRs");
    assert_eq!(
        binding.pull_requests,
        [fixture.prs[API].reference(), fixture.prs[LEDGER].reference()]
    );
    assert!(!binding.push_allowed);

    let sessions_folder = fixture.world.channels.join(&channel.folder).join("sessions");
    let mut review_folders = Vec::new();
    for at in [API, LEDGER] {
        let pr = &fixture.prs[at];
        let repo = repo_of(&session, pr);
        let path = std::fs::canonicalize(&repo.path).unwrap();
        assert_eq!(path.file_name().unwrap(), pr.name);
        let review = path.parent().unwrap().to_owned();
        assert_eq!(
            review.parent().unwrap(),
            std::fs::canonicalize(&sessions_folder).unwrap(),
            "a review clones into pr-reviews/sessions/<review>/<repo>"
        );
        review_folders.push(review);
        assert_eq!(repo.branch, pr.head_branch);
        assert_eq!(
            git(&["-C", &repo.path, "rev-parse", "--abbrev-ref", "HEAD"]),
            pr.head_branch
        );
        assert_eq!(clone_head(repo), pr.head());
    }
    assert_eq!(review_folders[0], review_folders[1], "one review, one folder");
    fixture.gh.assert_token_stayed_with_gh(fixture.daemon.dir.path());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_review_reuses_the_channel_and_clones_a_fork_pr_from_its_pull_ref() {
    let fixture = Fixture::new().await;
    let first = fixture.start_review(&[API]).await;
    let second = fixture.start_review(&[FORK]).await;

    let channels = fixture.pr_reviews_channels().await;
    assert_eq!(channels.len(), 1, "{channels:#?}");
    assert_eq!(first.channel, second.channel);
    assert_ne!(first.id, second.id);

    let fork = &fixture.prs[FORK];
    let repo = repo_of(&second, fork);
    // PR-6: the PR's own head branch name, at the head GitHub keeps for it.
    assert_eq!(repo.branch, "patch-1");
    assert_eq!(clone_head(repo), fork.head());
    assert_ne!(
        std::fs::canonicalize(&repo.path).unwrap(),
        std::fs::canonicalize(&repo_of(&first, &fixture.prs[API]).path).unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_review_chat_is_told_each_pr_and_the_users_context() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API, LEDGER]).await;
    let (api, ledger) = (&fixture.prs[API], &fixture.prs[LEDGER]);
    let said = fixture
        .told(
            &session,
            &[
                &api.url(),
                api.title,
                &ledger.url(),
                ledger.title,
                CONTEXT,
                // PR-5: the tools of PR-23, by name.
                "review_draft",
            ],
        )
        .await;
    // D743: no branch names in what the agent is told; it gets them from
    // session_info and review_context.
    for branch in [api.head_branch, api.base, ledger.head_branch, ledger.base] {
        assert!(!said.contains(branch), "the agent was told the branch {branch}: {said:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_stays_bound_to_its_prs_across_a_restart() {
    let mut fixture = Fixture::new().await;
    let session = fixture.start_review(&[API, LEDGER]).await;
    let id = session.id.clone().unwrap();
    fixture.restart().await;
    let back = get_session(&fixture.daemon, &id).await;
    assert_eq!(back.pull_request, session.pull_request);
    assert_eq!(back.channel, session.channel);
    assert_eq!(fixture.pr_reviews_channels().await.len(), 1);
    // The next review still lands in the same channel.
    let next = fixture.start_review(&[FORK]).await;
    assert_eq!(next.channel, session.channel);
}

// ---------------------------------------------------------------------------
// A3: read-only towards GitHub (PR-7, PR-8).

#[tokio::test(flavor = "multi_thread")]
async fn a_review_session_cant_push_until_the_user_lets_it() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let agent = fixture.agent(&session).await;
    let plan_push = |token: String| {
        let id = session.id.clone();
        let daemon = &fixture.daemon;
        async move {
            daemon
                .session_service()
                .await
                .plan_push(as_agent(
                    pb::PlanPushRequest {
                        id,
                        ..pb::PlanPushRequest::default()
                    },
                    &token,
                ))
                .await
        }
    };

    let refused = plan_push(agent.token.clone()).await.unwrap_err();
    let error = error_of(&refused);
    assert!(
        matches!(&error.kind, Some(pb::error::Kind::PushRefused(push))
            if push.reason == pb::push_refused::Reason::ReviewReadOnly as i32),
        "{error:?}"
    );
    assert!(!error.fix.is_empty(), "the refusal tells the agent what to do");

    fixture
        .reviews()
        .await
        .set_push_allowed(pb::SetPushAllowedRequest {
            idempotency_key: None,
            session: session.id.clone(),
            allowed: true,
            presence: None,
        })
        .await
        .unwrap();
    let allowed = get_session(&fixture.daemon, session.id.as_ref().unwrap()).await;
    assert!(allowed.pull_request.unwrap().push_allowed);
    plan_push(agent.token.clone()).await.unwrap();

    fixture
        .reviews()
        .await
        .set_push_allowed(pb::SetPushAllowedRequest {
            idempotency_key: None,
            session: session.id.clone(),
            allowed: false,
            presence: None,
        })
        .await
        .unwrap();
    let again = plan_push(agent.token.clone()).await.unwrap_err();
    assert!(matches!(error_of(&again).kind, Some(pb::error::Kind::PushRefused(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_user_can_let_a_review_push() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let agent = fixture.agent(&session).await;
    let status = fixture
        .reviews()
        .await
        .set_push_allowed(as_agent(
            pb::SetPushAllowedRequest {
                idempotency_key: None,
                session: session.id.clone(),
                allowed: true,
                presence: None,
            },
            &agent.token,
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(error_of(&status).kind, Some(pb::error::Kind::UserOnly(_))),
        "{status:?}"
    );
    let still = get_session(&fixture.daemon, session.id.as_ref().unwrap()).await;
    assert!(!still.pull_request.unwrap().push_allowed);
}

// ---------------------------------------------------------------------------
// A4: drafts (PR-9, PR-11, PR-23).

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_drafts_edits_and_drops_comments_without_reaching_github() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API, LEDGER]).await;
    let mut agent = fixture.agent(&session).await;
    let (api, ledger) = (&fixture.prs[API], &fixture.prs[LEDGER]);

    let kept = agent.draft(api, "refund.rs", 3, 3, "Half-even here?").await;
    let gone = agent.draft(ledger, "ledger.rs", 2, 2, "Name the refunds").await;
    let edited = agent
        .call(
            "review_draft_edit",
            json!({ "comment_id": kept, "text": "Is half-even what finance wants?" }),
        )
        .await;
    ok(&edited);
    let dropped = agent.call("review_draft_drop", json!({ "comment_id": gone })).await;
    assert_eq!(ok(&dropped)["dropped"], true, "{dropped}");

    let comments = fixture.list(&session).await;
    let draft = find(&comments, &kept);
    assert!(is(draft, pb::CommentStatus::Draft));
    assert!(by_agent(draft));
    assert!(destined_for(draft, api));
    assert_eq!(draft.text, "Is half-even what finance wants?");
    let anchor = draft.anchor.clone().unwrap();
    assert_eq!(anchor.file.unwrap().path, "refund.rs");
    assert_eq!(anchor.lines, Some(pb::LineRange { first: 3, last: 3 }));
    assert_eq!(anchor.side, pb::DiffSide::New as i32);
    assert!(!comments.iter().any(|comment| id_of(comment) == gone));

    assert_eq!(fixture.gh.post_count(), 0);
    assert!(
        !fixture.gh.log().contains("/reviews"),
        "drafting never reaches GitHub: {}",
        fixture.gh.log()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_user_drafts_for_github_beside_the_agents_drafts() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let mut agent = fixture.agent(&session).await;
    let api = &fixture.prs[API];
    let theirs = agent.draft(api, "refund.rs", 3, 3, "Half-even here?").await;

    let mine = fixture
        .comments()
        .await
        .create_comment(pb::CreateCommentRequest {
            idempotency_key: None,
            session: session.id.clone(),
            anchor: Some(pb::CommentAnchor {
                file: Some(pb::RepoPath {
                    repo: Some(repo_ref("acme", "api")),
                    path: "rounding.rs".to_owned(),
                }),
                lines: Some(pb::LineRange { first: 1, last: 1 }),
                side: pb::DiffSide::New as i32,
                quoted: api.line("rounding.rs", 1),
                ..pb::CommentAnchor::default()
            }),
            text: "Needs a test".to_owned(),
            destination: Some(pb::CommentDestination {
                destination: Some(pb::comment_destination::Destination::PullRequest(api.reference())),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(is(&mine, pb::CommentStatus::Draft));
    assert!(by_user(&mine));

    let drafts = fixture
        .comments()
        .await
        .list_comments(pb::ListCommentsRequest {
            session: session.id.clone(),
            statuses: vec![pb::CommentStatus::Draft as i32],
            ..pb::ListCommentsRequest::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ids(&drafts.comments), {
        let mut both = vec![theirs.clone(), id_of(&mine).to_owned()];
        both.sort();
        both
    });
    assert!(by_agent(find(&drafts.comments, &theirs)));
    assert!(by_user(find(&drafts.comments, id_of(&mine))));
    assert!(drafts.comments.iter().all(|draft| destined_for(draft, api)));
    assert_eq!(fixture.gh.post_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_note_of_the_review_chat_becomes_a_draft_in_place() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let mut agent = fixture.agent(&session).await;
    let api = &fixture.prs[API];
    fixture.show_diff(&session).await;
    let reply = agent
        .call(
            "review_note",
            json!({
                "repo": "acme/api",
                "path": "refund.rs",
                "lines": { "first": 3, "last": 3 },
                "quoted": api.line("refund.rs", 3),
                "text": "This is where the rounding changed",
            }),
        )
        .await;
    let note = ok(&reply)["note_id"].as_str().unwrap().to_owned();

    // D284: the same comment, now a draft for the PR; never a copy.
    let draft = fixture
        .comments()
        .await
        .make_note_a_draft(pb::MakeNoteADraftRequest {
            idempotency_key: None,
            id: Some(comment_id(&note)),
            pr: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(id_of(&draft), note);
    assert!(is(&draft, pb::CommentStatus::Draft));
    assert!(destined_for(&draft, api));
    assert!(by_agent(&draft), "the note's author stays its author");
    let comments = fixture.list(&session).await;
    assert_eq!(comments.len(), 1, "{comments:#?}");
    assert!(destined_for(&comments[0], api));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_draft_for_a_pr_outside_the_review_is_refused() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let mut agent = fixture.agent(&session).await;
    let ledger = &fixture.prs[LEDGER];
    let reply = agent
        .call(
            "review_draft",
            json!({
                "pr": ledger.slug(),
                "path": "ledger.rs",
                "lines": { "first": 2, "last": 2 },
                "quoted": "x",
                "text": "not mine to review",
            }),
        )
        .await;
    assert!(!failed(&reply).is_empty());
    assert!(fixture.list(&session).await.is_empty());
}

// ---------------------------------------------------------------------------
// A5: submitting (PR-12–PR-14, D287).

/// A review of api#482 and ledger#91 with an agent draft on each PR's
/// changed line, and one on an api line the PR didn't touch.
struct Drafted {
    fixture: Fixture,
    session: pb::Session,
    api_draft: String,
    ledger_draft: String,
    outside: String,
}

async fn drafted() -> Drafted {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API, LEDGER]).await;
    let mut agent = fixture.agent(&session).await;
    let api = fixture.prs[API].clone();
    let ledger = fixture.prs[LEDGER].clone();
    let api_draft = agent.draft(&api, "refund.rs", 3, 3, "Half-even here?").await;
    let ledger_draft = agent.draft(&ledger, "ledger.rs", 2, 2, "Name the refunds").await;
    let outside = agent.draft(&api, "refund.rs", 9, 9, "Unrelated, but odd").await;
    Drafted {
        fixture,
        session,
        api_draft,
        ledger_draft,
        outside,
    }
}

fn submission(pr: &Pr, verdict: pb::Verdict, summary: &str, drafts: &[&str]) -> pb::ReviewSubmission {
    pb::ReviewSubmission {
        pr: Some(pr.reference()),
        verdict: verdict as i32,
        summary: summary.to_owned(),
        drafts: drafts.iter().map(|id| comment_id(id)).collect(),
    }
}

fn result_for<'a>(response: &'a pb::SubmitReviewResponse, pr: &Pr) -> &'a pb::submit_review_response::Result {
    response
        .results
        .iter()
        .find(|result| result.pr == Some(pr.reference()))
        .unwrap_or_else(|| panic!("a result for {}: {response:#?}", pr.slug()))
}

#[tokio::test(flavor = "multi_thread")]
async fn the_submit_plan_lists_drafts_per_pr_and_sets_apart_those_outside_the_diff() {
    let Drafted {
        fixture,
        session,
        api_draft,
        ledger_draft,
        outside,
    } = drafted().await;
    let plan = fixture.plan(&session).await;
    assert!(plan.token.is_some());
    let api = per_pr(&plan, &fixture.prs[API]);
    assert_eq!(ids(&api.drafts), [api_draft]);
    assert_eq!(api.left_out.len(), 1, "{api:#?}");
    let left_out = &api.left_out[0];
    assert_eq!(left_out.draft.as_ref().map(id_of), Some(outside.as_str()));
    assert_eq!(left_out.reason, pb::submit_plan::left_out::Reason::OutOfDiff as i32);
    let ledger = per_pr(&plan, &fixture.prs[LEDGER]);
    assert_eq!(ids(&ledger.drafts), [ledger_draft]);
    assert!(ledger.left_out.is_empty());
    assert_eq!(fixture.gh.post_count(), 0, "planning posts nothing");
}

#[tokio::test(flavor = "multi_thread")]
async fn submitting_posts_one_review_per_pr_at_the_reviewed_head_with_its_drafts() {
    let Drafted {
        fixture,
        session,
        api_draft,
        ledger_draft,
        outside,
    } = drafted().await;
    let (api, ledger) = (fixture.prs[API].clone(), fixture.prs[LEDGER].clone());
    let plan = fixture.plan(&session).await;
    let response = fixture
        .submit(
            &session,
            &plan,
            "submit-1",
            vec![
                submission(&api, pb::Verdict::RequestChanges, "Round once, at the end", &[&api_draft]),
                submission(&ledger, pb::Verdict::Approve, "Fine by me", &[&ledger_draft]),
            ],
        )
        .await
        .unwrap();

    for (pr, event, summary, line) in [
        (&api, "REQUEST_CHANGES", "Round once, at the end", 3),
        (&ledger, "APPROVE", "Fine by me", 2),
    ] {
        let result = result_for(&response, pr);
        assert!(result.error.is_none(), "{result:?}");
        assert!(result.review_url.starts_with(&pr.url()), "{result:?}");
        let posted = fixture.gh.posted(pr);
        assert_eq!(posted.len(), 1, "one review per PR: {posted:?}");
        let body = &posted[0].1;
        assert_eq!(body["commit_id"], pr.head(), "{body}");
        assert_eq!(body["event"], event, "{body}");
        assert_eq!(body["body"], summary, "{body}");
        let comments = body["comments"].as_array().unwrap();
        assert_eq!(comments.len(), 1, "{body}");
        assert_eq!(comments[0]["path"], if pr.name == "api" { "refund.rs" } else { "ledger.rs" });
        assert_eq!(comments[0]["line"], line, "{body}");
        assert_eq!(comments[0]["side"], "RIGHT", "{body}");
    }

    let comments = fixture.list(&session).await;
    for id in [&api_draft, &ledger_draft] {
        let posted = find(&comments, id);
        assert!(is(posted, pb::CommentStatus::Posted), "{posted:?}");
        assert!(posted.posted_url.starts_with("https://github.com/acme/"), "{posted:?}");
    }
    assert!(is(find(&comments, &outside), pb::CommentStatus::Draft));
    fixture.gh.assert_token_stayed_with_gh(fixture.daemon.dir.path());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_submit_plan_posts_only_once() {
    let Drafted {
        fixture,
        session,
        api_draft,
        ..
    } = drafted().await;
    let api = fixture.prs[API].clone();
    let plan = fixture.plan(&session).await;
    let reviews = || vec![submission(&api, pb::Verdict::Comment, "Looked", &[&api_draft])];
    fixture.submit(&session, &plan, "first", reviews()).await.unwrap();
    let again = fixture.submit(&session, &plan, "second", reviews()).await.unwrap_err();
    assert_eq!(again.code(), Code::FailedPrecondition, "{again:?}");
    assert_eq!(fixture.gh.posted(&api).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_post_keeps_that_prs_drafts_and_says_which_pr_failed() {
    let Drafted {
        fixture,
        session,
        api_draft,
        ledger_draft,
        ..
    } = drafted().await;
    let (api, ledger) = (fixture.prs[API].clone(), fixture.prs[LEDGER].clone());
    fixture.gh.fail_post(&ledger, Some("bad-line"));
    let plan = fixture.plan(&session).await;
    let response = fixture
        .submit(
            &session,
            &plan,
            "submit-1",
            vec![
                submission(&api, pb::Verdict::Comment, "Looked", &[&api_draft]),
                submission(&ledger, pb::Verdict::Comment, "Looked", &[&ledger_draft]),
            ],
        )
        .await
        .unwrap();
    assert!(result_for(&response, &api).error.is_none());
    let failure = result_for(&response, &ledger)
        .error
        .clone()
        .expect("the PR that failed says so");
    assert!(!failure.fix.is_empty(), "{failure:?}");
    assert!(
        matches!(&failure.kind, Some(pb::error::Kind::GithubCli(problem))
            if problem.http_status == 422),
        "{failure:?}"
    );

    let comments = fixture.list(&session).await;
    assert!(is(find(&comments, &api_draft), pb::CommentStatus::Posted));
    let kept = find(&comments, &ledger_draft);
    assert!(is(kept, pb::CommentStatus::Draft), "{kept:?}");
    assert!(kept.posted_url.is_empty());

    // Nothing was lost: once GitHub takes it, the kept draft posts.
    fixture.gh.fail_post(&ledger, None);
    let plan = fixture.plan(&session).await;
    assert!(per_pr(&plan, &api).drafts.is_empty());
    assert_eq!(ids(&per_pr(&plan, &ledger).drafts), [ledger_draft.clone()]);
    let retried = fixture
        .submit(
            &session,
            &plan,
            "submit-2",
            vec![submission(&ledger, pb::Verdict::Comment, "Looked", &[&ledger_draft])],
        )
        .await
        .unwrap();
    assert!(result_for(&retried, &ledger).error.is_none());
    assert_eq!(fixture.gh.posted(&ledger).len(), 1);
    assert_eq!(fixture.gh.posted(&api).len(), 1);
}

/// Submits api#482's review with the daemon set to die right after GitHub
/// took it, and returns what GitHub got.
async fn crash_after_posting(drafted: &mut Drafted) -> (u64, Value) {
    let api = drafted.fixture.prs[API].clone();
    drafted
        .fixture
        .restart_with(&[("HUMATA2_STOP_AT", "pr.posted")])
        .await;
    wait_running(&drafted.fixture.daemon, drafted.session.id.as_ref().unwrap()).await;
    let plan = drafted.fixture.plan(&drafted.session).await;
    let reviews = vec![submission(
        &api,
        pb::Verdict::Comment,
        "Looked",
        &[&drafted.api_draft],
    )];
    assert!(
        drafted
            .fixture
            .submit(&drafted.session, &plan, "crash", reviews)
            .await
            .is_err(),
        "the daemon died before it answered"
    );
    drafted.fixture.daemon.stopped(SLOW).await;
    let posted = drafted.fixture.gh.posted(&api);
    assert_eq!(posted.len(), 1, "{posted:?}");
    posted.into_iter().next().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_posted_just_before_a_crash_is_found_on_github_not_posted_again() {
    let mut drafted = drafted().await;
    let api = drafted.fixture.prs[API].clone();
    let (id, body) = crash_after_posting(&mut drafted).await;
    drafted
        .fixture
        .gh
        .list_review(&api, id, &body, "2099-01-01T00:00:00Z");

    drafted.fixture.restart().await;
    let api_draft = drafted.api_draft.clone();
    let comments = drafted
        .fixture
        .comments_until(&drafted.session, |comments| {
            comments
                .iter()
                .any(|c| id_of(c) == api_draft && is(c, pb::CommentStatus::Posted))
        })
        .await;
    assert!(!find(&comments, &api_draft).posted_url.is_empty());
    assert_eq!(drafted.fixture.gh.posted(&api).len(), 1, "never posted twice");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_github_never_lists_is_left_unconfirmed_not_posted_again() {
    let mut drafted = drafted().await;
    let api = drafted.fixture.prs[API].clone();
    let (id, body) = crash_after_posting(&mut drafted).await;
    // GitHub lists only a review from before this attempt, with the same body.
    drafted
        .fixture
        .gh
        .list_review(&api, id + 1000, &body, "2000-01-01T00:00:00Z");

    drafted.fixture.restart().await;
    let api_draft = drafted.api_draft.clone();
    let comments = drafted
        .fixture
        .comments_until(&drafted.session, |comments| {
            comments
                .iter()
                .any(|c| id_of(c) == api_draft && c.post_unconfirmed)
        })
        .await;
    let unconfirmed = find(&comments, &api_draft);
    assert!(!is(unconfirmed, pb::CommentStatus::Posted), "{unconfirmed:?}");
    assert!(unconfirmed.posted_url.is_empty());
    assert_eq!(drafted.fixture.gh.posted(&api).len(), 1, "never posted again on its own");
}

// ---------------------------------------------------------------------------
// A6: when the PR changes (PR-15–PR-17, D286).


#[tokio::test(flavor = "multi_thread")]
async fn new_commits_on_a_prs_head_are_counted() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API, LEDGER]).await;
    let reviewed = clone_head(repo_of(&session, &fixture.prs[API]));
    let quiet = fixture.check_heads(&session).await;
    assert!(quiet.checked_at.is_some());
    assert!(
        quiet.changes.iter().all(|change| change.new_commits == 0 && !change.force_pushed),
        "{quiet:?}"
    );

    fixture.move_head(API, &TWO_COMMITS);
    let heads = fixture.check_heads(&session).await;
    let api = change_of(&heads, &fixture.prs[API]).expect("api#482 moved");
    assert_eq!(api.new_commits, 2);
    assert!(!api.force_pushed);
    assert_eq!(api.state, pb::PullRequestState::Open as i32);
    if let Some(ledger) = change_of(&heads, &fixture.prs[LEDGER]) {
        assert_eq!(ledger.new_commits, 0, "{heads:?}");
    }
    // Checking changes nothing: the clone stays where it was.
    let still = get_session(&fixture.daemon, session.id.as_ref().unwrap()).await;
    assert_eq!(clone_head(repo_of(&still, &fixture.prs[API])), reviewed);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_force_push_is_reported_as_one() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let api = &fixture.prs[API];
    let work = api.work.display().to_string();
    edit(&api.work, &[("refund.rs", 3, "    let rounded = (cents - fee).round();")]);
    git(&["-C", &work, "add", "-A"]);
    git(&["-C", &work, "commit", "-q", "--amend", "-m", "Round refunds, rewritten"]);
    push_head(api, true);
    fixture.gh.publish(&fixture.prs);

    let heads = fixture.check_heads(&session).await;
    let change = change_of(&heads, api).expect("api#482 moved");
    assert!(change.force_pushed, "{change:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn updating_the_clone_fetches_the_head_tells_the_agent_and_outdates_changed_lines() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let mut agent = fixture.agent(&session).await;
    let api = fixture.prs[API].clone();
    let on_changed = agent.draft(&api, "refund.rs", 3, 3, "Half-even here?").await;
    let on_kept = agent.draft(&api, "refund.rs", 6, 6, "Log before or after?").await;

    fixture.move_head(API, &TWO_COMMITS);
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

    let updated = get_session(&fixture.daemon, session.id.as_ref().unwrap()).await;
    let repo = repo_of(&updated, &api);
    assert_eq!(clone_head(repo), api.head());
    assert_eq!(repo.branch, api.head_branch);

    // PR-16: the new commits and the files changed since the last head.
    let said = fixture.told(&session, &["rounding_test.rs", "refund.rs"]).await;
    for (sha, subject) in api.commits().iter().rev().take(2) {
        assert!(
            said.contains(subject.as_str()) || said.contains(&sha[..7]),
            "the agent is told of {subject}: {said}"
        );
    }

    let comments = fixture.list(&session).await;
    assert!(find(&comments, &on_changed).anchor.as_ref().unwrap().outdated);
    assert!(!find(&comments, &on_kept).anchor.as_ref().unwrap().outdated);
    assert!(is(find(&comments, &on_changed), pb::CommentStatus::Draft), "kept, not dropped");

    // An outdated draft is set apart before submitting.
    let plan = fixture.plan(&session).await;
    let per = per_pr(&plan, &api);
    assert!(per.drafts.iter().all(|draft| id_of(draft) != on_changed));
    assert!(per.left_out.iter().any(|left| {
        left.draft.as_ref().map(id_of) == Some(on_changed.as_str())
            && left.reason == pb::submit_plan::left_out::Reason::Outdated as i32
    }));

    // Up to date now: nothing new to count.
    let heads = fixture.check_heads(&session).await;
    assert!(change_of(&heads, &api).is_none_or(|change| change.new_commits == 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merged_pr_shows_as_merged() {
    let mut fixture = Fixture::new().await;
    let session = fixture.start_review(&[API, LEDGER]).await;
    fixture.prs[LEDGER].state = "MERGED";
    fixture.gh.publish(&fixture.prs);
    let heads = fixture.check_heads(&session).await;
    let ledger = change_of(&heads, &fixture.prs[LEDGER]).expect("ledger#91 changed");
    assert_eq!(ledger.state, pb::PullRequestState::Merged as i32);
    if let Some(api) = change_of(&heads, &fixture.prs[API]) {
        assert_eq!(api.state, pb::PullRequestState::Open as i32);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daemon_notices_a_moved_head_on_its_own_and_says_so_on_the_feed() {
    // D286 polls every minute; a test daemon polls as often as it is told.
    let fixture = Fixture::with_env(&[("HUMATA2_PR_POLL_MS", "200")]).await;
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
    let api = fixture.prs[API].reference();
    let deadline = Instant::now() + SLOW;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = tokio::time::timeout(left, events.message())
            .await
            .expect("no head change came on the feed")
            .unwrap()
            .expect("the feed ended");
        if let Some(pb::event::Payload::PullRequestHeads(heads)) = event.payload {
            assert_eq!(heads.session, session.id);
            if let Some(change) = heads.changes.iter().find(|change| change.pr == Some(api.clone())) {
                if change.new_commits == 2 {
                    break;
                }
            }
        }
    }
}

// PR-17: Archive names a review's unsubmitted drafts.

#[tokio::test(flavor = "multi_thread")]
async fn archive_plans_count_a_reviews_unsubmitted_drafts_until_they_are_submitted() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let mut agent = fixture.agent(&session).await;
    let api = fixture.prs[API].clone();
    let draft = agent.draft(&api, "refund.rs", 3, 3, "Half-even here?").await;
    let plan_archive = || async {
        fixture
            .daemon
            .session_service()
            .await
            .plan_archive(pb::PlanArchiveRequest { id: session.id.clone() })
            .await
            .unwrap()
            .into_inner()
    };
    let before = plan_archive().await;
    assert_eq!(before.unsubmitted_drafts, 1, "{before:?}");
    assert!(before.unpushed.is_empty(), "nothing is unpushed, yet the draft counts: {before:?}");

    let plan = fixture.plan(&session).await;
    fixture
        .submit(
            &session,
            &plan,
            "submit-for-archive",
            vec![submission(&api, pb::Verdict::Comment, "Looked", &[&draft])],
        )
        .await
        .unwrap();
    let after = plan_archive().await;
    assert_eq!(after.unsubmitted_drafts, 0, "{after:?}");
}

// D513: archiving keeps a review's unsubmitted drafts; a restore brings
// them back.

#[tokio::test(flavor = "multi_thread")]
async fn archived_reviews_keep_unsubmitted_drafts_through_a_restore() {
    let fixture = Fixture::new().await;
    let session = fixture.start_review(&[API]).await;
    let mut agent = fixture.agent(&session).await;
    let api = fixture.prs[API].clone();
    let draft = agent.draft(&api, "refund.rs", 3, 3, "Half-even here?").await;
    let sessions = || async { fixture.daemon.session_service().await };
    let plan = sessions()
        .await
        .plan_archive(pb::PlanArchiveRequest { id: session.id.clone() })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(plan.unsubmitted_drafts, 1, "{plan:?}");
    let archived = sessions()
        .await
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: session.id.clone(),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
            end: Some(true),
        })
        .await
        .unwrap()
        .into_inner();
    finished(&fixture.daemon, &archived).await;
    let restored = sessions()
        .await
        .restore_session(pb::RestoreSessionRequest {
            idempotency_key: None,
            id: session.id.clone(),
            machine: None,
            stopped: false,
        })
        .await
        .unwrap()
        .into_inner();
    finished(&fixture.daemon, &restored).await;

    let comments = fixture.list(&session).await;
    assert!(is(find(&comments, &draft), pb::CommentStatus::Draft));
    let again = sessions()
        .await
        .plan_archive(pb::PlanArchiveRequest { id: session.id.clone() })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(again.unsubmitted_drafts, 1, "{again:?}");
}
