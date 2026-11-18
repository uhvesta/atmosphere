//! A channel Desk session whose target is gone, and the keyboard over the
//! sidebar's Desk row (CD-2, CD-5), through the real window.
use super::*;
use humata2_clients::model::ChannelId;
use humata2_sidebar::Move;

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn backfill() -> SessionId {
    SessionId::new("cdesk-backfill")
}

fn with_changes(change: impl FnOnce(&mut Workspace)) -> App {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    change(&mut workspace);
    App::new(workspace)
}

/// What the daemon says once the target is deleted or has left the channel.
fn ledger_deleted(workspace: &mut Workspace) {
    if let Some(record) = workspace.desk.channel_sessions.get_mut(&backfill()) {
        record.target_gone = true;
    }
}

fn target_of(app: &mut App) -> Option<SessionId> {
    app.ui.read(|state| {
        state
            .workspace
            .desk
            .channel_sessions
            .get(&backfill())
            .and_then(|record| record.target.clone())
    })
}

#[test]
fn a_target_the_daemon_says_is_gone_shows_so_on_the_row_and_the_bar_with_a_way_out() {
    let mut app = with_changes(ledger_deleted);
    app.send(AppAction::OpenChannelDesk(payments()));
    assert!(app.contains("channel-desk-target-gone-cdesk-backfill"));
    assert!(!app.contains("channel-desk-target-gone-cdesk-triage"));
    app.send(AppAction::Navigate(Route::Session(backfill())));
    assert!(app.contains("channel-desk-bar-target-gone"));
    assert!(app.contains("channel-desk-bar-pick-target"));
    assert!(app.contains("channel-desk-bar-clear-target"));
}

#[test]
fn a_target_that_is_there_offers_no_way_out() {
    let mut app = with_changes(|_| {});
    app.send(AppAction::Navigate(Route::Session(backfill())));
    assert!(app.contains("channel-desk-bar"));
    assert!(!app.contains("channel-desk-bar-target-gone"));
    assert!(!app.contains("channel-desk-bar-pick-target"));
}

#[test]
fn picking_a_session_in_the_bar_makes_it_the_target() {
    let mut app = with_changes(ledger_deleted);
    app.send(AppAction::Navigate(Route::Session(backfill())));
    app.click("channel-desk-bar-pick-target");
    assert!(app.contains("channel-desk-bar-target-menu"));
    assert!(app.contains("channel-desk-bar-target-brin"));
    assert!(!app.contains("channel-desk-bar-target-cdesk-triage"));
    app.click("channel-desk-bar-target-brin");
    assert_eq!(target_of(&mut app), Some(SessionId::new("brin")));
    assert!(!app.contains("channel-desk-bar-target-gone"));
    assert!(!app.contains("channel-desk-bar-target-menu"));
}

#[test]
fn clearing_the_target_in_the_bar_looks_after_the_whole_channel() {
    let mut app = with_changes(ledger_deleted);
    app.send(AppAction::Navigate(Route::Session(backfill())));
    app.click("channel-desk-bar-clear-target");
    assert_eq!(target_of(&mut app), None);
    assert!(!app.contains("channel-desk-bar-target-gone"));
}

fn cursor(app: &mut App) -> Option<RowKey> {
    app.ui.read(|state| state.sidebar.cursor.clone())
}

#[test]
fn down_from_a_channel_reaches_its_desk_row_and_enter_opens_the_desk_tab() {
    let mut app = with_changes(|_| {});
    app.send(AppAction::Sidebar(SidebarAction::SetCurrent(Some(
        RowKey::Channel(payments()),
    ))));
    app.send(AppAction::Sidebar(SidebarAction::Move(Move::Down)));
    assert_eq!(cursor(&mut app), Some(RowKey::ChannelDesk(payments())));
    app.send(AppAction::Sidebar(SidebarAction::Move(Move::Down)));
    assert!(matches!(cursor(&mut app), Some(RowKey::Session(_))));
    app.send(AppAction::Sidebar(SidebarAction::Move(Move::Up)));
    app.send(AppAction::Sidebar(SidebarAction::OpenCursor));
    let (route, tab) = app.ui.read(|state| {
        (
            state.route.clone(),
            state.channel_pages.get(&payments()).map(|view| view.tab),
        )
    });
    assert_eq!(route, Route::Channel(payments()));
    assert_eq!(tab, Some(humata2_app::channel::Tab::Desk));
    assert!(app.contains("channel-desk-row-cdesk-triage"));
}

#[test]
fn clicking_the_desk_row_opens_the_desk_tab_and_marks_the_cursor() {
    let mut app = with_changes(|_| {});
    app.click("sidebar-channel-desk-payments-rewrite");
    assert_eq!(cursor(&mut app), Some(RowKey::ChannelDesk(payments())));
    assert!(app.contains("channel-desk-row-cdesk-backfill"));
}
