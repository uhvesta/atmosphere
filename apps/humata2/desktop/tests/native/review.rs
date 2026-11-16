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
    assert!(app.fakes.comments.typed(&id).is_empty());
    app.click("send-comments");
    assert_eq!(app.fakes.comments.typed(&id).len(), 1);
    assert!(app.ui.read(|s| s.comments.sendable(&id).is_empty()));
    assert!(!app.contains("send-comments"));
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
        })
        .unwrap();
    app.click(&format!("doc-edit-comment-{comment}"));
    let (_, _, _, first, last, text) = diff_box(&app).expect("Edit opens the box");
    assert_eq!((first, last, text.as_str()), (42, 42, "Keep these writes in one transaction."));
}
