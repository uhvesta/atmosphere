//! The Desk through the real window (frames 36-38, 40 and 55).
use super::*;
use humata2_clients::model::MachineId;
use humata2_desk::{DeskAction, NewAction, Sort};

const OPEN: [&str; 3] = ["desk-plan-q4", "desk-brin-rfc", "desk-tidy-infra"];
const CLOSED: [&str; 4] = [
    "desk-release-checklist",
    "desk-onboarding-doc",
    "desk-port-cron",
    "desk-notes-cleanup",
];

/// The design workspace with the Desk of frame 36, channels folded as the
/// frames draw them.
fn desk_app() -> App {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let mut app = App::new(workspace);
    fold_all_channels(&mut app);
    app
}

/// The Desk frames draw every channel folded (`desk_side` in the design).
fn fold_all_channels(app: &mut App) {
    app.fold_quiet_channels();
    for id in ["payments-rewrite", "infra"] {
        app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
            humata2_clients::model::ChannelId::new(id),
        ))));
    }
}

fn route(app: &App) -> Route {
    app.ui.read(|s| s.route.clone())
}

/// The fake daemon's machine for `session`.
fn machine_of(app: &App, session: &str) -> Option<MachineId> {
    app.fakes
        .daemon
        .workspace()
        .session(&SessionId::new(session))
        .map(|s| s.machine.clone())
}

#[test]
fn the_desk_page_lists_open_documents_and_closed() {
    let mut app = desk_app();
    app.click("sidebar-desk");
    assert_eq!(route(&app), Route::Desk);
    for id in OPEN {
        assert!(app.contains(&format!("desk-open-{id}")), "{id}");
    }
    assert!(app.contains("desk-document-desk-brin-rfc-rfc-brin.md"));
    assert!(app.contains("desk-document-desk-plan-q4-q4-plan.md"));
    for id in CLOSED {
        assert!(app.contains(&format!("desk-closed-{id}")), "{id}");
        // Only the Sketch row has days left, so only it can be kept.
        assert_eq!(
            app.contains(&format!("desk-keep-{id}")),
            id == "desk-release-checklist",
            "{id}"
        );
        assert!(app.contains(&format!("desk-reopen-{id}")), "{id}");
    }
    app.snapshot("36-desk");
}

#[test]
fn a_new_desk_session_starts_from_the_form_and_opens() {
    let mut app = desk_app();
    let before: Vec<SessionId> = app
        .fakes
        .daemon
        .workspace()
        .desk
        .sessions
        .keys()
        .cloned()
        .collect();
    app.click("sidebar-desk");
    app.click("desk-new");
    assert!(app.contains("new-desk"));
    app.send(AppAction::Desk(DeskAction::New(NewAction::Title(
        "Plan Q4 board".into(),
    ))));
    app.snapshot("40-new-desk-session");
    app.send(AppAction::Desk(DeskAction::New(NewAction::Machine(
        MachineId::new("this-mac"),
    ))));
    app.click("new-desk-start");
    let workspace = app.fakes.daemon.workspace();
    let started: Vec<_> = workspace
        .desk
        .sessions
        .keys()
        .filter(|id| !before.contains(id))
        .cloned()
        .collect();
    assert_eq!(started.len(), 1, "one new Desk session: {started:?}");
    assert_eq!(route(&app), Route::Session(started[0].clone()));
    assert!(!app.contains("new-desk"));
    assert!(app.contains("desk-bar"));
}

#[test]
fn a_desk_sessions_page_has_the_folder_bar_and_no_fork() {
    let mut app = desk_app();
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.open("desk-plan-q4", Panel::Board);
    app.send(AppAction::Session(SessionAction::SelectFile {
        repo: String::new(),
        path: "q4-plan.md".into(),
    }));
    app.send(AppAction::Session(SessionAction::OpenFile));
    app.send(AppAction::Session(SessionAction::SelectTab(0)));
    assert!(app.contains("desk-bar"));
    assert!(!app.contains("session-fork"));
    app.snapshot("37-desk-board");
}

#[test]
fn a_desk_session_moves_to_another_machine_from_its_bar() {
    let mut app = desk_app();
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.open("desk-brin-rfc", Panel::Files);
    app.send(AppAction::Session(SessionAction::SelectFile {
        repo: String::new(),
        path: "rfc-brin.md".into(),
    }));
    app.send(AppAction::Session(SessionAction::OpenFile));
    app.send(AppAction::Session(SessionAction::SelectTab(1)));
    app.snapshot("38-desk-move");
    assert_eq!(
        machine_of(&app, "desk-brin-rfc"),
        Some(MachineId::new("devbox-1"))
    );
    app.click("desk-bar-machine");
    app.click("desk-bar-machine-this-mac");
    assert_eq!(
        machine_of(&app, "desk-brin-rfc"),
        Some(MachineId::new("this-mac"))
    );
}

#[test]
fn cleaning_up_deletes_the_ticked_closed_sessions() {
    let mut app = desk_app();
    app.click("sidebar-desk");
    app.click("desk-clean-up");
    assert!(app.contains("desk-clean-up-dialog"));
    for id in [
        "desk-release-checklist",
        "desk-onboarding-doc",
        "desk-port-cron",
        "desk-notes-cleanup",
    ] {
        assert!(app.contains(&format!("desk-clean-up-tick-{id}")), "{id}");
    }
    app.snapshot("55-desk-clean-up");
    app.click("desk-clean-up-delete");
    let workspace = app.fakes.daemon.workspace();
    for gone in ["desk-release-checklist", "desk-onboarding-doc"] {
        assert!(workspace.session(&SessionId::new(gone)).is_none(), "{gone}");
    }
    for kept in ["desk-port-cron", "desk-notes-cleanup"] {
        assert!(workspace.session(&SessionId::new(kept)).is_some(), "{kept}");
    }
    assert!(!app.contains("desk-clean-up-dialog"));
}

