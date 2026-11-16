//! Hosts and Settings (frames 28, 29).
use super::*;

#[test]
fn hosts_and_settings_reference_images() {
    use humata2_clients::model::MachineId;
    use humata2_hosts::HostsAction as H;
    let mut app = App::new(fixture::design());
    app.fold_quiet_channels();
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Hosts(H::Select(MachineId::new("devbox-1"))));
    assert!(app.contains("host-row devbox-1"));
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
    assert!(!app.contains("sidebar-pinned-ledger"));
    app.send(AppAction::Navigate(Route::Session(SessionId::new(
        "ledger",
    ))));
    assert!(app.contains("add-tab-0") && app.contains("add-tab-1"));
    app.send(AppAction::Switch(Route::Settings));
    app.click("settings-theme-Light");
    app.cx
        .update(|cx| assert!(!gpui_kit::component::ActiveTheme::theme(cx).is_dark()));
    app.click("settings-font-plus");
    assert_eq!(app.ui.read(|s| s.settings.preferences.font_size), 13);
    app.click("settings-long-lines-Wrap");
    assert!(app.ui.read(|s| s.review.wrap));
}
