//! A channel's own Desk through the real window (CD-2, CD-4, CD-6; frames
//! 91, 92, 94 and 95).
use super::*;
use humata2_clients::model::{ChannelId, RuleSetting};

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn with_desk(never: bool) -> App {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    if never
        && let Some(rule) = workspace.rules.get_mut(&payments()).and_then(|rules| {
            rules
                .iter_mut()
                .find(|rule| rule.capability.as_str() == "session_stop_other")
        })
    {
        rule.setting = Some(RuleSetting::Never);
    }
    App::new(workspace)
}

#[test]
fn the_desk_tab_lists_the_sessions_and_the_sidebar_has_its_row() {
    let mut app = with_desk(false);
    app.send(AppAction::OpenChannelDesk(payments()));
    assert!(app.contains("channel-tab-Desk"));
    assert!(app.contains("channel-desk-row-cdesk-triage"));
    assert!(app.contains("channel-desk-row-cdesk-backfill"));
    assert!(app.contains("sidebar-channel-desk-payments-rewrite"));
    assert!(app.contains("sidebar-session-cdesk-triage"));
}

#[test]
fn a_channel_with_no_desk_session_shows_the_empty_state_with_its_button() {
    let mut app = App::new(fixture::design());
    app.send(AppAction::OpenChannelDesk(payments()));
    assert!(app.contains("channel-desk-new"));
    assert!(!app.contains("sidebar-channel-desk-payments-rewrite"));
}

#[test]
fn new_desk_session_offers_the_switches_and_marks_a_blocked_one() {
    let mut app = with_desk(true);
    app.send(AppAction::OpenChannelDesk(payments()));
    app.click("channel-desk-new");
    assert!(app.contains("new-channel-desk"));
    for id in [
        "new-channel-desk-read-files",
        "new-channel-desk-send-notes",
        "new-channel-desk-type-into-terminal",
        "new-channel-desk-start-and-fork",
        "new-channel-desk-stop-and-archive",
        "new-channel-desk-change-board-items",
    ] {
        assert!(app.contains(id), "{id}");
    }
    assert!(app.contains("new-channel-desk-stop-and-archive-rule"));
    assert!(!app.contains("new-channel-desk-read-files-rule"));
}

#[test]
fn close_session_asks_first_then_the_session_is_gone() {
    let mut app = with_desk(false);
    let id = SessionId::new("cdesk-triage");
    app.send(AppAction::Navigate(Route::Session(id.clone())));
    assert!(app.contains("channel-desk-bar-close"));
    app.click("channel-desk-bar-close");
    assert!(app.contains("channel-desk-close"));
    assert!(app.contains("channel-desk-copy-flaky-report.md"));
    assert!(
        app.fakes.daemon.workspace().desk.channel_sessions.contains_key(&id),
        "it is still open until confirmed"
    );
    app.click("channel-desk-close-confirm");
    assert!(!app.fakes.daemon.workspace().desk.channel_sessions.contains_key(&id));
}

fn with_changes(change: impl FnOnce(&mut Workspace)) -> App {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    change(&mut workspace);
    App::new(workspace)
}

#[test]
fn a_blocked_switch_in_the_info_panel_links_to_the_channels_agent_access() {
    let mut app = with_changes(|workspace| {
        if let Some(record) = workspace
            .desk
            .channel_sessions
            .get_mut(&SessionId::new("cdesk-triage"))
        {
            record.blocked = vec![humata2_clients::desk::Switch::StopAndArchive];
        }
    });
    app.open("cdesk-triage", Panel::Info);
    assert!(app.contains("channel-desk-info-stop-and-archive"));
    assert!(app.contains("channel-desk-info-stop-and-archive-rule"));
    assert!(!app.contains("channel-desk-info-read-files-rule"));
    app.click("channel-desk-info-stop-and-archive-rule");
    let (route, tab) = app.ui.read(|state| {
        (
            state.route.clone(),
            state.channel_pages.get(&payments()).map(|view| view.tab),
        )
    });
    assert_eq!(route, Route::Channel(payments()));
    assert_eq!(tab, Some(humata2_app::channel::Tab::Access));
}

#[test]
fn the_header_of_a_channel_desk_session_links_to_its_channels_desk_tab() {
    let mut app = with_changes(|_| {});
    app.send(AppAction::Navigate(Route::Session(SessionId::new("cdesk-triage"))));
    app.click("session-channel");
    let (route, tab) = app.ui.read(|state| {
        (
            state.route.clone(),
            state.channel_pages.get(&payments()).map(|view| view.tab),
        )
    });
    assert_eq!(route, Route::Channel(payments()));
    assert_eq!(tab, Some(humata2_app::channel::Tab::Desk));
}

#[test]
fn a_session_whose_target_went_says_so_and_closes_through_its_bar() {
    let mut app = with_changes(|workspace| {
        if let Some(record) = workspace
            .desk
            .channel_sessions
            .get_mut(&SessionId::new("cdesk-backfill"))
        {
            record.target_gone = true;
        }
    });
    app.send(AppAction::OpenChannelDesk(payments()));
    assert!(app.contains("channel-desk-target-gone-cdesk-backfill"));
    assert!(!app.contains("channel-desk-target-gone-cdesk-triage"));
    app.send(AppAction::Navigate(Route::Session(SessionId::new("cdesk-backfill"))));
    assert!(app.contains("channel-desk-bar-target-gone"));
    app.click("channel-desk-bar-close");
    assert!(app.contains("channel-desk-close"));
}

