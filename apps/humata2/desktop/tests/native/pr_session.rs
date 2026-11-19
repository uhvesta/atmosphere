//! The pull request review session in the real window, on fakes (frame 68;
//! PR-6 to PR-11, PR-15 to PR-17): the Diff panel names the pull requests
//! and counts the drafts, a draft for GitHub has Edit and Drop, and the
//! notices under the terminal offer Update the clone and Archive.
//!
//! The capture is recorded by `native_test_record`; `look_test` compares it
//! with the golden.
use super::*;
use humata2_clients::comments::{Anchor, Author, Side};
use humata2_clients::model::{
    Agent, Comment, DiffLine, FileDiff, Line, LineKind, MachineId, PullRequest, RepoDiff,
    SessionClone, Speaker,
};
use humata2_clients::pull_requests::{
    Call, Draft, HeadChange, HeadsChanged, PullRequestInfo, PullRequestRef, PullRequestState,
    PullRequestsClient, ReviewStart,
};
use humata2_clients::review_chat::{
    LineSpan, Place, ReviewChange, ReviewChangeKind,
};
use humata2_pr_review::follower::FollowerAction;
use humata2_pr_review::heads::HeadsAction;
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;

const REVIEW: &str = "se_pr_1";

fn api() -> PullRequestRef {
    PullRequestRef::new("acme/api", 482)
}

fn ledger() -> PullRequestRef {
    PullRequestRef::new("acme/ledger", 91)
}

fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

fn info(pr: PullRequestRef, head: &str) -> PullRequestInfo {
    PullRequestInfo {
        pr,
        title: "Fix refund rounding".to_owned(),
        author: "rkhan".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: head.to_owned(),
        head_sha: "abc1234".to_owned(),
        files: 1,
        added: 1,
        removed: 1,
        state: PullRequestState::Open,
        checks: String::new(),
    }
}

/// The review of frame 68: acme/api #482 and acme/ledger #91 on This Mac,
/// the first one's `refund.rs` in the Diff panel.
fn review() -> App {
    review_with(Vec::new())
}

/// `review()`, with more changed files in acme/api after `refund.rs`.
fn review_with(more: Vec<FileDiff>) -> App {
    let mut app = App::new(review_workspace(more));
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.fold_quiet_channels();
    app.fakes.pull_requests.add_pull_request(info(api(), "uhvesta/fix-refund-rounding"));
    app.fakes.pull_requests.add_pull_request(info(ledger(), "uhvesta/refund-ledger"));
    let begun = app.runtime.block_on(app.fakes.pull_requests.start_review(ReviewStart {
        prs: vec![api(), ledger()],
        machine: MachineId::new("this-mac"),
        agent: Agent::Claude,
        model: String::new(),
        first_message: String::new(),
        push_allowed: false,
    }));
    assert_eq!(begun.map(|id| id.to_string()).ok().as_deref(), Some(REVIEW));
    app
}

