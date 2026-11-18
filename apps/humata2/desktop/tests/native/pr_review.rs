//! Reviewing a pull request in the real window, on fakes: the Review a pull
//! request dialog over Home (frame 67; PR-1 to PR-8, D283, pick 11) and
//! Submit review over the review session (frame 69; PR-9, PR-12, PR-13).
//!
//! The captures are recorded by `native_test_record`; `look_test` compares
//! them with the goldens.
use super::*;
use humata2_clients::comments::{Anchor, Author, Side};
use humata2_clients::model::{Agent, MachineId};
use humata2_clients::pull_requests::{
    Draft, PullRequestError, PullRequestInfo, PullRequestRef, PullRequestState, PullRequestsClient,
    ReviewStart, Verdict,
};
use humata2_pr_review::dialog::{DialogAction as D, Fetch};
use humata2_pr_review::submit::{Step, SubmitAction as S};
use std::time::Duration;

const LINK: &str = "https://github.com/acme/api/pull/482";
const REVIEW: &str = "se_pr_1";

fn api() -> PullRequestRef {
    PullRequestRef::new("acme/api", 482)
}

fn ledger() -> PullRequestRef {
    PullRequestRef::new("acme/ledger", 91)
}

fn info(pr: PullRequestRef, title: &str, head: &str, files: u32, added: u32, removed: u32, checks: &str) -> PullRequestInfo {
    PullRequestInfo {
        pr,
        title: title.to_owned(),
        author: "rkhan".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: head.to_owned(),
        head_sha: "abc1234".to_owned(),
        files,
        added,
        removed,
        state: PullRequestState::Open,
        checks: checks.to_owned(),
    }
}

fn github(app: &App) {
    app.fakes.pull_requests.add_pull_request(info(
        api(),
        "Fix refund rounding for partial captures",
        "uhvesta/fix-refund-rounding",
        7,
        214,
        38,
        "11 passed, 1 failed (lint)",
    ));
    app.fakes.pull_requests.add_pull_request(info(
        ledger(),
        "Record the rounded amount",
        "uhvesta/refund-ledger",
        2,
        30,
        4,
        "",
    ));
}

/// Home as frame 67 draws it (the frame lists three machines, so the stopped
/// cloud VM is left out), with Review a pull request open on This Mac.
fn dialog_over_home() -> App {
    let mut workspace = fixture::design();
    workspace
        .machines
        .retain(|machine| machine.id.as_str() != "cloud-vm-3");
    for s in &mut workspace.sessions {
        s.vip = s.id.as_str() == "ledger";
        s.pinned = ["ledger", "pr-482"].contains(&s.id.as_str());
    }
    let mut app = App::new(workspace);
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Home));
    app.fold_quiet_channels();
    github(&app);
    app.send(AppAction::PrDialog(D::Open {
        machine: Some(MachineId::new("this-mac")),
        agent: Agent::Claude,
    }));
    app
}

fn open(app: &App) -> bool {
    app.ui.read(|s| s.pr_dialog.open)
}

fn started(app: &App) -> usize {
    app.fakes.pull_requests.starts().len()
}

fn read(app: &App) -> bool {
    app.ui.read(|s| {
        matches!(s.pr_dialog.main.as_ref().map(|main| &main.fetch), Some(Fetch::Found(_)))
    })
}

/// PR-1: the pasted link is read and its card shows.
#[test]
fn a_pasted_link_shows_its_card() {
    let mut app = dialog_over_home();
    assert!(app.contains("pr-review-dialog"));
    assert!(!app.contains("pr-review-card-acme/api#482"));
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    assert!(read(&app));
    assert!(app.contains("pr-review-card-acme/api#482"));
    assert!(!app.contains("pr-review-fetching-acme/api#482"));
}

