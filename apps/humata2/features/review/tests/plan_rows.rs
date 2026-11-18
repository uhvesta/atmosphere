//! RV-32, P8: a review plan's rows follow its order, each group's title and
//! note before its first file and each file's notes on it. Files of the diff
//! that the order leaves out come last; order entries the diff doesn't have
//! are skipped. A session without a plan has no rows.
use std::collections::HashMap;
use std::sync::Arc;

use humata2_clients::Workspace;
use humata2_clients::model::{FileDiff, RepoDiff, SessionId};
use humata2_clients::views::{PlanGroup, PlanNote, PlanView};
use humata2_review::{Fetch, PlanRow, PlanRowNote, ReviewState};

fn session() -> SessionId {
    SessionId::new("ledger")
}

/// `paths` changed in `repo`, for `of`.
fn diff_of(of: &SessionId, repo: &str, paths: &[&str]) -> RepoDiff {
    RepoDiff {
        session: of.clone(),
        repo: repo.to_owned(),
        base: "origin/main".to_owned(),
        files: paths
            .iter()
            .map(|path| FileDiff {
                path: (*path).to_owned(),
                added: 1,
                removed: 0,
                lines: Vec::new(),
                ..FileDiff::default()
            })
            .collect(),
        comments: Vec::new(),
    }
}

fn diff(repo: &str, paths: &[&str]) -> RepoDiff {
    diff_of(&session(), repo, paths)
}

fn entry(repo: &str, path: &str) -> (String, String) {
    (repo.to_owned(), path.to_owned())
}

fn group(title: &str, note: &str, first: (String, String)) -> PlanGroup {
    PlanGroup {
        title: title.to_owned(),
        note: note.to_owned(),
        first,
    }
}

fn note(repo: &str, path: &str, text: &str) -> PlanNote {
    PlanNote {
        repo: repo.to_owned(),
        path: path.to_owned(),
        text: text.to_owned(),
        lines: None,
        outdated: false,
    }
}

fn line_note(repo: &str, path: &str, text: &str, lines: (u32, u32), outdated: bool) -> PlanNote {
    PlanNote {
        lines: Some(lines),
        outdated,
        ..note(repo, path, text)
    }
}

fn row_note(text: &str) -> PlanRowNote {
    PlanRowNote {
        text: text.to_owned(),
        lines: None,
        outdated: false,
    }
}

fn group_row(title: &str, note: &str) -> PlanRow {
    PlanRow::Group {
        title: title.to_owned(),
        note: note.to_owned(),
    }
}

fn file_row(repo: &str, path: &str, notes: &[&str]) -> PlanRow {
    PlanRow::File {
        repo: repo.to_owned(),
        path: path.to_owned(),
        notes: notes.iter().map(|text| row_note(text)).collect(),
    }
}

/// The review page with `diffs` read for the session, and `plan` set on it
/// when there is one. Another session's changes are in the workspace too.
fn page(diffs: Vec<RepoDiff>, plan: Option<PlanView>) -> ReviewState {
    let mut held = diffs.clone();
    held.push(diff_of(
        &SessionId::new("brin"),
        "acme/ledger",
        &["migrations/0099_other.sql"],
    ));
    let workspace = Workspace {
        diffs: held,
        ..Workspace::default()
    };
    ReviewState {
        workspace: Arc::new(workspace),
        session: Some(session()),
        fetch: HashMap::from([(
            session(),
            Fetch {
                diffs: Some(diffs),
                ..Fetch::default()
            },
        )]),
        plans: plan.into_iter().map(|plan| (session(), plan)).collect(),
        ..ReviewState::default()
    }
}

