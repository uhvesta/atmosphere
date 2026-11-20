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
    // Diffs wrap by default (ST-7).
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
        humata2_settings::SettingsAction::SetFontSize(20),
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

/// SM-9, SM-15: a drafted comment shows under its file and in the footer,
/// and Send types every draft into the session at once.
#[test]
fn drafted_comments_show_in_the_panel_and_send_together() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(humata2_review::ReviewAction::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    let id = SessionId::new("ledger");
    app.write_comment("ledger", "api", 43, "Keep these writes in one transaction.");
    app.write_comment("ledger", "api", 42, "Name this error.");
    assert_eq!(
        app.ui.read(|s| s.comments.sendable(&id).len()),
        2
    );
    assert!(app.contains("send-comments"));
    let before = app.send_button("ledger").expect("button names the agent");
    assert!(!before.contains("disabled"), "{before}");
    assert!(app.fakes.comments.typed(&id).is_empty());
    app.click("send-comments");
    assert_eq!(app.fakes.comments.typed(&id).len(), 1);
    assert!(app.ui.read(|s| s.comments.sendable(&id).is_empty()));
    assert!(app.contains("send-comments"));
    let after = app.send_button("ledger").expect("button names the agent");
    assert!(after.contains("disabled"), "{after}");
}

/// SM-9: Edit on a draft reopens its box with the text it has.
#[test]
fn a_draft_can_be_edited_from_its_card() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(humata2_review::ReviewAction::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    let id = SessionId::new("ledger");
    app.write_comment("ledger", "api", 43, "First draft.");
    let comment = app.fakes.comments.comments(&id)[0].id.clone();
    assert!(app.contains(&format!("doc-edit-comment-{comment}")));
    app.click(&format!("doc-edit-comment-{comment}"));
    let text = app.ui.read(|s| {
        s.comments.sessions[&id]
            .editing
            .as_ref()
            .map(|editing| editing.text.clone())
    });
    assert_eq!(text.as_deref(), Some("First draft."));
}

/// The Diff panel on api/src/client.rs of "ledger". Its lines, by index:
/// 0 the hunk, 1 context (40), 2 removed (old 41), 3–6 added (new 41–44).
fn changes_app() -> App {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(humata2_review::ReviewAction::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app
}

/// The open comment box: repo, path, side, first and last line, text.
fn diff_box(app: &App) -> Option<(String, String, humata2_clients::comments::Side, u32, u32, String)> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(&SessionId::new("ledger"))
            .and_then(|held| held.editing.as_ref())
            .map(|b| {
                (
                    b.anchor.repo.clone(),
                    b.anchor.path.clone(),
                    b.anchor.side,
                    b.anchor.first,
                    b.anchor.last,
                    b.text.clone(),
                )
            })
    })
}

/// SM-9, RV-27: clicking a changed line's number opens the comment box
/// under that line, and Comment types it into the agent at once.
#[test]
fn clicking_a_diff_line_opens_the_box_under_it_and_comment_types_it_at_once() {
    use humata2_clients::comments::Side;
    use humata2_review::comments::CommentsAction as C;
    let mut app = changes_app();
    let id = SessionId::new("ledger");
    app.click("change-gutter-4");
    assert_eq!(
        diff_box(&app),
        Some(("api".into(), "src/client.rs".into(), Side::New, 42, 42, String::new()))
    );
    assert!(app.contains("doc-comment-box"));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let line = window.find("change-line-4").bounds();
            let next = window.find("change-line-5").bounds();
            let card = window.find("doc-comment-box").bounds();
            assert!(card.origin.y >= line.origin.y + line.size.height - px(0.5));
            assert!(card.origin.y + card.size.height <= next.origin.y + px(0.5));
        })
        .unwrap();
    app.send(AppAction::Comments(C::Typed(id.clone(), "Name this error.".into())));
    app.click("doc-comment-send");
    let typed = app.fakes.comments.typed(&id);
    assert_eq!(typed.len(), 1);
    assert!(typed[0].contains("src/client.rs:42"), "{typed:?}");
    assert!(typed[0].contains("Name this error."), "{typed:?}");
    assert!(!app.contains("doc-comment-box"));
}