/// Pick 11: while the pull request is read the card says so and Start does
/// nothing yet; once it is read, Start opens the review's session.
#[test]
fn start_waits_for_the_pull_request_and_then_opens_the_session() {
    let mut app = dialog_over_home();
    // Sent without settling, so GitHub hasn't answered yet.
    app.cx
        .update(|cx| app.ui.send(AppAction::PrDialog(D::Paste(LINK.to_owned())), cx));
    assert!(app.contains("pr-review-fetching-acme/api#482"));
    app.click("pr-review-start");
    assert_eq!(started(&app), 0);

    assert!(read(&app));
    app.click("pr-review-start");
    assert_eq!(started(&app), 1);
    assert!(!open(&app));
    assert_eq!(
        app.ui.read(|s| s.route.clone()),
        Route::Session(SessionId::new(REVIEW))
    );
}

/// PR-5: Also review adds the second pull request, and Start asks for both,
/// the pasted one first.
#[test]
fn also_review_adds_a_pull_request_and_start_asks_for_both() {
    let mut app = dialog_over_home();
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    app.click("pr-review-add-also");
    assert!(app.contains("pr-review-also-field"));
    app.send(AppAction::PrDialog(D::AddAlso("acme/ledger#91".to_owned())));
    assert!(app.contains("pr-review-also-acme/ledger#91"));
    app.click("pr-review-start");
    let starts = app.fakes.pull_requests.starts();
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].prs, vec![api(), ledger()]);
}

/// Removing a pull request under Also review takes it out of the review.
#[test]
fn a_removed_also_review_pull_request_is_not_started() {
    let mut app = dialog_over_home();
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    app.send(AppAction::PrDialog(D::AddAlso("acme/ledger#91".to_owned())));
    app.click("pr-review-also-remove-acme/ledger#91");
    assert!(!app.contains("pr-review-also-acme/ledger#91"));
    app.click("pr-review-start");
    assert_eq!(app.fakes.pull_requests.starts()[0].prs, vec![api()]);
}

/// PR-4: a pull request under Also review shows as linked to the review; the
/// pasted one doesn't, and taking the other out takes its link off.
#[test]
fn only_an_also_review_pull_request_shows_as_linked() {
    let mut app = dialog_over_home();
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    assert!(!app.contains("pr-review-also-linked-acme/api#482"));
    app.send(AppAction::PrDialog(D::AddAlso("acme/ledger#91".to_owned())));
    assert!(app.contains("pr-review-also-linked-acme/ledger#91"));
    app.click("pr-review-also-remove-acme/ledger#91");
    assert!(!app.contains("pr-review-also-linked-acme/ledger#91"));
}

/// PR-3: a pull request that can't be read says why on its card, Start waits,
/// and Try again reads it once the cause is fixed.
#[test]
fn a_failed_read_blocks_start_until_try_again_reads_it() {
    let mut app = dialog_over_home();
    app.fakes.pull_requests.sign_out(true);
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    assert!(app.contains("pr-review-error-acme/api#482"));
    app.click("pr-review-start");
    assert_eq!(started(&app), 0);

    app.fakes.pull_requests.sign_out(false);
    app.click("pr-review-retry-acme/api#482");
    assert!(!app.contains("pr-review-error-acme/api#482"));
    app.click("pr-review-start");
    assert_eq!(started(&app), 1);
}

/// PR-3: a rate limit is shown on the card too, and still blocks Start.
#[test]
fn a_rate_limited_read_shows_on_the_card() {
    let mut app = dialog_over_home();
    app.fakes.pull_requests.set_rate_limit(Some(Duration::from_secs(90)));
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    assert!(app.contains("pr-review-error-acme/api#482"));
    app.click("pr-review-start");
    assert_eq!(started(&app), 0);
}

/// PR-4: the machine is the one picked in the pop-up.
#[test]
fn the_picked_machine_is_where_the_review_starts() {
    let mut app = dialog_over_home();
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    assert!(!app.contains("pr-review-machine-devbox-1"));
    app.click("pr-review-machine");
    app.click("pr-review-machine-devbox-1");
    assert!(!app.contains("pr-review-machine-devbox-1"));
    app.click("pr-review-start");
    assert_eq!(
        app.fakes.pull_requests.starts()[0].machine,
        MachineId::new("devbox-1")
    );
}

/// PR-8: the agent may push only when the box is ticked.
#[test]
fn pushing_is_off_until_it_is_ticked() {
    let mut app = dialog_over_home();
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    app.click("pr-review-push");
    app.click("pr-review-start");
    assert!(app.fakes.pull_requests.starts()[0].push_allowed);
}

