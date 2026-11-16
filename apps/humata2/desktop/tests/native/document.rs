//! A markdown file open as a session tab (D166, flow 26.20, frames 39 and
//! 41 to 43): the page, the source with comments on its lines, or both.
use super::*;
use humata2_clients::comments::{Anchor, Side, Status as CommentStatus};
use humata2_review::comments::CommentsAction as C;
use humata2_session::DocumentMode;

const SESSION: &str = "desk-brin-rfc";
const PATH: &str = "rfc-brin.md";

fn session() -> SessionId {
    SessionId::new(SESSION)
}

/// The design workspace with the Desk, channels folded as the frames draw
/// them. The window is active, so the comment field can take focus.
fn doc_app() -> App {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let mut app = App::new(workspace);
    app.fold_quiet_channels();
    app.cx
        .update_window(app.window, |_, window, _| window.activate_window())
        .unwrap();
    app.settle();
    app
}

/// Opens BRIN rollout RFC with `rfc-brin.md` as a tab, the panel as given.
fn open_document(app: &mut App, panel: Option<Panel>) {
    app.send(AppAction::Navigate(Route::Session(session())));
    app.send(AppAction::Session(SessionAction::SetPanel(panel)));
    app.send(AppAction::Session(SessionAction::SelectFile {
        repo: String::new(),
        path: PATH.into(),
    }));
    app.send(AppAction::Session(SessionAction::OpenFile));
}

fn show(app: &mut App, mode: DocumentMode) {
    app.send(AppAction::Session(SessionAction::DocumentMode {
        repo: String::new(),
        path: PATH.into(),
        mode,
    }));
}

fn shown(app: &App) -> (DocumentMode, bool) {
    app.ui.read(|state| {
        let view = state
            .session
            .view()
            .map(|view| view.document("", PATH))
            .unwrap_or_default();
        (view.mode, view.linked)
    })
}

fn anchor(first: u32, last: u32) -> Anchor {
    Anchor {
        repo: String::new(),
        path: PATH.into(),
        side: Side::New,
        first,
        last,
        quoted: Vec::new(),
        base: None,
        detached: false,
    }
}

/// The open box's lines and text.
fn boxed(app: &App) -> Option<(u32, u32, String)> {
    app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(&session())
            .and_then(|held| held.editing.as_ref())
            .map(|b| (b.anchor.first, b.anchor.last, b.text.clone()))
    })
}

/// Writes a comment on lines and sends it at once, as Comment does.
fn comment_on(app: &mut App, first: u32, last: u32, text: &str) {
    app.send(AppAction::Comments(C::OpenBox(session(), anchor(first, last))));
    app.send(AppAction::Comments(C::Typed(session(), text.into())));
    app.send(AppAction::Comments(C::Comment(session())));
}

/// Frame 39: the document in Preview with the Files panel open.
#[test]
fn a_desk_document_opens_as_a_page_beside_the_files_panel() {
    let mut app = doc_app();
    open_document(&mut app, Some(Panel::Files));
    assert_eq!(shown(&app).0, DocumentMode::Preview);
    assert!(app.contains("doc-preview"));
    assert!(!app.contains("doc-source"));
    app.snapshot("39-desk-document");
}

/// Frame 42: side by side, a comment sent on line 7, a draft box on 13-14.
#[test]
fn the_source_shows_a_sent_comment_and_a_draft_box_beside_the_page() {
    let mut app = doc_app();
    open_document(&mut app, None);
    show(&mut app, DocumentMode::SideBySide);
    comment_on(&mut app, 7, 7, "Where are the sizes from?");
    app.send(AppAction::Comments(C::OpenBox(session(), anchor(13, 14))));
    app.send(AppAction::Comments(C::Typed(
        session(),
        "Keep both until range p95 holds for a week on devbox-1, not just until the backfill ends."
            .into(),
    )));
    assert!(app.contains("doc-source"));
    assert!(app.contains("doc-preview"));
    assert!(app.contains("doc-comment-box"));
    app.snapshot("42-review-markdown-source");
}

/// Moves the document tab into the other group, as dragging it there does.
fn move_document_across(app: &mut App) {
    let index = app.ui.read(|s| s.session.view().unwrap().groups[0].tabs.len() - 1);
    app.send(AppAction::Session(SessionAction::MoveTab { group: 0, index }));
}

