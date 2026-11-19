//! RV-17–RV-22: a changed code file in the editor tab. Its diff and current
//! text become rows (Inline, Side by side, Full file), unchanged stretches
//! fold, and ↑/↓ step through the changes. The settings are kept per file
//! and per session.
use humata2_clients::model::{DiffLine, FileDiff, LineKind, SessionId};
use humata2_clients::{Clients, fixture};
use humata2_session::editor::{self, CodeMode, CodeView, Row, SideRow};
use humata2_session::{SessionAction as A, SessionState, SessionView, Sessions};
use std::sync::Arc;
use tca::{TestClock, TestStore};

/// The current file: `l1` to `l12`.
fn current() -> String {
    (1..=12).map(|n| format!("l{n}\n")).collect()
}

fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
    DiffLine {
        kind,
        old,
        new,
        text: text.to_owned(),
    }
}

fn context(old: u32, new: u32) -> DiffLine {
    line(LineKind::Context, Some(old), Some(new), &format!("l{new}"))
}

fn added(new: u32) -> DiffLine {
    line(LineKind::Added, None, Some(new), &format!("l{new}"))
}

fn removed(old: u32, text: &str) -> DiffLine {
    line(LineKind::Removed, Some(old), None, text)
}

fn hunk(text: &str) -> DiffLine {
    line(LineKind::Hunk, None, None, text)
}

/// Line 5 rewritten, line 7 added, the old line 11 ("gone") removed; two
/// hunks, lines 1–3, 9–10 and 12 not in them.
fn diff() -> FileDiff {
    FileDiff {
        path: "src/client.rs".into(),
        added: 2,
        removed: 2,
        lines: vec![
            hunk("@@ -4,4 +4,5 @@"),
            context(4, 4),
            removed(5, "old5"),
            added(5),
            context(6, 6),
            added(7),
            context(7, 8),
            hunk("@@ -10,2 +11,1 @@"),
            context(10, 11),
            removed(11, "gone"),
        ],
        ..FileDiff::default()
    }
}

fn fold(start: u32, count: u32) -> Row {
    Row::Fold { start, count }
}

fn row(line: DiffLine) -> Row {
    Row::Line(line)
}

fn inline() -> Vec<Row> {
    vec![
        fold(1, 3),
        row(context(4, 4)),
        row(removed(5, "old5")),
        row(added(5)),
        row(context(6, 6)),
        row(added(7)),
        row(context(7, 8)),
        fold(9, 2),
        row(context(10, 11)),
        row(removed(11, "gone")),
        fold(12, 1),
    ]
}

#[test]
fn inline_interleaves_the_changes_and_folds_what_the_hunks_leave_out() {
    let rows = editor::rows(Some(&diff()), &current(), &CodeView::default());
    assert_eq!(rows, inline());
    assert_eq!(CodeView::default().mode, CodeMode::Inline);
}

#[test]
fn an_unfolded_stretch_shows_its_lines_with_both_numbers() {
    let view = CodeView {
        unfolded: [9].into(),
        ..CodeView::default()
    };
    let rows = editor::rows(Some(&diff()), &current(), &view);
    let mut expected = inline();
    expected.splice(7..8, [row(context(8, 9)), row(context(9, 10))]);
    assert_eq!(rows, expected);
}

#[test]
fn full_file_folds_nothing_and_keeps_deleted_lines_in_place() {
    let view = CodeView {
        mode: CodeMode::FullFile,
        ..CodeView::default()
    };
    let rows = editor::rows(Some(&diff()), &current(), &view);
    assert!(rows.iter().all(|row| matches!(row, Row::Line(_))));
    let texts: Vec<_> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Line(line) => Some(line.text.as_str()),
            Row::Fold { .. } => None,
        })
        .collect();
    assert_eq!(
        texts,
        [
            "l1", "l2", "l3", "l4", "old5", "l5", "l6", "l7", "l8", "l9", "l10", "l11", "gone",
            "l12"
        ]
    );
    assert_eq!(editor::changes(&rows).len(), 3);
}

#[test]
fn full_file_without_changes_is_the_plain_current_file() {
    let view = CodeView {
        mode: CodeMode::FullFile,
        changes: false,
        ..CodeView::default()
    };
    let rows = editor::rows(Some(&diff()), &current(), &view);
    let plain: Vec<Row> = (1..=12)
        .map(|n| row(line(LineKind::Context, None, Some(n), &format!("l{n}"))))
        .collect();
    assert_eq!(rows, plain);
    assert!(editor::changes(&rows).is_empty());
}

#[test]
fn an_unchanged_file_is_a_plain_file_in_every_mode() {
    for mode in [CodeMode::Inline, CodeMode::SideBySide, CodeMode::FullFile] {
        let view = CodeView {
            mode,
            ..CodeView::default()
        };
        let rows = editor::rows(None, &current(), &view);
        assert_eq!(rows.len(), 12);
        assert!(editor::changes(&rows).is_empty());
    }
}

#[test]
fn each_run_of_changed_lines_is_one_change() {
    let rows = inline();
    assert_eq!(editor::changes(&rows), [2, 5, 9]);
}

