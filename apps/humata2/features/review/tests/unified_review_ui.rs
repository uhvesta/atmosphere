use std::sync::Arc;

use humata2_clients::{FakeDaemon, Workspace};
use humata2_clients::comments::{Anchor, Author, Comment, CommentId, Destination, Side, Status};
use humata2_clients::model::{
    DiffLine, FileDiff, LineKind, RepoBase, RepoDiff, RepoDiffSpec, SessionId,
};
use humata2_clients::views::{LayerRepoView, LayerView, PlanView, StackView};
use humata2_review::comments::{
    CommentsState, PrFeedbackCardModel, PrReviewStatus, SessionComments, post_replies_cta_label,
    queued_github_reply_count,
};
use humata2_review::looks::{LayerLook, layer_look};
use humata2_review::{Review, ReviewAction, ReviewState};
use tca::{Reducer as _, TestClock};

fn reducer() -> Review {
    let clock = Arc::new(TestClock::new());
    Review::new(Arc::new(FakeDaemon::new(Workspace::default(), clock.clone())), clock)
}

fn sample_diff(session: &SessionId, repo: &str, path: &str, text: &str) -> RepoDiff {
    RepoDiff {
        session: session.clone(),
        repo: repo.to_owned(),
        base: "origin/main".to_owned(),
        files: vec![FileDiff {
            path: path.to_owned(),
            lines: vec![DiffLine {
                kind: LineKind::Added,
                old: None,
                new: Some(1),
                text: text.to_owned(),
            }],
            added: 1,
            ..FileDiff::default()
        }],
        comments: Vec::new(),
    }
}

#[test]
fn rebased_unchanged_layer_preserves_looks_good_and_clears_on_clear_and_evict() {
    let s1 = SessionId::new("se_1");
    let mut state = ReviewState {
        session: Some(s1.clone()),
        ..ReviewState::default()
    };
    let diff = sample_diff(&s1, "acme/api", "src/lib.rs", "fn a() {}");
    state.fetch.entry(s1.clone()).or_default().diffs = Some(vec![diff.clone()]);
    state.layer.insert(s1.clone(), Some("schema".to_owned()));

    let r = reducer();
    let _ = r.reduce(&mut state, ReviewAction::LooksGood);
    assert_eq!(layer_look(&state, &s1, "schema"), LayerLook::LooksGood);

    let _ = r.reduce(
        &mut state,
        ReviewAction::MarkLayerRebasedUnchanged {
            layer: "schema".to_owned(),
        },
    );
    assert!(state.is_layer_rebased_unchanged(&s1, "schema"));
    assert_eq!(layer_look(&state, &s1, "schema"), LayerLook::LooksGood);

    let _ = r.reduce(&mut state, ReviewAction::ClearLooks);
    assert!(!state.is_layer_rebased_unchanged(&s1, "schema"));

    let _ = r.reduce(
        &mut state,
        ReviewAction::MarkLayerRebasedUnchanged {
            layer: "schema".to_owned(),
        },
    );
    assert!(state.is_layer_rebased_unchanged(&s1, "schema"));
    state.evict_session(&s1);
    assert!(!state.is_layer_rebased_unchanged(&s1, "schema"));
}

