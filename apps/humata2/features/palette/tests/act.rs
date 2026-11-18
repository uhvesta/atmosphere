//! Acting from the palette (flow 26.22; KP-2 to KP-5, PR-1, MV-1, ST-7):
//! the current session's Move ranks first with its shortcut, asks inline
//! for a machine and hands the move to the app; a pasted pull request URL
//! offers Review this PR; "long" reaches Settings › Long lines.
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::fixture;
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_clients::Workspace;
use humata2_palette::{
    Ask, Command, Delegate, Palette, PaletteAction as A, PaletteState, PullRef,
};
use humata2_settings::Row;
use tca::TestStore;

/// The design workspace, with "API rate limiter" (`rate`, on devbox-1) in
/// payments-rewrite. Its channel also lists gpu-box, online here, and
/// cloud-vm-3, offline; and a session whose title also matches "move".
fn workspace() -> Workspace {
    let mut workspace = fixture::design();
    for machine in &mut workspace.machines {
        if machine.id == MachineId::new("gpu-box") {
            machine.online = true;
        }
    }
    let channel = ChannelId::new("payments-rewrite");
    for channel in workspace.channels.iter_mut().filter(|c| c.id == channel) {
        channel.machines.push(MachineId::new("gpu-box"));
        channel.machines.push(MachineId::new("cloud-vm-3"));
    }
    let mut decoy = workspace
        .session(&SessionId::new("refund"))
        .cloned()
        .unwrap_or_else(|| panic!("the design workspace has refund"));
    decoy.id = SessionId::new("remove-env");
    decoy.title = "Remove legacy env vars".to_owned();
    decoy.parent = None;
    workspace.sessions.push(decoy);
    workspace
}

fn rate() -> SessionId {
    SessionId::new("rate")
}

/// A palette opened over "API rate limiter".
fn opened() -> TestStore<Palette> {
    let state = PaletteState {
        workspace: Arc::new(workspace()),
        context: Some(rate()),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette)
        .non_exhaustive()
        .with_timeout(Duration::from_millis(100));
    store.send(A::Open { in_session: true }, |s| s.open = true);
    store
}

fn typed(store: &mut TestStore<Palette>, query: &str) {
    store.send(A::Query(query.to_owned()), |s| s.query = query.to_owned());
}

fn index_of(store: &TestStore<Palette>, command: &Command) -> usize {
    store
        .state()
        .entries
        .iter()
        .position(|entry| &entry.command == command)
        .unwrap_or_else(|| {
            panic!(
                "{command:?} is offered; entries: {:?}",
                store.state().entries
            )
        })
}

/// Nothing reaches the app: no Run and no Closed.
fn delegates_nothing(store: &mut TestStore<Palette>) {
    let sent = store.try_receive_matching(
        "a delegate",
        |action| matches!(action, A::Delegate(_)),
        |_| {},
    );
    assert!(sent.is_err(), "nothing is handed to the app");
}

#[test]
fn typing_move_puts_the_sessions_move_first_with_its_shortcut() {
    let mut store = opened();
    typed(&mut store, "move");
    let Some(first) = store.state().entries.first() else {
        panic!("typing move offers something");
    };
    assert_eq!(first.command, Command::MoveSession);
    assert_eq!(first.title, "Move session to another machine…");
    assert_eq!(first.hint, "⌥⌘M");
    // A session whose title matches too is still offered, after it.
    assert!(
        store
            .state()
            .entries
            .iter()
            .any(|e| e.command == Command::Session(SessionId::new("remove-env")))
    );
}

#[test]
fn move_is_not_offered_outside_a_session() {
    let state = PaletteState {
        workspace: Arc::new(workspace()),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette).non_exhaustive();
    store.send(A::Open { in_session: false }, |s| s.open = true);
    typed(&mut store, "move");
    assert!(
        store
            .state()
            .entries
            .iter()
            .all(|e| e.command != Command::MoveSession)
    );
}

#[test]
fn running_move_asks_inline_for_another_online_machine_of_the_channel() {
    let mut store = opened();
    typed(&mut store, "move");
    store.send(A::Run, |s| {
        s.open = true;
        s.asking = Some(Ask::MoveSession {
            session: rate(),
            query: "move".to_owned(),
        });
        s.query = String::new();
    });
    delegates_nothing(&mut store);
    let offered: BTreeSet<MachineId> = store
        .state()
        .entries
        .iter()
        .map(|entry| match &entry.command {
            Command::MoveTo { session, machine } => {
                assert_eq!(session, &rate(), "the move is for the session under the palette");
                machine.clone()
            }
            other => panic!("only machines are offered while asking, not {other:?}"),
        })
        .collect();
    // Not devbox-1, where it runs; not cloud-vm-3, which is offline (MV-1).
    let expected: BTreeSet<MachineId> = ["this-mac", "gpu-box"]
        .into_iter()
        .map(MachineId::new)
        .collect();
    assert_eq!(offered, expected);
}

#[test]
fn a_review_may_move_to_any_other_online_machine_not_only_its_channels() {
    // PR-21: the PR reviews channel lists the machines a review was started
    // on; moving gives the target the channel, so every online machine is offered.
    let mut workspace = workspace();
    for session in workspace.sessions.iter_mut().filter(|s| s.id == rate()) {
        session.pull_request_review = true;
    }
    for channel in &mut workspace.channels {
        channel.machines.retain(|machine| machine != &MachineId::new("gpu-box"));
    }
    let state = PaletteState {
        workspace: Arc::new(workspace),
        context: Some(rate()),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette)
        .non_exhaustive()
        .with_timeout(Duration::from_millis(100));
    store.send(A::Open { in_session: true }, |s| s.open = true);
    typed(&mut store, "move");
    store.send(A::Run, |s| s.open = true);
    let offered: BTreeSet<MachineId> = store
        .state()
        .entries
        .iter()
        .filter_map(|entry| match &entry.command {
            Command::MoveTo { machine, .. } => Some(machine.clone()),
            _ => None,
        })
        .collect();
    assert!(offered.contains(&MachineId::new("gpu-box")), "{offered:?}");
}