/// SM-9: shift-clicking a second line stretches the box over the range.
#[test]
fn shift_clicking_a_second_diff_line_stretches_the_box_to_a_range() {
    let mut app = changes_app();
    app.click("change-gutter-3");
    // The box opened under the line; click where the next line is drawn now.
    app.draw();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.click_with_modifiers("change-gutter-5", gpui_kit::Modifiers::shift(), cx)
        })
        .unwrap();
    app.settle();
    let (_, _, _, first, last, _) = diff_box(&app).expect("a box is open");
    assert_eq!((first, last), (41, 43));
}

/// RV-27: a removed line's comment is about the old side of the file.
#[test]
fn a_removed_line_takes_its_comment_on_the_old_side() {
    use humata2_clients::comments::Side;
    let mut app = changes_app();
    app.click("change-gutter-2");
    let (_, _, side, first, last, _) = diff_box(&app).expect("a box is open");
    assert_eq!((side, first, last), (Side::Old, 41, 41));
}

/// SM-9: the panel draws the same cards as the document tab, under the
/// line they are about, and Edit on a draft reopens its box there.
#[test]
fn diff_comments_are_the_shared_cards_under_their_line() {
    let mut app = changes_app();
    let id = SessionId::new("ledger");
    app.write_comment("ledger", "api", 42, "Keep these writes in one transaction.");
    let comment = app.fakes.comments.comments(&id)[0].id.clone();
    assert!(app.contains(&format!("doc-comment-{comment}")));
    assert!(app.contains(&format!("doc-edit-comment-{comment}")));
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            let line = window.find("change-line-4").bounds();
            let next = window.find("change-line-5").bounds();
            let card = window.find(gpui_kit::SharedString::from(format!("doc-comment-{comment}"))).bounds();
            assert!(card.origin.y >= line.origin.y + line.size.height - px(0.5));
            assert!(card.origin.y + card.size.height <= next.origin.y + px(0.5));
            // On Scroll the rows run wider than the panel; the card stays
            // inside what can be seen.
            let view = window.find("change-code api/src/client.rs").bounds();
            assert!(card.origin.x + card.size.width <= view.origin.x + view.size.width + px(0.5));
        })
        .unwrap();
    app.click(&format!("doc-edit-comment-{comment}"));
    let (_, _, _, first, last, text) = diff_box(&app).expect("Edit opens the box");
    assert_eq!((first, last, text.as_str()), (42, 42, "Keep these writes in one transaction."));
}

/// UXSIMPLE (b): once the agent changed a sent comment's lines it is done;
/// a file whose comments are all done folds them into one line, Show
/// unfolds them into cards that offer Reopen, and the file row says done.
#[test]
fn done_comments_fold_into_one_line_and_show_unfolds_them() {
    use humata2_review::comments::CommentsAction as C;
    let mut app = changes_app();
    let id = SessionId::new("ledger");
    app.click("change-gutter-4");
    app.send(AppAction::Comments(C::Typed(id.clone(), "Name this error.".into())));
    app.click("doc-comment-send");
    let comment = app.fakes.comments.comments(&id)[0].id.clone();
    assert!(app.contains(&format!("doc-comment-{comment}")));
    assert!(!app.contains(&format!("doc-reopen-comment-{comment}")), "a sent comment is not done");
    app.fakes.comments.lines_changed(&comment, &["fixed"]).expect("addressed");
    app.send(AppAction::Comments(C::Open(id.clone())));
    assert!(app.contains(&format!("doc-reopen-comment-{comment}")), "an unticked file shows its done cards");
    tick_client(&mut app);
    let fold = "doc-done-fold api/src/client.rs";
    assert!(app.contains(fold), "done comments fold into one line");
    assert!(!app.contains(&format!("doc-comment-{comment}")));
    app.click("doc-done-toggle api/src/client.rs");
    assert!(app.contains(&format!("doc-comment-{comment}")));
    assert!(app.contains(&format!("doc-reopen-comment-{comment}")));
}

