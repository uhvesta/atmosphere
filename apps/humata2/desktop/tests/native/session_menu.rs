//! The header's ⋯ session menu and the Rename dialog, natively (SE-7,
//! FC-17). No frame draws them (D295); the images are for review by eye.
use super::*;

/// The menu opens from ⋯, lists the session's actions and closes on Escape;
/// Rename renames, branch too when asked; Stop ends the agent and then leaves the menu.
#[test]
fn the_session_menu_renames_and_stops_the_open_session() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    assert!(!app.contains("session-menu-card"));
    app.snapshot("session-menu-closed");

    app.click("session-menu");
    for item in [
        "session-menu-rename",
        "session-menu-move",
        "session-menu-continue",
        "session-menu-stop",
        "session-menu-push",
        "session-menu-archive",
    ] {
        assert!(app.contains(item), "the menu lists {item}");
    }
    app.snapshot("session-menu-open");
    app.send(AppAction::Escape);
    assert!(!app.contains("session-menu-card"));

    app.click("session-menu");
    app.click("session-menu-rename");
    assert!(!app.contains("session-menu-card"));
    assert!(app.contains("rename-dialog"));
    app.send(AppAction::Session(SessionAction::RenameTitle("Ledger v2".into())));
    app.click("rename-branch");
    assert!(app.ui.read(|s| s.session.renaming.as_ref().is_some_and(|d| d.rename_branch)));
    app.snapshot("rename-dialog");
    app.click("rename-confirm");
    assert!(!app.contains("rename-dialog"));
    let renamed = app
        .fakes
        .daemon
        .workspace()
        .session(&SessionId::new("ledger"))
        .cloned()
        .expect("the session stays");
    assert_eq!(renamed.title, "Ledger v2");
    assert!(renamed.branch.ends_with("ledger-v2"));

    app.click("session-menu");
    app.click("session-menu-stop");
    let stopped = app
        .fakes
        .daemon
        .workspace()
        .session(&SessionId::new("ledger"))
        .map(|s| s.status)
        .expect("the session stays");
    assert!(matches!(stopped, Status::Exited { .. }));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.click("session-menu");
    assert!(!app.contains("session-menu-stop"), "a stopped session offers no Stop");
    app.snapshot("session-menu-stopped-dark");
}

/// On the Desk the menu has no Move, and Archive reads Close session.
#[test]
fn a_desk_sessions_menu_has_no_move() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let mut app = App::new(workspace);
    app.open("desk-plan-q4", Panel::Info);
    app.click("session-menu");
    assert!(app.contains("session-menu-archive"));
    assert!(!app.contains("session-menu-move"));
    assert!(!app.contains("session-menu-continue"));
}

/// Continue in another channel… (CV-8): the dialog lists the channels,
/// the session's own greyed out; Continue starts the new session in the
/// picked one and opens it, and the session it came from stays put.
#[test]
fn continue_in_another_channel_opens_the_new_session_there() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Changes);
    app.click("session-menu");
    app.click("session-menu-continue");
    assert!(!app.contains("session-menu-card"));
    assert!(app.contains("continue-dialog"));
    app.click("continue-channel-payments-rewrite");
    assert!(app.ui.read(|s| s.session.continuing.as_ref().is_some_and(|d| d.channel.is_none())));
    app.click("continue-channel-web");
    app.click("continue-confirm");
    assert!(!app.contains("continue-dialog"));
    let web = humata2_clients::model::ChannelId::new("web");
    let workspace = app.fakes.daemon.workspace();
    let new = workspace
        .sessions
        .iter()
        .find(|s| s.channel == web && s.title == "Ledger schema migration")
        .expect("a new session in web");
    assert_eq!(app.ui.read(|s| s.session.open.clone()), Some(new.id.clone()));
    assert_eq!(
        workspace.session(&SessionId::new("ledger")).map(|s| s.channel.as_str().to_owned()),
        Some("payments-rewrite".to_owned())
    );
}