#[test]
fn a_sessions_menu_starts_a_desk_session_for_it() {
    let mut app = with_changes(|_| {});
    app.send(AppAction::Navigate(Route::Session(SessionId::new("ledger"))));
    app.click("session-menu");
    assert!(app.contains("session-menu-new-desk"));
    app.click("session-menu-new-desk");
    assert!(app.contains("new-channel-desk"));
    assert!(!app.contains("session-menu-card"));
    let target = app.ui.read(|state| {
        state
            .desk
            .channel_new
            .as_ref()
            .and_then(|form| form.target().cloned())
    });
    assert_eq!(target, Some(SessionId::new("ledger")));
}

#[test]
fn a_desk_sessions_menu_does_not_offer_a_desk_session_for_itself() {
    let mut app = with_changes(|_| {});
    app.send(AppAction::Navigate(Route::Session(SessionId::new("cdesk-triage"))));
    app.click("session-menu");
    assert!(app.contains("session-menu-card"));
    assert!(!app.contains("session-menu-new-desk"));
}

#[test]
fn the_info_panel_lists_the_desk_sessions_looking_after_a_session() {
    let mut app = with_changes(|_| {});
    app.open("ledger", Panel::Info);
    assert!(app.contains("desk-info-targeting-cdesk-backfill"));
    assert!(!app.contains("desk-info-targeting-cdesk-triage"));
}

/// DOCV (DV-13): a closed channel Desk session whose folder holds a version
/// is under Closed with Reopen, and its Markdown is under Documents. The
/// fake daemon cannot reopen a channel Desk session, so the click is not
/// driven here.
#[test]
fn docv_a_closed_kept_desk_session_shows_under_closed_and_documents() {
    let mut app = with_changes(|workspace| {
        if let Some(record) = workspace
            .desk
            .channel_sessions
            .get_mut(&SessionId::new("cdesk-triage"))
        {
            record.desk.closed = Some(humata2_clients::desk::Closed {
                when: "today".to_owned(),
                days_left: None,
            });
            record.desk.version_count = 2;
        }
    });
    app.send(AppAction::OpenChannelDesk(payments()));
    assert!(!app.contains("channel-desk-row-cdesk-triage"));
    assert!(app.contains("channel-desk-row-cdesk-backfill"));
    assert!(app.contains("channel-desk-closed-cdesk-triage"));
    assert!(app.contains("channel-desk-reopen-cdesk-triage"));
    assert!(app.contains("channel-desk-document-cdesk-triage-flaky-report.md"));
    assert!(app.contains("channel-desk-document-cdesk-triage-retry-plan.md"));
    assert!(!app.contains("channel-desk-versions-cdesk-backfill"));
}

#[test]
fn docv_an_open_desk_session_with_versions_says_how_many() {
    let mut app = with_changes(|workspace| {
        if let Some(record) = workspace
            .desk
            .channel_sessions
            .get_mut(&SessionId::new("cdesk-triage"))
        {
            record.desk.version_count = 2;
        }
    });
    app.send(AppAction::OpenChannelDesk(payments()));
    assert!(app.contains("channel-desk-versions-cdesk-triage"));
    assert!(!app.contains("channel-desk-versions-cdesk-backfill"));
    assert!(app.contains("channel-desk-document-cdesk-triage-flaky-report.md"));
    assert!(!app.contains("channel-desk-closed-cdesk-triage"));
}

/// DOCV (DV-13): closing a channel Desk session whose folder holds a version
/// says the folder stays under Closed; one with no version says it goes to
/// the trash.
#[test]
fn docv_closing_a_session_with_a_version_says_the_folder_is_kept() {
    let mut app = with_changes(|workspace| {
        if let Some(record) = workspace
            .desk
            .channel_sessions
            .get_mut(&SessionId::new("cdesk-triage"))
        {
            record.desk.version_count = 1;
        }
    });
    app.send(AppAction::Navigate(Route::Session(SessionId::new("cdesk-triage"))));
    app.click("channel-desk-bar-close");
    assert!(app.contains("channel-desk-close-kept"));
    assert!(!app.contains("channel-desk-close-trash"));
}

#[test]
fn docv_closing_a_session_with_no_version_says_the_folder_goes_to_the_trash() {
    let mut app = with_desk(false);
    app.send(AppAction::Navigate(Route::Session(SessionId::new("cdesk-triage"))));
    app.click("channel-desk-bar-close");
    assert!(app.contains("channel-desk-close-trash"));
    assert!(!app.contains("channel-desk-close-kept"));
}

/// NOPATH (PL-10): the channel Desk session's folder bar hides the path, and
/// Copy path puts the folder's absolute path on the clipboard.
#[test]
fn nopath_the_channel_desk_bar_copies_the_path_it_does_not_show() {
    let mut app = with_desk(false);
    let id = SessionId::new("cdesk-triage");
    let path = app
        .fakes
        .daemon
        .workspace()
        .desk
        .channel_sessions
        .get(&id)
        .map(|record| record.desk.folder.path().to_owned())
        .expect("the fixture has the session");
    app.send(AppAction::Navigate(Route::Session(id)));
    assert!(app.contains("channel-desk-bar"));
    app.click("channel-desk-bar-copy-path");
    let copied = app
        .cx
        .update(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied, Some(path));
}