#[test]
fn push_confirmation_summary_defaults_to_draft_and_counts_unreviewed() {
    let s1 = SessionId::new("se_1");
    let mut state = ReviewState {
        session: Some(s1.clone()),
        ..ReviewState::default()
    };
    assert!(state.push_draft_mode);

    let layer_view = |title: &str, repo: &str| LayerView {
        title: title.to_owned(),
        repos: vec![LayerRepoView {
            repo: repo.to_owned(),
            spec: RepoDiffSpec {
                repo: repo.to_owned(),
                base: RepoBase::Range {
                    base: "main".to_owned(),
                    head: "HEAD".to_owned(),
                },
                options: Default::default(),
            },
            pull_request: String::new(),
            files: 1,
            drift: None,
        }],
        ..LayerView::default()
    };
    state.plans.insert(
        s1.clone(),
        PlanView {
            stack: Some(StackView {
                layers: vec![
                    layer_view("schema", "acme/api"),
                    layer_view("ui", "acme/web"),
                ],
                whole: Vec::new(),
            }),
            ..PlanView::default()
        },
    );

    let summary = state.push_confirmation_summary(&s1, 2);
    assert!(summary.draft);
    assert_eq!(summary.entries.len(), 2);
    assert_eq!(summary.unreviewed_count, 2);
    assert_eq!(summary.open_notes, 2);

    let r = reducer();
    let _ = r.reduce(&mut state, ReviewAction::SetPushDraftMode(false));
    let ready = state.push_confirmation_summary(&s1, 0);
    assert!(!ready.draft);
    assert_eq!(ready.open_notes, 0);
}

#[test]
fn details_and_split_diff_toggles_and_refresh_github_feedback() {
    let s1 = SessionId::new("se_1");
    let mut state = ReviewState {
        session: Some(s1.clone()),
        ..ReviewState::default()
    };
    let r = reducer();

    assert!(!state.details_open);
    let _ = r.reduce(&mut state, ReviewAction::ToggleDetails);
    assert!(state.details_open);
    let _ = r.reduce(&mut state, ReviewAction::ToggleDetails);
    assert!(!state.details_open);

    assert!(!state.split_diff);
    let _ = r.reduce(&mut state, ReviewAction::ToggleSplitDiff);
    assert!(state.split_diff);

    let _ = r.reduce(&mut state, ReviewAction::RefreshGithubFeedback);
    assert_eq!(
        state.github_checked_ago.get(&s1).map(String::as_str),
        Some("checking…")
    );
}

#[test]
fn github_reply_helpers_and_pr_feedback_card_model() {
    let s1 = SessionId::new("se_1");
    let mut comments = CommentsState::default();
    assert_eq!(queued_github_reply_count(&comments, &s1), 0);
    assert_eq!(post_replies_cta_label(1), "Post 1 reply to GitHub");
    assert_eq!(post_replies_cta_label(3), "Post 3 replies to GitHub");

    let gh_draft = Comment {
        id: CommentId::new("c1"),
        session: s1.clone(),
        anchor: Anchor {
            repo: "acme/api".to_owned(),
            path: "src/lib.rs".to_owned(),
            side: Side::New,
            first: 10,
            last: 10,
            quoted: Vec::new(),
            base: None,
            detached: false,
            outdated: false,
        },
        text: "Fixed in latest commit.".to_owned(),
        status: Status::Draft,
        destination: Destination::PullRequest {
            repo: "acme/api".to_owned(),
            number: 42,
        },
        author: Author::User,
        archived: false,
        unconfirmed: false,
        revision: 1,
        replies: Vec::new(),
        resolved_by: None,
        addressed: None,
        posted_url: None,
        post_unconfirmed: false,
    };
    let mut session_comments = SessionComments::default();
    session_comments.comments.push(gh_draft);
    comments.sessions.insert(s1.clone(), session_comments);
    assert_eq!(queued_github_reply_count(&comments, &s1), 1);

    let card = PrFeedbackCardModel {
        reviewer: "@maya".to_owned(),
        status: PrReviewStatus::RequestedChanges,
        repo: "acme/api".to_owned(),
        pr_number: 142,
        body: "Please handle empty input.".to_owned(),
        location: Some("src/lib.rs:42".to_owned()),
        selected: true,
        queued_reply: Some("Done — added guard.".to_owned()),
    };
    assert_eq!(card.pr_link_label(), "api #142");
    let formatted = card.format_for_agent();
    assert!(formatted.contains("@maya"));
    assert!(formatted.contains("requested changes"));
    assert!(formatted.contains("api #142"));
    assert!(formatted.contains("Please handle empty input."));
}
