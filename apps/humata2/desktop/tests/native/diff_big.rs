//! A very long diff opens quickly (DV-5): the code view builds only the
//! lines in view, still reaches the file's last line, and wraps a huge line.
use super::*;
use humata2_clients::model::{DiffLine, FileDiff, LineKind};
use humata2_review::ReviewAction as R;

const VIEW: &str = "change-code api/src/client.rs";

/// The "ledger" session's `api` repository with `src/client.rs` replaced by
/// `lines` and selected in the Diff panel.
fn app_with(lines: Vec<DiffLine>) -> App {
    let mut workspace = fixture::design();
    let repo = workspace.diffs.iter_mut().find(|d| d.repo == "api").unwrap();
    let file = repo.files.iter_mut().find(|f| f.path == "src/client.rs").unwrap();
    *file = FileDiff {
        added: lines.iter().filter(|l| l.kind == LineKind::Added).count() as u32,
        removed: lines.iter().filter(|l| l.kind == LineKind::Removed).count() as u32,
        lines,
        ..file.clone()
    };
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    app.draw();
    app
}

/// A hunk removing 20,000 lines and adding 20,000: line 0 is the hunk
/// header, 1..=20000 the removed lines and 20001..=40000 the added ones.
fn twenty_thousand() -> Vec<DiffLine> {
    let mut lines = vec![DiffLine {
        kind: LineKind::Hunk,
        old: None,
        new: None,
        text: "@@ -1,20000 +1,20000 @@".into(),
    }];
    lines.extend((1..=20_000).map(|n| DiffLine {
        kind: LineKind::Removed,
        old: Some(n),
        new: None,
        text: format!("let old_{n} = {n};"),
    }));
    lines.extend((1..=20_000).map(|n| DiffLine {
        kind: LineKind::Added,
        old: None,
        new: Some(n),
        text: format!("let new_{n} = {n};"),
    }));
    lines
}

/// How many line rows the code view drew in the last frame.
fn drawn_lines(app: &mut App) -> usize {
    app.cx
        .update_window(app.window, |_, window, _| {
            window
                .within(VIEW)
                .elements()
                .iter()
                .filter(|element| {
                    element
                        .path()
                        .last()
                        .is_some_and(|id| format!("{id:?}").contains("change-line-"))
                })
                .count()
        })
        .unwrap()
}

/// 40,000 changed lines open with only a screenful of rows built.
#[test]
fn a_huge_diff_draws_only_the_lines_in_view() {
    let mut app = app_with(twenty_thousand());
    assert!(app.contains("change-line-1"), "the first lines are drawn");
    let drawn = drawn_lines(&mut app);
    assert!(drawn > 0 && drawn < 200, "drew {drawn} line rows");
}

/// Scrolling to the end of the huge diff reaches the last added line.
#[test]
fn a_huge_diff_scrolls_to_its_last_line() {
    let mut app = app_with(twenty_thousand());
    for _ in 0..3 {
        app.cx
            .update_window(app.window, |_, window, cx| {
                window.scroll(
                    VIEW,
                    gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-10_000_000.))),
                    cx,
                );
            })
            .unwrap();
        app.draw();
    }
    assert!(app.contains("change-line-40000"), "the last line is drawn");
    assert!(!app.contains("change-line-1"), "the first lines are gone");
    let drawn = drawn_lines(&mut app);
    assert!(drawn < 200, "drew {drawn} line rows at the end");
    app.snapshot("diff-big-40k-end");
}

/// With Wrap on, a 50,000-character line in a long file wraps into rows.
#[test]
fn a_huge_line_wraps_in_a_long_file() {
    let mut lines = twenty_thousand();
    lines.truncate(1_500);
    lines[1].text = "x".repeat(50_000);
    let mut app = app_with(lines);
    // Wrap is on by default (ST-7); make sure of it.
    let wrap = app.ui.read(|state| state.review.wrap);
    if !wrap {
        app.send(AppAction::Review(R::ToggleWrap));
    }
    assert!(app.contains("change-code-1-0"), "the huge line's first row");
    assert!(app.contains("change-code-1-1"), "the huge line wraps");
}
