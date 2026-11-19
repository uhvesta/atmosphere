//! A pull request review moved to another machine (PR-21, MV-1, MV-13): the
//! review's binding, its drafts and its reviewed snapshots go with it, and the
//! host's looks at its clones (heads check, Submit review's line checks) run
//! where the review lives now, on the box or on the host again.
//!
//! A host and one box on the multi-machine rig; GitHub is the fake `gh` of
//! `common/pr_github.rs`, on the host alone.

use humatad_test_common as common;
// The shared fixture's agent speaks MCP; these tests never start one.
use humatad_test_common::mcp_client;
use humatad_test_common::pr_github;
use humatad_test_common::remote_rig;

use common::{get_session, git, repo_ref};
use humata2_grpc::humata::v1 as pb;
use pr_github::{API, FakeGh, Pr, TWO_COMMITS, change_of, clone_head, repo_of, seed_pull_requests};
use remote_rig::{Rig, is_live};
use tonic::Code;

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

    /// Starts a review of `api` on `machine` and returns its session once
    /// its agent runs.
    async fn start_on(&self, machine: pb::MachineId) -> pb::SessionId {
        let operation = self
            .reviews()
            .await
            .start_review(pb::StartReviewRequest {
                idempotency_key: None,
                prs: vec![self.prs[API].reference()],
                machine: Some(machine),
                agent: None,
                model: String::new(),
                first_message: pr_github::CONTEXT.to_owned(),
                push_allowed: false,
                presence: None,
            })
            .await
            .expect("the review starts")
            .into_inner();
        let ended = self.rig.ended(&operation).await;
        assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "{ended:?}");
        let Some(pb::operation::Result::Session(id)) = ended.result else {
            panic!("a started review names its session: {ended:?}");
        };
        self.rig.wait_session(&id, "live", is_live).await;
        id
    }

    /// Moves `session` to `to` and returns the move's operation once it ended.
    async fn move_to(&self, session: &pb::SessionId, to: &pb::MachineId) -> pb::Operation {
        let started = pb::session_service_client::SessionServiceClient::new(self.rig.channel().await)
            .move_session(pb::MoveSessionRequest {
                idempotency_key: Some(pb::IdempotencyKey {
                    value: format!("move-{}-{}", session.value, to.value),
                }),
                id: Some(session.clone()),
                to: Some(to.clone()),
            })
            .await
            .expect("the move starts")
            .into_inner();
        self.rig.ended(&started).await
    }

    /// The user's draft on `rounding.rs` line 1 of api#482; its id.
    async fn draft(&self, session: &pb::SessionId, text: &str) -> String {
        let api = &self.prs[API];
        pb::comment_service_client::CommentServiceClient::new(self.rig.channel().await)
            .create_comment(pb::CreateCommentRequest {
                idempotency_key: None,
                session: Some(session.clone()),
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
                text: text.to_owned(),
                destination: Some(pb::CommentDestination {
                    destination: Some(pb::comment_destination::Destination::PullRequest(api.reference())),
                }),
            })
            .await
            .expect("the host takes the draft")
            .into_inner()
            .id
            .expect("a draft has an id")
            .value
    }

    async fn drafts(&self, session: &pb::SessionId) -> Vec<String> {
        pb::comment_service_client::CommentServiceClient::new(self.rig.channel().await)
            .list_comments(pb::ListCommentsRequest {
                session: Some(session.clone()),
                ..pb::ListCommentsRequest::default()
            })
            .await
            .expect("the host lists the review's comments")
            .into_inner()
            .comments
            .into_iter()
            .filter_map(|comment| comment.id.map(|id| id.value))
            .collect()
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

    fn move_head(&self, commits: &[(&str, &[(&str, usize, &str)])]) {
        let pr = &self.prs[API];
        for (subject, edits) in commits {
            pr_github::commit(&pr.work, subject, edits);
        }
        pr_github::push_head(pr, false);
        self.gh.publish(&self.prs);
    }

    /// The review's clone of api where `clones` (a session as one machine
    /// shows it) says it is.
    fn clone_in(&self, clones: &pb::Session) -> String {
        repo_of(clones, &self.prs[API]).path.clone()
    }
}

/// A reviewed snapshot (RV-28) the clone keeps: a commit no remote has under
/// `refs/humata/reviewed/<session>/1`. Its commit.
fn snapshot_in(clone: &str, session: &pb::SessionId) -> String {
    let commit = git(&["-C", clone, "commit-tree", "HEAD^{tree}", "-p", "HEAD", "-m", "what the user saw"]);
    git(&["-C", clone, "update-ref", &format!("refs/humata/reviewed/{}/1", session.value), &commit]);
    commit
}