#[test]
fn side_by_side_pairs_removed_lines_with_the_added_lines_after_them() {
    let pair = |old: Option<DiffLine>, new: Option<DiffLine>| SideRow::Pair { old, new };
    let same = |old, new| pair(Some(context(old, new)), Some(context(old, new)));
    assert_eq!(
        editor::side_by_side(&inline()),
        [
            SideRow::Fold { start: 1, count: 3 },
            same(4, 4),
            pair(Some(removed(5, "old5")), Some(added(5))),
            same(6, 6),
            pair(None, Some(added(7))),
            same(7, 8),
            SideRow::Fold { start: 9, count: 2 },
            same(10, 11),
            pair(Some(removed(11, "gone")), None),
            SideRow::Fold { start: 12, count: 1 },
        ]
    );
    assert_eq!(editor::side_changes(&editor::side_by_side(&inline())), [2, 4, 8]);
}

#[test]
fn a_current_file_shorter_than_the_diff_says_still_draws() {
    let rows = editor::rows(Some(&diff()), "l1\nl2\n", &CodeView::default());
    assert_eq!(editor::changes(&rows).len(), 3);
}

// The reducer: settings per file and session, kept across saves; ↑/↓ wrap.

fn store() -> TestStore<Sessions> {
    let workspace = fixture::design();
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        ..Default::default()
    };
    state.open = Some(SessionId::new("ledger"));
    state
        .views
        .insert(SessionId::new("ledger"), SessionView::default());
    state
        .views
        .insert(SessionId::new("other"), SessionView::default());
    TestStore::new(state, Sessions::new(clients))
}

const PATH: &str = "src/client.rs";

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

fn code(state: &SessionState, session: &SessionId, path: &str) -> CodeView {
    state.views[session].code("api", path)
}

#[test]
fn mode_show_changes_and_folds_are_kept_for_that_file_and_session_only() {
    let mut store = store();
    store.send(
        A::CodeMode {
            repo: "api".into(),
            path: PATH.into(),
            mode: CodeMode::FullFile,
        },
        |s| {
            let view = s.views.get_mut(&ledger()).unwrap();
            let mut code = view.code("api", PATH);
            code.mode = CodeMode::FullFile;
            view.set_code("api", PATH, code);
        },
    );
    store.send(
        A::ShowChanges {
            repo: "api".into(),
            path: PATH.into(),
            shown: false,
        },
        |s| {
            let view = s.views.get_mut(&ledger()).unwrap();
            let mut code = view.code("api", PATH);
            code.changes = false;
            view.set_code("api", PATH, code);
        },
    );
    store.send(
        A::Unfold {
            repo: "api".into(),
            path: PATH.into(),
            start: 9,
        },
        |s| {
            let view = s.views.get_mut(&ledger()).unwrap();
            let mut code = view.code("api", PATH);
            code.unfolded.insert(9);
            view.set_code("api", PATH, code);
        },
    );
    let state = store.state();
    let picked = code(&state, &ledger(), PATH);
    assert_eq!(picked.mode, CodeMode::FullFile);
    assert!(!picked.changes);
    assert!(picked.unfolded.contains(&9));
    assert_eq!(code(&state, &ledger(), "src/entry.rs"), CodeView::default());
    assert_eq!(
        code(&state, &SessionId::new("other"), PATH),
        CodeView::default()
    );

    let json = serde_json::to_string(&state.views[&ledger()]).unwrap();
    let back: SessionView = serde_json::from_str(&json).unwrap();
    assert_eq!(back.code("api", PATH), picked);
}

#[test]
fn a_view_saved_before_the_editor_loads_with_defaults() {
    let saved = r#"{"tab":0,"panel":null,"widened":false,"draft":"","queue":[]}"#;
    let view: SessionView = serde_json::from_str(saved).unwrap();
    let code = view.code("api", PATH);
    assert_eq!(code, CodeView::default());
    assert!(code.changes);
    assert!(code.unfolded.is_empty());
}

fn step(forward: bool, count: usize) -> A {
    A::StepChange {
        repo: "api".into(),
        path: PATH.into(),
        forward,
        count,
    }
}

fn at(store: &TestStore<Sessions>) -> Option<usize> {
    store.state().change(&ledger(), "api", PATH)
}

#[test]
fn stepping_goes_to_the_next_change_and_wraps_around() {
    let mut store = store().non_exhaustive();
    store.send(step(true, 3), |_| {});
    assert_eq!(at(&store), Some(0));
    store.send(step(true, 3), |_| {});
    store.send(step(true, 3), |_| {});
    assert_eq!(at(&store), Some(2));
    store.send(step(true, 3), |_| {});
    assert_eq!(at(&store), Some(0));
    store.send(step(false, 3), |_| {});
    assert_eq!(at(&store), Some(2));
}

#[test]
fn stepping_back_first_goes_to_the_last_change_and_no_changes_go_nowhere() {
    let mut store = store().non_exhaustive();
    store.send(step(false, 3), |_| {});
    assert_eq!(at(&store), Some(2));
    let mut empty = self::store().non_exhaustive();
    empty.send(step(true, 0), |_| {});
    assert_eq!(at(&empty), None);
}