/// Frame 43, dark: the agent's terminal in one group, the file alone in the
/// second, side by side with its page.
#[test]
fn the_terminal_the_source_and_the_page_fit_side_by_side() {
    let mut app = doc_app();
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    open_document(&mut app, None);
    move_document_across(&mut app);
    show(&mut app, DocumentMode::SideBySide);
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
    // The file is alone in the second group: no shell opened beside it.
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups[1].tabs.len()), 1);
    assert!(app.contains("doc-source"));
    assert!(app.contains("doc-preview"));
    // The frame's state: line 7 sent, a draft on 13-14, the agent's tab shown.
    comment_on(&mut app, 7, 7, "Where are the sizes from?");
    app.send(AppAction::Comments(C::OpenBox(session(), anchor(13, 14))));
    app.send(AppAction::Comments(C::Typed(
        session(),
        "Keep both until range p95 holds for a week on devbox-1, not just until the backfill ends."
            .into(),
    )));
    app.send(AppAction::Session(SessionAction::SelectGroupTab { group: 0, index: 0 }));
    // The user is writing in the file's group: it is the focused one.
    app.send(AppAction::Session(SessionAction::SelectGroupTab { group: 1, index: 0 }));
    assert!(app.contains("doc-comment-box"));
    app.snapshot("43-review-terminal-source-page");
}

/// Frame 41, dark: the agent's terminal in one group, the file's page alone
/// in the second, the mode switch in that group's row.
#[test]
fn the_page_reads_beside_the_agents_terminal() {
    let mut app = doc_app();
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    open_document(&mut app, None);
    move_document_across(&mut app);
    app.send(AppAction::Session(SessionAction::SelectGroupTab { group: 0, index: 0 }));
    assert_eq!(shown(&app).0, DocumentMode::Preview);
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
    assert!(app.contains("doc-preview"));
    assert!(!app.contains("doc-source"));
    app.snapshot("41-review-beside-terminal");
}

#[test]
fn moving_the_document_tab_draws_both_groups_and_moving_it_back_joins_them() {
    let mut app = doc_app();
    open_document(&mut app, None);
    // The file is the third tab (claude, zsh, the file) of the only group.
    assert!(app.contains("file-tab-0-2"));
    assert!(!app.contains("file-tab-1-0"));
    // The right-edge drop zone shows only while a tab is dragged.
    assert!(!app.contains("tab-drop-right"));
    move_document_across(&mut app);
    assert!(app.contains("file-tab-1-0"));
    assert!(!app.contains("file-tab-0-2"));
    assert!(!app.contains("tab-drop-right"));
    app.send(AppAction::Session(SessionAction::MoveTab { group: 1, index: 0 }));
    assert!(app.contains("file-tab-0-2"));
    assert!(!app.contains("file-tab-1-0"));
}

#[test]
fn clicking_a_source_line_opens_the_comment_box_on_it() {
    let mut app = doc_app();
    open_document(&mut app, None);
    show(&mut app, DocumentMode::Source);
    assert!(boxed(&app).is_none());
    app.click("doc-line-7");
    let (first, last, text) = boxed(&app).expect("the box opens");
    assert_eq!((first, last), (7, 7));
    assert!(text.is_empty());
    assert!(app.contains("doc-comment-box"));
}

#[test]
fn shift_clicking_a_second_line_stretches_the_box_to_a_range() {
    let mut app = doc_app();
    open_document(&mut app, None);
    show(&mut app, DocumentMode::Source);
    app.click("doc-line-13");
    // The box opened under the line; click where the next line is drawn now.
    app.draw();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.click_with_modifiers("doc-line-14", gpui_kit::Modifiers::shift(), cx)
        })
        .unwrap();
    app.settle();
    let (first, last, _) = boxed(&app).expect("the box stays open");
    assert_eq!((first, last), (13, 14));
}

#[test]
fn typing_in_the_box_and_pressing_comment_sends_it_to_the_agent() {
    let mut app = doc_app();
    open_document(&mut app, None);
    show(&mut app, DocumentMode::SideBySide);
    app.click("doc-line-7");
    assert!(app.contains("doc-comment-box"));
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.input("Where are the sizes from?", cx)
        })
        .unwrap();
    app.settle();
    assert_eq!(
        boxed(&app).map(|b| b.2).as_deref(),
        Some("Where are the sizes from?")
    );
    assert!(app.fakes.comments.typed(&session()).is_empty());
    app.click("doc-comment-send");
    let typed = app.fakes.comments.typed(&session());
    assert_eq!(typed.len(), 1);
    assert!(typed[0].contains("Where are the sizes from?"));
    let held = app.fakes.comments.comments(&session());
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].status, CommentStatus::Sent);
    assert!(boxed(&app).is_none());
    assert!(!app.contains("doc-comment-box"));
}

#[test]
fn cancel_closes_the_box_and_leaves_no_comment() {
    let mut app = doc_app();
    open_document(&mut app, None);
    show(&mut app, DocumentMode::Source);
    app.click("doc-line-5");
    app.click("doc-comment-cancel");
    assert!(boxed(&app).is_none());
    assert!(app.fakes.comments.comments(&session()).is_empty());
}