#[test]
fn move_with_no_other_machine_online_says_none_can_take_it() {
    let mut workspace = workspace();
    for machine in &mut workspace.machines {
        if machine.id != MachineId::new("devbox-1") {
            machine.online = false;
        }
    }
    let state = PaletteState {
        workspace: Arc::new(workspace),
        context: Some(rate()),
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette)
        .non_exhaustive()
        .with_timeout(Duration::from_millis(100));
    store.send(A::Open { in_session: true }, |s| s.open = true);
    typed(&mut store, "move");
    store.send(A::Run, |s| s.open = true);
    assert!(store.state().asking.is_some(), "Move asks for a machine");
    assert!(store.state().entries.is_empty());
    assert!(store.state().no_machine_to_move_to());
}

#[test]
fn a_query_that_hides_every_machine_is_not_a_machine_missing() {
    let mut store = opened();
    typed(&mut store, "move");
    store.send(A::Run, |s| s.open = true);
    assert!(!store.state().no_machine_to_move_to());
    typed(&mut store, "zzzz");
    assert!(store.state().entries.is_empty());
    assert!(!store.state().no_machine_to_move_to());
}

#[test]
fn typing_while_asking_narrows_the_machines() {
    let mut store = opened();
    typed(&mut store, "move");
    store.send(A::Run, |s| s.open = true);
    typed(&mut store, "gpu");
    let commands: Vec<Command> = store
        .state()
        .entries
        .iter()
        .map(|e| e.command.clone())
        .collect();
    assert_eq!(
        commands,
        vec![Command::MoveTo {
            session: rate(),
            machine: MachineId::new("gpu-box"),
        }]
    );
}

#[test]
fn picking_gpu_box_hands_the_move_to_the_app() {
    let mut store = opened();
    typed(&mut store, "move");
    store.send(A::Run, |s| s.open = true);
    let gpu_box = Command::MoveTo {
        session: rate(),
        machine: MachineId::new("gpu-box"),
    };
    let at = index_of(&store, &gpu_box);
    store.send(A::Pick(at), |s| {
        s.open = false;
        s.asking = None;
    });
    store.receive(A::Delegate(Delegate::Closed), |_| {});
    store.receive(A::Delegate(Delegate::Run(gpu_box)), |_| {});
}

#[test]
fn escape_while_asking_steps_back_without_running_anything() {
    let mut store = opened();
    typed(&mut store, "move");
    store.send(A::Run, |s| s.open = true);
    store.send(A::Escape, |s| {
        s.open = true;
        s.asking = None;
        s.query = "move".to_owned();
    });
    delegates_nothing(&mut store);
    assert_eq!(
        store.state().entries.first().map(|e| e.command.clone()),
        Some(Command::MoveSession),
        "the list is back as it was before Move asked"
    );
    // A second Escape, with nothing asked, closes the palette.
    store.send(A::Escape, |s| s.open = false);
    store.receive(A::Delegate(Delegate::Closed), |_| {});
}

#[test]
fn a_pasted_pull_request_url_offers_review_this_pr_first() {
    // PR-1: a pasted pull request offers Review this PR at once.
    let workspace = workspace();
    let Some(reviewed) = workspace.pulls.iter().find(|p| p.session.is_some()) else {
        panic!("the design workspace has a pull request under review");
    };
    let Some((owner, repo)) = reviewed.repo.split_once('/') else {
        panic!("{} is owner/repo", reviewed.repo);
    };
    let pull = PullRef {
        owner: owner.to_owned(),
        repo: repo.to_owned(),
        number: reviewed.number,
    };
    for pasted in [
        format!("https://github.com/{}/pull/{}", reviewed.repo, reviewed.number),
        format!("{}#{}", reviewed.repo, reviewed.number),
    ] {
        let mut store = opened();
        typed(&mut store, &pasted);
        let Some(first) = store.state().entries.first() else {
            panic!("pasting {pasted} offers something");
        };
        assert_eq!(first.command, Command::ReviewPull(pull.clone()), "{pasted}");
        store.send(A::Run, |s| s.open = false);
        store.receive(A::Delegate(Delegate::Closed), |_| {});
        store.receive(A::Delegate(Delegate::Run(Command::ReviewPull(pull.clone()))), |_| {});
    }
}

#[test]
fn other_text_is_not_taken_for_a_pull_request() {
    let mut store = opened();
    typed(&mut store, "ledger");
    assert!(
        store
            .state()
            .entries
            .iter()
            .all(|e| !matches!(e.command, Command::ReviewPull(_)))
    );
}

#[test]
fn typing_long_opens_settings_at_long_lines() {
    let mut store = opened();
    typed(&mut store, "long");
    let long_lines = Command::SettingsRow(Row::LongLines);
    let at = index_of(&store, &long_lines);
    assert_eq!(store.state().entries[at].title, "Settings › Long lines");
    store.send(A::Pick(at), |s| s.open = false);
    store.receive(A::Delegate(Delegate::Closed), |_| {});
    store.receive(A::Delegate(Delegate::Run(long_lines)), |_| {});
}
