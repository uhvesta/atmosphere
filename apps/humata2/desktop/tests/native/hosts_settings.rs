//! Hosts and Settings (frames 28, 29).
use super::*;

#[test]
fn hosts_and_settings_reference_images() {
    use humata2_clients::model::MachineId;
    use humata2_hosts::HostsAction as H;
    let mut app = App::new(fixture::design());
    // D784: two terminals no channel has count in devbox-1's "N sessions".
    let orphans = [39_001, 39_002].map(|pid| humata2_clients::RunningHolder {
        tab: Some(format!("tab-{pid}")),
        session: None,
        title: String::new(),
        channel: None,
        desk: false,
        agent: humata2_clients::model::Agent::Shell,
        pid: Some(pid),
        folder: "~/scratch/probe".to_owned(),
        started_unix_ms: None,
        attached: false,
        machine: MachineId::new("devbox-1"),
        stopped: false,
        agent_tab: false,
        archived: false,
        paused_for_move: false,
    });
    app.fakes.daemon.set_running_holders(orphans.to_vec());
    app.fold_quiet_channels();
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    // Opening the page reads the holders, so the orphans are counted.
    app.send(AppAction::Switch(Route::Hosts));
    app.send(AppAction::Hosts(H::Select(MachineId::new("devbox-1"))));
    assert!(app.contains("host-row devbox-1"));
    assert_eq!(app.ui.read(|state| state.hosts.orphans.get(&MachineId::new("devbox-1")).copied()), Some(2));
    // The golden's tolerance can hide one changed count, so the image is
    // also written out for a person to look at, when the run keeps outputs.
    if let Some(out) = std::env::var_os("TEST_UNDECLARED_OUTPUTS_DIR") {
        app.draw();
        let folder = std::path::PathBuf::from(out).join("candidates");
        std::fs::create_dir_all(&folder).expect("candidate folder");
        let image = app.cx.capture_screenshot(app.window).expect("headless image");
        image.save(folder.join("28-hosts-candidate.png")).expect("image written");
    }
    app.snapshot("28-hosts");
    app.send(AppAction::Switch(Route::Settings));
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.snapshot("29-settings");
}

#[test]
fn host_reconnect_and_shell_use_the_selected_machine() {
    use humata2_clients::model::{Agent, MachineId};
    use humata2_hosts::HostsAction as H;
    let mut app = App::new(fixture::design());
    app.send(AppAction::Hosts(H::Select(MachineId::new("gpu-box"))));
    app.click("host-connect gpu-box");
    assert!(app.ui.read(|s| {
        s.workspace
            .machine(&MachineId::new("gpu-box"))
            .unwrap()
            .online
    }));
    app.cx
        .update_window(app.window, |_, window, cx| {
            window.resize(size(px(720.), px(760.)));
            window.bounds_changed(cx);
        })
        .expect("resize Hosts");
    app.settle();
    app.click("host-open-shell");
    assert!(app.ui.read(|s|matches!(s.overlay(),Some(humata2_app::Overlay::NewSession(sheet)) if sheet.machine==Some(MachineId::new("gpu-box"))&&sheet.agent==Agent::Shell)));
}

#[test]
fn settings_visit_keeps_both_tab_groups_and_layout_rows_work() {
    let mut workspace = fixture::design();
    workspace
        .sessions
        .iter_mut()
        .find(|s| s.id.as_str() == "ledger")
        .unwrap()
        .pinned = true;
    let mut app = App::new(workspace);
    app.open("ledger", Panel::Changes);
    app.click("add-tab-0");
    app.click("split-terminal-0-zsh");
    assert!(app.contains("add-tab-1"));
    app.send(AppAction::Switch(Route::Settings));
    app.click("settings-pinned");
    assert!(!app.contains("sidebar-pinned"));
    app.send(AppAction::Navigate(Route::Session(SessionId::new(
        "ledger",
    ))));
    assert!(app.contains("add-tab-0") && app.contains("add-tab-1"));
    app.send(AppAction::Switch(Route::Settings));
    app.click("settings-theme-Light");
    app.cx
        .update(|cx| assert!(!gpui_kit::component::ActiveTheme::theme(cx).is_dark()));
    app.click("settings-font");
    assert!(app.contains("settings-font-13"));
    app.click("settings-font-13");
    assert!(!app.contains("settings-font-13"));
    assert_eq!(app.ui.read(|s| s.settings.preferences.font_size), 13);
    app.click("settings-long-lines-Wrap");
    assert!(app.ui.read(|s| s.review.wrap));
}

#[test]
fn permission_request_alerts_can_be_toggled_without_removing_requests() {
    use humata2_settings::{Section, SettingsAction};
    let mut app = App::new(fixture::design());
    let waiting = app.ui.read(|state| state.workspace.waiting().count());
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Notifications,
    )));
    assert!(
        !app.ui
            .read(|state| state.settings.preferences.permission_request_alerts)
    );
    app.click("settings-permission-alerts");
    assert!(
        app.ui
            .read(|state| state.settings.preferences.permission_request_alerts)
    );
    app.cx.update(|cx| Theme::change(ThemeMode::Dark, None, cx));
    app.click("settings-permission-alerts");
    assert!(
        !app.ui
            .read(|state| state.settings.preferences.permission_request_alerts)
    );
    assert_eq!(
        app.ui.read(|state| state.workspace.waiting().count()),
        waiting
    );
}