#[test]
fn drafts_wait_in_the_link_bar_until_send_types_them_together() {
    let mut app = doc_app();
    open_document(&mut app, None);
    show(&mut app, DocumentMode::SideBySide);
    for (line, text) in [(7, "Where are the sizes from?"), (12, "Say what a range is.")] {
        app.send(AppAction::Comments(C::OpenBox(session(), anchor(line, line))));
        app.send(AppAction::Comments(C::Typed(session(), text.into())));
        app.send(AppAction::Comments(C::Close(session())));
    }
    assert!(app.contains("doc-send-comments"));
    assert!(app.fakes.comments.typed(&session()).is_empty());
    app.click("doc-send-comments");
    assert_eq!(app.fakes.comments.typed(&session()).len(), 1);
    assert!(!app.contains("doc-send-comments"));
}

#[test]
fn the_mode_switch_changes_what_is_drawn() {
    let mut app = doc_app();
    open_document(&mut app, None);
    assert!(app.contains("doc-modes-0"));
    assert!(app.contains("doc-preview"));
    assert!(!app.contains("doc-source"));
    app.click("doc-mode-0-source");
    assert_eq!(shown(&app).0, DocumentMode::Source);
    assert!(app.contains("doc-source"));
    assert!(!app.contains("doc-preview"));
    app.click("doc-mode-0-side-by-side");
    assert_eq!(shown(&app).0, DocumentMode::SideBySide);
    assert!(app.contains("doc-source"));
    assert!(app.contains("doc-preview"));
    assert!(app.contains("doc-link-scrolling"));
    app.click("doc-mode-0-preview");
    assert_eq!(shown(&app).0, DocumentMode::Preview);
    assert!(!app.contains("doc-source"));
}

#[test]
fn a_plain_file_has_no_mode_switch() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Files);
    app.file_tab();
    let index = app.ui.read(|state| {
        state.session.view().and_then(|view| {
            view.groups[0]
                .tabs
                .iter()
                .position(|tab| matches!(tab, humata2_session::Tab::File { .. }))
        })
    });
    app.send(AppAction::Session(SessionAction::SelectGroupTab {
        group: 0,
        index: index.expect("the file tab is open"),
    }));
    assert!(app.contains("session-file"));
    assert!(!app.contains("doc-modes-0"));
    assert!(!app.contains("doc-source"));
}

#[test]
fn link_scrolling_unticks_and_ticks_again() {
    let mut app = doc_app();
    open_document(&mut app, None);
    show(&mut app, DocumentMode::SideBySide);
    assert!(shown(&app).1);
    app.click("doc-link-scrolling");
    assert!(!shown(&app).1);
    app.click("doc-link-scrolling");
    assert!(shown(&app).1);
}

/// The page lights a draft and the box being written, never a sent comment:
/// a sent one shows on its source line only.
#[test]
fn the_page_marks_where_a_comment_is() {
    let mut app = doc_app();
    open_document(&mut app, None);
    comment_on(&mut app, 7, 7, "Where are the sizes from?");
    app.send(AppAction::Comments(C::OpenBox(session(), anchor(13, 14))));
    app.send(AppAction::Comments(C::Typed(
        session(),
        "Keep both until range p95 holds for a week on devbox-1.".into(),
    )));
    let held = app.fakes.comments.comments(&session());
    let sent = held
        .iter()
        .find(|comment| comment.anchor.first == 7)
        .expect("the sent comment is held");
    let draft = held
        .iter()
        .find(|comment| comment.anchor.first == 13)
        .expect("the draft is held");
    assert_eq!(sent.status, CommentStatus::Sent);
    assert_eq!(draft.status, CommentStatus::Draft);
    assert!(app.contains(&format!("md-comment-{}", draft.id)));
    assert!(!app.contains(&format!("md-comment-{}", sent.id)));
}

/// Flow 26.14 step 3 for the editor: Settings › Long lines switches the open
/// source at once, one visual row per line on Scroll, wrapped on Wrap.
#[test]
fn long_lines_switch_the_open_source_between_wrap_and_scroll() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let long = format!("Long: {}", "every shard keeps its own range index ".repeat(12));
    workspace.file_contents.insert(
        (session(), String::new(), PATH.to_owned()),
        format!("# RFC\n\n{long}\n\nEnd.\n"),
    );
    let mut app = App::new(workspace);
    open_document(&mut app, None);
    show(&mut app, DocumentMode::Source);
    let height = |app: &mut App| {
        app.draw();
        app.cx
            .update_window(app.window, |_, window, _| {
                f32::from(window.find("doc-line-3").bounds().size.height)
            })
            .unwrap()
    };
    let wrapped = app.ui.read(|s| s.review.wrap);
    if !wrapped {
        app.send(AppAction::Settings(humata2_settings::SettingsAction::ToggleWrap));
    }
    let tall = height(&mut app);
    assert!(tall > 30., "a wrapped long line takes several rows: {tall}");
    app.send(AppAction::Settings(humata2_settings::SettingsAction::ToggleWrap));
    let one = height(&mut app);
    assert!(one < 24., "a scrolled long line is one row: {one}");
}
