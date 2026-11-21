//! Full screen with one header, a real name, and three ways in and out
//! (DV-18: DS-12, DS-13, DS-14, DS-58).
use super::*;
use gpui_kit::{InputEvent as _, MouseButton, MouseDownEvent};
use humata2_review::ReviewAction as R;

fn app() -> App {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app
}

fn press(app: &mut App, key: &str) {
    app.cx
        .update_window(app.window, |_, window, cx| window.press(key, cx))
        .unwrap();
    app.settle();
}

fn full(app: &mut App) -> bool {
    app.ui
        .read(|state| state.full_screen() == Some(FullScreen::Panel))
}

/// One title bar: Exit full screen replaces the arrow, Close stays, and the
/// repository count (two repos, D868) sits in the 52 pt bar.
#[test]
fn full_screen_draws_one_header() {
    let mut app = app();
    assert!(app.contains("expand-panel"));
    assert!(app.contains("close-panel"));
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    assert!(app.contains("show-terminal"));
    assert!(!app.contains("expand-panel"));
    assert!(app.contains("close-panel"));
    assert!(!app.contains("panel-header"));
    app.cx
        .update_window(app.window, |_, window, _| {
            let panel = window.find("panel-full-screen").bounds();
            let base = window.find("diff-repos-count").bounds();
            assert!(base.origin.y - panel.origin.y < px(52.));
            assert!(base.origin.y >= panel.origin.y);
        })
        .unwrap();
}

#[test]
fn alt_cmd_enter_enters_and_leaves_full_screen() {
    let mut app = app();
    assert!(app.contains("side-panel"));
    press(&mut app, "ctrl-cmd-enter");
    assert!(app.contains("panel-full-screen"));
    press(&mut app, "ctrl-cmd-enter");
    assert!(app.contains("side-panel"));
    assert!(!app.contains("panel-full-screen"));
}

/// The accessibility description of a drawn control, which carries the
/// Full screen key as its hover text does.
fn description_of(app: &mut App, id: &'static str) -> String {
    app.draw();
    app.cx
        .update_window(app.window, |_, window, _| {
            window.find(id).description().unwrap_or_default().to_owned()
        })
        .unwrap()
}

/// RV-4: the Full screen button and the Exit full screen button both name
/// the chord that toggles full screen (⌃⌘↩).
#[test]
fn the_full_screen_button_names_its_key() {
    const CHORD: &str = "⌃⌘↩";
    let mut app = app();
    let beside = description_of(&mut app, "expand-panel");
    assert!(beside.contains(CHORD), "{beside}");
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    let full = description_of(&mut app, "show-terminal");
    assert!(full.contains(CHORD), "{full}");
}

/// RV-4: the terminal's own arrow names its chord (⇧⌘↩) in both states.
#[test]
fn the_terminal_arrow_names_its_key() {
    const CHORD: &str = "⇧⌘↩";
    let mut app = app();
    let expand = description_of(&mut app, "terminal-full-screen");
    assert!(expand.contains(CHORD), "{expand}");
    app.send(AppAction::ToggleFullScreen(FullScreen::Terminal));
    let back = description_of(&mut app, "terminal-full-screen");
    assert!(back.contains(CHORD), "{back}");
    assert_ne!(expand, back);
}

/// DS-80: the full-screen title line keeps the whole session line.
#[test]
fn the_full_screen_title_line_keeps_its_whole_text() {
    let mut app = app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.draw();
    let label = app
        .cx
        .update_window(app.window, |_, window, _| {
            window.find("panel-session-line").label().map(str::to_owned)
        })
        .expect("window stays open");
    assert!(label.is_some_and(|line| !line.is_empty() && !line.contains('…')));
}

#[test]
fn double_clicking_the_panel_header_enters_full_screen() {
    let mut app = app();
    assert!(app.contains("panel-header"));
    app.cx
        .update_window(app.window, |_, window, cx| {
            let bounds = window.find("panel-header").bounds();
            // The left end of the row, over the title and not a button.
            let at = point(bounds.origin.x + px(8.), bounds.origin.y + px(18.));
            humata2_engine::frame::move_pointer(window, at, None, cx);
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position: at,
                    modifiers: window.modifiers(),
                    click_count: 2,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
        })
        .unwrap();
    app.settle();
    assert!(full(&mut app));
    assert!(app.contains("panel-full-screen"));
}

