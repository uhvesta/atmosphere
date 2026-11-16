//! Diff wrapping and review comments.
use super::*;

/// ST-8: a wrapped line keeps its number aligned to its first visual row.
#[test]
fn wrapped_changes_align_the_gutter_and_switch_back_to_one_scrolling_row() {
    use humata2_review::ReviewAction as R;
    let mut workspace = fixture::design();
    let file = workspace
        .diffs
        .iter_mut()
        .find(|d| d.repo == "api")
        .unwrap()
        .files
        .iter_mut()
        .find(|f| f.path == "src/client.rs")
        .unwrap();
    file.lines
        .iter_mut()
        .find(|l| l.kind == humata2_clients::model::LineKind::Removed)
        .unwrap()
        .text = format!("        {}.await?;", "long_identifier_".repeat(8));
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    // Diffs clip by default; this flow is about wrapped lines.
    app.send(AppAction::Review(R::ToggleWrap));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let gutter = window.find("change-gutter-2").bounds();
            let code = window.find("change-code-2-0").bounds();
            assert!((f32::from(gutter.origin.y - code.origin.y)).abs() < 0.5);
            assert!(window.find("change-line-2").bounds().size.height > gutter.size.height);
        })
        .unwrap();
    app.snapshot("changes-wrapped-long-line");
    app.send(AppAction::Review(R::ToggleWrap));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let line = window.find("change-line-2").bounds();
            let gutter = window.find("change-gutter-2").bounds();
            assert!((f32::from(line.size.height - gutter.size.height)).abs() < 0.5);
        })
        .unwrap();
    app.send(AppAction::Review(R::ToggleWrap));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("changes-wrapped-long-line-dark");
    app.send(AppAction::Settings(
        humata2_settings::SettingsAction::FontSize(8),
    ));
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 220.)));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let gutter = window.find("change-gutter-2").bounds();
            let code = window.find("change-code-2-0").bounds();
            assert!((f32::from(gutter.origin.y - code.origin.y)).abs() < 0.5);
            assert!(f32::from(gutter.size.height) >= 24.5);
        })
        .unwrap();
}

/// SM-9/10: a failed comment hand-over waits visibly, Remove returns the comments to drafts.
#[test]
fn review_comments_wait_in_the_session_queue_and_retry_without_losing_their_anchor() {
    use humata2_review::{Anchor, ReviewAction as R};
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.send(AppAction::Review(R::StartComment(Anchor {
        path: "src/client.rs".into(),
        line: 43,
    })));
    app.send(AppAction::Review(R::CommentText(
        "Keep these writes in one transaction.".into(),
    )));
    app.send(AppAction::Review(R::SaveComment));
    // Comments deliver at once when the session takes input, so the capture
    // of comments visibly waiting comes from a failed delivery.
    app.fakes.daemon.fail_next(
        humata2_clients::Operation::SendComments,
        "The connection dropped. Reconnect and try again.",
    );
    app.click("send-comments");
    assert!(app.ui.read(|s| s.review.queued().is_empty()));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().queue.len()), 1);
    assert!(
        !app.fakes
            .daemon
            .workspace()
            .diffs
            .iter()
            .flat_map(|d| &d.comments)
            .any(|c| c.body == "Keep these writes in one transaction.")
    );
    app.snapshot("review-comments-queued");
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("review-comments-queued-dark");
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.click("show-queued");
    app.click("remove-queued-0");
    assert!(app.ui.read(|s| s.session.view().unwrap().queue.is_empty()));
    assert_eq!(app.ui.read(|s| s.review.queued().len()), 1);
    // Removing the only failed message lifts the pause, so this send goes
    // at once; make it fail too, to show the retry state.
    app.fakes.daemon.fail_next(
        humata2_clients::Operation::SendComments,
        "The connection dropped. Reconnect and try again.",
    );
    app.click("send-comments");
    assert!(app.contains("session-error"));
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().queue.len()), 1);
    app.snapshot("review-comments-retry");
    app.click("send-next-queued");
    assert!(!app.contains("session-queue"));
    let workspace = app.fakes.daemon.workspace();
    let sent: Vec<_> = workspace
        .diffs
        .iter()
        .filter(|d| d.session == SessionId::new("ledger"))
        .flat_map(|d| &d.comments)
        .filter(|c| c.body == "Keep these writes in one transaction.")
        .collect();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].sent);
    app.snapshot("review-comments-sent");
}

/// SM-9/SE-23: a draft's own Send delivers only that comment; Send all takes the rest.
#[test]
fn a_saved_draft_sends_alone_and_send_all_takes_the_rest() {
    use humata2_review::{Anchor, ReviewAction as R};
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    let bodies = ["First draft goes alone.", "Second draft waits its turn."];
    for (line, body) in [(43, bodies[0]), (42, bodies[1])] {
        app.send(AppAction::Review(R::StartComment(Anchor {
            path: "src/client.rs".into(),
            line,
        })));
        app.send(AppAction::Review(R::CommentText(body.into())));
        app.send(AppAction::Review(R::SaveComment));
    }
    assert_eq!(app.ui.read(|s| s.review.queued().len()), 2);
    let delivered = |app: &App, body: &str| {
        app.fakes
            .daemon
            .workspace()
            .diffs
            .iter()
            .filter(|d| d.session == SessionId::new("ledger"))
            .flat_map(|d| &d.comments)
            .any(|c| c.body == body && c.sent)
    };
    assert!(app.contains("send-comment-0"));
    app.click("send-comment-0");
    assert!(delivered(&app, bodies[0]));
    assert!(!delivered(&app, bodies[1]));
    assert_eq!(app.ui.read(|s| s.review.queued().len()), 1);
    assert!(app.ui.read(|s| s.session.view().unwrap().queue.is_empty()));
    app.click("send-comments");
    assert!(delivered(&app, bodies[1]));
    assert!(app.ui.read(|s| s.review.queued().is_empty()));
}
