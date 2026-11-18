//! The comment box and cards of a pull request review in the real window, on
//! fakes (PR-11, PR-19, PR-23): the box offers Ask and Draft for GitHub, a
//! note offers Make it a draft, and a question that can't reach the review
//! chat says so under it until dismissed.
use super::*;
use humata2_clients::comments::{Anchor, Author, Destination, Side};
use humata2_clients::model::PullRequest;
use humata2_clients::{DaemonError, Operation};
use humata2_review::comments::CommentsAction;

/// "ledger" reviews acme/api #482 ("refund" reviews too); its Diff is open
/// on api/src/client.rs.
/// Lines by index: 0 the hunk, 1 context (40), 2 removed (old 41), 3–6
/// added (new 41–44).
fn reviewing() -> App {
    let mut workspace = fixture::design();
    let id = SessionId::new("ledger");
    for each in &mut workspace.sessions {
        // "refund" reviews too, so a failure can be seen to stay put.
        if each.id == id || each.id.as_str() == "refund" {
            each.pull_request_review = true;
        }
    }
    workspace.pulls.retain(|pull| pull.session.as_ref() != Some(&id));
    workspace.pulls.push(PullRequest {
        number: 482,
        title: "Fix refund rounding".to_owned(),
        repo: "acme/api".to_owned(),
        session: Some(id.clone()),
        checks: String::new(),
    });
    let app = App::new(workspace);
    app.fakes
        .comments
        .set_review_pull_requests(&id, vec![("acme/api".to_owned(), 482)]);
    app
}

fn shown(app: &mut App) {
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(humata2_review::ReviewAction::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
}

/// The box under new line 42, with `text` in it.
fn boxed(app: &mut App, text: &str) {
    app.click("change-gutter-4");
    app.send(AppAction::Comments(CommentsAction::Typed(
        SessionId::new("ledger"),
        text.to_owned(),
    )));
}

/// Points at `id`, so the Ask its row shows on hover can be clicked.
fn hover(app: &mut App, id: &str) {
    app.draw();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.hover(gpui_kit::SharedString::from(id.to_owned()), cx);
        })
        .expect("window stays open");
    app.settle();
}

fn sends(app: &App) -> Vec<String> {
    app.fakes
        .daemon
        .calls()
        .into_iter()
        .filter(|call| call.starts_with("send ledger"))
        .collect()
}