/// Ticks client.rs as looked at, as its row's circle does.
fn tick_client(app: &mut App) {
    app.send(AppAction::Review(humata2_review::ReviewAction::ToggleLooked {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
}

/// Two sent comments the agent answered, as the done states draw them: open
/// cards on an unticked file (4-2), folded once it is ticked (4-3).
fn answered_comments(app: &mut App) -> Vec<humata2_clients::comments::CommentId> {
    use humata2_review::comments::CommentsAction as C;
    let id = SessionId::new("ledger");
    for (gutter, text) in [
        ("change-gutter-4", "Make the legacy insert an upsert on entry.id."),
        ("change-gutter-5", "One transaction for the whole batch, not one per entry."),
    ] {
        app.click(gutter);
        app.send(AppAction::Comments(C::Typed(id.clone(), text.into())));
        app.click("doc-comment-send");
    }
    let comments = app.fakes.comments.comments(&id);
    let ids: Vec<_> = comments.iter().map(|comment| comment.id.clone()).collect();
    let replies = [
        "upsert_legacy keys on entry.id, so a retried commit writes the row once.",
        "post_many opens one transaction and commits once.",
    ];
    for (comment, reply) in ids.iter().zip(replies) {
        app.fakes.comments.agent_reply(comment, reply).expect("reply");
        app.fakes.comments.lines_changed(comment, &["fixed"]).expect("addressed");
    }
    app.send(AppAction::Comments(C::Open(id)));
    ids
}

/// UXSIMPLE 4-2: done cards are quiet, with the agent's reply under a
/// divider and Reopen at the right; the file row says how many are done.
#[test]
fn done_by_agent_cards_show_the_reply_and_reopen() {
    let mut app = changes_app();
    let ids = answered_comments(&mut app);
    assert!(!app.contains("doc-done-fold api/src/client.rs"), "4-2: an unticked file doesn't fold");
    for id in &ids {
        assert!(app.contains(&format!("doc-comment-{id}")));
        assert!(app.contains(&format!("doc-reopen-comment-{id}")));
    }
    app.snapshot("review-done-by-agent");
}

/// The done-by-agent state in full screen, on a file that has no sent comment
/// of its own (client.rs keeps one, so its row shows the count, not "✓ N done").
#[test]
fn review_done_by_agent_full() {
    let mut workspace = fixture::design();
    let api = workspace.diffs.iter_mut().find(|d| d.repo == "api").unwrap();
    // The fixture gives entry.rs no lines to comment on, so it takes client.rs's.
    let lines = api
        .files
        .iter()
        .find(|f| f.path == "src/client.rs")
        .unwrap()
        .lines
        .clone();
    api.files
        .iter_mut()
        .find(|f| f.path == "src/entry.rs")
        .unwrap()
        .lines = lines;
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(humata2_review::ReviewAction::SelectFile {
        repo: "api".into(),
        path: "src/entry.rs".into(),
    }));
    let ids = answered_comments(&mut app);
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    for id in &ids {
        assert!(app.contains(&format!("doc-comment-{id}")));
        assert!(app.contains(&format!("doc-reopen-comment-{id}")));
    }
    assert!(app.contains("change-file-done api/src/entry.rs"));
    app.snapshot("review-done-by-agent-full");
}

/// The done-by-agent state in the dark theme: the done card, its chip and
/// Reopen, the reply divider and the fold line.
#[test]
fn review_done_by_agent_dark() {
    let mut app = changes_app();
    let ids = answered_comments(&mut app);
    for id in &ids {
        assert!(app.contains(&format!("doc-comment-{id}")));
        assert!(app.contains(&format!("doc-reopen-comment-{id}")));
    }
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("review-done-by-agent-dark");
}

/// An agent's note on a line of client.rs, as UXSIMPLE 3-4 draws it.
fn with_agent_note(app: &mut App) -> humata2_clients::comments::Comment {
    use humata2_clients::comments::{Anchor, Author, Side};
    let note = app.fakes.comments.seed_note(
        &SessionId::new("ledger"),
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
        "The balance is read on db, not tx, so two posts can both pass the overdraft check. Read it through tx.",
        Author::Agent("copilot".to_owned()),
    );
    app.send(AppAction::Comments(humata2_review::comments::CommentsAction::Open(SessionId::new("ledger"))));
    note
}

/// UXSIMPLE 3-4: what an agent writes on a line is one card, with Send,
/// Edit and Drop, and the file row counts it as a note.
#[test]
fn review_agent_note() {
    let mut app = changes_app();
    let note = with_agent_note(&mut app);
    assert!(app.contains(&format!("doc-send-note-{}", note.id)));
    app.snapshot("review-agent-note");
}

#[test]
fn review_agent_note_dark() {
    let mut app = changes_app();
    with_agent_note(&mut app);
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("review-agent-note-dark");
}

/// UXSIMPLE 4-3: a file the person ticked whose comments are all done folds
/// them into one line with a green check and Show; 4-2 leaves an unticked
/// file's done cards open.
#[test]
fn all_done_comments_fold_into_one_line() {
    let mut app = changes_app();
    answered_comments(&mut app);
    assert!(!app.contains("doc-done-fold api/src/client.rs"));
    tick_client(&mut app);
    assert!(app.contains("doc-done-fold api/src/client.rs"));
    app.snapshot("review-done-folded");
}

/// Flow 26.14 step 2: widening the panel rewraps the long lines, and the
/// line at the top of the view stays at the top.
#[test]
fn widening_a_wrapped_diff_keeps_its_top_line_in_place() {
    use humata2_clients::model::{DiffLine, LineKind};
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
    file.lines = (1..=60)
        .map(|n| DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(n),
            text: format!("    let value_{n:02} = compute(first_argument, second_argument, third_argument_{n:02});"),
        })
        .collect();
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 200.)));
    app.draw();
    let view = "change-code api/src/client.rs";
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.scroll(view, gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-900.))), cx);
        })
        .unwrap();
    app.settle();
    app.draw();
    // The first line whose bottom is below the view's top, and how far its
    // top sits from the view's top.
    let top = |app: &mut App| {
        app.cx
            .update_window(app.window, |_, window, _| {
                let top = window.find(view).bounds().origin.y;
                (0..60)
                    .map(|i| (i, window.find(gpui_kit::SharedString::from(format!("change-line-{i}"))).bounds()))
                    .find(|(_, b)| b.origin.y + b.size.height > top + px(0.5))
                    .map(|(i, b)| (i, f32::from(b.origin.y - top)))
                    .expect("a line is at the top")
            })
            .unwrap()
    };
    let (before, into) = top(&mut app);
    assert!(before > 5, "the view is scrolled well down: top line {before}");
    let (wrapped_rows, line) = app
        .cx
        .update_window(app.window, |_, window, _| {
            let b = window.find(gpui_kit::SharedString::from(format!("change-line-{before}"))).bounds();
            let row = window.find(gpui_kit::SharedString::from(format!("change-code-{before}-0"))).bounds();
            (f32::from(b.size.height / row.size.height).round(), f32::from(row.size.height))
        })
        .unwrap();
    assert!(wrapped_rows >= 3., "a long line wraps at 200: {wrapped_rows} rows");

    app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 420.)));
    app.draw();
    app.draw();
    let (after, offset) = top(&mut app);
    assert_eq!(after, before, "the same line stays at the top");
    // The rewrapped line is shorter now; the view starts the same share
    // of its height into it as before.
    let height = app
        .cx
        .update_window(app.window, |_, window, _| {
            let b = window.find(gpui_kit::SharedString::from(format!("change-line-{after}"))).bounds();
            f32::from(b.size.height)
        })
        .unwrap();
    assert!(height < wrapped_rows * line, "the line wraps to fewer rows when wider");
    let share = |offset: f32, height: f32| -offset / height;
    assert!(
        (share(offset, height) - share(into, wrapped_rows * line)).abs() < 0.1,
        "the view starts as far into the top line as before: {offset} of {height}, was {into}"
    );
}

