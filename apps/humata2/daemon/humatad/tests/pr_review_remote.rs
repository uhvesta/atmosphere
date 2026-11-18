//! A pull request review started on another machine (PR-4, PR-21; flow
//! 26.17 step 2 with devbox-1). The host asks GitHub through its own `gh`,
//! the box clones each head from its remote with no token, and every look
//! at those clones (the heads check, Update the clone, the line checks of
//! Submit review) runs in the box's own folder through the link.
//!
//! A host and one box on the multi-machine rig; GitHub is the fake `gh` of
//! `common/pr_github.rs`, on the host alone.

mod common;
// The shared fixture's agent speaks MCP; these tests never start one.
#[path = "common/mcp_client.rs"]
#[allow(dead_code)]
mod mcp_client;
#[path = "common/pr_github.rs"]
mod pr_github;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{get_session, repo_ref};
use humata2_grpc::humata::v1 as pb;
use pr_github::{API, FakeGh, LEDGER, Pr, SLOW, TWO_COMMITS, change_of, clone_head, repo_of, seed_pull_requests};
use remote_rig::{Rig, is_live};
use tonic::Status;

const BOX: &str = "devbox-1";

/// A poll interval no test outlives: only the tests' own checks run.
const NEVER: &str = "3600000";

struct World {
    rig: Rig,
    gh: FakeGh,
    prs: Vec<Pr>,
}

async fn world() -> World {
    let rig = Rig::start_seen_with_host_env(
        &[BOX],
        &[],
        &[
            ("HUMATA2_GH", "{dir}/gh/gh"),
            ("HUMATA2_PR_CHECK_REUSE_MS", "0"),
            ("HUMATA2_PR_POLL_MS", NEVER),
        ],
    )
    .await;
    let gh = FakeGh::new(rig.world.root.join("gh"));
    let prs = seed_pull_requests(&rig.world, &gh);
    World { rig, gh, prs }
}

impl World {
    async fn reviews(&self) -> pb::pull_request_review_service_client::PullRequestReviewServiceClient<tonic::transport::Channel> {
        pb::pull_request_review_service_client::PullRequestReviewServiceClient::new(self.rig.channel().await)
    }

    /// Asks the host to review `which` on the box and returns the review's
    /// session once its start has ended, or what refused the start.
    async fn start(&self, which: &[usize]) -> Result<pb::SessionId, Status> {
        let operation = self
            .reviews()
            .await
            .start_review(pb::StartReviewRequest {
                idempotency_key: None,
                prs: which.iter().map(|at| self.prs[*at].reference()).collect(),
                machine: Some(Rig::box_id(BOX)),
                agent: None,
                model: String::new(),
                first_message: pr_github::CONTEXT.to_owned(),
                push_allowed: false,
                presence: None,
            })
            .await?
            .into_inner();
        let ended = self.rig.ended(&operation).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{ended:?}");
        let Some(pb::operation::Result::Session(id)) = ended.result else {
            panic!("a started review names its session: {ended:?}");
        };
        Ok(id)
    }

    async fn start_live(&self, which: &[usize]) -> pb::SessionId {
        let id = self.start(which).await.expect("the review starts on the box");
        self.rig.wait_session(&id, "live", is_live).await;
        id
    }

    async fn check_heads(&self, session: &pb::SessionId) -> pb::CheckHeadsResponse {
        self.reviews()
            .await
            .check_heads(pb::CheckHeadsRequest {
                session: Some(session.clone()),
            })
            .await
            .expect("the host checks the review's heads")
            .into_inner()
    }

    /// The session as the box's own daemon shows it: its clones are the
    /// box's folders.
    async fn on_the_box(&self, session: &pb::SessionId) -> pb::Session {
        get_session(self.rig.the(BOX).daemon(), session).await
    }

    fn move_head(&self, at: usize, commits: &[(&str, &[(&str, usize, &str)])]) {
        let pr = &self.prs[at];
        for (subject, edits) in commits {
            pr_github::commit(&pr.work, subject, edits);
        }
        pr_github::push_head(pr, false);
        self.gh.publish(&self.prs);
    }
}

