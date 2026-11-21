//! D770, ST-15: the new session and Review a pull request dialogs preselect
//! the agent the daemon says a start takes in that channel, not the one the
//! Settings screen holds locally.
use humata2_app::{App, AppAction as A, AppState, Overlay, Route};
use humata2_clients::{
    Clients, fixture,
    model::{Agent, Channel, ChannelId, MachineId},
};
use humata2_clients::pull_requests::{PullRequestInfo, PullRequestRef, PullRequestState};
use humata2_pr_review::dialog::DialogAction;
use humata2_palette::{Command, PaletteAction, PullRef};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn rig(workspace: humata2_clients::Workspace, route: Route) -> TestStore<App> {
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    fakes.pull_requests.add_pull_request(PullRequestInfo {
        pr: PullRequestRef::new("acme/api", 777),
        title: "Change 777".to_owned(),
        author: "mara".to_owned(),
        base_branch: "main".to_owned(),
        head_branch: "change-777".to_owned(),
        head_sha: "abc123".to_owned(),
        files: 1,
        added: 1,
        removed: 0,
        state: PullRequestState::Open,
        checks: "none".to_owned(),
    });
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    state.route = route;
    state.settings.preferences.default_agent = Agent::Claude;
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

#[test]
fn a_new_session_starts_on_the_agent_the_daemon_says_the_channel_takes() {
    let mut workspace = fixture::design();
    workspace
        .channels
        .iter_mut()
        .find(|channel| channel.id.as_str() == "infra")
        .unwrap()
        .default_agent = Some(Agent::Copilot);
    let mut store = rig(workspace, Route::Channel(ChannelId::new("infra")));
    store.send(A::StartNewSession, |_| {});
    let Some(Overlay::NewSession(sheet)) = store.state().overlay() else {
        panic!("launch dialog")
    };
    assert_eq!(sheet.agent, Agent::Copilot);
    store.finish();
}

#[test]
fn a_channel_the_daemon_has_not_answered_for_takes_the_settings_pick() {
    let mut workspace = fixture::design();
    for channel in &mut workspace.channels {
        channel.default_agent = None;
    }
    let mut store = rig(workspace, Route::Channel(ChannelId::new("infra")));
    store.send(A::StartNewSession, |_| {});
    let Some(Overlay::NewSession(sheet)) = store.state().overlay() else {
        panic!("launch dialog")
    };
    assert_eq!(sheet.agent, Agent::Claude);
    store.finish();
}

#[test]
fn review_a_pull_request_starts_on_the_agent_the_reviews_channel_takes() {
    let mut workspace = fixture::design();
    workspace.channels.push(Channel {
        id: ChannelId::new("pr-reviews-channel"),
        name: "PR Reviews".to_owned(),
        repos: Vec::new(),
        machines: vec![MachineId::new("this-mac")],
        purpose: String::new(),
        default_agent: Some(Agent::Codex),
    });
    let mut store = rig(workspace, Route::Home);
    let pull = PullRef { owner: "acme".to_owned(), repo: "api".to_owned(), number: 777 };
    store.send(A::Palette(PaletteAction::Begin(Command::ReviewPull(pull))), |_| {});
    store.receive_matching(
        "GitHub answered for the pasted pull request",
        |action| matches!(action, A::PrDialog(DialogAction::Resolved { .. })),
        |_| {},
    );
    assert!(store.state().pr_dialog.open);
    assert_eq!(store.state().pr_dialog.agent, Agent::Codex);
}