#[test]
fn escape_leaves_full_screen_but_not_while_the_base_menu_is_open() {
    // One repository keeps the header's base chip (D868).
    let mut workspace = fixture::design();
    workspace.diffs.retain(|d| d.session != SessionId::new("ledger") || d.repo == "api");
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.click("diff-base");
    assert!(app.contains("diff-base-menu"));
    app.send(AppAction::Escape);
    assert!(full(&mut app), "the menu took the key");
    assert!(!app.contains("diff-base-menu"));
    app.send(AppAction::Escape);
    assert!(!full(&mut app));
    assert!(app.contains("side-panel"));
}

/// Close in full screen closes the panel and leaves full screen.
#[test]
fn close_in_full_screen_closes_the_panel() {
    let mut app = app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    assert!(full(&mut app));
    app.click("close-panel");
    assert!(!full(&mut app));
    assert!(!app.contains("panel-full-screen"));
    assert!(!app.contains("side-panel"));
}

/// Exit full screen and back keeps the selected file.
#[test]
fn show_the_terminal_keeps_the_selected_file_and_its_scroll() {
    let mut app = app();
    app.send(AppAction::Review(R::SelectFile {
        repo: "api".into(),
        path: "src/client.rs".into(),
    }));
    let code = "change-code api/src/client.rs";
    assert!(app.contains(code));
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    assert!(app.contains(code));
    app.click("show-terminal");
    assert!(app.contains(code));
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    assert!(app.contains(code));
}

#[test]
fn a_session_page_has_a_back_button() {
    let mut app = app();
    assert!(app.contains("session-back"));
}

#[test]
fn session_back_button_hides_when_there_is_nowhere_to_go_back() {
    let mut app = App::new(fixture::design());
    // Switching directly (like clicking a session in the sidebar or opening a new PR review)
    // clears the navigation trail, so there is nowhere to go back to.
    app.send(AppAction::Switch(Route::Session(SessionId::new("ledger"))));
    assert!(!app.contains("session-back"));

    // Navigating from another page populates the trail and shows the back button.
    app.send(AppAction::Navigate(Route::Home));
    app.send(AppAction::Navigate(Route::Session(SessionId::new("ledger"))));
    assert!(app.contains("session-back"));
}

fn first_row_tabs(app: &mut App) -> usize {
    app.ui
        .read(|state| state.session.view().map_or(0, |view| view.groups[0].tabs.len()))
}

/// Full screen offers the `Agent · N ⌘J` dock; its `+` menu lists Terminal and
/// the agents, and a pick opens a tab in the dock and session while staying in
/// full screen.
#[test]
fn new_agent_tab_opens_a_tab_beside_the_diff() {
    let mut app = app();
    assert!(!app.contains("agent-dock-toggle"), "only full screen offers it");
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    if !app.contains("agent-dock") {
        app.click("agent-dock-toggle");
    }
    assert!(app.contains("agent-dock"));
    app.click("agent-dock-add");
    assert!(app.contains("agent-dock-add-menu"));
    assert!(app.contains("agent-dock-add-zsh"));
    assert!(app.contains("agent-dock-add-claude"));
    let before = first_row_tabs(&mut app);
    app.click("agent-dock-add-copilot");
    assert!(!app.contains("agent-dock-add-menu"));
    assert!(full(&mut app));
    assert!(app.contains("agent-dock"));
    assert_eq!(first_row_tabs(&mut app), before + 1);
    app.click("agent-dock-close");
}

/// D858: an agent pick calls `open_agent_tab` directly with its agent and
/// opening prompt (never `send_to_tab`), while Terminal calls `open ... zsh`
/// and never `open_agent_tab`.
#[test]
fn an_agent_pick_waits_with_its_prompt_and_terminal_has_none() {
    let mut app = app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    if !app.contains("agent-dock") {
        app.click("agent-dock-toggle");
    }
    // Anchor at the bottom so the `+` popover stays well inside the viewport
    // even after multiple tabs are added.
    app.click("agent-dock-position");
    app.click("agent-dock-add");
    app.click("agent-dock-add-zsh");
    let calls_after_zsh = app.fakes.daemon.calls();
    assert!(
        calls_after_zsh.iter().any(|call| call.contains("zsh")),
        "zsh opens a shell tab: {calls_after_zsh:?}"
    );
    assert!(
        !calls_after_zsh.iter().any(|call| call.starts_with("open_agent_tab")),
        "zsh never calls open_agent_tab: {calls_after_zsh:?}"
    );
    app.click("agent-dock-add");
    app.click("agent-dock-add-copilot");
    let calls = app.fakes.daemon.calls();
    assert!(
        calls
            .iter()
            .any(|call| call.starts_with("open_agent_tab") && call.contains("copilot")),
        "copilot calls open_agent_tab: {calls:?}"
    );
    let typed = calls.iter().any(|call| call.starts_with("send_to_tab"));
    assert!(!typed, "nothing is typed via send_to_tab: {calls:?}");
    app.click("agent-dock-close");
}