/// The design workspace with pr-482 as the review of frame 68: its two
/// clones, its two pull requests and their diffs (`more` after `refund.rs`).
pub(super) fn review_workspace(more: Vec<FileDiff>) -> Workspace {
    let mut workspace = fixture::design();
    workspace
        .machines
        .retain(|machine| machine.id.as_str() != "cloud-vm-3");
    for s in &mut workspace.sessions {
        if s.id.as_str() == "pr-482" {
            s.id = SessionId::new(REVIEW);
            s.status = Status::Working;
            s.pull_request_review = true;
            s.clones = vec![
                SessionClone {
                    repo: "acme/api".to_owned(),
                    branch: "uhvesta/fix-refund-rounding".to_owned(),
                    path: "/work/api".to_owned(),
                },
                SessionClone {
                    repo: "acme/ledger".to_owned(),
                    branch: "uhvesta/refund-ledger".to_owned(),
                    path: "/work/ledger".to_owned(),
                },
            ];
            // The review chat of frame 68: the person asks, the agent
            // focuses the line and notes it there.
            s.transcript = [
                (Speaker::System, "~/src/api (fix-refund-rounding) $ claude"),
                (Speaker::Agent, ""),
                (Speaker::You, "> why round() and not floor()?"),
                (Speaker::Agent, "● review_focus refund.rs:140"),
                (Speaker::System, "  It rounds half away from zero …"),
                (Speaker::Agent, ""),
                (Speaker::You, "> note that on the line for me"),
                (Speaker::Agent, "● review_note refund.rs:140"),
            ]
            .into_iter()
            .map(|(from, text)| Line { from, text: text.to_owned() })
            .collect();
        }
    }
    let session = SessionId::new(REVIEW);
    // The review asks one question of its own (the rail's Inbox 1, frame 68).
    for request in &mut workspace.requests {
        if request.id.as_str() == "review-question" {
            request.session = session.clone();
        }
    }
    workspace.pulls = vec![
        PullRequest {
            number: 482,
            title: "Fix refund rounding".to_owned(),
            repo: "acme/api".to_owned(),
            session: Some(session.clone()),
            checks: String::new(),
        },
        PullRequest {
            number: 91,
            title: "Record the rounded amount".to_owned(),
            repo: "acme/ledger".to_owned(),
            session: Some(session.clone()),
            checks: String::new(),
        },
    ];
    workspace.diffs.extend([
        RepoDiff {
            session: session.clone(),
            repo: "acme/api".to_owned(),
            base: "main".to_owned(),
            files: [FileDiff {
                path: "src/refund.rs".to_owned(),
                added: 62,
                removed: 21,
                lines: vec![
                    line(LineKind::Context, Some(138), Some(138), "    let amount = capture.remaining();"),
                    line(LineKind::Removed, Some(139), None, "    let cents = amount.floor();"),
                    line(LineKind::Added, None, Some(140), "    let cents = amount.round();"),
                    line(LineKind::Context, Some(141), Some(141), "    refund.issue(cents)?;"),
                ],
                ..FileDiff::default()
            }]
            .into_iter()
            .chain(more)
            .collect(),
            comments: Vec::new(),
        },
        RepoDiff {
            session,
            repo: "acme/ledger".to_owned(),
            base: "main".to_owned(),
            files: vec![FileDiff {
                path: "src/ledger.rs".to_owned(),
                added: 4,
                removed: 0,
                lines: Vec::new(),
                ..FileDiff::default()
            }],
            // Two comments already sent to the agent on acme/ledger (the
            // rail's Diff 2, frame 68); that repo is folded in the frame.
            comments: [33, 41]
                .into_iter()
                .map(|line| Comment {
                    path: "src/ledger.rs".to_owned(),
                    line,
                    body: "Keep the requested amount beside the rounded one.".to_owned(),
                    sent: true,
                    context: None,
                })
                .collect(),
        },
    ]);
    workspace
}

/// A draft for GitHub on `pr`, as the daemon holds it.
fn draft(app: &App, pr: &PullRequestRef, file: &str, line: u32, text: &str, author: Author) -> String {
    let session = SessionId::new(REVIEW);
    let at = Anchor {
        repo: pr.repo.clone(),
        path: format!("src/{file}"),
        side: Side::New,
        first: line,
        last: line,
        quoted: Vec::new(),
        base: None,
        detached: false,
        outdated: false,
    };
    let comment = app.fakes.comments.seed_pull_request_draft(
        &session,
        at.clone(),
        text,
        author.clone(),
        &pr.repo,
        pr.number,
    );
    let id = comment.id.to_string();
    app.fakes.pull_requests.add_draft(
        &session,
        pr,
        Draft {
            id: comment.id,
            anchor: at,
            text: text.to_owned(),
            author,
            revision: comment.revision,
        },
    );
    id
}

/// Frame 68's drafts: three by the agent, one by the person. Returns the id
/// of the one under line 140.
fn drafts(app: &App) -> String {
    let claude = || Author::Agent("claude".to_owned());
    let round = draft(
        app,
        &api(),
        "refund.rs",
        140,
        "round() is half-away-from-zero, so a split capture can refund a cent more than was taken. Use floor() on each part.",
        claude(),
    );
    draft(app, &api(), "refund.rs", 212, "This retry can double-refund.", claude());
    draft(app, &api(), "refund_test.rs", 58, "Add a case for three-way splits.", Author::User);
    draft(app, &ledger(), "ledger.rs", 33, "Record the rounded amount.", claude());
    round
}

