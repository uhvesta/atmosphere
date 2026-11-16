//! The session page: lifecycle frames 01-14, panels, tabs, the split, views and Fork.
use super::*;

/// SE-14/16, N1: the real session layouts in their approved lifecycle states.
#[test]
fn session_lifecycle_and_full_screen_reference_images() {
    for (name, mode, status, panel, full, folded) in [
        (
            "03-session-needs-you-light",
            ThemeMode::Light,
            Status::NeedsYou,
            Some(Panel::Inbox),
            None,
            false,
        ),
        (
            "04-session-needs-you-dark",
            ThemeMode::Dark,
            Status::NeedsYou,
            Some(Panel::Inbox),
            None,
            false,
        ),
        (
            "05-session-exited",
            ThemeMode::Dark,
            Status::Exited { code: 0 },
            Some(Panel::Info),
            None,
            false,
        ),
        (
            "06-session-offline",
            ThemeMode::Light,
            Status::Offline,
            Some(Panel::Files),
            None,
            false,
        ),
        (
            "07-session-board-panel",
            ThemeMode::Dark,
            Status::Working,
            Some(Panel::Board),
            None,
            false,
        ),
        (
            "08-session-journal-panel",
            ThemeMode::Light,
            Status::Working,
            Some(Panel::Journal),
            None,
            false,
        ),
        (
            "09-session-no-panel",
            ThemeMode::Dark,
            Status::Working,
            None,
            None,
            false,
        ),
        (
            "10-terminal-full-screen",
            ThemeMode::Dark,
            Status::Working,
            None,
            Some(FullScreen::Terminal),
            false,
        ),
        (
            "11-panel-full-screen-diff",
            ThemeMode::Light,
            Status::Working,
            Some(Panel::Changes),
            Some(FullScreen::Panel),
            false,
        ),
        (
            "12-panel-full-screen-board",
            ThemeMode::Dark,
            // The golden's PAY-3 card asks to move to Done: Ledger needs you.
            Status::NeedsYou,
            Some(Panel::Board),
            Some(FullScreen::Panel),
            false,
        ),
        (
            "13-columns-collapsed",
            ThemeMode::Dark,
            Status::Working,
            None,
            None,
            true,
        ),
    ] {
        // An offline session's program can't redraw when the layout
        // changes, so frame 06 is laid out while Ledger runs and the link
        // drops just before the capture, as it would for the person.
        let mut start = fixture::design_session(status);
        if status == Status::Offline {
            for machine in &mut start.machines {
                if machine.lost_at.is_some() {
                    machine.online = true;
                }
            }
            for session in &mut start.sessions {
                if session.status == Status::Offline {
                    session.status = Status::Working;
                }
            }
        }
        let mut app = App::new(start);
        app.cx.update(|cx| Theme::change(mode, None, cx));
        app.send(AppAction::Navigate(Route::Session(SessionId::new(
            "ledger",
        ))));
        app.send(AppAction::Session(SessionAction::SetPanel(panel)));
        app.file_tab();
        if status == Status::NeedsYou {
            for message in [
                "Keep both indexes until the backfill ends.",
                "Then check the read path before dropping the old index.",
            ] {
                app.send(AppAction::Session(SessionAction::Draft(message.into())));
                app.send(AppAction::Session(SessionAction::Submit));
            }
        }
        if let Some(full) = full {
            app.send(AppAction::ToggleFullScreen(full));
        }
        if folded {
            app.send(AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)));
            app.send(AppAction::Pane(PaneAction::Drag(PaneId::Panel, 0.)));
        }
        // Every frame's Ledger has one sent comment and this saved draft, so
        // the Diff rail reads 2 (frames 01–13).
        app.write_comment(
            "ledger",
            "api",
            43,
            "Keep insert_legacy until the backfill ends.",
        );
        if panel == Some(Panel::Changes) {
            app.send(AppAction::Review(
                humata2_review::ReviewAction::SelectFile {
                    repo: "api".into(),
                    path: "src/client.rs".into(),
                },
            ));
        }
        if panel == Some(Panel::Files) {
            app.send(AppAction::Session(SessionAction::SelectFile {
                repo: "api".into(),
                path: "src/client.rs".into(),
            }));
        }
        for id in ["pr-reviews", "web"] {
            app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
                humata2_clients::model::ChannelId::new(id),
            ))));
        }
        if status == Status::Offline {
            let offline = fixture::design_session(Status::Offline);
            let machine = offline
                .machines
                .iter()
                .find(|machine| machine.lost_at.is_some())
                .expect("the offline fixture says when devbox-1 was lost")
                .clone();
            let ledger = offline
                .sessions
                .iter()
                .find(|session| session.id == SessionId::new("ledger"))
                .expect("the fixture has Ledger")
                .clone();
            app.fakes.daemon.emit(humata2_clients::Event::Machine(machine));
            app.fakes.daemon.emit(humata2_clients::Event::Session(ledger));
            app.settle();
        }
        app.snapshot(name);
    }
}

