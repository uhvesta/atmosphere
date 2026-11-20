//! AGENTOFF: with an agent turned off in Settings › Agents nothing picks it.
//! Every default falls to an agent that is on, and with none on the app says
//! so and points to Settings › Agents.
use humata2_app::{App, AppAction as A, AppState, Overlay, Route, ToastAction};
use humata2_clients::model::{Agent, ForkKind, SessionId};
use humata2_clients::{AgentSwitch, fixture};
use humata2_desk::DeskAction;
use humata2_palette::{Command, PaletteAction, PullRef};
use humata2_session::SessionAction;
use humata2_settings::Section;
use std::sync::Arc;
use tca::{TestClock, TestStore};

/// Every built-in agent's switch, on when `on` says so.
fn switches(on: impl Fn(Agent) -> bool) -> Vec<AgentSwitch> {
    Agent::ALL
        .into_iter()
        .filter(|agent| *agent != Agent::Shell)
        .map(|agent| AgentSwitch {
            kind: agent.command().to_owned(),
            name: agent.name().to_owned(),
            agent: Some(agent),
            enabled: on(agent),
            install_when_missing: false,
            can_install: true,
            install_command: None,
            asks_in_inbox: true,
        })
        .collect()
}

/// The owner's report: Claude and Codex off, the rest on.
fn claude_and_codex_off(agent: Agent) -> bool {
    agent != Agent::Claude && agent != Agent::Codex
}

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

/// The app on the design workspace with Ledger a Claude session, Settings'
/// default Claude, and the switches `on`.
fn rig(on: impl Fn(Agent) -> bool) -> TestStore<App> {
    let mut workspace = fixture::design();
    for session in &mut workspace.sessions {
        if session.id == ledger() {
            session.agent = Agent::Claude;
        }
    }
    let (clients, fakes) = humata2_clients::fake_clients(workspace, Arc::new(TestClock::new()));
    fakes.daemon.set_agent_switches(switches(&on));
    let mut state = AppState::launch(&clients);
    state.settings.preferences.default_agent = Agent::Claude;
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.settle();
    // Launch reads the switches; set them again in case it hasn't yet.
    store.send(
        A::Settings(humata2_settings::SettingsAction::Agents(
            humata2_settings::AgentsAction::Loaded(Ok(switches(&on))),
        )),
        |_| {},
    );
    store
}

fn new_session_sheet(store: &TestStore<App>) -> humata2_app::NewSession {
    match store.state().overlay() {
        Some(Overlay::NewSession(sheet)) => sheet.clone(),
        other => panic!("the new session sheet is open, not {other:?}"),
    }
}

/// The last toast offers to open Settings › Agents.
fn toast_points_to_agent_settings(store: &TestStore<App>) -> u64 {
    let toast = store.state().toasts.last().expect("a toast says why nothing opened");
    assert_eq!(toast.action, Some(ToastAction::OpenAgentSettings));
    toast.id
}

/// Review a pull request, from the palette: the palette hands the command
/// back to the app, which runs it.
fn review_pull(store: &mut TestStore<App>, pull: PullRef) {
    store.send(A::Palette(PaletteAction::Begin(Command::ReviewPull(pull))), |_| {});
    store.receive_matching(
        "the palette runs the command",
        |action| matches!(action, A::Palette(PaletteAction::Delegate(humata2_palette::Delegate::Run(_)))),
        |_| {},
    );
}

#[test]
fn a_new_session_defaults_to_the_first_agent_that_is_on() {
    let mut store = rig(claude_and_codex_off);
    store.send(A::StartNewSession, |_| {});
    let sheet = new_session_sheet(&store);
    assert_eq!(sheet.agent, Agent::Copilot);
    assert_eq!(sheet.note, None);
}

#[test]
fn a_channel_default_that_is_off_falls_to_the_settings_default_when_that_is_on() {
    let store = rig(|agent| agent != Agent::Claude);
    let mut state = store.state().clone();
    state.settings.preferences.default_agent = Agent::Copilot;
    let channel = state.workspace.channels.first().map(|c| c.id.clone()).expect("a channel");
    if let Some(found) = Arc::make_mut(&mut state.workspace)
        .channels
        .iter_mut()
        .find(|c| c.id == channel)
    {
        found.default_agent = Some(Agent::Claude);
    }
    // Codex is on and comes first in the list, but Settings' pick is on too.
    assert_eq!(state.default_agent(Some(&channel)), Ok(Agent::Copilot));
}

#[test]
fn with_every_agent_off_a_new_session_offers_terminal_and_says_why() {
    let mut store = rig(|_| false);
    store.send(A::StartNewSession, |_| {});
    let sheet = new_session_sheet(&store);
    assert_eq!(sheet.agent, Agent::Shell);
    assert!(sheet.note.is_some(), "the sheet says every agent is off");
    assert_eq!(sheet.error, None, "it is a note, not an error");
    assert!(store.state().default_agent(None).is_err());
}