/// The review open on its Diff, `refund.rs` picked.
fn shown(app: &mut App) {
    app.open(REVIEW, Panel::Changes);
    app.send(AppAction::Review(ReviewAction::SelectFile {
        repo: "acme/api".to_owned(),
        path: "src/refund.rs".to_owned(),
    }));
}

pub(super) fn moved(app: &mut App, pr: PullRequestRef, new_commits: u32, state: PullRequestState) {
    pushed(app, pr, new_commits, false, state);
}

fn pushed(app: &mut App, pr: PullRequestRef, new_commits: u32, force_pushed: bool, state: PullRequestState) {
    app.send(AppAction::PrHeads(HeadsAction::Pushed(HeadsChanged {
        session: SessionId::new(REVIEW),
        changes: vec![HeadChange {
            pr,
            new_commits,
            force_pushed,
            state,
            base_branch: None,
            stack: None,
        }],
    })));
}

/// PR-6: the Diff panel names the pull request and one group per repository.
#[test]
fn the_diff_names_the_pull_request_and_each_repository() {
    let mut app = review();
    shown(&mut app);
    assert!(app.contains("review-pr-line"));
    assert!(app.contains("review-repo-acme/api"));
    assert!(app.contains("review-repo-acme/ledger"));
}

/// A session that reviews no pull request has none of it.
#[test]
fn a_plain_session_has_no_review_lines() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    assert!(!app.contains("review-pr-line"));
    assert!(!app.contains("pr-new-commits"));
}

/// PR-9: a draft for GitHub shows Edit and Drop; the footer counts the
/// drafts by who wrote them.
#[test]
fn a_draft_for_github_has_edit_and_drop_and_the_footer_counts_the_drafts() {
    let mut app = review();
    let round = drafts(&app);
    shown(&mut app);
    assert!(app.contains(&format!("doc-comment-{}", round)));
    assert!(app.contains(&format!("doc-edit-comment-{}", round)));
    assert!(app.contains(&format!("doc-drop-comment-{}", round)));
    assert!(app.contains("review-drafts-3-1"));
}

/// Submit review… in the Diff footer opens the submit sheet over the review.
#[test]
fn submit_review_in_the_footer_opens_the_sheet() {
    let mut app = review();
    drafts(&app);
    shown(&mut app);
    app.click("diff-submit-review");
    assert!(app.contains("submit-review-dialog"));
}

/// UXSIMPLE: a review with only drafts for GitHub has nothing to send to the
/// agent, so its footer has no Send button.
#[test]
fn a_review_footer_has_no_send_button_when_nothing_is_sendable() {
    let mut app = review();
    drafts(&app);
    shown(&mut app);
    app.draw();
    assert!(!app.contains("send-comments"));
    assert!(app.contains("diff-submit-review"));
}

/// UXSIMPLE: with a comment waiting for the agent, a review's Send is a
/// secondary button left of the filled Submit review….
#[test]
fn a_review_footer_offers_send_as_a_secondary_left_of_submit() {
    let mut app = review();
    shown(&mut app);
    app.write_comment(REVIEW, "acme/api", 140, "Check the rounding here.");
    app.draw();
    assert!(app.contains("send-comments"));
    let line = app.send_button(REVIEW).expect("button names the agent");
    assert!(!line.contains("disabled"), "{line}");
    app.cx
        .update_window(app.window, |_, window, _| {
            let send = window.find("send-comments").bounds();
            let submit = window.find("diff-submit-review").bounds();
            assert!(send.origin.x + send.size.width <= submit.origin.x);
        })
        .unwrap();
}

/// With the Diff panel open, the session bar's Submit review is hidden while
/// the footer's stays; closing the panel brings the bar's back.
#[test]
fn the_session_bar_hides_submit_review_while_the_diff_shows() {
    let mut app = review();
    shown(&mut app);
    assert!(app.contains("diff-submit-review"));
    assert!(!app.contains("submit-review"));
    app.send(AppAction::Session(SessionAction::SetPanel(None)));
    assert!(app.contains("submit-review"));
}

