//! Settings clamp usable sizes and retain choices independently of sections.
use humata2_clients::{Clients, fixture};
use humata2_clients::daemon::TodoAutomation;
use humata2_settings::TodoAutomationAction;
use humata2_settings::{AgentsAction, Section, Settings, SettingsAction as A, SettingsState, Theme};
use std::sync::Arc;
use tca::{TestClock, TestStore};
#[test]
fn preferences_change_without_losing_other_choices() {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    fakes.daemon.set_agent_switches(all_on());
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));
    store.send(A::SetTheme(Theme::Light), |s| {
        s.preferences.theme = Theme::Light
    });
    store.send(A::SetFontSize(100), |s| s.preferences.font_size = 20);
    store.send(A::SetFontSize(3), |s| s.preferences.font_size = 10);
    store.send(A::TogglePinned, |s| s.preferences.show_pinned = false);
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.receive(A::CodexTrustShown(Ok(false)), |_| {});
    store.receive(A::Agents(AgentsAction::Loaded(Ok(all_on()))), |s| {
        s.agents.switches = Some(all_on());
    });
    store.receive_matching(
        "general loaded",
        |a| matches!(a, A::General(humata2_settings::GeneralAction::Loaded(Ok(_)))),
        |s| {
            let mut full = humata2_clients::daemon::FullSettings::default();
            full.terminal.theme = "light".to_owned();
            full.terminal.font_size = 10;
            full.revision = 3;
            s.general.settings = Some(full);
            s.general.terminal_font_draft = "Menlo".to_owned();
            s.general.scrollback_draft = "10000".to_owned();
            s.general.archive_pr_days_draft = "0".to_owned();
            s.general.archive_idle_days_draft = "0".to_owned();
        },
    );
    store.send(A::ToggleWrap, |s| {
        s.preferences.wrap_diffs = false;
        if let Some(full) = s.general.settings.as_mut() {
            full.long_lines = humata2_clients::daemon::LongLinesMode::Scroll;
            full.revision = 4;
        }
    });
    store.finish();
}
#[test]
fn picking_a_font_size_sets_it_and_closes_the_list() {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));
    store.send(A::FontMenu, |s| s.font_menu = true);
    store.send(A::SetFontSize(15), |s| {
        s.preferences.font_size = 15;
        s.font_menu = false;
    });
    store.finish();
}
#[test]
fn switching_section_closes_an_open_font_list() {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));
    store.send(A::FontMenu, |s| s.font_menu = true);
    store.send(A::Section(Section::General), |s| {
        s.section = Section::General;
        s.font_menu = false;
    });
    store.receive(A::Todo(TodoAutomationAction::Loaded(Ok(TodoAutomation::default()))), |s| {
        s.todo.section = Some(TodoAutomation::default());
    });
    store.receive(
        A::WebTodo(humata2_settings::WebTodoAction::Loaded(Ok(
            humata2_clients::daemon::WebTodoSnapshot::default(),
        ))),
        |s| {
            s.web_todo.snapshot = Some(humata2_clients::daemon::WebTodoSnapshot::default());
        },
    );
    store.receive_matching(
        "general loaded",
        |a| matches!(a, A::General(humata2_settings::GeneralAction::Loaded(Ok(_)))),
        |s| {
            s.general.settings = Some(humata2_clients::daemon::FullSettings::default());
            s.general.terminal_font_draft = "Menlo".to_owned();
            s.general.scrollback_draft = "10000".to_owned();
            s.general.archive_pr_days_draft = "0".to_owned();
            s.general.archive_idle_days_draft = "0".to_owned();
        },
    );
    store.finish();
}

fn all_on() -> Vec<humata2_clients::AgentSwitch> {
    use humata2_clients::model::Agent;
    [
        Agent::Claude,
        Agent::Codex,
        Agent::Copilot,
        Agent::OpenCode,
        Agent::Agy,
    ]
        .map(|agent| humata2_clients::AgentSwitch {
            kind: agent.command().to_owned(),
            name: agent.name().to_owned(),
            agent: Some(agent),
            enabled: true,
            install_when_missing: false,
            can_install: true,
            install_command: None,
            asks_in_inbox: true,
        })
        .to_vec()
}
