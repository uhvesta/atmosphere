//! D784 on the machine page: its running holders load, split into its
//! sessions and a "Not in a channel" group drawn only when there are any;
//! each row's ⋯ menu offers what its kind can do; Stop asks nothing and
//! End for good… confirms once; an offline machine offers no row action.
use std::sync::Arc;

use humata2_clients::machines::FakeMachines;
use humata2_clients::model::{Agent, MachineId, SessionId, Status};
use humata2_clients::{Daemon, FakeDaemon, RunningHolder, fixture};
use humata2_hosts::{
    MachinePage, MachinePageAction as P, RowAction, Running, RunningAction as A, RunningDelegate, RunningState,
};
use tca::{TestClock, TestStore};

fn mac() -> MachineId {
    MachineId::new("this-mac")
}

fn holder(tab: Option<&str>, session: Option<&str>, title: &str, agent: Agent, stopped: bool) -> RunningHolder {
    RunningHolder {
        tab: tab.map(str::to_owned),
        session: session.map(SessionId::new),
        title: title.to_owned(),
        channel: None,
        desk: false,
        agent,
        pid: (!stopped).then_some(4000),
        folder: "/work/sample".to_owned(),
        started_unix_ms: (!stopped).then_some(100),
        attached: false,
        machine: mac(),
        stopped,
    }
}

/// An agent session of this Mac in the standard workspace.
fn agent_session(daemon: &FakeDaemon) -> SessionId {
    match daemon
        .workspace()
        .sessions
        .iter()
        .find(|session| session.machine == mac() && session.agent != Agent::Shell && !session.archived)
    {
        Some(session) => session.id.clone(),
        None => panic!("the standard workspace has an agent session on this Mac"),
    }
}

fn running_row(session: &SessionId) -> RunningHolder {
    holder(Some("agent-tab"), Some(session.as_str()), "Sample", Agent::Claude, false)
}

fn orphan() -> RunningHolder {
    holder(Some("orphan-tab"), None, "", Agent::Copilot, false)
}

fn seeded(with_orphan: bool) -> (TestStore<Running>, FakeDaemon, SessionId) {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    let session = agent_session(&daemon);
    let mut holders = vec![
        running_row(&session),
        holder(None, Some("sample-stopped"), "Stopped one", Agent::Codex, true),
    ];
    if with_orphan {
        holders.push(orphan());
    }
    daemon.set_running_holders(holders);
    let reducer = Running::new(Arc::new(daemon.clone()) as Arc<dyn Daemon>);
    (TestStore::new(RunningState::default(), reducer).non_exhaustive(), daemon, session)
}