#[test]
fn rows_follow_the_order_with_each_groups_title_and_each_files_notes() {
    let state = page(
        vec![
            diff(
                "acme/ledger",
                &["migrations/0042_accounts.sql", "migrations/0043_index.sql"],
            ),
            diff("acme/api", &["src/client.rs"]),
        ],
        Some(PlanView {
            targets: Vec::new(),
            order: vec![
                entry("acme/api", "src/client.rs"),
                entry("acme/ledger", "migrations/0042_accounts.sql"),
                entry("acme/ledger", "migrations/0043_index.sql"),
            ],
            groups: vec![
                group("Client", "Start here.", entry("acme/api", "src/client.rs")),
                group(
                    "Schema",
                    "The risky part.",
                    entry("acme/ledger", "migrations/0042_accounts.sql"),
                ),
            ],
            notes: vec![
                note("acme/api", "src/client.rs", "One transaction."),
                note("acme/ledger", "migrations/0043_index.sql", "Built concurrently."),
                note("acme/api", "src/client.rs", "The retry is safe."),
            ],
        }),
    );

    assert_eq!(
        state.plan_rows(&session()),
        Some(vec![
            group_row("Client", "Start here."),
            file_row(
                "acme/api",
                "src/client.rs",
                &["One transaction.", "The retry is safe."],
            ),
            group_row("Schema", "The risky part."),
            file_row("acme/ledger", "migrations/0042_accounts.sql", &[]),
            file_row(
                "acme/ledger",
                "migrations/0043_index.sql",
                &["Built concurrently."],
            ),
        ])
    );
}

#[test]
fn files_the_order_leaves_out_come_last() {
    let state = page(
        vec![
            diff("acme/ledger", &["migrations/0042_accounts.sql"]),
            diff("acme/api", &["src/client.rs", "src/entry.rs"]),
        ],
        Some(PlanView {
            targets: Vec::new(),
            order: vec![
                entry("acme/api", "src/client.rs"),
                entry("acme/ledger", "migrations/0042_accounts.sql"),
            ],
            groups: Vec::new(),
            notes: Vec::new(),
        }),
    );

    assert_eq!(
        state.plan_rows(&session()),
        Some(vec![
            file_row("acme/api", "src/client.rs", &[]),
            file_row("acme/ledger", "migrations/0042_accounts.sql", &[]),
            file_row("acme/api", "src/entry.rs", &[]),
        ])
    );
}

#[test]
fn order_entries_missing_from_the_diff_are_skipped() {
    let state = page(
        vec![
            diff("acme/ledger", &["migrations/0042_accounts.sql"]),
            diff("acme/api", &["src/client.rs"]),
        ],
        Some(PlanView {
            targets: Vec::new(),
            order: vec![
                entry("acme/ledger", "migrations/0042_accounts.sql"),
                entry("acme/web", "src/form.tsx"),
                entry("acme/api", "src/gone.rs"),
                entry("acme/api", "src/client.rs"),
            ],
            groups: Vec::new(),
            notes: vec![note("acme/api", "src/gone.rs", "Deleted upstream.")],
        }),
    );

    assert_eq!(
        state.plan_rows(&session()),
        Some(vec![
            file_row("acme/ledger", "migrations/0042_accounts.sql", &[]),
            file_row("acme/api", "src/client.rs", &[]),
        ])
    );
}

#[test]
fn a_session_without_a_plan_has_no_rows() {
    let diffs = vec![diff("acme/api", &["src/client.rs"])];
    assert_eq!(page(diffs.clone(), None).plan_rows(&session()), None);

    let planned = page(
        diffs,
        Some(PlanView {
            targets: Vec::new(),
            order: vec![entry("acme/api", "src/client.rs")],
            groups: Vec::new(),
            notes: Vec::new(),
        }),
    );
    assert_eq!(planned.plan_rows(&SessionId::new("brin")), None);
}

#[test]
fn a_line_note_carries_its_lines_and_an_outdated_one_is_marked() {
    let state = page(
        vec![diff("acme/api", &["src/client.rs"])],
        Some(PlanView {
            targets: Vec::new(),
            order: vec![entry("acme/api", "src/client.rs")],
            groups: Vec::new(),
            notes: vec![
                line_note("acme/api", "src/client.rs", "Moved down.", (12, 14), false),
                line_note("acme/api", "src/client.rs", "Rewritten since.", (3, 3), true),
                note("acme/api", "src/client.rs", "About the file."),
            ],
        }),
    );

    assert_eq!(
        state.plan_rows(&session()),
        Some(vec![PlanRow::File {
            repo: "acme/api".to_owned(),
            path: "src/client.rs".to_owned(),
            notes: vec![
                PlanRowNote {
                    text: "Moved down.".to_owned(),
                    lines: Some((12, 14)),
                    outdated: false,
                },
                PlanRowNote {
                    text: "Rewritten since.".to_owned(),
                    lines: Some((3, 3)),
                    outdated: true,
                },
                row_note("About the file."),
            ],
        }])
    );
}