#[test]
fn a_new_desk_session_defaults_to_an_agent_that_is_on() {
    let mut store = rig(claude_and_codex_off);
    store.send(A::StartNewDeskSession, |_| {});
    let agent = store.state().desk.new.as_ref().map(|form| form.agent);
    assert_eq!(agent, Some(Agent::Copilot));
}

#[test]
fn a_channel_desk_session_never_starts_on_an_agent_that_is_off() {
    let mut store = rig(claude_and_codex_off);
    let channel = store.state().workspace.channels.first().map(|c| c.id.clone()).expect("a channel");
    // The channel's Desk tab asks with Settings' saved default.
    store.send(
        A::Desk(DeskAction::StartChannelNew {
            channel,
            target: None,
            agent: Agent::Claude,
        }),
        |_| {},
    );
    let agent = store.state().desk.channel_new.as_ref().map(|form| form.agent);
    assert_eq!(agent, Some(Agent::Copilot));
}

#[test]
fn with_every_agent_off_a_new_desk_session_opens_nothing_and_points_to_settings() {
    let mut store = rig(|_| false);
    store.send(A::StartNewDeskSession, |_| {});
    assert!(store.state().desk.new.is_none());
    let toast = toast_points_to_agent_settings(&store);
    store.send(A::UndoToast(toast), |_| {});
    assert_eq!(store.state().route, Route::Settings);
    assert_eq!(store.state().settings.section, Section::Agents);
}

#[test]
fn forking_a_session_whose_agent_is_off_preselects_one_that_is_on() {
    let mut store = rig(claude_and_codex_off);
    store.send(A::Navigate(Route::Session(ledger())), |_| {});
    store.settle();
    store.send(A::Session(SessionAction::StartFork(ForkKind::Worktree)), |_| {});
    let fork = store.state().session.fork.clone().expect("the fork dialog is open");
    assert_eq!(fork.agent, Some(Agent::Copilot));
    assert!(fork.more, "the agent picker is shown, so the change is seen");
}

#[test]
fn forking_with_every_agent_off_opens_nothing_and_points_to_settings() {
    let mut store = rig(|_| false);
    store.send(A::Navigate(Route::Session(ledger())), |_| {});
    store.settle();
    store.send(A::Session(SessionAction::StartFork(ForkKind::Worktree)), |_| {});
    assert!(store.state().session.fork.is_none());
    toast_points_to_agent_settings(&store);
}

#[test]
fn a_pull_request_review_defaults_to_an_agent_that_is_on() {
    let mut store = rig(claude_and_codex_off);
    let pull = PullRef {
        owner: "acme".to_owned(),
        repo: "api".to_owned(),
        number: 777,
    };
    review_pull(&mut store, pull);
    assert!(store.state().pr_dialog.open);
    assert_eq!(store.state().pr_dialog.agent, Agent::Copilot);
}

#[test]
fn with_every_agent_off_a_pull_request_review_opens_nothing_and_points_to_settings() {
    let mut store = rig(|_| false);
    let pull = PullRef {
        owner: "acme".to_owned(),
        repo: "api".to_owned(),
        number: 777,
    };
    review_pull(&mut store, pull);
    assert!(!store.state().pr_dialog.open);
    toast_points_to_agent_settings(&store);
}

#[test]
fn turning_the_saved_default_back_on_restores_it() {
    let mut store = rig(claude_and_codex_off);
    assert_eq!(store.state().default_agent(None), Ok(Agent::Copilot));
    store.send(
        A::Settings(humata2_settings::SettingsAction::Agents(
            humata2_settings::AgentsAction::Loaded(Ok(switches(|_| true))),
        )),
        |_| {},
    );
    assert_eq!(store.state().default_agent(None), Ok(Agent::Claude));
}

/// The open session's Continue in another channel dialog.
fn start_continue(store: &mut TestStore<App>) {
    store.send(A::Navigate(Route::Session(ledger())), |_| {});
    store.settle();
    store.send(A::Session(SessionAction::StartContinue), |_| {});
}

#[test]
fn continuing_a_session_whose_agent_is_on_opens_the_dialog() {
    let mut store = rig(|_| true);
    start_continue(&mut store);
    assert!(store.state().session.continuing.is_some());
}

#[test]
fn continuing_a_session_whose_agent_is_off_opens_nothing_and_points_to_settings() {
    let mut store = rig(claude_and_codex_off);
    start_continue(&mut store);
    assert!(store.state().session.continuing.is_none());
    toast_points_to_agent_settings(&store);
}

#[test]
fn turning_the_agent_off_while_continuing_closes_the_dialog_and_points_to_settings() {
    let mut store = rig(|_| true);
    start_continue(&mut store);
    assert!(store.state().session.continuing.is_some());
    store.send(
        A::Settings(humata2_settings::SettingsAction::Agents(
            humata2_settings::AgentsAction::Loaded(Ok(switches(claude_and_codex_off))),
        )),
        |_| {},
    );
    assert!(store.state().session.continuing.is_none());
    toast_points_to_agent_settings(&store);
}