/// Cancel closes the dialog and starts nothing.
#[test]
fn cancel_closes_the_dialog() {
    let mut app = dialog_over_home();
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    app.click("pr-review-cancel");
    assert!(!open(&app));
    assert!(!app.contains("pr-review-dialog"));
    assert_eq!(started(&app), 0);
}

/// Frame 67: the pasted link, its facts, ledger #91 under Also review, This
/// Mac and Claude Code, and what the agent is told first.
#[test]
fn pr_review_dialog_frame_67() {
    let mut app = dialog_over_home();
    app.send(AppAction::PrDialog(D::Paste(LINK.to_owned())));
    app.send(AppAction::PrDialog(D::AddAlso("acme/ledger#91".to_owned())));
    app.send(AppAction::PrDialog(D::SetContext(
        "rounding on partial captures; anything that can double-refund".to_owned(),
    )));
    assert!(app.contains("pr-review-card-acme/api#482"));
    assert!(app.contains("pr-review-also-acme/ledger#91"));
    app.snapshot("67-pr-review-dialog");
}

/// The review session of frame 68 behind Submit review: the fixture's PR #482
/// session, under the id the fake daemon gives a started review.
fn review_session() -> App {
    let mut app = unopened_review();
    app.open(REVIEW, Panel::Changes);
    app
}