/// RV-1, RV-6: the base chip opens a menu of bases; the fork base shows
/// only for forks, other sessions of the channel are listed, and Looks
/// good makes "Since I last looked" start from here.
#[test]
fn the_base_menu_picks_what_the_diff_is_against_and_looks_good_starts_it_over() {
    use humata2_clients::model::DiffBase;
    // One repository keeps the header's base menu (D868).
    let mut workspace = fixture::design();
    workspace.diffs.retain(|d| d.session != SessionId::new("ledger") || d.repo == "api");
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    let id = SessionId::new("ledger");
    assert!(!app.contains("diff-base-menu"));
    app.click("diff-base");
    assert!(app.contains("diff-base-menu"));
    assert!(app.contains("diff-base-origin-main"));
    assert!(app.contains("diff-base-uncommitted"));
    assert!(!app.contains("diff-base-since-reviewed"));
    assert!(!app.contains("diff-mark-reviewed"));
    assert!(app.contains("diff-base-other-brin"));
    // "ledger" is not a fork.
    assert!(!app.contains("diff-base-since-fork"));

    app.click("diff-base");
    assert!(!app.contains("diff-base-menu"));
    app.send(AppAction::Review(humata2_review::ReviewAction::LooksGood));
    assert!(
        app.fakes
            .daemon
            .calls()
            .iter()
            .any(|call| call == "mark reviewed ledger")
    );

    app.send(AppAction::Review(humata2_review::ReviewAction::SinceLooked(true)));
    assert_eq!(
        app.ui.read(|s| Some(s.review.targets_of(&id).preset)),
        Some(DiffBase::SinceLastReviewed)
    );
    // Nothing changed since Looks good: the panel says so, not "No
    // changes yet".
    assert!(app.contains("diff-none-against-base"));
    assert!(!app.contains("diff-no-changes-yet"));

    app.click("diff-base");
    app.click("diff-base-other-brin");
    assert_eq!(
        app.ui.read(|s| Some(s.review.targets_of(&id).preset)),
        Some(DiffBase::OtherSession(SessionId::new("brin")))
    );
}