/// Closing the Agent dock closes its add menu and stays in full screen.
#[test]
fn escape_closes_the_new_agent_tab_menu_first() {
    let mut app = app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    if !app.contains("agent-dock") {
        app.click("agent-dock-toggle");
    }
    app.click("agent-dock-add");
    assert!(app.contains("agent-dock-add-menu"));
    // First click hits the occluding scrim, dismissing the add menu without
    // accidentally closing the dock underneath; second click closes the dock.
    app.click("agent-dock-close");
    assert!(!app.contains("agent-dock-add-menu"));
    assert!(app.contains("agent-dock"));
    app.click("agent-dock-close");
    assert!(!app.contains("agent-dock"));
    assert!(full(&mut app));
}

/// ⌘T (UXSIMPLE 3-3) opens a Terminal tab from full screen, the Diff
/// beside it.
#[test]
fn cmd_t_opens_a_terminal_tab_and_leaves_full_screen() {
    let mut app = app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    let before = first_row_tabs(&mut app);
    press(&mut app, "cmd-t");
    assert!(!full(&mut app));
    assert!(app.contains("side-panel"));
    assert_eq!(first_row_tabs(&mut app), before + 1);
}

/// D857: an agent the session's machine lacks is listed as "not installed",
/// after the installed ones, and clicking it opens nothing.
#[test]
fn a_missing_agent_row_is_listed_and_opens_nothing() {
    use humata2_clients::machines::{MachineDetail, MachineHealth, Reach};
    use humata2_clients::model::{Agent, SessionId};
    let mut app = app();
    let machine = app
        .ui
        .read(|state| state.workspace.session(&SessionId::new("ledger")).map(|s| s.machine.clone()))
        .expect("the fixture has the ledger session");
    app.fakes.machines.set_detail(Ok(MachineDetail {
        machine: machine.clone(),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: "devbox-1.lan".to_owned(),
        reach: Reach::Ssh,
        daemon_version: "2.0.3".to_owned(),
        health: MachineHealth {
            gh: None,
            agents: vec![(Agent::Claude, "2.4.1".to_owned())],
            disk_free: 11_000_000_000,
            disk_total: 500_000_000_000,
            variables: Vec::new(),
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: "2.83.0".to_owned(),
        },
        folders: Vec::new(),
        sessions: Vec::new(),
        rules: Vec::new(),
        stop: None,
    }));
    // Load the machine's health so `missing_agents()` sees only Claude installed.
    app.send(AppAction::Hosts(humata2_hosts::HostsAction::ReadHealth(machine)));
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    if !app.contains("agent-dock") {
        app.click("agent-dock-toggle");
    }
    app.click("agent-dock-add");
    assert!(
        app.contains("agent-dock-add-codex-disabled"),
        "a missing agent still has its disabled row"
    );
    let before = app.ui.read(|state| state.session.open.as_ref().and_then(|id| state.workspace.session(id)).map_or(0, |s| s.shells.len()));
    app.click("agent-dock-add-codex-disabled");
    assert!(app.contains("agent-dock-add-menu"), "a missing agent can't be picked");
    assert!(full(&mut app));
    let after = app.ui.read(|state| state.session.open.as_ref().and_then(|id| state.workspace.session(id)).map_or(0, |s| s.shells.len()));
    assert_eq!(before, after);
    app.click("agent-dock-add-claude");
    assert!(!app.contains("agent-dock-add-menu"));
    app.click("agent-dock-close");
}

#[test]
fn non_markdown_files_do_not_show_markdown_mode_switcher() {
    let mut app = app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    assert!(full(&mut app));
    assert!(
        !app.contains("diff-md-modes"),
        "non-markdown diff files must not show Diff | Split | Rendered"
    );
}

