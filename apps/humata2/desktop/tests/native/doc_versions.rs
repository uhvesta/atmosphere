//! DV-3 to DV-8 through the real window: the History strip shows on a Desk
//! Markdown tab with versions and nowhere else, ◀ steps to an old version
//! that is read-only, Save version needs a change, Restore adds a version.
use super::*;
use humata2_clients::docs::VersionBy;
use humata2_session::{DocumentMode, HistoryAction as H, Position};

const SESSION: &str = "desk-brin-rfc";
const PATH: &str = "rfc-brin.md";
const V1: &str = "# BRIN\none\n";
const V2: &str = "# BRIN\none\ntwo\n";

fn session() -> SessionId {
    SessionId::new(SESSION)
}

/// The Desk of the design workspace, two agent versions of the RFC saved
/// before the tab opens.
fn versioned_app() -> App {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let mut app = App::new(workspace);
    app.fold_quiet_channels();
    // Save version takes the whole folder (DV-5): the agent has saved the
    // folder's other file too, so only the RFC changes below count.
    app.fakes.daemon.commit_as_agent(
        &session(),
        "notes/bench-results.md",
        "# Bench results\n",
        "Bench notes",
    );
    app.fakes.daemon.commit_as_agent(&session(), PATH, V1, "First draft");
    app.fakes.daemon.commit_as_agent(&session(), PATH, V2, "Add two");
    app.settle();
    open(&mut app, SESSION, PATH);
    app
}

fn open(app: &mut App, session: &str, path: &str) {
    app.send(AppAction::Navigate(Route::Session(SessionId::new(session))));
    app.send(AppAction::Session(SessionAction::SetPanel(None)));
    app.send(AppAction::Session(SessionAction::SelectFile {
        repo: String::new(),
        path: path.into(),
    }));
    app.send(AppAction::Session(SessionAction::OpenFile));
}

fn history(app: &App) -> humata2_session::DocHistory {
    app.ui
        .read(|state| state.session.history(&session(), PATH).cloned())
        .expect("the file has a history")
}

fn history_action(app: &mut App, action: H) {
    app.send(AppAction::Session(SessionAction::History(action)));
}

#[test]
fn the_strip_shows_on_a_desk_markdown_tab_with_versions() {
    let mut app = versioned_app();
    assert!(app.contains("doc-history-strip"));
    assert_eq!(history(&app).versions.len(), 2);
    assert!(!app.contains("doc-version-bar"));
}

#[test]
fn the_strip_is_not_on_a_folder_that_keeps_no_versions() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let mut app = App::new(workspace);
    app.fold_quiet_channels();
    // A picked folder keeps no versions; give it a Markdown file to open.
    app.fakes
        .daemon
        .edit_document(&SessionId::new("desk-tidy-infra"), "notes.md", "# Notes\n");
    open(&mut app, "desk-tidy-infra", "notes.md");
    assert!(app.ui.read(|state| {
        state
            .session
            .history(&SessionId::new("desk-tidy-infra"), "notes.md")
            .is_none()
    }));
    assert!(!app.contains("doc-history-strip"));
}

#[test]
fn stepping_back_shows_an_old_version_that_takes_no_comment() {
    let mut app = versioned_app();
    app.send(AppAction::Session(SessionAction::DocumentMode {
        repo: String::new(),
        path: PATH.into(),
        mode: DocumentMode::Source,
    }));
    assert!(app.contains("doc-line-1"));
    app.click("doc-history-back");
    let shown = history(&app);
    assert_eq!(shown.viewing().map(|version| version.number), Some(2));
    assert!(app.ui.read(|state| state.session.doc_read_only(&session(), PATH)));
    assert!(app.contains("doc-version-bar"));
    // No line takes a comment, and no box is open on the old text.
    assert!(!app.contains("doc-line-1"));
    assert!(app.contains("doc-old-line-1"));
    assert!(app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(&session())
            .is_none_or(|held| held.editing.is_none())
    }));
    app.click("doc-back-to-current");
    assert!(!app.ui.read(|state| state.session.doc_read_only(&session(), PATH)));
    assert!(app.contains("doc-line-1"));
}