/// Drop takes the draft off its line and out of the footer's count.
#[test]
fn drop_removes_the_draft_and_the_footer_follows() {
    let mut app = review();
    let round = drafts(&app);
    shown(&mut app);
    app.click(&format!("doc-drop-comment-{}", round));
    assert!(!app.contains(&format!("doc-comment-{}", round)));
    assert!(app.contains("review-drafts-2-1"));
    let left = app
        .ui
        .read(|s| s.comments.github_drafts(&SessionId::new(REVIEW)).len());
    assert_eq!(left, 3);
}

/// Edit opens the draft in the comment box; closing it keeps the draft.
#[test]
fn edit_opens_the_draft_in_the_box_and_closing_keeps_it() {
    let mut app = review();
    let round = drafts(&app);
    shown(&mut app);
    assert!(!app.contains("doc-comment-box"));
    app.click(&format!("doc-edit-comment-{}", round));
    assert!(app.contains("doc-comment-box"));
    // It is already a draft for GitHub, so the box doesn't offer that again.
    assert!(!app.contains("doc-comment-github"));
    app.send(AppAction::Comments(CommentsAction::Close(SessionId::new(REVIEW))));
    assert!(!app.contains("doc-comment-box"));
    assert!(app.contains("review-drafts-3-1"));
}

/// PR-15: new commits show under the terminal; Update the clone asks the
/// daemon to update it, and the notice goes.
#[test]
fn update_the_clone_sends_the_update_and_clears_the_notice() {
    let mut app = review();
    shown(&mut app);
    assert!(!app.contains("pr-new-commits"));
    moved(&mut app, api(), 2, PullRequestState::Open);
    assert!(app.contains("pr-new-commits"));
    app.click("pr-update-clone");
    assert!(app
        .fakes
        .pull_requests
        .calls()
        .contains(&Call::UpdateClone(SessionId::new(REVIEW))));
    assert!(!app.contains("pr-new-commits"));
}

/// Case 12: a force-push that also adds commits says both, not just the
/// count, and a force-push alone still reads as one.
#[test]
fn a_force_push_with_new_commits_names_both() {
    let notice = |commits: u32, force: bool| {
        let mut app = review();
        shown(&mut app);
        pushed(&mut app, api(), commits, force, PullRequestState::Open);
        app.draw();
        app.cx
            .update_window(app.window, |_, window, _| {
                window.find("pr-new-commits").label().map(str::to_owned)
            })
            .expect("window stays open")
            .expect("the notice has its words")
    };
    let added = notice(2, false);
    let rewritten = notice(0, true);
    let both = notice(2, true);
    assert!(added.contains('2'), "{added}");
    assert!(both.contains('2'), "{both}");
    assert_ne!(both, added);
    assert_ne!(both, rewritten);
}

/// PR-17: a merged pull request offers Archive, not Update the clone.
#[test]
fn a_merged_pull_request_offers_archive() {
    let mut app = review();
    shown(&mut app);
    moved(&mut app, api(), 0, PullRequestState::Merged);
    assert!(app.contains("pr-ended"));
    assert!(app.contains("pr-archive"));
    assert!(!app.contains("pr-new-commits"));
}

/// Frame 68: the review session with the agent's draft under line 140, two
/// new commits, and four drafts.
#[test]
fn pr_review_session_frame_68() {
    let mut app = review();
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    drafts(&app);
    // The commits land before the terminal is drawn, so the agent's input
    // box sits above the notice rather than under it.
    moved(&mut app, api(), 2, PullRequestState::Open);
    shown(&mut app);
    assert!(app.contains("pr-new-commits"));
    assert!(app.contains("review-drafts-3-1"));
    // The agent has focused line 140, as the frame's accent shows.
    agent_focuses(&mut app, 1, "src/refund.rs", 140);
    assert!(app.contains("review-focus-140"));
    // The drafts line gives way, so Submit review… keeps the footer's padding.
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("side-panel").bounds();
            let submit = window.find("diff-submit-review").bounds();
            let right = panel.origin.x + panel.size.width;
            assert!(
                submit.origin.x + submit.size.width <= right - gpui_kit::px(8.),
                "Submit review… ends inside the footer's padding"
            );
        })
        .unwrap();
    app.snapshot("68-pr-review-session");
}