/// SE-14: either Resume control starts the same exited agent and retains its view.
#[test]
fn resuming_an_exited_session_keeps_its_open_panel() {
    let mut app = App::new(fixture::design_session(Status::Exited { code: 0 }));
    app.open("ledger", Panel::Info);
    app.click("resume-agent");
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .session(&SessionId::new("ledger"))
            .expect("session")
            .status),
        Status::Idle
    );
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Info));
    assert!(!app.contains("resume-agent"));
}

/// SE-11: picking the same strip choice closes it; picking it again reopens it.
#[test]
fn clicking_the_open_panel_choice_closes_and_reopens_it() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Inbox);
    assert!(app.contains("panel-badge Inbox"));
    app.click("panel-tab Inbox");
    assert_eq!(app.ui.read(|s| s.panel()), None);
    assert!(!app.contains("side-panel"));
    app.click("panel-tab Inbox");
    assert_eq!(app.ui.read(|s| s.panel()), Some(Panel::Inbox));
    assert!(app.contains("side-panel"));
}

/// SE-8/10: shell tabs show their own output, and live output remains in its tab.
#[test]
fn terminal_tabs_show_only_the_selected_terminals_live_output() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    let offset = app.ui.read(|s| {
        s.session
            .terminal_bytes
            .get(&(SessionId::new("ledger"), 1))
            .map_or(0, |b| b.len())
    });
    app.send(AppAction::Session(SessionAction::TerminalData {
        session: SessionId::new("ledger"),
        tab: 1,
        offset,
        bytes: b"\r\nshell-result: 42\r\n".to_vec(),
    }));
    let capture = |app: &mut App| {
        app.draw();
        app.cx
            .update_window(app.window, |_, window, _| view::capture(window))
            .expect("window")
            .expect("view")
    };
    assert!(!capture(&mut app).contains("shell-result: 42"));
    app.click("terminal-tab 1");
    assert_eq!(app.ui.read(|s| s.session.view().expect("view").tab), 1);
    assert!(capture(&mut app).contains("shell-result: 42"));
    app.click("terminal-tab 0");
    assert!(!capture(&mut app).contains("shell-result: 42"));
}

/// SE-8: Open big creates a file tab; selecting/closing it keeps the terminals.
#[test]
fn files_open_in_the_tab_row_and_close_back_to_the_terminal() {
    use humata2_session::Tab;
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Files);
    app.click("session-file api/src/client.rs");
    app.click("files-open-big");
    assert!(app.contains("session-file"));
    assert!(app.ui.read(|s|matches!(s.session.view().unwrap().selected(0),Some(Tab::File {path,..}) if path=="src/client.rs")));
    app.snapshot("file-tab-open");
    app.click("terminal-tab 0");
    assert!(app.contains("terminal"));
    app.click("file-tab-0-2");
    app.click("close-tab-0-2");
    assert!(!app.contains("session-file"));
    assert!(app.contains("terminal"));
}

