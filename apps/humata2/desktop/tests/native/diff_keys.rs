//! The Diff's file list: a repository that fails beside the others, the file
//! bar of the inline pull request review, and the keyboard (DS-35, DS-53,
//! DS-84).
use super::*;
use humata2_review::{ListCursor, ReviewAction as R};

const LEDGER: &str = "ledger";
const REVIEW: &str = "se_pr_1";

fn press(app: &mut App, key: &str) {
    app.cx
        .update_window(app.window, |_, window, cx| window.press(key, cx))
        .unwrap();
    app.settle();
}

/// The "ledger" session's Diff in full screen, where its file list stays
/// whichever file is open.
fn full_screen(app: &mut App) {
    app.open(LEDGER, Panel::Changes);
    app.send(AppAction::Session(SessionAction::ToggleWiden));
}

fn cursor(app: &App) -> Option<ListCursor> {
    app.ui
        .read(|state| state.review.list_cursor(&SessionId::new(LEDGER)).cloned())
}

fn open_file(app: &App, session: &str) -> Option<(String, String)> {
    app.ui.read(|state| {
        state
            .review
            .saved
            .selected
            .get(&SessionId::new(session))
            .cloned()
    })
}

/// DS-35: a repository that can't be read has its own row with its message
/// and Try again; the others still list their files. Try again brings it
/// back once it can be read.
#[test]
fn a_failing_repo_has_its_own_row_and_try_again_brings_it_back() {
    let mut app = App::new(fixture::design());
    app.fakes
        .daemon
        .fail_repo(&SessionId::new(LEDGER), "api", "The clone is locked.");
    full_screen(&mut app);
    assert!(app.contains("repo-error-api"));
    assert!(app.contains("repo-retry-api"));
    assert!(app.contains("change-file ledger/migrations/0042_accounts.sql"));
    assert!(!app.contains("change-file api/src/client.rs"));
    assert!(!app.contains("repo-error-ledger"));

    app.fakes.daemon.heal_repo(&SessionId::new(LEDGER), "api");
    app.click("repo-retry-api");
    assert!(!app.contains("repo-error-api"));
    assert!(app.contains("change-file api/src/client.rs"));
    assert!(app.contains("change-file ledger/migrations/0042_accounts.sql"));
}

/// DS-53: the inline pull request review has the file bar, and its next
/// button moves the open file on into the review's other pull request.
#[test]
fn the_inline_review_steps_files_across_its_pull_requests() {
    let mut app = App::new(super::pr_session::review_workspace(Vec::new()));
    app.open(REVIEW, Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "acme/api".into(),
        path: "src/refund.rs".into(),
    }));
    assert!(app.contains("diff-file-count"));
    assert!(app.contains("diff-prev-file"));
    assert!(app.contains("diff-next-file"));
    app.click("diff-next-file");
    assert_eq!(
        open_file(&app, REVIEW),
        Some(("acme/ledger".to_owned(), "src/ledger.rs".to_owned()))
    );
    assert!(app.contains("diff-file-count"));
}

/// DS-84: with the file list focused, the arrows move its cursor, which wears
/// a ring; Left goes up to the repository and Enter folds it.
#[test]
fn the_focused_file_list_takes_the_arrows_and_enter() {
    let mut app = App::new(fixture::design());
    full_screen(&mut app);
    let file = "migrations/0042_accounts.sql";
    assert_eq!(cursor(&app), None);
    assert!(!app.contains("diff-list-cursor"));

    // A click in the list focuses it; the cursor starts on the open file.
    app.click(&format!("change-file ledger/{file}"));
    press(&mut app, "down");
    let first = cursor(&app);
    assert_eq!(
        first,
        Some(ListCursor::File {
            repo: "ledger".into(),
            path: file.into(),
        })
    );
    assert!(app.contains("diff-list-cursor"));

    press(&mut app, "down");
    assert_ne!(cursor(&app), first);
    press(&mut app, "up");
    assert_eq!(cursor(&app), first);

    // Left on a file goes to its repository; Enter there folds it.
    press(&mut app, "left");
    assert_eq!(cursor(&app), Some(ListCursor::Repo("ledger".into())));
    assert!(app.contains(&format!("change-file ledger/{file}")));
    press(&mut app, "enter");
    assert!(!app.contains(&format!("change-file ledger/{file}")));
    assert_eq!(cursor(&app), Some(ListCursor::Repo("ledger".into())));
}

/// DS-84: without focus the list's keys do nothing, and no ring is drawn.
#[test]
fn an_unfocused_file_list_ignores_the_arrows() {
    let mut app = App::new(fixture::design());
    full_screen(&mut app);
    press(&mut app, "down");
    assert_eq!(cursor(&app), None);
    assert!(!app.contains("diff-list-cursor"));
}