fn file(path: &str, lines: Vec<DiffLine>) -> FileDiff {
    FileDiff {
        path: path.to_owned(),
        added: 1,
        removed: 0,
        lines,
        ..FileDiff::default()
    }
}

/// The agent focuses `path:line`, as the follower hears it.
fn agent_focuses(app: &mut App, seq: u64, path: &str, line: u32) {
    app.send(AppAction::PrFollower(FollowerAction::Changed(ReviewChange {
        undo_id: format!("u-{seq}"),
        seq,
        tool: "review_focus".to_owned(),
        summary: format!("{path}:{line}"),
        change: Some(ReviewChangeKind::Focus(Place {
            repo: "acme/api".to_owned(),
            path: path.to_owned(),
            visible: Some(LineSpan { first: line, last: line }),
            selection: None,
            side: Side::New,
        })),
        highlights: Vec::new(),
        undid: String::new(),
        at_unix_ms: 0,
    })));
}

/// PR-22: the line the agent focuses is marked in the Diff; the others are not.
#[test]
fn the_line_the_agent_focuses_is_marked() {
    let mut app = review();
    shown(&mut app);
    assert!(!app.contains("review-focus-140"));
    agent_focuses(&mut app, 1, "src/refund.rs", 140);
    assert!(app.contains("review-focus-140"));
    assert!(!app.contains("review-focus-139"));
    // A later focus moves the mark.
    agent_focuses(&mut app, 2, "src/refund.rs", 141);
    assert!(app.contains("review-focus-141"));
    assert!(!app.contains("review-focus-140"));
}

/// The Diff scrolls so a focus below the fold is in view.
#[test]
fn a_focus_below_the_fold_is_scrolled_into_view() {
    let long: Vec<DiffLine> = (1..=120)
        .map(|n| line(LineKind::Context, Some(n), Some(n), "    let step = next(step);"))
        .collect();
    let mut app = review_with(vec![file("src/long.rs", long)]);
    shown(&mut app);
    agent_focuses(&mut app, 1, "src/long.rs", 110);
    assert!(app.contains("review-focus-110"));
    for _ in 0..3 {
        app.draw();
    }
    app.cx
        .update_window(app.window, |_, window, _| {
            let view = window.find("review-diff").bounds();
            let focus = window.find("review-focus-110").bounds();
            assert!(focus.top() >= view.top() && focus.bottom() <= view.bottom());
        })
        .expect("window stays open");
}

/// §26.17 step 4: the files read bottom-up, a file after those it uses.
#[test]
fn files_are_listed_bottom_up() {
    let handler = file(
        "src/handler.rs",
        vec![line(LineKind::Added, None, Some(1), "    refund::issue(cents)?;")],
    );
    let mut app = review_with(vec![handler]);
    shown(&mut app);
    assert!(app.contains("change-file acme/api/src/handler.rs"));
    app.cx
        .update_window(app.window, |_, window, _| {
            let used = window.find("change-file acme/api/src/refund.rs").bounds();
            let user = window.find("change-file acme/api/src/handler.rs").bounds();
            assert!(used.origin.y < user.origin.y);
        })
        .expect("window stays open");
}

/// The review panel at its narrowest still draws Full screen and Close
/// inside its edge; the PR line ellipsizes instead.
#[test]
fn a_narrow_review_panel_keeps_full_screen_and_close() {
    let mut app = review();
    shown(&mut app);
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("side-panel").bounds();
            let right = panel.origin.x + panel.size.width;
            for id in ["expand-panel", "close-panel"] {
                let button = window.find(id).bounds();
                assert!(button.size.width > px(0.), "{id} is drawn");
                assert!(button.origin.x + button.size.width <= right + px(0.5), "{id} fits");
            }
        })
        .unwrap();
}