fn comment_id(id: &str) -> pb::CommentId {
    pb::CommentId { value: id.to_owned() }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_started_on_a_box_clones_each_head_there_and_keeps_gh_on_the_host() {
    let world = world().await;
    let (api, ledger) = (world.prs[API].clone(), world.prs[LEDGER].clone());
    let id = world.start_live(&[API, LEDGER]).await;

    // The host shows a review that runs on the box.
    let shown = world.rig.session(&id).await;
    assert_eq!(shown.machine, Some(Rig::box_id(BOX)));
    let binding = shown.pull_request.expect("the host knows it as a review");
    assert_eq!(binding.pull_requests.len(), 2, "{binding:?}");

    // The box's own folders hold each head, on the PR's own branch.
    let on_the_box = world.on_the_box(&id).await;
    let home = world.rig.the(BOX).home();
    for pr in [&api, &ledger] {
        let repo = repo_of(&on_the_box, pr);
        assert_eq!(clone_head(repo), pr.head(), "{}", pr.slug());
        assert_eq!(repo.branch, pr.head_branch);
        assert!(Path::new(&repo.path).starts_with(home), "{} is the box's folder", repo.path);
    }

    // The box is one of the PR reviews channel's machines, beside the host.
    let channels = pb::channel_service_client::ChannelServiceClient::new(world.rig.channel().await)
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .channels;
    let reviews = channels
        .iter()
        .find(|channel| channel.is_pr_reviews)
        .expect("the PR reviews channel exists");
    assert!(reviews.machines.contains(&Rig::box_id(BOX)), "{reviews:?}");
    assert!(reviews.machines.contains(&world.rig.host_id().await), "{reviews:?}");

    // GitHub was reached through the host's `gh` alone, which never handed
    // out its token: not to the box, not to any file.
    world.gh.assert_token_stayed_with_gh(&world.rig.world.root);
}

#[tokio::test(flavor = "multi_thread")]
async fn checking_and_updating_a_box_review_work_on_the_boxs_clones() {
    let world = world().await;
    let api = world.prs[API].clone();
    let id = world.start_live(&[API]).await;
    let reviewed = clone_head(repo_of(&world.on_the_box(&id).await, &api));

    world.move_head(API, &TWO_COMMITS);
    let heads = world.check_heads(&id).await;
    let change = change_of(&heads, &api).expect("the check finds api#482's new commits");
    assert_eq!(change.new_commits, 2, "{change:?}");
    assert!(!change.force_pushed, "{change:?}");
    // A check moves nothing of the box's clone.
    assert_eq!(clone_head(repo_of(&world.on_the_box(&id).await, &api)), reviewed);

    let operation = world
        .reviews()
        .await
        .update_clone(pb::UpdateCloneRequest {
            idempotency_key: None,
            session: Some(id.clone()),
        })
        .await
        .expect("the host updates the review's clones")
        .into_inner();
    let ended = world.rig.ended(&operation).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{ended:?}");
    assert_eq!(clone_head(repo_of(&world.on_the_box(&id).await, &api)), api.head());

    // The box's agent was told what came in.
    let conversation = {
        let remote = world.rig.the(BOX);
        remote.conversation(&remote.conversation_id(&id))
    };
    let deadline = Instant::now() + SLOW;
    loop {
        let said = std::fs::read_to_string(&conversation).unwrap_or_default();
        if TWO_COMMITS.iter().all(|(subject, _)| said.contains(subject)) {
            break;
        }
        assert!(Instant::now() < deadline, "the box's agent was never told of the commits; it was told {said:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Brought up to date, the review has nothing new.
    let after = world.check_heads(&id).await;
    assert!(change_of(&after, &api).is_none(), "{after:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn submitting_a_box_review_checks_lines_in_the_boxs_clone_and_posts_through_the_hosts_gh() {
    let world = world().await;
    let api = world.prs[API].clone();
    let id = world.start_live(&[API]).await;

    let drafted = pb::comment_service_client::CommentServiceClient::new(world.rig.channel().await)
        .create_comment(pb::CreateCommentRequest {
            idempotency_key: None,
            session: Some(id.clone()),
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
        .expect("the host takes the draft")
        .into_inner();
    let draft = drafted.id.clone().expect("a draft has an id").value;

    let plan = world
        .reviews()
        .await
        .plan_submit(pb::PlanSubmitRequest { session: Some(id.clone()) })
        .await
        .expect("the host plans the submit")
        .into_inner();
    let per = plan
        .pull_requests
        .iter()
        .find(|per| per.pr == Some(api.reference()))
        .expect("the plan lists api#482");
    // The line is in the diff of the box's clone, so the draft is postable.
    assert!(
        per.drafts.iter().any(|held| held.id.as_ref().map(|id| id.value.as_str()) == Some(draft.as_str())),
        "{per:?}"
    );
    assert!(per.left_out.is_empty(), "{per:?}");

    let response = world
        .reviews()
        .await
        .submit_review(pb::SubmitReviewRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: "submit-on-the-box".to_owned(),
            }),
            session: Some(id.clone()),
            reviews: vec![pb::ReviewSubmission {
                pr: Some(api.reference()),
                verdict: pb::Verdict::Comment as i32,
                summary: "Looks fine".to_owned(),
                drafts: vec![comment_id(&draft)],
            }],
            plan: plan.token.clone(),
            presence: None,
        })
        .await
        .expect("the host submits")
        .into_inner();
    assert!(response.results.iter().all(|result| result.error.is_none()), "{response:?}");

    let posted = world.gh.posted(&api);
    assert_eq!(posted.len(), 1, "the host's gh posted one review:\n{}", world.gh.log());
    assert_eq!(posted[0].1["commit_id"], api.head());
    assert_eq!(posted[0].1["comments"][0]["path"], "rounding.rs");
    world.gh.assert_token_stayed_with_gh(&world.rig.world.root);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_on_a_box_that_is_offline_is_refused_and_leaves_nothing() {
    let world = world().await;
    world.rig.link_lost(BOX).await;

    let refused = world.start(&[API]).await.expect_err("an offline box can't start a review");
    assert_ne!(refused.code(), tonic::Code::Ok);

    let sessions = pb::session_service_client::SessionServiceClient::new(world.rig.channel().await)
        .list_sessions(pb::ListSessionsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .sessions;
    assert!(sessions.is_empty(), "no session was left behind: {sessions:?}");
    let channels = pb::channel_service_client::ChannelServiceClient::new(world.rig.channel().await)
        .list_channels(pb::ListChannelsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .channels;
    for channel in channels.iter().filter(|channel| channel.is_pr_reviews) {
        assert!(!channel.machines.contains(&Rig::box_id(BOX)), "{channel:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_host_may_ask_a_box_to_run_git_in_a_review_clone() {
    let world = world().await;
    let id = world.start_live(&[API]).await;
    let refused = pb::session_carry_service_client::SessionCarryServiceClient::new(
        world.rig.the(BOX).daemon().channel().await,
    )
    .run_review_git(pb::RunReviewGitRequest {
        session: Some(id),
        repo: "api".to_owned(),
        args: vec!["rev-parse".to_owned(), "HEAD".to_owned()],
        stdin: Vec::new(),
    })
    .await
    .expect_err("a person at the box is not the host");
    assert_eq!(refused.code(), tonic::Code::PermissionDenied);
}

fn pull_request_base() -> Option<pb::DiffBase> {
    Some(pb::DiffBase {
        base: Some(pb::diff_base::Base::PullRequestBase(pb::PullRequestBase {})),
    })
}

/// The paths of `repo` (`owner/name`) in `diff`, sorted.
fn paths_in(diff: &pb::GetDiffResponse, repo: &str) -> Vec<String> {
    let mut paths: Vec<String> = diff
        .repos
        .iter()
        .filter(|own| own.repo.as_ref().is_some_and(|own| format!("{}/{}", own.owner, own.name) == repo))
        .flat_map(|own| own.files.iter().map(|file| file.path.clone()))
        .collect();
    paths.sort();
    paths
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hosts_diff_of_a_box_review_is_each_pull_request_against_its_base_on_the_box() {
    let world = world().await;
    let id = world.start_live(&[API, LEDGER]).await;
    let mut diffs = pb::diff_service_client::DiffServiceClient::new(world.rig.channel().await);

    let diff = diffs
        .get_diff(pb::GetDiffRequest {
            session: Some(id.clone()),
            base: pull_request_base(),
            repos: Vec::new(),
        })
        .await
        .expect("the host shows the box review's diff")
        .into_inner();
    assert_eq!(paths_in(&diff, "acme/api"), ["refund.rs", "rounding.rs"], "{diff:#?}");
    assert_eq!(paths_in(&diff, "acme/ledger"), ["ledger.rs"], "{diff:#?}");

    // One file's hunks come from the box's clone against the same base.
    let file = diffs
        .get_file_diff(pb::GetFileDiffRequest {
            session: Some(id.clone()),
            base: pull_request_base(),
            file: Some(pb::RepoPath {
                repo: Some(repo_ref("acme", "ledger")),
                path: "ledger.rs".to_owned(),
            }),
            ..Default::default()
        })
        .await
        .expect("the host shows a file of the box review")
        .into_inner();
    assert!(!file.hunks.is_empty(), "{file:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_diff_of_a_box_review_while_the_box_is_offline_says_the_machine_is_offline() {
    let world = world().await;
    let id = world.start_live(&[API]).await;
    world.rig.link_lost(BOX).await;

    let refused = pb::diff_service_client::DiffServiceClient::new(world.rig.channel().await)
        .get_diff(pb::GetDiffRequest {
            session: Some(id),
            base: pull_request_base(),
            repos: Vec::new(),
        })
        .await
        .expect_err("an offline box has no diff to show");
    let error = common::error_of(&refused);
    assert!(
        matches!(error.kind, Some(pb::error::Kind::MachineOffline(_))),
        "the box is named offline: {refused:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_reviewed_on_a_box_review_is_refused_as_not_yet_available_there() {
    let world = world().await;
    let id = world.start_live(&[API]).await;
    let refused = pb::diff_service_client::DiffServiceClient::new(world.rig.channel().await)
        .mark_reviewed(pb::MarkReviewedRequest {
            session: Some(id),
            ..Default::default()
        })
        .await
        .expect_err("Mark reviewed isn't forwarded to a box yet (D736)");
    assert_eq!(refused.code(), tonic::Code::Unimplemented, "{refused:?}");
}