#[test]
fn full_screen_diff_markdown_renders_mermaid_switches_modes_and_comments_on_blocks() {
    use humata2_clients::model::{DiffLine, FileDiff, LineKind, SessionId};
    use humata2_review::DiffMarkdownMode;

    const MD_PATH: &str = "docs/architecture.md";
    let md_lines = [
        "# Architecture",
        "",
        "Overview of the pipeline.",
        "",
        "```mermaid",
        "graph TD",
        "  A[Diff] --> B[MarkdownView]",
        "```",
    ];
    let md_full = md_lines.join("\n");

    let mut workspace = fixture::design();
    let session = SessionId::new("ledger");
    let repo = workspace
        .diffs
        .iter()
        .find(|diff| diff.session == session)
        .expect("fixture has a diff for ledger")
        .repo
        .clone();
    let md_file = FileDiff {
        path: MD_PATH.to_owned(),
        added: md_lines.len() as u32,
        lines: std::iter::once(DiffLine {
            kind: LineKind::Hunk,
            old: None,
            new: None,
            text: format!("@@ -0,0 +1,{} @@", md_lines.len()),
        })
        .chain(md_lines.iter().enumerate().map(|(ix, text)| DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some((ix + 1) as u32),
            text: (*text).to_owned(),
        }))
        .collect(),
        ..FileDiff::default()
    };
    workspace
        .diffs
        .iter_mut()
        .find(|diff| diff.session == session && diff.repo == repo)
        .unwrap()
        .files
        .insert(0, md_file);
    let diagram_offset = md_full
        .find("```mermaid")
        .expect("fixture markdown contains a mermaid diagram");
    workspace
        .file_contents
        .insert((session.clone(), repo.clone(), MD_PATH.to_owned()), md_full);

    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.send(AppAction::Review(R::SelectFile {
        repo: repo.clone(),
        path: MD_PATH.to_owned(),
    }));

    // Narrow diff does not show the full-screen Markdown mode switcher.
    assert!(!full(&mut app));
    assert!(!app.contains("diff-md-modes"));

    // Enter full-screen Diff: switcher appears, defaulting to Diff.
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    assert!(full(&mut app));
    assert!(app.contains("diff-md-modes"));
    assert!(app.contains("diff-md-mode-diff"));
    assert!(app.contains("diff-md-mode-split"));
    assert!(app.contains("diff-md-mode-rendered"));
    assert!(!app.contains("diff-md-rendered"));
    assert!(!app.contains("diff-md-split"));

    // Switch to Split: both code diff and smart MarkdownView render side by side.
    app.click("diff-md-mode-split");
    assert_eq!(
        app.ui.read(|state| state.review.markdown_mode(&session)),
        DiffMarkdownMode::Split
    );
    assert!(app.contains("diff-md-split"));
    assert!(app.contains("diff-md-rendered"));
    assert!(app.contains("md-view"));
    // Mermaid diagram block is parsed and rendered by MarkdownView.
    assert!(
        app.cx
            .update_window(app.window, |_, window, _| {
                window.try_find(("md-diagram", diagram_offset)).is_some()
            })
            .unwrap(),
        "expected Mermaid diagram block to render in smart MarkdownView"
    );

    // Switch to Rendered: full rendered Markdown without split wrapper.
    app.click("diff-md-mode-rendered");
    assert_eq!(
        app.ui.read(|state| state.review.markdown_mode(&session)),
        DiffMarkdownMode::Rendered
    );
    assert!(app.contains("diff-md-rendered"));
    assert!(!app.contains("diff-md-split"));
    assert!(app.contains("md-view"));

    // Click the rendered block's '+' gutter button to open a draft comment.
    app.draw();
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.click(("md-comment-block", 0usize), cx);
        })
        .unwrap();
    app.settle();
    assert!(
        app.contains("doc-comment-box"),
        "clicking a rendered Markdown block should open the review draft comment box"
    );
    let anchor = app.ui.read(|state| {
        state
            .comments
            .sessions
            .get(&session)
            .and_then(|held| held.editing.as_ref())
            .map(|b| (b.anchor.repo.clone(), b.anchor.path.clone(), b.anchor.first, b.anchor.last))
    });
    assert_eq!(
        anchor,
        Some((repo, MD_PATH.to_owned(), 1, 1)),
        "comment box should anchor to the clicked rendered block's lines"
    );

    // Switch back to Diff: rendered pane closes cleanly.
    app.click("diff-md-mode-diff");
    assert_eq!(
        app.ui.read(|state| state.review.markdown_mode(&session)),
        DiffMarkdownMode::Diff
    );
    assert!(!app.contains("diff-md-rendered"));
    assert!(!app.contains("diff-md-split"));
}