/// Full screen, then Show the terminal: the review goes back beside the
/// terminal with the same file open.
#[test]
fn show_the_terminal_returns_the_review_beside_the_terminal() {
    let mut app = review();
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    drafts(&app);
    shown(&mut app);
    agent_focuses(&mut app, 1, "src/refund.rs", 140);
    app.click("expand-panel");
    assert!(app.contains("show-terminal"));
    app.snapshot("68b-pr-review-full-screen");
    app.click("show-terminal");
    assert!(!app.contains("show-terminal"));
    assert!(app.contains("expand-panel"));
    let picked = app.ui.read(|state| {
        state.review.saved.selected.get(&SessionId::new(REVIEW)).cloned()
    });
    assert_eq!(
        picked,
        Some(("acme/api".to_owned(), "src/refund.rs".to_owned()))
    );
    app.snapshot("68c-pr-review-after-show-terminal");
}

/// Scrolls the review's list by `(dx, dy)` pixels (negative scrolls down or
/// to the right).
fn scroll_review(app: &mut App, dx: f32, dy: f32) {
    scroll_over(app, "review-diff", dx, dy);
}

fn scroll_over(app: &mut App, id: &'static str, dx: f32, dy: f32) {
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.scroll(id, gpui_kit::ScrollDelta::Pixels(point(px(dx), px(dy))), cx);
        })
        .expect("window stays open");
    app.draw();
    app.draw();
}

/// DS-26: the open file's head stays at the top of the review while its
/// lines scroll.
#[test]
fn a_review_file_head_stays_while_its_lines_scroll() {
    let long: Vec<DiffLine> = (1..=200)
        .map(|n| line(LineKind::Context, Some(n), Some(n), "    let step = next(step);"))
        .collect();
    let mut app = review_with(vec![file("src/long.rs", long)]);
    app.open(REVIEW, Panel::Changes);
    app.send(AppAction::Review(ReviewAction::SelectFile {
        repo: "acme/api".to_owned(),
        path: "src/long.rs".to_owned(),
    }));
    app.draw();
    app.draw();
    let head = "change-file acme/api/src/long.rs";
    let top_of = |app: &mut App| {
        app.cx
            .update_window(app.window, |_, window, _| {
                let view = window.find("review-diff").bounds();
                let head = window.find(head).bounds();
                (f32::from(head.top() - view.top()), view.top() <= head.top() && head.bottom() <= view.bottom())
            })
            .expect("window stays open")
    };
    let (before, _) = top_of(&mut app);
    scroll_review(&mut app, 0., -1500.);
    let (after, inside) = top_of(&mut app);
    assert!(inside, "the head is still inside the review's view");
    assert!(after <= before + 1., "the head stays at the top of the view");
    assert!(after >= 0.);
    // Scrolled a long way down, the head stays at the same place under the view.
    scroll_review(&mut app, 0., -300.);
    let (later, inside) = top_of(&mut app);
    assert!(inside);
    assert!((later - after).abs() < 1.5, "the head does not move as the lines do");
}

/// DS-42: with Wrap off, a review's numbers stay put and only the code slides.
#[test]
fn scroll_in_a_review_keeps_the_gutter_still() {
    let wide = format!("    let total = {};", "amount + ".repeat(60));
    let lines = vec![
        line(LineKind::Context, Some(1), Some(1), &wide),
        line(LineKind::Context, Some(2), Some(2), "    done();"),
    ];
    let mut app = review_with(vec![file("src/wide.rs", lines)]);
    app.open(REVIEW, Panel::Changes);
    app.send(AppAction::Review(ReviewAction::SelectFile {
        repo: "acme/api".to_owned(),
        path: "src/wide.rs".to_owned(),
    }));
    if app.ui.read(|state| state.review.wrap) {
        app.send(AppAction::Review(ReviewAction::ToggleWrap));
    }
    app.draw();
    app.draw();
    let x_of = |app: &mut App| {
        app.cx
            .update_window(app.window, |_, window, _| {
                (
                    f32::from(window.find("change-gutter-0").bounds().left()),
                    f32::from(window.find("change-code-0-0").bounds().left()),
                )
            })
            .expect("window stays open")
    };
    let (gutter, code) = x_of(&mut app);
    scroll_over(&mut app, "change-code acme/api/src/wide.rs", -120., 0.);
    let (gutter_after, code_after) = x_of(&mut app);
    assert!((gutter_after - gutter).abs() < 0.5, "the gutter keeps its x");
    assert!(code_after < code - 50., "the code slides left");
}