/// D76: plus opens a program; splitting twice still leaves just two groups.
#[test]
fn the_plus_menu_splits_once_and_joining_keeps_the_program_running() {
    let mut workspace = fixture::design();
    workspace.views.clear();
    // Frame 14 shows the agent's plan only, before the question.
    if let Some(ledger) = workspace
        .sessions
        .iter_mut()
        .find(|x| x.id == SessionId::new("ledger"))
        && let Some(end) = ledger
            .transcript
            .iter()
            .position(|line| line.text.contains("Switch reads"))
    {
        ledger.transcript.truncate(end + 1);
    }
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Info);
    app.send(AppAction::Session(SessionAction::SetPanel(None)));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    // Golden 14's Diff rail reads 2: one sent comment and this saved draft.
    app.write_comment(
        "ledger",
        "api",
        43,
        "Keep insert_legacy until the backfill ends.",
    );
    for channel in ["pr-reviews", "web"] {
        app.send(AppAction::Sidebar(SidebarAction::Toggle(RowKey::Channel(
            humata2_clients::model::ChannelId::new(channel),
        ))));
    }
    app.click("add-tab-0");
    assert!(app.contains("new-terminal-menu"));
    app.click("split-terminal-0-codex");
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
    assert!(app.contains("terminal-secondary"));
    app.snapshot("14-two-tab-groups");
    app.click("add-tab-1");
    app.click("split-terminal-1-codex");
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 2);
    let shells = app.ui.read(|s| {
        s.workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .shells
            .len()
    });
    app.click("close-group-1");
    assert_eq!(app.ui.read(|s| s.session.view().unwrap().groups.len()), 1);
    assert_eq!(
        app.ui.read(|s| s
            .workspace
            .session(&SessionId::new("ledger"))
            .unwrap()
            .shells
            .len()),
        shells
    );
    assert!(app.contains("terminal-tab 2"));
}

/// VW-2/4/6: the offer waits for consent; Later returns, Open it creates a tab.
#[test]
fn prepared_views_wait_for_open_and_later_returns_on_the_next_visit() {
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Inbox);
    assert!(app.contains("offer-open"));
    assert!(!app.contains("prepared-view-content"));
    app.click("offer-later");
    assert!(!app.contains("offer-open"));
    app.click("rail-home");
    app.click("sidebar-session-ledger");
    assert!(app.contains("offer-open"));
    app.click("offer-open");
    assert!(app.contains("prepared-view-content"));
    assert!(!app.contains("offer-open"));
    app.click("terminal-tab 0");
    assert!(app.contains("terminal"));
}

/// FK-1/2: the header opens a real dialog and the selected kind creates a child.
#[test]
fn the_fork_dialog_creates_the_selected_kind_and_can_cancel() {
    use humata2_clients::model::ForkKind;
    let mut app = App::new(fixture::design());
    app.open("ledger", Panel::Info);
    app.click("session-fork");
    assert!(app.contains("fork-create"));
    app.click("fork-kind-1");
    app.send(AppAction::Session(SessionAction::ForkTitle(
        "Review the migration".into(),
    )));
    app.click("fork-create");
    let id = app.ui.read(|s| s.session.open.clone()).unwrap();
    assert_ne!(id, SessionId::new("ledger"));
    assert!(app.ui.read(|s| {
        s.workspace.session(&id).is_some_and(|v| {
            v.parent.as_ref() == Some(&SessionId::new("ledger"))
                && v.fork == Some(ForkKind::Conversation)
        })
    }));
    app.click("session-fork");
    app.cx
        .update_window(app.window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    app.settle();
    assert!(!app.contains("fork-create"));
    assert!(app.ui.read(|s| s.session.fork.is_none()));
    app.click("session-fork");
    app.click("fork-cancel");
    assert!(app.ui.read(|s| s.session.fork.is_none()));
}
