//! Archiving a review session whose pull request drafts were never
//! submitted asks first, with Archive and Cancel (PR-17, D513).
use super::*;

/// The design workspace with the review session `pr-482` open on Info, and
/// the fake daemon's archive plan counting `drafts` unsubmitted drafts.
fn review_app(drafts: u32) -> (App, SessionId) {
    let id = SessionId::new("pr-482");
    let mut app = App::new(fixture::design());
    app.fakes.daemon.set_unsubmitted_drafts(&id, drafts);
    app.fold_quiet_channels();
    app.open("pr-482", Panel::Info);
    (app, id)
}

fn archived(app: &App, id: &SessionId) -> bool {
    app.ui
        .read(|s| s.workspace.session(id).is_some_and(|session| session.archived))
}

fn asking(app: &App, id: &SessionId) -> bool {
    app.ui.read(|s| {
        s.session
            .archiving
            .as_ref()
            .is_some_and(|confirm| &confirm.session == id)
    })
}

#[test]
fn unsubmitted_drafts_ask_before_archiving() {
    let (mut app, id) = review_app(2);
    app.send(AppAction::Session(SessionAction::Archive));
    assert!(asking(&app, &id), "the dialog opens instead of archiving");
    assert!(app.contains("archive-drafts"));
    assert!(app.contains("archive-confirm"));
    assert!(!archived(&app, &id));
}

#[test]
fn cancel_keeps_a_session_with_unsubmitted_drafts() {
    let (mut app, id) = review_app(1);
    app.send(AppAction::Session(SessionAction::Archive));
    app.click("archive-cancel");
    assert!(!asking(&app, &id));
    assert!(!archived(&app, &id));
}

#[test]
fn archive_goes_ahead_once_confirmed_over_the_drafts() {
    let (mut app, id) = review_app(3);
    app.send(AppAction::Session(SessionAction::Archive));
    app.click("archive-confirm");
    assert!(archived(&app, &id));
}

#[test]
fn no_drafts_archive_at_once_with_no_drafts_line() {
    let (mut app, id) = review_app(0);
    app.send(AppAction::Session(SessionAction::Archive));
    assert!(!app.contains("archive-drafts"));
    assert!(archived(&app, &id));
}
