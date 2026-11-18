//! A channel Desk session and the one it looks after (CD-2, CD-6): the Info
//! panel's list of Desk sessions, and New desk session from the session menu.
use super::*;
use humata2_clients::model::ChannelId;

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn with_changes(change: impl FnOnce(&mut Workspace)) -> App {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    change(&mut workspace);
    App::new(workspace)
}

#[test]
fn a_desk_session_in_the_info_panel_opens_that_desk_session() {
    let mut app = with_changes(|_| {});
    app.open("ledger", Panel::Info);
    assert!(app.contains("desk-info-targeting-cdesk-backfill"));
    app.click("desk-info-targeting-cdesk-backfill");
    let route = app.ui.read(|state| state.route.clone());
    assert_eq!(route, Route::Session(SessionId::new("cdesk-backfill")));
}

#[test]
fn a_closed_desk_session_is_no_longer_listed_in_the_info_panel() {
    let mut app = with_changes(|workspace| {
        if let Some(record) = workspace
            .desk
            .channel_sessions
            .get_mut(&SessionId::new("cdesk-backfill"))
        {
            record.desk.closed = Some(humata2_clients::desk::Closed {
                when: "today".to_owned(),
                days_left: None,
            });
        }
    });
    app.open("ledger", Panel::Info);
    assert!(!app.contains("desk-info-targeting-cdesk-backfill"));
}

#[test]
fn the_menus_dialog_is_for_the_sessions_channel_with_it_as_the_target() {
    let mut app = with_changes(|_| {});
    app.send(AppAction::Navigate(Route::Session(SessionId::new("ledger"))));
    app.click("session-menu");
    app.click("session-menu-new-desk");
    let form = app.ui.read(|state| state.desk.channel_new.clone());
    let Some(form) = form else {
        panic!("the dialog is open")
    };
    assert_eq!(form.channel, payments());
    assert!(form.on_session);
    assert_eq!(form.target(), Some(&SessionId::new("ledger")));
    assert!(form.machine.is_some(), "it starts on a machine");
}

#[test]
fn starting_from_the_menu_puts_the_new_desk_session_against_that_session() {
    let mut app = with_changes(|_| {});
    app.send(AppAction::Navigate(Route::Session(SessionId::new("ledger"))));
    app.click("session-menu");
    app.click("session-menu-new-desk");
    app.click("new-channel-desk-start");
    let workspace = app.fakes.daemon.workspace();
    let looking_after_ledger = workspace
        .desk
        .channel_sessions
        .values()
        .filter(|record| record.target == Some(SessionId::new("ledger")))
        .count();
    assert_eq!(looking_after_ledger, 2, "the fixture's one and the new one");
}

#[test]
fn the_info_panel_follows_a_desk_session_that_starts_and_then_closes() {
    use humata2_clients::desk::{DeskClient, NewChannelDesk, Switches};
    use humata2_clients::model::{Agent, MachineId};
    use humata2_desk::DeskAction;

    let mut app = with_changes(|_| {});
    app.open("ledger", Panel::Info);
    assert!(app.contains("desk-info-targeting-cdesk-backfill"));

    // Another window starts a Desk session against it; the app reads the
    // Desk again when told a change came back.
    let started = app.runtime.block_on(app.fakes.daemon.start_channel(NewChannelDesk {
        channel: payments(),
        target: Some(SessionId::new("ledger")),
        title: "Check the ledger totals".to_owned(),
        agent: Agent::Claude,
        machine: MachineId::new("this-mac"),
        switches: Switches::default(),
    }));
    let Ok(id) = started else {
        panic!("the fake starts it")
    };
    assert!(!app.contains(&format!("desk-info-targeting-{id}")));
    app.send(AppAction::Desk(DeskAction::Copied(Ok(()))));
    assert!(app.contains(&format!("desk-info-targeting-{id}")));
    assert!(app.contains("desk-info-targeting-cdesk-backfill"));

    // Closing one removes it from the open panel.
    app.send(AppAction::Desk(DeskAction::AskClose(id.clone())));
    app.click("channel-desk-close-confirm");
    assert!(!app.contains(&format!("desk-info-targeting-{id}")));
    assert!(app.contains("desk-info-targeting-cdesk-backfill"));
    let route = app.ui.read(|state| state.route.clone());
    assert_eq!(route, Route::Session(SessionId::new("ledger")));
}
