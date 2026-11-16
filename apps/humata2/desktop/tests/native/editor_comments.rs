//! RV-20: comments in the code editor work as in the diff, on new-side line
//! numbers, and share the diff's drafts; a footer shows the drafts with
//! Send N comments.
//!
//! Registered in native.rs next to `mod editor;`; it uses that module's
//! `editor_app` and `show_client` and `text` (make them `pub(super)`).
use super::*;
use super::editor::{editor_app, show_client, text};
use humata2_clients::comments::Side;
use humata2_review::comments::CommentsAction as C;
use humata2_session::editor::CodeMode;

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

/// Where the open comment box points: (path, side, first line).
fn editing(app: &App) -> Option<(String, Side, u32)> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(&ledger())
            .and_then(|session| session.editing.as_ref())
            .map(|open| (open.anchor.path.clone(), open.anchor.side, open.anchor.first))
    })
}

fn draft_here(app: &mut App, line: u32, text: &str) {
    app.click(&format!("editor-comment-{line}"));
    assert_eq!(
        editing(app),
        Some(("src/client.rs".to_owned(), Side::New, line)),
        "the box opens on new-side line {line}"
    );
    app.send(AppAction::Comments(C::Typed(ledger(), text.into())));
    app.send(AppAction::Comments(C::Close(ledger())));
}

#[test]
fn a_comment_written_in_the_editor_is_a_draft_on_its_new_line() {
    let mut app = editor_app();
    assert!(!app.contains("editor-drafts"), "no drafts, no footer");
    draft_here(&mut app, 7, "Why a new line here?");
    assert!(app.contains("editor-drafts"));
    assert!(app.contains("editor-send-comments"));
    let drawn = text(&mut app);
    assert!(drawn.contains("1 draft comment"), "{drawn}");
    assert!(drawn.contains("Send 1 comment"), "{drawn}");
    assert!(drawn.contains("Why a new line here?"), "the draft shows under its line");
}

#[test]
fn side_by_side_comments_on_the_new_column_only() {
    let mut app = editor_app();
    app.send(AppAction::Session(SessionAction::CodeMode {
        repo: "api".into(),
        path: "src/client.rs".into(),
        mode: CodeMode::SideBySide,
    }));
    // l5 is new line 5; the removed old5 has no new-side number.
    draft_here(&mut app, 5, "Rename this.");
    assert!(text(&mut app).contains("1 draft comment"));
}

#[test]
fn the_editor_and_the_diff_share_drafts_and_send_them_together() {
    let mut app = editor_app();
    draft_here(&mut app, 7, "First, from the editor.");
    app.write_comment("ledger", "api", 8, "Second, from the diff.");
    show_client(&mut app);
    let drawn = text(&mut app);
    assert!(drawn.contains("2 draft comments"), "{drawn}");
    app.click("editor-send-comments");
    let typed = app.fakes.comments.typed(&ledger()).join("\n");
    assert!(typed.contains("First, from the editor."), "{typed}");
    assert!(typed.contains("Second, from the diff."), "{typed}");
    assert!(!app.contains("editor-drafts"), "nothing left to send");
}
