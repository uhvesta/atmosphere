//! Home, the global Inbox and the empty Home (frames 25-27, 34).
use super::*;

/// N1: Home and wide Inbox are real pages over the clients, in both appearances.
#[test]
fn home_and_global_inbox_reference_images() {
    for (name, route, mode) in [
        ("25-home", Route::Home, ThemeMode::Light),
        ("26-inbox", Route::Inbox, ThemeMode::Light),
        ("27-inbox-dark", Route::Inbox, ThemeMode::Dark),
    ] {
        let mut workspace = fixture::design();
        for s in &mut workspace.sessions {
            s.vip = s.id.as_str() == "ledger";
            s.pinned = route == Route::Home && ["ledger", "pr-482"].contains(&s.id.as_str());
        }
        let mut app = App::new(workspace);
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.send(AppAction::Switch(route));
        for id in ["pr-reviews", "web"] {
            app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
        app.snapshot(name);
    }
}

/// IN-3/7/15: a card's answer answers only that request, a channel chip
/// narrows the cards, and History shows the answered ones.
#[test]
fn the_global_inbox_answers_only_the_selected_request() {
    use humata2_clients::model::{Answer, RequestId};
    let mut app = App::new(fixture::design());
    app.click("rail-inbox");
    assert!(app.contains("request move-ledger"));
    assert!(app.contains("request push-brin"));

    // A channel chip shows only that channel's cards; All shows them again.
    assert!(app.contains("request benchmark-command"));
    app.click("chip payments-rewrite");
    assert!(app.contains("request push-brin"));
    assert!(!app.contains("request benchmark-command"));
    app.click("chip all");
    assert!(app.contains("request benchmark-command"));

    app.scroll_to("global-inbox-detail", "deny push-brin");
    app.click("deny push-brin");
    assert!(app.ui.read(|s| {
        s.workspace
            .requests
            .iter()
            .find(|r| r.id.as_str() == "push-brin")
            .unwrap()
            .answer
            == Some(Answer::Denied { reply: None })
    }));
    assert!(app.ui.read(|s| {
        s.workspace
            .requests
            .iter()
            .find(|r| r.id == RequestId::new("move-ledger"))
            .unwrap()
            .answer
            .is_none()
    }));
    assert!(!app.contains("request push-brin"));
    assert!(app.contains("request move-ledger"));

    // History shows what was answered; leaving it shows what waits.
    app.click("inbox-history");
    assert!(app.contains("request push-brin"));
    assert!(!app.contains("request move-ledger"));
    app.click("inbox-history");
    assert!(!app.contains("request push-brin"));
    assert!(app.contains("request move-ledger"));
}

/// Home actions open the right session and Resume the exited process.
#[test]
fn home_review_and_resume_act_on_their_own_rows() {
    let mut workspace = fixture::design();
    // The design fixture's only exited session, postgres, also has a pending
    // request, which makes its row offer Review. Move that request to another
    // session so postgres's row offers Resume.
    for request in &mut workspace.requests {
        if request.session.as_str() == "postgres" {
            request.session = SessionId::new("partial-brin");
        }
    }
    let mut app = App::new(workspace);
    app.click("home-needs-action-0");
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("ledger"))
    );
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Inbox));
    app.click("rail-home");
    // Home's Needs you rows come from `needs_you`, which also lists sessions
    // with a pending request. Find postgres's row by its session, not by its
    // place among the sessions that need the person.
    let postgres = app.ui.read(|s| {
        humata2_app::needs_you(&s.workspace)
            .rows
            .iter()
            .position(|row| {
                matches!(
                    row,
                    humata2_app::NeedsRow::Session { session, .. }
                        if session.id.as_str() == "postgres"
                )
            })
    });
    let Some(postgres) = postgres else {
        panic!("the design workspace has an exited postgres session");
    };
    app.click(&format!("home-needs-action-{postgres}"));
    assert_eq!(
        app.ui.read(|s| s.session.open.clone()),
        Some(SessionId::new("postgres"))
    );
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .session(&SessionId::new("postgres"))
            .unwrap()
            .status),
        Status::Idle
    );
}

/// Frame 34: with no channel yet, Home and the sidebar open the same forms.
#[test]
fn empty_home_and_sidebar_actions_open_their_forms() {
    let mut app = App::new(fixture::empty());
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.snapshot("34-empty-home");
    for id in ["first-channel-create", "sidebar-first-channel"] {
        app.click(id);
        assert!(
            app.ui
                .read(|s| matches!(s.overlay(), Some(humata2_app::Overlay::NewChannel(_)))),
            "overlay after {id}: {:?}",
            app.ui.read(|s| s.overlays.clone())
        );
        assert!(app.contains("new-channel-create"));
        app.send(AppAction::Escape);
    }
    // The Desk needs no channel: Home starts a session on it and opens it.
    app.click("home-new-desk-session");
    assert!(app.contains("new-desk"));
    app.send(AppAction::Escape);
    assert!(!app.contains("new-desk"));
    app.click("first-desk-open");
    assert!(app.ui.read(|s| s.route == Route::Desk));
    app.click("rail-home");
    app.click("first-machine-add");
    assert!(app.contains("add-machine-start"));
    app.click("add-machine-cancel");
    assert!(!app.contains("add-machine-start"));
    // The Hosts page has its own "Add a machine…"; the sidebar offers it
    // from the other pages.
    assert!(!app.contains("sidebar-add-machine"));
    app.click("rail-home");
    app.click("sidebar-add-machine");
    assert!(app.contains("add-machine-start"));
    app.send(AppAction::AddMachine(humata2_hosts::AddMachineAction::SelectHost(
        "dev@newbox".into(),
    )));
    app.click("add-machine-start");
    assert!(app.ui.read(|s| {
        s.workspace
            .machines
            .iter()
            .any(|m| m.name == "newbox" && m.online)
    }));
    // With a second machine listed, the sidebar no longer offers the first.
    assert!(!app.contains("sidebar-add-machine"));
}