/// RV-1: a fork can be compared with where it was forked.
#[test]
fn a_fork_offers_since_the_fork_as_a_base() {
    let mut app = App::new(fixture::design());
    app.open("brin", Panel::Changes);
    app.click("diff-base");
    assert!(app.contains("diff-base-since-fork"));
    assert!(!app.contains("diff-base-other-brin"));
}

/// D179: a failed refresh keeps the last diff under one line with Try
/// again; Try again reads it once more.
#[test]
fn a_failed_refresh_keeps_the_last_diff_with_try_again() {
    use humata2_clients::Operation;
    let mut app = changes_app();
    assert!(!app.contains("diff-refresh-failed"));
    app.fakes.daemon.fail_next(Operation::Diffs, "The daemon closed the connection.");
    app.click("diff-file-count");
    app.click("repo-base-0");
    app.click("repo-base-uncommitted");
    assert!(app.contains("diff-refresh-failed"));
    assert!(app.contains("diff-retry"));
    // The diff read before stays.
    assert!(app.contains("change-line-1"));
    app.click("diff-retry");
    assert!(!app.contains("diff-refresh-failed"));
    assert!(app.contains("change-line-1"));
}

/// D179: when the first read fails the panel names the error with Try
/// again, and never says "No changes yet".
#[test]
fn a_failed_first_read_names_the_error_with_try_again() {
    use humata2_clients::Operation;
    let mut app = App::new(fixture::design());
    // Showing the session reads its changes, and opening the Diff panel
    // reads them again: the daemon is down for both.
    app.fakes.daemon.fail_next(Operation::Diffs, "The daemon closed the connection.");
    app.send(AppAction::Navigate(Route::Session(SessionId::new("ledger"))));
    app.fakes.daemon.fail_next(Operation::Diffs, "The daemon closed the connection.");
    app.send(AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))));
    assert!(app.contains("diff-read-failed"));
    assert!(app.contains("diff-retry"));
    assert!(!app.contains("diff-no-changes-yet"));
    app.click("diff-retry");
    assert!(!app.contains("diff-read-failed"));
}
