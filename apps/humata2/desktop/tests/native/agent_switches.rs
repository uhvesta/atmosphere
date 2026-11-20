//! Settings › Agents › Agents (AGENTSET, D725): a switch turns an agent off,
//! and the pickers stop offering it. Headless.
use super::*;

#[test]
fn turning_an_agent_off_in_settings_takes_it_out_of_the_pickers() {
    use humata2_clients::model::Agent;
    use humata2_settings::{Section, SettingsAction};
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Agents,
    )));
    assert!(app.contains("settings-agent-enabled-copilot"));
    assert!(
        app.ui
            .read(|state| state.settings.offered_agents())
            .contains(&Agent::Copilot)
    );
    app.click("settings-agent-enabled-copilot");
    let offered = app.ui.read(|state| state.settings.offered_agents());
    assert!(!offered.contains(&Agent::Copilot));
    assert!(offered.contains(&Agent::Claude));
    // The switch shows off, and the daemon kept it.
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Notifications,
    )));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Agents,
    )));
    assert!(
        !app.ui
            .read(|state| state.settings.offered_agents())
            .contains(&Agent::Copilot)
    );
}

/// Draws the Agents card with Claude on, Codex on with "Install when
/// missing" on, and Copilot off, into the folder `AGENTSET_FRAME_OUT` names.
/// Does nothing when the variable is unset, and never touches a golden.
#[test]
fn the_agents_card_can_be_drawn_for_review() {
    use humata2_settings::{Section, SettingsAction};
    let Some(folder) = std::env::var_os("AGENTSET_FRAME_OUT") else {
        return;
    };
    let mut app = App::new(fixture::design());
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Agents,
    )));
    app.click("settings-agent-install-codex");
    app.click("settings-agent-enabled-copilot");
    app.draw();
    let image = app
        .cx
        .capture_screenshot(app.window)
        .expect("headless image");
    let folder = std::path::PathBuf::from(folder);
    std::fs::create_dir_all(&folder).expect("the frame folder is made");
    image
        .save(folder.join("settings-agents.png"))
        .expect("the frame is written");
}

/// D782: each enabled agent has its own "Ask in the Inbox" switch. Turning
/// Copilot's off changes only Copilot, the daemon keeps it, and the row goes
/// while Copilot itself is off.
#[test]
fn each_agent_has_its_own_ask_in_the_inbox_switch() {
    use humata2_clients::model::Agent;
    use humata2_settings::{Section, SettingsAction};
    let asks = |app: &App, agent: Agent| {
        app.ui.read(|state| {
            state
                .settings
                .agents
                .switches
                .as_ref()
                .and_then(|all| all.iter().find(|s| s.agent == Some(agent)))
                .map(|s| s.asks_in_inbox)
        })
    };
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Agents,
    )));
    assert!(app.contains("settings-agent-asks-claude"));
    assert!(app.contains("settings-agent-asks-copilot"));
    assert!(!app.contains("settings-asks-in-inbox"));
    assert_eq!(asks(&app, Agent::Copilot), Some(true));
    app.click("settings-agent-asks-copilot");
    assert_eq!(asks(&app, Agent::Copilot), Some(false));
    assert_eq!(asks(&app, Agent::Claude), Some(true));
    // Leaving and coming back reads what the daemon kept.
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Notifications,
    )));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Agents,
    )));
    assert_eq!(asks(&app, Agent::Copilot), Some(false));
    // A turned-off agent has no row.
    app.click("settings-agent-enabled-copilot");
    assert!(!app.contains("settings-agent-asks-copilot"));
    assert!(app.contains("settings-agent-asks-claude"));
}

/// Frame 117 (D782, light): every agent on, Copilot's "Ask in the Inbox"
/// off. Fable accepted this capture on 2026-11-17.
#[test]
fn the_ask_in_the_inbox_rows_match_frame_117() {
    use humata2_settings::{Section, SettingsAction};
    let mut app = App::new(fixture::design());
    app.cx
        .update(|cx| Theme::change(ThemeMode::Light, None, cx));
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Agents,
    )));
    app.click("settings-agent-asks-copilot");
    app.snapshot("117-settings-agents-ask-in-inbox");
}

/// AGY: Antigravity (`agy`) appears in Settings › Agents with enabled on,
/// install-when-missing off (opt-in with its built-in install command), and
/// ask-in-the-inbox on; toggling each updates only Antigravity and persists.
#[test]
fn antigravity_switches_control_offering_install_and_inbox_asks() {
    use humata2_clients::model::Agent;
    use humata2_settings::{Section, SettingsAction};
    let agy_switch = |app: &App| {
        app.ui.read(|state| {
            state
                .settings
                .agents
                .switches
                .as_ref()
                .and_then(|all| all.iter().find(|s| s.agent == Some(Agent::Agy)))
                .cloned()
        })
    };
    let mut app = App::new(fixture::design());
    app.send(AppAction::Switch(Route::Settings));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Agents,
    )));
    assert!(app.contains("settings-agent-enabled-agy"));
    assert!(app.contains("settings-agent-install-agy"));
    assert!(app.contains("settings-agent-asks-agy"));
    assert!(
        app.ui
            .read(|state| state.settings.offered_agents())
            .contains(&Agent::Agy)
    );
    let initial = agy_switch(&app).expect("agy switch present");
    assert!(initial.enabled);
    assert!(!initial.install_when_missing);
    assert!(initial.can_install);
    assert_eq!(
        initial.install_command.as_deref(),
        Some("curl -fsSL https://antigravity.google/cli/install.sh | bash")
    );
    assert!(initial.asks_in_inbox);

    // Opt in to install-when-missing and turn off ask-in-the-inbox.
    app.scroll_to("settings-content", "settings-agent-install-agy");
    app.click("settings-agent-install-agy");
    app.scroll_to("settings-content", "settings-agent-asks-agy");
    app.click("settings-agent-asks-agy");
    let updated = agy_switch(&app).expect("agy switch present");
    assert!(updated.install_when_missing);
    assert!(!updated.asks_in_inbox);

    // Turning Antigravity off removes it from the pickers and hides its inbox row.
    app.scroll_to("settings-content", "settings-agent-enabled-agy");
    app.click("settings-agent-enabled-agy");
    assert!(
        !app.ui
            .read(|state| state.settings.offered_agents())
            .contains(&Agent::Agy)
    );
    assert!(!app.contains("settings-agent-asks-agy"));

    // Leaving and returning reads what the daemon kept.
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Notifications,
    )));
    app.send(AppAction::Settings(SettingsAction::Section(
        Section::Agents,
    )));
    let persisted = agy_switch(&app).expect("agy switch persisted");
    assert!(!persisted.enabled);
    assert!(persisted.install_when_missing);
    assert!(!persisted.asks_in_inbox);
}
