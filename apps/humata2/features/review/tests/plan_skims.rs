//! D802, DS-67: a review plan's list with each file's tier. Skim files leave
//! the rows and their groups and fold into one row per repository, after
//! its last row; a group left without files draws no header. With no order
//! sent, key files come first in their repository; with one, they stay
//! where it put them.
use std::collections::HashMap;
use std::sync::Arc;

use humata2_clients::Workspace;
use humata2_clients::model::{FileDiff, RepoDiff, SessionId};
use humata2_clients::views::{PlanGroup, PlanView};
use humata2_review::stack::Tier;
use humata2_review::{Fetch, PlanLayout, PlanRow, PlanSkim, ReviewState};

fn session() -> SessionId {
    SessionId::new("ledger")
}

fn diff(repo: &str, paths: &[&str]) -> RepoDiff {
    RepoDiff {
        session: session(),
        repo: repo.to_owned(),
        base: "origin/main".to_owned(),
        files: paths
            .iter()
            .map(|path| FileDiff {
                path: (*path).to_owned(),
                added: 1,
                ..FileDiff::default()
            })
            .collect(),
        comments: Vec::new(),
    }
}

fn entry(repo: &str, path: &str) -> (String, String) {
    (repo.to_owned(), path.to_owned())
}

fn file_row(repo: &str, path: &str) -> PlanRow {
    PlanRow::File {
        repo: repo.to_owned(),
        path: path.to_owned(),
        notes: Vec::new(),
    }
}

fn skim(repo: &str, paths: &[&str], after: usize) -> PlanSkim {
    PlanSkim {
        repo: repo.to_owned(),
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        after,
    }
}

fn page(diffs: Vec<RepoDiff>, plan: PlanView) -> ReviewState {
    ReviewState {
        workspace: Arc::new(Workspace::default()),
        session: Some(session()),
        fetch: HashMap::from([(
            session(),
            Fetch {
                diffs: Some(diffs),
                ..Fetch::default()
            },
        )]),
        plans: HashMap::from([(session(), plan)]),
        ..ReviewState::default()
    }
}

/// Lockfiles are skim, `*.key.rs` files are key, the rest normal.
fn tiers(_repo: &str, file: &FileDiff) -> Tier {
    if file.path.ends_with(".lock") || file.path.ends_with("-lock.json") {
        Tier::Skim
    } else if file.path.ends_with(".key.rs") {
        Tier::Key
    } else {
        Tier::Normal
    }
}

#[test]
fn skim_files_fold_into_one_row_after_their_repositorys_last_row() {
    let state = page(
        vec![
            diff("acme/ledger", &["Cargo.lock", "src/ledger.rs", "MODULE.bazel.lock"]),
            diff("acme/web", &["package-lock.json", "src/app.ts"]),
        ],
        PlanView {
            order: vec![
                entry("acme/ledger", "Cargo.lock"),
                entry("acme/ledger", "src/ledger.rs"),
                entry("acme/web", "src/app.ts"),
            ],
            ..PlanView::default()
        },
    );

    assert_eq!(
        state.plan_layout_with(&session(), tiers),
        Some(PlanLayout {
            rows: vec![file_row("acme/ledger", "src/ledger.rs"), file_row("acme/web", "src/app.ts")],
            skims: vec![
                skim("acme/ledger", &["Cargo.lock", "MODULE.bazel.lock"], 1),
                skim("acme/web", &["package-lock.json"], 2),
            ],
        })
    );
}

#[test]
fn a_group_left_without_files_draws_no_header() {
    let state = page(
        vec![
            diff("acme/ledger", &["Cargo.lock"]),
            diff("acme/api", &["src/client.rs"]),
        ],
        PlanView {
            order: vec![entry("acme/ledger", "Cargo.lock"), entry("acme/api", "src/client.rs")],
            groups: vec![
                PlanGroup {
                    title: "Locks".to_owned(),
                    note: String::new(),
                    first: entry("acme/ledger", "Cargo.lock"),
                },
                PlanGroup {
                    title: "Client".to_owned(),
                    note: String::new(),
                    first: entry("acme/api", "src/client.rs"),
                },
            ],
            ..PlanView::default()
        },
    );

    let layout = state.plan_layout_with(&session(), tiers).expect("a plan");
    assert_eq!(
        layout.rows,
        vec![
            PlanRow::Group {
                title: "Client".to_owned(),
                note: String::new(),
            },
            file_row("acme/api", "src/client.rs"),
        ]
    );
    // The repository has no other rows, so its fold comes last.
    assert_eq!(layout.skims, vec![skim("acme/ledger", &["Cargo.lock"], 2)]);
}

#[test]
fn a_groups_header_waits_for_its_first_file_that_is_not_skim() {
    let state = page(
        vec![diff("acme/ledger", &["Cargo.lock", "src/ledger.rs"])],
        PlanView {
            order: vec![entry("acme/ledger", "Cargo.lock"), entry("acme/ledger", "src/ledger.rs")],
            groups: vec![PlanGroup {
                title: "Ledger".to_owned(),
                note: String::new(),
                first: entry("acme/ledger", "Cargo.lock"),
            }],
            ..PlanView::default()
        },
    );

    let layout = state.plan_layout_with(&session(), tiers).expect("a plan");
    assert_eq!(
        layout.rows,
        vec![
            PlanRow::Group {
                title: "Ledger".to_owned(),
                note: String::new(),
            },
            file_row("acme/ledger", "src/ledger.rs"),
        ]
    );
}

#[test]
fn with_no_order_key_files_come_first_in_their_repository() {
    let state = page(
        vec![
            diff("acme/ledger", &["src/a.rs", "src/b.rs", "src/c.key.rs"]),
            diff("acme/api", &["src/d.rs", "src/e.key.rs"]),
        ],
        PlanView::default(),
    );

    let layout = state.plan_layout_with(&session(), tiers).expect("a plan");
    assert_eq!(
        layout.rows,
        vec![
            file_row("acme/ledger", "src/c.key.rs"),
            file_row("acme/ledger", "src/a.rs"),
            file_row("acme/ledger", "src/b.rs"),
            file_row("acme/api", "src/e.key.rs"),
            file_row("acme/api", "src/d.rs"),
        ]
    );
}

#[test]
fn with_an_order_key_files_stay_where_it_put_them() {
    let state = page(
        vec![diff("acme/ledger", &["src/a.rs", "src/c.key.rs", "src/b.rs"])],
        PlanView {
            order: vec![entry("acme/ledger", "src/a.rs")],
            ..PlanView::default()
        },
    );

    let layout = state.plan_layout_with(&session(), tiers).expect("a plan");
    assert_eq!(
        layout.rows,
        vec![
            file_row("acme/ledger", "src/a.rs"),
            file_row("acme/ledger", "src/c.key.rs"),
            file_row("acme/ledger", "src/b.rs"),
        ]
    );
}

#[test]
fn a_session_without_a_plan_has_no_layout() {
    let mut state = page(vec![diff("acme/ledger", &["Cargo.lock"])], PlanView::default());
    state.plans.clear();
    assert_eq!(state.plan_layout_with(&session(), tiers), None);
}