fn snapshot_of(clone: &str, session: &pb::SessionId) -> String {
    git(&["-C", clone, "rev-parse", &format!("refs/humata/reviewed/{}/1", session.value)])
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_moved_from_this_mac_to_a_box_keeps_its_binding_drafts_and_snapshots_and_runs_git_there() {
    let world = world().await;
    let api = world.prs[API].clone();
    let (host, the_box) = (world.rig.host_id().await, Rig::box_id(BOX));
    let id = world.start_on(host).await;
    let draft = world.draft(&id, "Needs a test").await;
    let here = world.clone_in(&world.rig.session(&id).await);
    let snapshot = snapshot_in(&here, &id);
    let binding = world.rig.session(&id).await.pull_request.expect("the host knows it as a review");

    let ended = world.move_to(&id, &the_box).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);
    let after = world.rig.wait_session(&id, "live on the box", |s| s.machine.as_ref() == Some(&the_box) && is_live(s)).await;

    // It is the same review: the binding, the draft and the snapshot.
    assert_eq!(after.pull_request, Some(binding));
    assert_eq!(world.drafts(&id).await, vec![draft.clone()]);
    let on_the_box = get_session(world.rig.the(BOX).daemon(), &id).await;
    let there = world.clone_in(&on_the_box);
    assert!(std::path::Path::new(&there).starts_with(world.rig.the(BOX).home()), "{there} is the box's folder");
    assert_eq!(snapshot_of(&there, &id), snapshot, "the reviewed snapshot came with the clone");
    assert_eq!(clone_head(repo_of(&on_the_box, &api)), api.head());
    let record = world.rig.the(BOX).store().review_session(&id.value).unwrap().expect("the box knows it as a review");
    assert_eq!(record.prs.len(), 1);
    assert_eq!((record.prs[0].number, record.prs[0].head_sha.as_str()), (api.number, api.head().as_str()));
    assert!(!std::path::Path::new(&here).exists(), "the host's copy went to its trash");

    // The host's checks now run in the box's clone.
    world.move_head(&TWO_COMMITS);
    let heads = world.check_heads(&id).await;
    let change = change_of(&heads, &api).expect("the check finds api#482's new commits on the box's clone");
    assert_eq!(change.new_commits, 2, "{change:?}");

    // Submit review checks the draft's line there and posts through the host's gh.
    let plan = world
        .reviews()
        .await
        .plan_submit(pb::PlanSubmitRequest { session: Some(id.clone()) })
        .await
        .expect("the host plans the submit")
        .into_inner();
    let per = plan.pull_requests.iter().find(|per| per.pr == Some(api.reference())).expect("the plan lists api#482");
    assert!(per.drafts.iter().any(|held| held.id.as_ref().map(|id| id.value.as_str()) == Some(draft.as_str())), "{per:?}");
    assert!(per.left_out.is_empty(), "{per:?}");
    let response = world
        .reviews()
        .await
        .submit_review(pb::SubmitReviewRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: "submit-after-the-move".to_owned(),
            }),
            session: Some(id.clone()),
            reviews: vec![pb::ReviewSubmission {
                pr: Some(api.reference()),
                verdict: pb::Verdict::Comment as i32,
                summary: "Looks fine".to_owned(),
                drafts: vec![pb::CommentId { value: draft }],
            }],
            plan: plan.token.clone(),
            presence: None,
        })
        .await
        .expect("the host submits")
        .into_inner();
    assert!(response.results.iter().all(|result| result.error.is_none()), "{response:?}");
    assert_eq!(world.gh.posted(&api).len(), 1, "{}", world.gh.log());
    world.gh.assert_token_stayed_with_gh(&world.rig.world.root);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_moved_from_a_box_back_to_this_mac_runs_git_here_again() {
    let world = world().await;
    let api = world.prs[API].clone();
    let (host, the_box) = (world.rig.host_id().await, Rig::box_id(BOX));
    let id = world.start_on(the_box.clone()).await;
    let draft = world.draft(&id, "Needs a test").await;
    let there = world.clone_in(&get_session(world.rig.the(BOX).daemon(), &id).await);
    let snapshot = snapshot_in(&there, &id);

    let ended = world.move_to(&id, &host).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);
    let after = world.rig.wait_session(&id, "live on the host", |s| s.machine.as_ref() != Some(&the_box) && is_live(s)).await;

    assert!(after.pull_request.is_some(), "{after:?}");
    assert_eq!(world.drafts(&id).await, vec![draft]);
    let here = world.clone_in(&after);
    assert!(!std::path::Path::new(&here).starts_with(world.rig.the(BOX).home()), "{here} is the host's folder");
    assert_eq!(snapshot_of(&here, &id), snapshot, "the reviewed snapshot came home with the clone");
    assert_eq!(clone_head(repo_of(&after, &api)), api.head());
    assert!(world.rig.the(BOX).store().review_session(&id.value).unwrap().is_none(), "the box no longer holds the review");
    assert!(!std::path::Path::new(&there).exists(), "the box's copy went to its trash");

    // Git runs on the host: its own clone moves when it is updated.
    world.move_head(&TWO_COMMITS);
    let heads = world.check_heads(&id).await;
    assert_eq!(change_of(&heads, &api).expect("the check finds the new commits").new_commits, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_review_that_cannot_move_is_refused_and_stays_whole_where_it_was() {
    let world = world().await;
    let api = world.prs[API].clone();
    let (host, the_box) = (world.rig.host_id().await, Rig::box_id(BOX));
    let id = world.start_on(host).await;
    let draft = world.draft(&id, "Needs a test").await;
    let here = world.clone_in(&world.rig.session(&id).await);
    let snapshot = snapshot_in(&here, &id);
    world.rig.link_lost(BOX).await;

    let refused = pb::session_service_client::SessionServiceClient::new(world.rig.channel().await)
        .move_session(pb::MoveSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: "move-offline".to_owned() }),
            id: Some(id.clone()),
            to: Some(the_box.clone()),
        })
        .await
        .expect_err("a box that is offline can't take the review");
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");

    let after = world.rig.session(&id).await;
    assert_ne!(after.machine.as_ref(), Some(&the_box));
    assert!(is_live(&after), "{after:?}");
    assert!(after.pull_request.is_some());
    assert_eq!(world.drafts(&id).await, vec![draft]);
    assert_eq!(snapshot_of(&here, &id), snapshot);
    assert_eq!(clone_head(repo_of(&after, &api)), api.head());
    assert!(world.rig.the(BOX).store().review_session(&id.value).unwrap().is_none(), "nothing of it reached the box");
    // The host still looks at its own clone.
    world.move_head(&TWO_COMMITS);
    assert_eq!(change_of(&world.check_heads(&id).await, &api).expect("new commits").new_commits, 2);
}