/// `review_session()` before it is opened.
fn unopened_review() -> App {
    // The review of frame 68: its clones, pull requests and diffs show in
    // the Changes panel behind the dialogs.
    let mut app = App::new(super::pr_session::review_workspace(Vec::new()));
    app.cx.update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.fold_quiet_channels();
    github(&app);
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

/// A draft on `pr`, as the ledger holds it and the plan shows it.
fn draft(app: &App, pr: &PullRequestRef, file: &str, line: u32, text: &str, author: Author) -> String {
    draft_at(app, pr, file, line, text, author, false)
}

/// A draft the plan left out: its lines changed since it was written.
fn left_out_draft(app: &App, pr: &PullRequestRef, file: &str, line: u32, text: &str) -> String {
    draft_at(app, pr, file, line, text, Author::Agent("claude".to_owned()), true)
}

fn draft_at(
    app: &App,
    pr: &PullRequestRef,
    file: &str,
    line: u32,
    text: &str,
    author: Author,
    outdated: bool,
) -> String {
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
        outdated,
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

struct Drafts {
    round: String,
    retry: String,
    test: String,
    ledger: String,
}

/// Frame 69's drafts, and the agent's summary for #482.
fn drafts(app: &mut App) -> Drafts {
    let claude = || Author::Agent("claude".to_owned());
    let drafts = Drafts {
        round: draft(app, &api(), "refund.rs", 140, "round() can refund a cent more than was captured.", claude()),
        retry: draft(app, &api(), "refund.rs", 212, "This retry can double-refund; key it on the capture id.", claude()),
        test: draft(app, &api(), "refund_test.rs", 58, "Add a case for three-way splits.", Author::User),
        ledger: draft(app, &ledger(), "ledger.rs", 33, "Record the rounded amount, not the requested one.", claude()),
    };
    let summary = app.runtime.block_on(app.fakes.pull_requests.draft_summary(
        &SessionId::new(REVIEW),
        &api(),
        "Rounding on partial captures can over-refund by a cent, and the retry path can refund twice. Both need fixing before this merges.",
    ));
    assert!(summary.is_ok());
    drafts
}

fn submit_open(app: &mut App) {
    app.send(AppAction::PrSubmit(S::Open(SessionId::new(REVIEW))));
}

fn verdict_of(app: &App, pr: &PullRequestRef) -> Option<Verdict> {
    app.ui.read(|s| {
        s.pr_submit
            .reviews
            .iter()
            .find(|review| &review.pr == pr)
            .map(|review| review.verdict)
    })
}

/// PR-12: each pull request lists its own drafts, and a verdict is chosen
/// for each on its own.
#[test]
fn drafts_are_listed_per_pull_request_with_a_verdict_each() {
    let mut app = review_session();
    let ids = drafts(&mut app);
    submit_open(&mut app);
    assert!(app.contains("submit-pr-acme/api#482"));
    assert!(app.contains("submit-pr-acme/ledger#91"));
    for id in [&ids.round, &ids.retry, &ids.test, &ids.ledger] {
        assert!(app.contains(&format!("submit-draft-{id}")), "{id} is listed");
    }
    assert_eq!(verdict_of(&app, &api()), Some(Verdict::Comment));
    app.click("submit-verdict-acme/api#482-request-changes");
    assert_eq!(verdict_of(&app, &api()), Some(Verdict::RequestChanges));
    assert_eq!(verdict_of(&app, &ledger()), Some(Verdict::Comment));
}

/// PR-12: the agent's summary is in its box; a pull request with none can
/// have one added.
#[test]
fn the_agents_summary_shows_and_another_can_be_added() {
    let mut app = review_session();
    drafts(&mut app);
    submit_open(&mut app);
    assert!(app.contains("submit-summary-acme/api#482"));
    assert!(!app.contains("submit-summary-acme/ledger#91"));
    app.click("submit-add-summary-acme/ledger#91");
    assert!(app.contains("submit-summary-acme/ledger#91"));
}

/// PR-12, PR-14: Submit posts one review per pull request, with the verdict
/// chosen for it, and the dialog closes.
#[test]
fn submit_posts_one_review_per_pull_request() {
    let mut app = review_session();
    drafts(&mut app);
    submit_open(&mut app);
    app.click("submit-verdict-acme/api#482-request-changes");
    app.click("submit-review-submit");
    let posted = app.fakes.pull_requests.posted();
    assert_eq!(posted.len(), 2);
    let verdicts: Vec<_> = posted
        .iter()
        .map(|(pr, review)| (pr.clone(), review.verdict, review.drafts.len()))
        .collect();
    assert!(verdicts.contains(&(api(), Verdict::RequestChanges, 3)));
    assert!(verdicts.contains(&(ledger(), Verdict::Comment, 1)));
    assert_eq!(app.ui.read(|s| s.pr_submit.step), Step::Closed);
    assert!(!app.contains("submit-review-dialog"));
}

/// PR-13: a pull request that fails to post shows its error on its own card
/// and keeps its drafts; the other one posts.
#[test]
fn a_failed_post_shows_its_error_on_its_card_and_keeps_its_drafts() {
    let mut app = review_session();
    let ids = drafts(&mut app);
    app.fakes.pull_requests.fail_post(
        &ledger(),
        PullRequestError::NotVisible {
            repo: "acme/ledger".to_owned(),
        },
    );
    submit_open(&mut app);
    app.click("submit-review-submit");
    let posted = app.fakes.pull_requests.posted();
    assert_eq!(posted.len(), 1);
    assert_eq!(posted[0].0, api());
    assert!(app.contains("submit-review-dialog"));
    assert!(app.contains("submit-error-acme/ledger#91"));
    assert!(!app.contains("submit-error-acme/api#482"));
    assert!(app.contains(&format!("submit-draft-{}", ids.ledger)));
}

/// PR-9: Drop takes a draft out of the review.
#[test]
fn dropping_a_draft_takes_it_off_the_list() {
    let mut app = review_session();
    let ids = drafts(&mut app);
    submit_open(&mut app);
    app.click(&format!("submit-draft-drop-{}", ids.test));
    assert!(!app.contains(&format!("submit-draft-{}", ids.test)));
    assert!(app.contains(&format!("submit-draft-{}", ids.round)));
}

/// PR-16: a draft whose lines changed is left out and offers Re-attach beside
/// Drop. Re-attach keeps its lines, now current, and the plan takes it back.
#[test]
fn a_left_out_draft_re_attaches_into_the_plan() {
    let mut app = review_session();
    drafts(&mut app);
    let old = left_out_draft(&app, &api(), "refund.rs", 90, "About lines that changed");
    submit_open(&mut app);
    assert!(left_out_by_plan(&app, &api(), &old));
    assert!(!drafted_by_plan(&app, &api(), &old));
    app.click(&format!("submit-draft-reattach-{old}"));
    assert!(!left_out_by_plan(&app, &api(), &old));
    assert!(drafted_by_plan(&app, &api(), &old));
    let held = app
        .fakes
        .comments
        .comments(&SessionId::new(REVIEW))
        .into_iter()
        .find(|comment| comment.id.to_string() == old);
    assert!(held.is_some_and(|comment| !comment.anchor.outdated));
}

fn drafted_by_plan(app: &App, pr: &PullRequestRef, id: &str) -> bool {
    app.ui.read(|s| {
        s.pr_submit
            .reviews
            .iter()
            .find(|review| &review.pr == pr)
            .is_some_and(|review| review.drafts.iter().any(|draft| draft.id.to_string() == id))
    })
}

fn left_out_by_plan(app: &App, pr: &PullRequestRef, id: &str) -> bool {
    app.ui.read(|s| {
        s.pr_submit
            .reviews
            .iter()
            .find(|review| &review.pr == pr)
            .is_some_and(|review| review.left_out.iter().any(|left| left.draft.id.to_string() == id))
    })
}

/// PR-9: Edit turns a draft into a field and Save puts the text back.
#[test]
fn edit_opens_the_draft_in_a_field_and_save_closes_it() {
    let mut app = review_session();
    let ids = drafts(&mut app);
    submit_open(&mut app);
    assert!(!app.contains(&format!("submit-draft-field-{}", ids.round)));
    app.click(&format!("submit-draft-edit-{}", ids.round));
    assert!(app.contains(&format!("submit-draft-field-{}", ids.round)));
    app.click(&format!("submit-draft-save-{}", ids.round));
    assert!(!app.contains(&format!("submit-draft-field-{}", ids.round)));
    assert!(app.contains(&format!("submit-draft-{}", ids.round)));
}

/// A plan that can't be made says why, and nothing can be submitted.
#[test]
fn a_plan_that_cannot_be_made_says_why_and_submits_nothing() {
    let mut app = review_session();
    drafts(&mut app);
    app.fakes.pull_requests.sign_out(true);
    submit_open(&mut app);
    assert!(app.contains("submit-review-error"));
    app.click("submit-review-submit");
    assert!(app.fakes.pull_requests.posted().is_empty());
}

/// Cancel closes Submit review without posting.
#[test]
fn cancel_closes_submit_review_without_posting() {
    let mut app = review_session();
    drafts(&mut app);
    submit_open(&mut app);
    app.click("submit-review-cancel");
    assert_eq!(app.ui.read(|s| s.pr_submit.step), Step::Closed);
    assert!(!app.contains("submit-review-dialog"));
    assert!(app.fakes.pull_requests.posted().is_empty());
}

/// Frame 69: three drafts on #482 (Request changes), one on #91 (Comment),
/// and the agent's summary.
#[test]
fn pr_submit_review_frame_69() {
    let mut app = unopened_review();
    // Two commits landed on #482 since the clone: the notice under the
    // terminal offers Update the clone (frame 69). They land before the
    // terminal is drawn, so the agent's input box sits above the notice.
    super::pr_session::moved(&mut app, api(), 2, PullRequestState::Open);
    app.open(REVIEW, Panel::Changes);
    app.send(AppAction::Review(humata2_review::ReviewAction::SelectFile {
        repo: "acme/api".to_owned(),
        path: "src/refund.rs".to_owned(),
    }));
    drafts(&mut app);
    submit_open(&mut app);
    app.click("submit-verdict-acme/api#482-request-changes");
    assert!(app.contains("submit-review-dialog"));
    app.snapshot("69-pr-submit-review");
}

#[test]
fn a_review_session_opens_submit_review_from_its_header() {
    let mut app = review_session();
    assert_eq!(app.ui.read(|s| s.pr_submit.step), Step::Closed);
    app.click("submit-review");
    assert_ne!(app.ui.read(|s| s.pr_submit.step), Step::Closed);
}

#[test]
fn a_session_that_reviews_no_pull_request_has_no_submit_review() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    assert!(!app.contains("submit-review"));
}
