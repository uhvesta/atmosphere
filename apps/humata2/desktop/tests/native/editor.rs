//! RV-17–RV-22: a changed code file open as a session tab. Inline, Side by
//! side and Full file; unchanged stretches fold and unfold; ↑/↓ step through
//! the changes with a count; Show changes turns the inline changes off.
use super::*;
use humata2_clients::model::{DiffLine, FileDiff, LineKind};
use humata2_session::editor::CodeMode;

fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

/// Ledger's api/src/client.rs is `l1` to `l12`: line 5 rewritten (it was
/// `old5`), line 7 added, the old line 11 (`gone`) removed.
pub(super) fn editor_app() -> App {
    let mut workspace = fixture::design();
    let context = |old, new| line(LineKind::Context, Some(old), Some(new), &format!("l{new}"));
    let diff = FileDiff {
        path: "src/client.rs".into(),
        added: 2,
        removed: 2,
        lines: vec![
            line(LineKind::Hunk, None, None, "@@ -4,4 +4,5 @@"),
            context(4, 4),
            line(LineKind::Removed, Some(5), None, "old5"),
            line(LineKind::Added, None, Some(5), "l5"),
            context(6, 6),
            line(LineKind::Added, None, Some(7), "l7"),
            context(7, 8),
            line(LineKind::Hunk, None, None, "@@ -10,2 +11,1 @@"),
            context(10, 11),
            line(LineKind::Removed, Some(11), None, "gone"),
        ],
        ..FileDiff::default()
    };
    let api = workspace
        .diffs
        .iter_mut()
        .find(|repo| repo.session == SessionId::new("ledger") && repo.repo == "api")
        .expect("Ledger changes api");
    api.files = vec![diff];
    api.comments.clear();
    workspace.file_contents.insert(
        (SessionId::new("ledger"), "api".into(), "src/client.rs".into()),
        (1..=12).map(|n| format!("l{n}\n")).collect(),
    );
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Files);
    show_client(&mut app);
    app
}

/// Shows api/src/client.rs as a tab, opening it if it isn't open yet.
pub(super) fn show_client(app: &mut App) {
    app.send(AppAction::Session(SessionAction::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.send(AppAction::Session(SessionAction::OpenFile));
    app.settle();
}

pub(super) fn text(app: &mut App) -> String {
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| view::capture(window))
        .expect("window")
        .expect("view")
}

fn mode(app: &App) -> CodeMode {
    app.ui.read(|state| {
        state
            .session
            .view()
            .map(|view| view.code("api", "src/client.rs").mode)
            .unwrap_or_default()
    })
}

#[test]
fn a_changed_file_opens_inline_with_folds_that_open_and_stay_open() {
    let mut app = editor_app();
    assert!(app.contains("editor-modes"));
    assert_eq!(mode(&app), CodeMode::Inline);
    assert!(app.contains("editor-fold-1"));
    assert!(app.contains("editor-fold-9"));
    assert!(app.contains("editor-fold-12"));
    let drawn = text(&mut app);
    assert!(drawn.contains("old5"), "{drawn}");
    assert!(drawn.contains("2 unchanged lines"), "{drawn}");
    assert!(!drawn.contains("l9"), "{drawn}");
    app.snapshot("editor-inline");

    app.click("editor-fold-9");
    assert!(!app.contains("editor-fold-9"));
    let drawn = text(&mut app);
    assert!(drawn.contains("l9") && drawn.contains("l10"), "{drawn}");

    // The terminal and back: the stretch stays open.
    app.send(AppAction::Session(SessionAction::SelectTab(0)));
    assert!(!app.contains("editor-modes"));
    show_client(&mut app);
    assert!(!app.contains("editor-fold-9"));
    assert!(app.contains("editor-fold-1"));
}

#[test]
fn up_and_down_step_through_the_changes_and_wrap_around() {
    let mut app = editor_app();
    assert!(text(&mut app).contains("3 changes"));
    app.click("editor-next-change");
    assert!(text(&mut app).contains("1 of 3"));
    app.click("editor-next-change");
    app.click("editor-next-change");
    assert!(text(&mut app).contains("3 of 3"));
    app.click("editor-next-change");
    assert!(text(&mut app).contains("1 of 3"));
    app.click("editor-previous-change");
    assert!(text(&mut app).contains("3 of 3"));
}

#[test]
fn side_by_side_puts_removed_lines_beside_the_added_ones() {
    let mut app = editor_app();
    app.click("editor-mode-side-by-side");
    assert_eq!(mode(&app), CodeMode::SideBySide);
    assert!(app.contains("editor-fold-9"));
    assert!(text(&mut app).contains("3 changes"));
    app.snapshot("editor-side-by-side");
}

#[test]
fn full_file_shows_everything_and_show_changes_turns_the_changes_off() {
    let mut app = editor_app();
    app.click("editor-mode-full-file");
    assert_eq!(mode(&app), CodeMode::FullFile);
    assert!(!app.contains("editor-fold-1"));
    assert!(!app.contains("editor-fold-9"));
    assert!(app.contains("editor-show-changes"));
    let drawn = text(&mut app);
    assert!(drawn.contains("l9") && drawn.contains("old5") && drawn.contains("gone"), "{drawn}");
    assert!(drawn.contains("3 changes"), "{drawn}");
    app.snapshot("editor-full-file");

    app.click("editor-show-changes");
    let drawn = text(&mut app);
    assert!(!drawn.contains("old5") && !drawn.contains("gone"), "{drawn}");
    assert!(drawn.contains("l12"), "{drawn}");
    assert!(!app.contains("editor-next-change"));
}

#[test]
fn an_unchanged_file_shows_as_a_plain_file_without_modes() {
    let mut workspace = fixture::design();
    workspace.file_contents.insert(
        (SessionId::new("ledger"), "api".into(), "src/plain.rs".into()),
        "fn plain() {}\n".into(),
    );
    workspace
        .session_files
        .entry(SessionId::new("ledger"))
        .or_default()
        .push(humata2_clients::model::SessionFile {
            repo: "api".into(),
            path: "src/plain.rs".into(),
            folder: false,
        });
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Files);
    app.send(AppAction::Session(SessionAction::SelectFile {
        repo: "api".into(),
        path: "src/plain.rs".into(),
    }));
    app.send(AppAction::Session(SessionAction::OpenFile));
    app.settle();
    assert!(app.contains("session-file"));
    assert!(!app.contains("editor-modes"));
    assert!(text(&mut app).contains("fn plain() {}"));
}
