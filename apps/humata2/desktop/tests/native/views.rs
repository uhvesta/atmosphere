//! Prepared views: the offer bar, the Showing bar, the view tab and the Journal.
use super::*;
use humata2_clients::model::{ViewId, ViewState};
use humata2_clients::views::{ViewContent, WalkthroughStep};

fn step(path: &str, note: &str) -> WalkthroughStep {
    WalkthroughStep {
        repo: "acme/api".into(),
        path: path.into(),
        note: note.into(),
    }
}

/// A session open on its Journal with a two-step walkthrough offered to it.
fn offered() -> (App, ViewId) {
    let mut app = App::new(fixture::design_session(Status::Working));
    app.open("ledger", Panel::Journal);
    let id = app.fakes.views.offer_view(
        &SessionId::new("ledger"),
        "Walk the migration",
        "Two files to read first.",
        "Claude Code",
        ViewContent::Walkthrough(vec![
            step("migrations/0042_accounts.sql", "New tables"),
            step("src/client.rs", "The dual write"),
        ]),
    );
    app.settle();
    (app, id)
}

fn state_of(app: &App, id: &ViewId) -> ViewState {
    app.ui
        .read(|s| s.views.record(id).map(|record| record.state))
        .expect("the view is known")
}

/// VW-2, VW-6, VW-8, VW-9: Open it shows the bar and the tab; Revert puts the
/// terminal back.
#[test]
fn an_offered_view_opens_with_steps_and_reverts_to_the_terminal() {
    let (mut app, id) = offered();
    assert!(app.contains("offer-open"));
    assert!(!app.contains("view-revert"));
    app.click("offer-open");
    assert!(!app.contains("offer-open"));
    assert!(app.contains("view-revert"));
    assert!(app.contains("view-keep"));
    assert!(app.contains("view-next"));
    assert_eq!(state_of(&app, &id), ViewState::Opened);
    app.click("view-next");
    assert_eq!(app.ui.read(|s| s.view_step.get(&id).copied()), Some(1));
    app.click("view-revert");
    assert!(!app.contains("view-revert"));
    assert!(!app.contains("view-next"));
    assert!(app.contains("terminal-tab 0"));
    assert_eq!(state_of(&app, &id), ViewState::Reverted);
}

/// VW-2: Not now takes the bar away and the daemon keeps the answer.
#[test]
fn not_now_hides_the_offer_bar() {
    let (mut app, id) = offered();
    app.click("offer-dismiss");
    assert!(!app.contains("offer-open"));
    assert_eq!(state_of(&app, &id), ViewState::NotNow);
}

/// VW-10: a reverted view stays in the Journal and opens again from it.
#[test]
fn the_journal_lists_a_reverted_view_and_opens_it_again() {
    let (mut app, id) = offered();
    app.click("offer-open");
    app.click("view-revert");
    assert!(app.contains("journal-open-0"));
    app.click("journal-open-0");
    assert!(app.contains("view-keep"));
    assert_eq!(state_of(&app, &id), ViewState::Opened);
}

fn route(app: &App) -> Route {
    app.ui.read(|s| s.route.clone())
}

/// VW-8 on the To-do page: a board view opens the To-do list narrowed to
/// its channel (D874), its Showing bar is drawn there, and Revert brings the
/// session page back.
#[test]
fn a_board_view_shows_its_bar_on_the_todo_list_and_revert_goes_back() {
    use humata2_clients::model::{ChannelId, Scope};
    let mut app = App::new(fixture::design_session(Status::Working));
    app.open("ledger", Panel::Journal);
    let board = Scope::Channel(ChannelId::new("payments-rewrite"));
    let id = app.fakes.views.offer_view(
        &SessionId::new("ledger"),
        "Payments board",
        "What is left before the cut-over",
        "Claude Code",
        ViewContent::Board(board.clone()),
    );
    app.settle();

    app.click("offer-open");
    assert_eq!(route(&app), Route::Todo);
    assert!(app.contains("todo-row PAY-3"));
    assert!(!app.contains("todo-row INF-2"));
    assert!(app.contains("view-revert"));
    assert!(app.contains("view-keep"));

    app.click("view-revert");
    assert!(!app.contains("view-revert"));
    assert_eq!(route(&app), Route::Session(SessionId::new("ledger")));
    assert_eq!(state_of(&app, &id), ViewState::Reverted);
}

/// RV-32: a review plan's group headers are drawn in the Diff panel while
/// it shows, and Revert takes them away.
#[test]
fn a_review_plans_groups_are_drawn_in_the_diff_panel_until_revert() {
    use humata2_clients::views::{PlanGroup, PlanNote, PlanView};
    // The daemon names each repository `owner/name`.
    let mut workspace = fixture::design_session(Status::Working);
    for diff in &mut workspace.diffs {
        diff.repo = format!("acme/{}", diff.repo);
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Journal);
    let entry = |repo: &str, path: &str| (repo.to_owned(), path.to_owned());
    let id = app.fakes.views.offer_view(
        &SessionId::new("ledger"),
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
            groups: vec![
                PlanGroup {
                    title: "Schema".to_owned(),
                    note: "The risky part.".to_owned(),
                    first: entry("acme/ledger", "migrations/0042_accounts.sql"),
                },
                PlanGroup {
                    title: "Client".to_owned(),
                    note: "The dual write.".to_owned(),
                    first: entry("acme/api", "src/client.rs"),
                },
            ],
            notes: vec![PlanNote {
                repo: "acme/api".to_owned(),
                path: "src/client.rs".to_owned(),
                text: "One transaction.".to_owned(),
                lines: None,
                outdated: false,
            }],
        }),
    );
    app.settle();
    assert!(!app.contains("plan-group-0"));

    app.click("offer-open");
    assert!(app.contains("plan-group-0"));

    app.click("view-revert");
    assert!(!app.contains("plan-group-0"));
    assert_eq!(state_of(&app, &id), ViewState::Reverted);
}

/// RV-34: a plan's note on lines that changed since the agent wrote it keeps
/// showing, tagged outdated; a note whose lines are as written has no tag.
#[test]
fn a_review_plans_outdated_line_note_is_tagged_in_the_diff_panel() {
    use humata2_clients::views::{PlanNote, PlanView};
    let mut workspace = fixture::design_session(Status::Working);
    for diff in &mut workspace.diffs {
        diff.repo = format!("acme/{}", diff.repo);
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Journal);
    let note = |text: &str, lines: (u32, u32), outdated: bool| PlanNote {
        repo: "acme/api".to_owned(),
        path: "src/client.rs".to_owned(),
        text: text.to_owned(),
        lines: Some(lines),
        outdated,
    };
    app.fakes.views.offer_view(
        &SessionId::new("ledger"),
        "Review plan: client",
        "The client",
        "Claude Code",
        ViewContent::ReviewPlan(PlanView {
            importance: Vec::new(),
            stack: None,
            targets: Vec::new(),
            order: vec![("acme/api".to_owned(), "src/client.rs".to_owned())],
            groups: Vec::new(),
            notes: vec![note("Moved down.", (12, 14), false), note("Rewritten since.", (3, 3), true)],
        }),
    );
    app.settle();

    app.click("offer-open");
    assert!(app.contains("plan-note-outdated-0"), "the rewritten note is tagged");
    assert!(!app.contains("plan-note-outdated-1"), "only that one is");
}