/// PR-11, PR-19: a review's box offers Ask and Draft for GitHub; a plain
/// session's box offers neither.
#[test]
fn a_review_box_offers_ask_and_draft_for_github() {
    let mut app = reviewing();
    shown(&mut app);
    boxed(&mut app, "Why round here?");
    assert!(app.contains("doc-comment-ask"));
    assert!(app.contains("doc-comment-github"));

    let mut plain = App::new(fixture::design());
    plain.open("ledger", Panel::Changes);
    plain.send(AppAction::Review(humata2_review::ReviewAction::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    plain.click("change-gutter-4");
    assert!(plain.contains("doc-comment-box"));
    assert!(!plain.contains("doc-comment-ask"));
    assert!(!plain.contains("doc-comment-github"));
}

/// PR-19, D59: Ask types the question into the review chat once, closes the
/// box, and makes no comment.
#[test]
fn ask_types_into_the_review_chat_and_leaves_no_comment() {
    let mut app = reviewing();
    shown(&mut app);
    boxed(&mut app, "Why round here?");
    app.click("doc-comment-ask");
    let sent = sends(&app);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("Why round here?"), "{sent:?}");
    assert!(!app.contains("doc-comment-box"));
    assert!(app.fakes.comments.comments(&SessionId::new("ledger")).is_empty());
    assert!(!app.contains("pr-ask-error"));
}

/// PR-19: a question that can't be typed shows why under the terminal, and
/// Dismiss takes it away.
#[test]
fn a_failed_question_shows_under_the_terminal_until_dismissed() {
    let mut app = reviewing();
    shown(&mut app);
    app.fakes.daemon.fail_next_with(
        Operation::Send,
        DaemonError::Refused {
            what: "Typing into ledger".to_owned(),
            reason: "the session has exited".to_owned(),
        },
    );
    boxed(&mut app, "Why round here?");
    app.click("doc-comment-ask");
    assert!(app.contains("pr-ask-error"));
    // Another review never shows this one's failure.
    app.open("refund", Panel::Changes);
    assert!(!app.contains("pr-ask-error"));
    app.open("ledger", Panel::Changes);
    app.click("pr-ask-dismiss");
    assert!(!app.contains("pr-ask-error"));
}

/// PR-11: Draft for GitHub saves the box as a draft for the pull request,
/// not a comment for the agent, and types nothing.
#[test]
fn draft_for_github_saves_a_pull_request_draft_and_types_nothing() {
    let mut app = reviewing();
    shown(&mut app);
    boxed(&mut app, "This rounds half away from zero.");
    app.click("doc-comment-github");
    let held = app.fakes.comments.comments(&SessionId::new("ledger"));
    assert_eq!(held.len(), 1);
    assert!(matches!(
        &held[0].destination,
        Destination::PullRequest { repo, number: 482 } if repo == "acme/api"
    ));
    assert!(app.fakes.comments.typed(&SessionId::new("ledger")).is_empty());
    assert!(sends(&app).is_empty());
    assert!(!app.contains("doc-comment-box"));
    assert!(app.contains(&format!("doc-drop-comment-{}", held[0].id)));
}

/// PR-23: a note from the review chat offers Make it a draft, which makes it
/// a draft for the pull request; a draft has no such button.
#[test]
fn make_it_a_draft_turns_a_note_into_a_pull_request_draft() {
    let app_id = SessionId::new("ledger");
    let mut app = reviewing();
    let note = app.fakes.comments.seed_note(
        &app_id,
        Anchor {
            repo: "api".to_owned(),
            path: "src/client.rs".to_owned(),
            side: Side::New,
            first: 42,
            last: 42,
            quoted: Vec::new(),
            base: None,
            detached: false,
            outdated: false,
        },
        "This error loses the request id.",
        Author::Agent("claude".to_owned()),
    );
    shown(&mut app);
    let button = format!("doc-make-draft-{}", note.id);
    assert!(app.contains(&button));
    app.click(&button);
    let held = app.fakes.comments.comments(&app_id);
    assert!(matches!(held[0].destination, Destination::PullRequest { .. }));
    assert!(!app.contains(&button));
}

/// The open box's side, first and last line, and how many lines it quotes.
fn box_span(app: &App) -> Option<(Side, u32, u32, usize)> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(&SessionId::new("ledger"))
            .and_then(|held| held.editing.as_ref())
            .map(|b| (b.anchor.side, b.anchor.first, b.anchor.last, b.anchor.quoted.len()))
    })
}

/// PR-19: Ask on a hunk's header opens the box over the whole hunk, and its
/// Ask types the hunk's lines with the question.
#[test]
fn ask_on_a_hunk_covers_the_hunk() {
    let mut app = reviewing();
    shown(&mut app);
    // PR-19: Ask shows on the hunk header's hover.
    hover(&mut app, "change-hunk-0");
    app.click("change-ask-hunk-0");
    let (side, first, last, quoted) = box_span(&app).expect("the box opens");
    assert_eq!((side, first, last), (Side::New, 40, 44));
    assert!(quoted > 0);
    app.send(AppAction::Comments(CommentsAction::Typed(
        SessionId::new("ledger"),
        "What does this hunk change?".to_owned(),
    )));
    app.click("doc-comment-ask");
    let sent = sends(&app);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].contains("40-44"), "{sent:?}");
}

/// PR-19: Ask on a file's row opens the box over the file without quoting
/// all of it.
#[test]
fn ask_on_a_file_names_the_file() {
    let mut app = reviewing();
    shown(&mut app);
    // PR-19: Ask shows on the file row's hover.
    hover(&mut app, "change-file api/src/client.rs");
    app.click("change-ask-file api/src/client.rs");
    let (side, first, last, quoted) = box_span(&app).expect("the box opens");
    assert_eq!((side, first, last, quoted), (Side::New, 40, 44, 0));
    assert!(app.contains("doc-comment-ask"));
}

/// A session that reviews nothing has no Ask on its hunks or files.
#[test]
fn a_plain_session_has_no_ask_on_hunks_or_files() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(humata2_review::ReviewAction::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    assert!(!app.contains("change-ask-hunk-0"));
    assert!(!app.contains("change-ask-file api/src/client.rs"));
}
