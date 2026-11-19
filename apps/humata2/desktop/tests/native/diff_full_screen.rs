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
/// base chip sits in the 52 pt bar.
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
            let base = window.find("diff-base").bounds();
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
    let mut app = app();
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

fn first_row_tabs(app: &mut App) -> usize {
    app.ui
        .read(|state| state.session.view().map_or(0, |view| view.groups[0].tabs.len()))
}

/// UXSIMPLE 3-3, 3-4 (REVIEWER simplified): full screen offers New agent
/// tab; its menu lists Terminal and the agents, and a pick opens a tab in
/// the session's own tab strip with the Diff beside it.
#[test]
fn new_agent_tab_opens_a_tab_beside_the_diff() {
    let mut app = app();
    assert!(!app.contains("new-agent-tab"), "only full screen offers it");
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.click("new-agent-tab");
    assert!(app.contains("new-agent-tab-menu"));
    assert!(app.contains("new-agent-tab-zsh"));
    assert!(app.contains("new-agent-tab-claude"));
    let before = first_row_tabs(&mut app);
    app.click("new-agent-tab-copilot");
    assert!(!app.contains("new-agent-tab-menu"));
    assert!(!full(&mut app));
    assert!(app.contains("side-panel"));
    assert_eq!(first_row_tabs(&mut app), before + 1);
    // The menu remembers the pick as last used.
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.click("new-agent-tab");
    assert!(app.contains("new-agent-tab-last-used-copilot"));
}

/// Escape closes the menu before it leaves full screen.
#[test]
fn escape_closes_the_new_agent_tab_menu_first() {
    let mut app = app();
    app.send(AppAction::Session(SessionAction::ToggleWiden));
    app.click("new-agent-tab");
    app.send(AppAction::Escape);
    assert!(!app.contains("new-agent-tab-menu"));
    assert!(full(&mut app));
}
