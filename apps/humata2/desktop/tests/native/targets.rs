//! RV-31, frames 84 to 86: each repository of the Diff panel has its own
//! base and menu. Ledger's design session has two repos: ledger first, then
//! api.
use super::*;
use humata2_clients::model::RepoBase;
use humata2_review::ReviewAction as R;

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

/// Ledger's Diff panel beside the terminal, api's client.rs shown.
fn changes_app() -> App {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    // A picked file folds the list into the file bar; these tests use its rows.
    app.send(AppAction::Review(R::ToggleFileList));
    app
}

fn base_of(app: &App, repo: &str) -> RepoBase {
    app.ui.read(|state| state.review.targets_of(&ledger()).of(repo))
}

#[test]
fn a_repository_gets_its_own_base_by_clicks_and_a_row_puts_every_repo_on_its_base() {
    let mut app = changes_app();
    assert!(app.contains("repo-base-0"));
    assert!(app.contains("repo-base-1"));
    assert!(!app.contains("diff-base-per-repo"));
    assert!(!app.contains("repo-base-menu"));

    // api's menu: its branches as the clone last fetched them.
    app.click("repo-base-1");
    assert!(app.contains("repo-base-menu"));
    assert!(app.contains("repo-base-default"));
    assert!(app.contains("repo-base-pick-0"));
    assert!(app.contains("repo-base-rev"));
    app.click("repo-base-pick-0");
    assert!(!app.contains("repo-base-menu"), "a pick closes the menu");
    assert!(matches!(base_of(&app, "api"), RepoBase::Branch(_)));
    assert_eq!(base_of(&app, "ledger"), RepoBase::DefaultBranch, "ledger keeps its base");
    assert!(app.ui.read(|state| state.review.targets_of(&ledger()).per_repo()));
    assert!(!app.contains("diff-base"), "two repositories have no header base");

    // ledger's menu: a commit typed into the filter.
    app.click("repo-base-0");
    assert!(app.contains("repo-base-menu"));
    app.send(AppAction::Review(R::FilterRepoBases("v1.2".into())));
    app.click("repo-base-rev");
    assert_eq!(base_of(&app, "ledger"), RepoBase::Rev("v1.2".into()));
    assert!(matches!(base_of(&app, "api"), RepoBase::Branch(_)));

    // "Use for every repo" on ledger's row gives api ledger's base too.
    app.click("repo-base-0");
    assert!(app.contains("repo-base-0-every"));
    app.click("repo-base-0-every");
    assert!(!app.contains("repo-base-menu"));
    assert_eq!(base_of(&app, "ledger"), RepoBase::Rev("v1.2".into()));
    assert_eq!(base_of(&app, "api"), RepoBase::Rev("v1.2".into()));
}

#[test]
fn a_click_outside_closes_a_repository_menu_and_one_menu_shows_at_a_time() {
    let mut app = changes_app();
    app.click("repo-base-0");
    assert!(app.contains("repo-base-menu"));
    app.click("repo-base-scrim");
    assert!(!app.contains("repo-base-menu"));

    assert!(!app.contains("diff-base-menu"), "two repositories have no header menu");
    app.send(AppAction::Review(R::ToggleRepoBase { repo: "api".into() }));
    assert!(app.contains("repo-base-menu"));
    assert!(app.contains("repo-base-1-every"));
    assert!(!app.contains("repo-base-0-every"), "only api's menu shows");
}

#[test]
fn full_screen_draws_each_repository_base_too() {
    let mut app = changes_app();
    app.click("expand-panel");
    assert_eq!(app.ui.read(|s| s.full_screen()), Some(FullScreen::Panel));
    assert!(app.contains("repo-base-0"));
    app.click("repo-base-1");
    assert!(app.contains("repo-base-menu"));
}

/// RV-31 with RV-32: a review plan's groups can mix repositories, so each
/// repository's base sits once above them and its menu still works there.
#[test]
fn a_review_plan_keeps_each_repository_base_above_its_groups() {
    use humata2_clients::views::{PlanGroup, PlanView, ViewContent};
    let mut workspace = fixture::design_session(Status::Working);
    for diff in &mut workspace.diffs {
        diff.repo = format!("acme/{}", diff.repo);
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Journal);
    let entry = |repo: &str, path: &str| (repo.to_owned(), path.to_owned());
    app.fakes.views.offer_view(
        &ledger(),
        "Review plan: ledger schema",
        "The migration first, then the client",
        "Claude Code",
        ViewContent::ReviewPlan(PlanView {
            importance: Vec::new(),
            stack: None,
            targets: Vec::new(),
            order: vec![
                entry("acme/ledger", "migrations/0042_accounts.sql"),
                entry("acme/api", "src/client.rs"),
            ],
            groups: vec![PlanGroup {
                title: "Everything".to_owned(),
                note: String::new(),
                first: entry("acme/ledger", "migrations/0042_accounts.sql"),
            }],
            notes: Vec::new(),
        }),
    );
    app.settle();
    app.click("offer-open");
    assert!(app.contains("plan-group-0"));
    assert!(app.contains("repo-base-0"));
    assert!(app.contains("repo-base-1"));
    assert!(!app.contains("plan-group-1"), "one group for both repositories");

    app.click("repo-base-1");
    assert!(app.contains("repo-base-menu"));
    app.click("repo-base-pick-0");
    assert!(matches!(base_of(&app, "acme/api"), RepoBase::Branch(_)));
    assert_eq!(base_of(&app, "acme/ledger"), RepoBase::DefaultBranch);
    assert!(app.contains("plan-group-0"), "the plan still shows");
}