#[test]
fn closing_a_desk_session_lists_it_under_closed() {
    let mut app = desk_app();
    app.open("desk-plan-q4", Panel::Info);
    app.send(AppAction::Desk(DeskAction::Close(SessionId::new(
        "desk-plan-q4",
    ))));
    assert_eq!(route(&app), Route::Desk);
    assert!(app.contains("desk-closed-desk-plan-q4"));
    assert!(!app.contains("desk-open-desk-plan-q4"));
}

#[test]
fn keeping_and_reopening_a_closed_session() {
    let mut app = desk_app();
    let id = SessionId::new("desk-release-checklist");
    app.click("sidebar-desk");
    let days_left = |app: &App| {
        app.fakes
            .daemon
            .workspace()
            .desk
            .sessions
            .get(&id)
            .and_then(|record| record.closed.as_ref())
            .map(|closed| closed.days_left)
    };
    assert_eq!(days_left(&app), Some(Some(6)));
    app.click("desk-keep-desk-release-checklist");
    assert_eq!(days_left(&app), Some(None));
    app.click("desk-reopen-desk-release-checklist");
    assert_eq!(route(&app), Route::Session(id.clone()));
    assert!(
        app.fakes
            .daemon
            .workspace()
            .desk
            .sessions
            .get(&id)
            .is_some_and(|record| record.closed.is_none())
    );
}

#[test]
fn a_document_is_copied_into_a_channels_shared_folder() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    for channel in &mut workspace.channels {
        if channel.id.as_str() == "web" {
            channel.machines = vec![MachineId::new("this-mac")];
        }
    }
    let mut app = App::new(workspace);
    app.click("sidebar-desk");
    app.click("desk-copy-desk-plan-q4-q4-plan.md");
    assert!(app.contains("desk-copy-dialog"));
    app.click("desk-copy-channel-web");
    app.click("desk-copy-confirm");
    let files = app.fakes.daemon.workspace().files;
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("/web/shared/q4-plan.md")),
        "{:?}",
        files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    assert!(!app.contains("desk-copy-dialog"));
}

#[test]
fn sorting_by_title_closes_the_pop_up() {
    let mut app = desk_app();
    app.click("sidebar-desk");
    app.click("desk-sort");
    assert!(app.contains("desk-sort-title"));
    app.click("desk-sort-title");
    assert_eq!(app.ui.read(|s| s.desk.sort), Sort::Title);
    assert!(!app.contains("desk-sort-menu"));
}

#[test]
fn the_sidebar_lists_desk_sessions_only_on_the_desk() {
    let mut app = desk_app();
    app.send(AppAction::Navigate(Route::Home));
    assert!(app.contains("sidebar-desk"));
    assert!(!app.contains("sidebar-session-desk-plan-q4"));
    app.click("sidebar-desk");
    for id in OPEN {
        assert!(app.contains(&format!("sidebar-session-{id}")), "{id}");
    }
}

#[test]
fn closing_an_unkept_folder_with_files_asks_and_save_version_closes_it() {
    let mut app = desk_app();
    let id = SessionId::new("desk-plan-q4");
    app.open("desk-plan-q4", Panel::Info);
    app.send(AppAction::Desk(DeskAction::AskClose(id.clone())));
    assert!(app.contains("desk-close-dialog"));
    assert!(app.fakes.daemon.workspace().desk.sessions[&id].closed.is_none());
    app.click("desk-close-save");
    assert!(!app.contains("desk-close-dialog"));
    let record = app.fakes.daemon.workspace().desk.sessions[&id].clone();
    assert!(record.closed.is_some());
    assert!(!record.doc_head.is_empty());
}

#[test]
fn close_in_the_question_closes_without_saving() {
    let mut app = desk_app();
    let id = SessionId::new("desk-plan-q4");
    app.open("desk-plan-q4", Panel::Info);
    app.send(AppAction::Desk(DeskAction::AskClose(id.clone())));
    app.click("desk-close-anyway");
    let record = app.fakes.daemon.workspace().desk.sessions[&id].clone();
    assert!(record.closed.is_some());
    assert!(record.doc_head.is_empty());
}

/// Bug 6: a long repo-search failure wraps inside New desk session, and the
/// org field and the search field stay inside it too.
#[test]
fn a_long_repo_search_error_wraps_inside_the_new_desk_session_card() {
    let mut app = desk_app();
    app.click("sidebar-desk");
    app.click("desk-new");
    let long = "GitHub search needs the GitHub CLI on this host, and gh couldn't be run \
        (No such file or directory). Install gh with Homebrew (brew install gh), \
        sign in with gh auth login, then retry the search from here.";
    app.fakes
        .daemon
        .fail_next(humata2_clients::Operation::SearchRepos, long);
    app.send(AppAction::Desk(DeskAction::New(NewAction::AddingRepo(true))));
    app.send(AppAction::RepoSearch(
        humata2_app::RepoPicker::NewDeskSession,
        humata2_app::RepoSearchAction::AddOwnerOpened,
    ));
    app.settle();
    assert!(app.contains("new-desk-repos-problem"));
    app.cx
        .update_window(app.window, |_, window, _| {
            let card = window.find("new-desk").bounds();
            let right = card.origin.x + card.size.width;
            for id in ["new-desk-repos-problem", "new-desk-repos-retry", "new-desk-owner-cancel"] {
                let inner = window.find(id).bounds();
                assert!(
                    inner.origin.x + inner.size.width <= right,
                    "{id} ends at {:?}, past the card's right edge {right:?}",
                    inner.origin.x + inner.size.width
                );
            }
        })
        .unwrap();
}