#[test]
fn save_version_needs_a_change_since_the_newest_version() {
    let mut app = versioned_app();
    assert!(!history(&app).can_save());
    app.click("doc-save-version");
    assert_eq!(history(&app).versions.len(), 2);

    app.fakes
        .daemon
        .edit_document(&session(), PATH, "# BRIN\none\ntwo\nthree\n");
    history_action(
        &mut app,
        H::Load {
            session: session(),
            path: PATH.into(),
        },
    );
    assert!(history(&app).can_save());
    app.click("doc-save-version");
    let saved = history(&app);
    assert_eq!(saved.versions.len(), 3);
    assert_eq!(saved.versions[0].by, VersionBy::You);
    assert!(!saved.can_save());
}

#[test]
fn restoring_an_old_version_adds_a_version_and_returns_to_current() {
    let mut app = versioned_app();
    app.click("doc-history-back");
    app.click("doc-history-back");
    assert_eq!(history(&app).viewing().map(|version| version.number), Some(1));
    app.click("doc-restore-version");
    let restored = history(&app);
    assert_eq!(restored.versions.len(), 3);
    assert_eq!(restored.position, Position::Current);
    assert_eq!(restored.versions[0].by, VersionBy::You);
}

#[test]
fn compare_with_current_asks_the_diff_view_for_the_shown_version() {
    let mut app = versioned_app();
    app.click("doc-history-back");
    app.click("doc-history-back");
    app.click("doc-compare-current");
    let compare = app
        .ui
        .read(|state| state.session.compare.clone())
        .expect("a comparison is asked for");
    assert_eq!(compare.session, session());
    assert_eq!(compare.path, PATH);
    assert_eq!(Some(compare.version), history(&app).viewing().map(|v| v.id.clone()));
}

/// A Markdown text long enough that the page and the source both scroll.
fn long(tag: &str) -> String {
    let mut text = String::from("# BRIN\n\n");
    for n in 1..=150 {
        text.push_str(&format!("## Section {n}\n\nParagraph {n} {tag}.\n\n"));
    }
    text
}

/// The first block of the page that is drawn now, or `None` when none is.
fn first_page_block(app: &mut App) -> Option<usize> {
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            (0..400usize).find(|n| window.try_find(("md-block", *n)).is_some())
        })
        .expect("window stays open")
}

#[test]
fn side_by_side_scrolls_both_sides_together_on_an_old_version() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let mut app = App::new(workspace);
    app.fold_quiet_channels();
    app.fakes.daemon.commit_as_agent(&session(), PATH, &long("one"), "First draft");
    app.fakes.daemon.commit_as_agent(&session(), PATH, &long("two"), "Second draft");
    app.settle();
    open(&mut app, SESSION, PATH);
    app.send(AppAction::Session(SessionAction::DocumentMode {
        repo: String::new(),
        path: PATH.into(),
        mode: DocumentMode::SideBySide,
    }));
    app.click("doc-history-back");
    assert!(app.ui.read(|state| state.session.doc_read_only(&session(), PATH)));
    // The source column shows the old text and the page is the old page.
    assert!(app.contains("doc-old-line-1"));
    assert_eq!(first_page_block(&mut app), Some(0));

    // Scrolling the source moves the page with it, and keeps them linked.
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.scroll(
                "doc-source",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-3000.))),
                cx,
            );
        })
        .expect("the source is drawn");
    app.settle();
    app.draw();
    let moved = first_page_block(&mut app);
    assert!(moved.is_some_and(|block| block > 0), "the page followed: {moved:?}");
    assert!(app.ui.read(|state| {
        state
            .session
            .view()
            .is_some_and(|view| view.document("", PATH).linked)
    }));
}