fn loaded(store: &mut TestStore<Running>) {
    store.send(A::Load(mac()), |_| {});
    store.receive_matching("the list", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
}

#[test]
fn the_list_splits_sessions_from_terminals_no_channel_has() {
    let (mut store, _, _) = seeded(true);
    loaded(&mut store);
    let state = store.state();
    assert_eq!(state.sessions().len(), 2);
    assert_eq!(state.not_in_a_channel().map(|group| group.len()), Some(1));
}

#[test]
fn not_in_a_channel_is_absent_without_orphans() {
    let (mut store, _, _) = seeded(false);
    loaded(&mut store);
    assert!(store.state().not_in_a_channel().is_none());
}

#[test]
fn each_row_kind_offers_its_own_menu() {
    let (mut store, _, session) = seeded(true);
    loaded(&mut store);
    let state = store.state();
    let stopped = holder(None, Some("sample-stopped"), "Stopped one", Agent::Codex, true);
    assert_eq!(
        state.row_actions(&running_row(&session)),
        vec![RowAction::Open, RowAction::Stop, RowAction::EndForGood]
    );
    assert_eq!(state.row_actions(&stopped), vec![RowAction::Resume, RowAction::Archive]);
    assert_eq!(
        state.row_actions(&orphan()),
        vec![RowAction::Open, RowAction::Adopt, RowAction::EndForGood]
    );
}

#[test]
fn stop_from_the_menu_asks_nothing_and_stops_the_session() {
    let (mut store, daemon, session) = seeded(false);
    loaded(&mut store);
    store.send(A::Row(running_row(&session), RowAction::Stop), |_| {});
    store.receive_matching("stopped", |a| matches!(a, A::Done(Ok(()))), |_| {});
    store.receive_matching("listed again", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    assert!(store.state().confirm_end.is_none());
    let status = daemon.workspace().session(&session).map(|found| found.status);
    assert!(matches!(status, Some(Status::Exited { .. })), "{status:?}");
}

#[test]
fn ending_a_terminal_no_channel_has_confirms_once_then_it_is_gone() {
    let (mut store, daemon, _) = seeded(true);
    loaded(&mut store);
    store.send(A::Row(orphan(), RowAction::EndForGood), |_| {});
    assert_eq!(store.state().confirm_end, Some(orphan()), "it asks first");
    assert!(daemon.running_holders_now().contains(&orphan()), "nothing ended before the confirm");
    store.send(A::ConfirmEnd, |_| {});
    store.receive_matching("ended", |a| matches!(a, A::Done(Ok(()))), |_| {});
    store.receive_matching("listed again", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    assert!(store.state().not_in_a_channel().is_none());
}

#[test]
fn ending_a_session_for_good_hands_it_to_archive() {
    let (mut store, _, session) = seeded(false);
    loaded(&mut store);
    store.send(A::Row(running_row(&session), RowAction::EndForGood), |_| {});
    store.send(A::ConfirmEnd, |_| {});
    store.receive_matching(
        "archive it",
        |a| matches!(a, A::Delegate(RunningDelegate::ArchiveSession(id)) if *id == session),
        |_| {},
    );
}

#[test]
fn an_offline_machine_offers_no_row_action_and_keeps_what_it_listed() {
    let (mut store, daemon, session) = seeded(true);
    loaded(&mut store);
    daemon.take_offline(&mac());
    store.send(A::Load(mac()), |_| {});
    store.receive_matching("refused", |a| matches!(a, A::Loaded(_, Err(_))), |_| {});
    let state = store.state();
    assert!(state.offline);
    assert_eq!(state.sessions().len(), 2, "what was listed stays");
    assert!(state.row_actions(&running_row(&session)).is_empty());
    assert!(state.row_actions(&orphan()).is_empty());
}

#[test]
fn the_machine_page_lists_its_holders_when_shown() {
    let daemon = FakeDaemon::new(fixture::standard(), Arc::new(TestClock::new()));
    daemon.set_running_holders(vec![orphan()]);
    let machines = FakeMachines::new().through(daemon.clone());
    let page = MachinePage::new(Arc::new(machines)).with_daemon(Arc::new(daemon) as Arc<dyn Daemon>);
    let mut store = TestStore::new(humata2_hosts::MachinePageState::default(), page).non_exhaustive();
    store.send(P::Show(mac()), |_| {});
    store.receive_matching("the list", |a| matches!(a, P::Running(A::Loaded(_, Ok(_)))), |_| {});
    assert_eq!(store.state().running.not_in_a_channel().map(|group| group.len()), Some(1));
}

#[test]
fn adopting_a_terminal_no_channel_has_puts_it_in_a_session() {
    let (mut store, daemon, _) = seeded(true);
    loaded(&mut store);
    store.send(A::Row(orphan(), RowAction::Adopt), |_| {});
    store.send(A::ConfirmAdopt(humata2_hosts::AdoptTarget::Desk), |_| {});
    store.receive_matching("adopted", |a| matches!(a, A::Done(Ok(()))), |_| {});
    store.receive_matching("listed again", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    assert!(store.state().not_in_a_channel().is_none(), "it is no longer an orphan");
    assert!(daemon.running_holders_now().iter().all(|holder| !holder.is_orphan()));
}
