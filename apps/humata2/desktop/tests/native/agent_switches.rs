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
