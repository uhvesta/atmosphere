//! Channel Desk agent tab strip + multi-agent cap tests (DESKTABS, MULTIAGENT).
use std::sync::Arc;

use humata2_clients::model::{Agent, ChannelId, MachineId, SessionId};
use humata2_clients::{FakeDaemon, Workspace, fixture};
use humata2_desk::{
    Desk, DeskAction as A, DeskState, KindDisabledReason, MAX_TABS_PER_KIND, compute_tab_strip,
    flat_tab_cap,
};
use tca::{TestClock, TestStore};

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn machine_id(workspace: &Workspace) -> MachineId {
    workspace
        .machines
        .iter()
        .find(|m| m.online)
        .expect("online machine")
        .id
        .clone()
}

fn store(workspace: Workspace) -> (TestStore<Desk>, FakeDaemon) {
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    let store = TestStore::new(state, Desk::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn show(store: &mut TestStore<Desk>, fake: &FakeDaemon) {
    store.send(A::Show(Arc::new(fake.workspace())), |_| {});
}

fn receive_started(store: &mut TestStore<Desk>) -> SessionId {
    let mut created = None;
    store.receive_matching(
        "ChannelTabStarted(Ok(_))",
        |a| matches!(a, A::ChannelTabStarted { result: Ok(_), .. }),
        |state| {
            created = state.active_channel_tab.values().next().cloned();
        },
    );
    created.expect("active tab set")
}

#[test]
fn compute_tab_strip_enforces_flat_cap_and_per_kind_limit_including_shell() {
    let enabled = vec![Agent::Claude, Agent::Codex, Agent::Copilot, Agent::OpenCode];
    assert_eq!(flat_tab_cap(4), 6);
    assert_eq!(MAX_TABS_PER_KIND, 2);

    // 2 Claude + 2 Shell = 4 open out of 6 total cap.
    let open = vec![Agent::Claude, Agent::Claude, Agent::Shell, Agent::Shell];
    let summary = compute_tab_strip(&open, &enabled);
    assert_eq!(summary.total_open, 4);
    assert_eq!(summary.total_cap, 6);

    let find = |agent: Agent| {
        summary
            .options
            .iter()
            .find(|opt| opt.agent == agent)
            .expect("agent present")
    };

    assert_eq!(find(Agent::Claude).open_count, 2);
    assert_eq!(
        find(Agent::Claude).disabled_reason,
        Some(KindDisabledReason::AtKindLimit)
    );
    assert_eq!(find(Agent::Shell).open_count, 2);
    assert!(find(Agent::Shell).enabled);
    assert_eq!(
        find(Agent::Shell).disabled_reason,
        Some(KindDisabledReason::AtKindLimit)
    );
    assert_eq!(find(Agent::Codex).disabled_reason, None);
    assert!(!find(Agent::Agy).enabled);
    assert_eq!(
        find(Agent::Agy).disabled_reason,
        Some(KindDisabledReason::OffInSettings)
    );

    // Fill to 6 total open -> remaining enabled kinds hit AtTotalLimit.
    let full = vec![
        Agent::Claude,
        Agent::Claude,
        Agent::Shell,
        Agent::Shell,
        Agent::Codex,
        Agent::Copilot,
    ];
    let full_summary = compute_tab_strip(&full, &enabled);
    assert_eq!(full_summary.total_open, 6);
    assert_eq!(
        full_summary
            .options
            .iter()
            .find(|o| o.agent == Agent::OpenCode)
            .unwrap()
            .disabled_reason,
        Some(KindDisabledReason::AtTotalLimit { limit: 6 })
    );
}

#[test]
fn open_channel_tab_starts_immediately_without_modal_and_enforces_caps() {
    let ws = fixture::design();
    let machine = machine_id(&ws);
    let (mut store, fake) = store(ws);
    let enabled = vec![Agent::Claude]; // flat cap = 1 + 2 = 3

    // Open 1st Claude tab -> no channel_new modal, selects new tab.
    store.send(
        A::OpenChannelTab {
            channel: payments(),
            agent: Agent::Claude,
            machine: machine.clone(),
            enabled_agents: enabled.clone(),
        },
        |state| {
            assert!(state.channel_new.is_none());
            assert!(state.tab_picker_open.is_none());
        },
    );
    let _first = receive_started(&mut store);
    show(&mut store, &fake);
    assert_eq!(store.state().channel_tabs(&payments()).len(), 1);

    // Open 2nd Claude tab -> allowed.
    store.send(
        A::OpenChannelTab {
            channel: payments(),
            agent: Agent::Claude,
            machine: machine.clone(),
            enabled_agents: enabled.clone(),
        },
        |_| {},
    );
    store.receive_matching(
        "ChannelTabStarted(Ok(_))",
        |a| matches!(a, A::ChannelTabStarted { result: Ok(_), .. }),
        |_| {},
    );
    show(&mut store, &fake);
    assert_eq!(store.state().channel_tabs(&payments()).len(), 2);

    // 3rd Claude tab -> rejected with clear error, no client call.
    store.send(
        A::OpenChannelTab {
            channel: payments(),
            agent: Agent::Claude,
            machine: machine.clone(),
            enabled_agents: enabled.clone(),
        },
        |state| {
            assert!(
                state
                    .error
                    .as_deref()
                    .is_some_and(|msg| msg.contains("limit of 2"))
            );
        },
    );
    assert_eq!(store.state().channel_tabs(&payments()).len(), 2);

    // 3rd total tab as Shell -> allowed (reaches flat cap 3).
    store.send(
        A::OpenChannelTab {
            channel: payments(),
            agent: Agent::Shell,
            machine: machine.clone(),
            enabled_agents: enabled.clone(),
        },
        |_| {},
    );
    store.receive_matching(
        "ChannelTabStarted(Ok(_))",
        |a| matches!(a, A::ChannelTabStarted { result: Ok(_), .. }),
        |_| {},
    );
    show(&mut store, &fake);
    assert_eq!(store.state().channel_tabs(&payments()).len(), 3);

    // 4th total tab as Shell -> rejected by flat total cap 3.
    store.send(
        A::OpenChannelTab {
            channel: payments(),
            agent: Agent::Shell,
            machine,
            enabled_agents: enabled,
        },
        |state| {
            assert!(
                state
                    .error
                    .as_deref()
                    .is_some_and(|msg| msg.contains("limit of 3"))
            );
        },
    );
    assert_eq!(store.state().channel_tabs(&payments()).len(), 3);
}

#[test]
fn selecting_tabs_remembers_per_channel_independently() {
    let ws = fixture::design();
    let machine = machine_id(&ws);
    let ch1 = ws.channels[0].id.clone();
    let ch2 = ws.channels[1].id.clone();
    let (mut store, fake) = store(ws);
    let enabled = vec![Agent::Claude, Agent::Codex];

    for (channel, agent) in [
        (ch1.clone(), Agent::Claude),
        (ch1.clone(), Agent::Codex),
        (ch2.clone(), Agent::Claude),
        (ch2.clone(), Agent::Shell),
    ] {
        store.send(
            A::OpenChannelTab {
                channel,
                agent,
                machine: machine.clone(),
                enabled_agents: enabled.clone(),
            },
            |_| {},
        );
        store.receive_matching(
            "ChannelTabStarted(Ok(_))",
            |a| matches!(a, A::ChannelTabStarted { result: Ok(_), .. }),
            |_| {},
        );
    }
    show(&mut store, &fake);

    let pay_tabs = store.state().channel_tabs(&ch1);
    let ops_tabs = store.state().channel_tabs(&ch2);
    assert_eq!(pay_tabs.len(), 2);
    assert_eq!(ops_tabs.len(), 2);

    let pay_first = pay_tabs[0].session.id.clone();
    let ops_second = ops_tabs[1].session.id.clone();

    store.send(
        A::SelectChannelTab {
            channel: ch1.clone(),
            session: pay_first.clone(),
        },
        |_| {},
    );
    store.send(
        A::SelectChannelTab {
            channel: ch2.clone(),
            session: ops_second.clone(),
        },
        |_| {},
    );

    assert_eq!(
        store
            .state()
            .active_channel_tab_row(&ch1)
            .map(|r| r.session.id.clone()),
        Some(pay_first)
    );
    assert_eq!(
        store
            .state()
            .active_channel_tab_row(&ch2)
            .map(|r| r.session.id.clone()),
        Some(ops_second)
    );
}
