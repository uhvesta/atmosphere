//! D784: closing a terminal tab asks Keep running, Stop or End for good.
//! Keep running hides the tab; Stop and End for good of the agent go
//! through StopSession and Archive, of a shell through the machine's
//! running-holder calls; an offline machine is never asked; the
//! "always keep running" box skips the dialog and says where it still runs.
use humata2_clients::{
    Clients, Fakes, HolderTab, Operation, RunningHolder, fixture,
    model::{Agent, MachineId, SessionId, Status},
};
use humata2_session::{
    CloseAction as C, CloseChoice, CloseKey, CloseKind, CloseThen, Delegate, SessionAction as A, SessionState,
    SessionView, Sessions, Tab, TabGroup,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn ledger() -> SessionId {
    SessionId::new("ledger")
}

/// The ledger session open with its agent's tab and one shell tab, a
/// Claude Code session titled "Ledger" on This Mac.
fn store(online: bool) -> (TestStore<Sessions>, Fakes) {
    store_with(online, None)
}

/// As `store`, with tab 1 running `extra` (an agent started in a second
/// tab) when given.
fn store_with(online: bool, extra: Option<Agent>) -> (TestStore<Sessions>, Fakes) {
    let mut workspace = fixture::design();
    for session in &mut workspace.sessions {
        session.unread = 0;
        session.recovery = None;
        if session.id == ledger() {
            session.agent = Agent::Claude;
            session.title = "Ledger".into();
            session.machine = MachineId::new("this-mac");
            session.shells = vec!["zsh".into()];
            session.terminal_agents.clear();
            if let Some(agent) = extra {
                session.terminal_agents.insert(1, agent);
            }
        }
    }
    for machine in &mut workspace.machines {
        machine.online = online;
    }
    let (clients, fakes) = Clients::fake(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = SessionState {
        workspace: Arc::new(workspace),
        open: Some(ledger()),
        ..Default::default()
    };
    state.views.insert(
        ledger(),
        SessionView {
            groups: vec![TabGroup {
                tabs: vec![Tab::Terminal(0), Tab::Terminal(1)],
                selected: 0,
            }],
            ..Default::default()
        },
    );
    (TestStore::new(state, Sessions::new(clients)).non_exhaustive(), fakes)
}

fn tabs(store: &TestStore<Sessions>) -> Vec<Tab> {
    store.state().view().map(|view| view.groups[0].tabs.clone()).unwrap_or_default()
}

fn notice(action: &A) -> Option<(String, CloseThen)> {
    match action {
        A::Delegate(Delegate::CloseNotice(notice)) => Some((notice.text.clone(), notice.then.clone())),
        _ => None,
    }
}

#[test]
fn closing_the_agents_tab_asks_and_names_the_session_agent_and_machine() {
    let (mut store, _) = store(true);
    store.send(A::Close(C::Ask { group: 0, index: 0 }), |_| {});
    let asking = store.state().closing.clone();
    let asked = asking.as_ref().map(|confirm| {
        (
            confirm.kind,
            confirm.title.clone(),
            confirm.agent.clone(),
            confirm.machine.clone(),
            confirm.choices(),
            confirm.focused(),
        )
    });
    assert_eq!(
        asked,
        Some((
            CloseKind::Agent,
            Some("Ledger".into()),
            "Claude Code".into(),
            "This Mac".into(),
            vec![CloseChoice::KeepRunning, CloseChoice::Stop, CloseChoice::EndForGood],
            CloseChoice::KeepRunning,
        ))
    );
    assert_eq!(tabs(&store).len(), 2, "asking closes nothing");
}

#[test]
fn a_shell_tab_asks_as_an_untitled_shell() {
    let (mut store, _) = store(true);
    store.send(A::Close(C::Ask { group: 0, index: 1 }), |_| {});
    let asked = store.state().closing.as_ref().map(|confirm| (confirm.kind, confirm.title.clone(), confirm.agent.clone()));
    assert_eq!(asked, Some((CloseKind::Shell, None, "Shell".into())));
}

#[test]
fn escape_cancels_and_the_tab_stays() {
    let (mut store, fakes) = store(true);
    let before = fakes.daemon.calls().len();
    store.send(A::Close(C::Ask { group: 0, index: 1 }), |_| {});
    store.send(A::Close(C::Key(CloseKey::Escape)), |_| {});
    assert!(store.state().closing.is_none());
    assert_eq!(tabs(&store), vec![Tab::Terminal(0), Tab::Terminal(1)]);
    assert_eq!(fakes.daemon.calls().len(), before, "cancel asks the daemon nothing");
}

#[test]
fn return_keeps_it_running_hides_the_tab_and_calls_nothing() {
    let (mut store, fakes) = store(true);
    let before = fakes.daemon.calls().len();
    store.send(A::Close(C::Ask { group: 0, index: 1 }), |_| {});
    store.send(A::Close(C::Key(CloseKey::Return)), |_| {});
    assert!(store.state().closing.is_none());
    assert_eq!(tabs(&store), vec![Tab::Terminal(0)]);
    assert_eq!(fakes.daemon.calls().len(), before);
    assert!(!store.state().always_keep_running);
}

#[test]
fn the_ticked_box_skips_the_dialog_next_time_and_the_toast_opens_it_again() {
    let (mut store, _) = store(true);
    store.send(A::Close(C::Ask { group: 0, index: 1 }), |_| {});
    store.send(A::Close(C::AlwaysKeepRunning(true)), |_| {});
    store.send(A::Close(C::Choose(CloseChoice::KeepRunning)), |_| {});
    assert!(store.state().always_keep_running);

    store.send(A::Close(C::Ask { group: 0, index: 0 }), |_| {});
    assert!(store.state().closing.is_none(), "no dialog once the box was ticked");
    store.receive_matching(
        "the keep-running toast",
        |a| matches!(notice(a), Some((_, CloseThen::Open { tab: 0, .. }))),
        |_| {},
    );
    assert_eq!(tabs(&store), vec![Tab::Terminal(0)], "a lone tab falls back to the agent's");

    store.send(A::Close(C::Then(CloseThen::Open { session: ledger(), tab: 0 })), |_| {});
    store.receive_matching(
        "the session opens",
        |a| matches!(a, A::Delegate(Delegate::OpenSession(id)) if *id == ledger()),
        |_| {},
    );
}

#[test]
fn stopping_the_agent_stops_the_session_and_offers_resume() {
    let (mut store, fakes) = store(true);
    store.send(A::Close(C::Ask { group: 0, index: 0 }), |_| {});
    store.send(A::Close(C::Choose(CloseChoice::Stop)), |_| {});
    store.receive_matching("stopped", |a| matches!(a, A::Close(C::Stopped(Ok(())))), |_| {});
    store.receive_matching(
        "the stop toast offers Resume of the agent",
        |a| matches!(notice(a), Some((_, CloseThen::ResumeAgent(id))) if id == ledger()),
        |_| {},
    );
    let stopped = fakes.daemon.workspace().session(&ledger()).map(|found| found.status == Status::AgentStopped);
    assert_eq!(stopped, Some(true), "the agent was stopped");
    assert!(store.state().closing.is_none());
    assert_eq!(tabs(&store), vec![Tab::Terminal(1)]);
}

#[test]
fn stopping_a_shell_stops_its_tab_and_resume_resumes_that_tab() {
    let (mut store, fakes) = store(true);
    fakes.daemon.set_running_holders(Vec::<RunningHolder>::new());
    store.send(A::Close(C::Ask { group: 0, index: 1 }), |_| {});
    store.send(A::Close(C::Choose(CloseChoice::Stop)), |_| {});
    store.receive_matching("stopped", |a| matches!(a, A::Close(C::Stopped(Ok(())))), |_| {});
    store.receive_matching(
        "the stop toast offers Resume of the shell",
        |a| matches!(notice(a), Some((_, CloseThen::ResumeShell { tab: 1, .. }))),
        |_| {},
    );
    let stopped = format!("stop running holder {:?} on this-mac", HolderTab::Session { session: ledger(), index: 1 });
    assert!(fakes.daemon.calls().contains(&stopped), "{:?}", fakes.daemon.calls());
    assert_eq!(tabs(&store), vec![Tab::Terminal(0)]);
}

#[test]
fn ending_a_shell_for_good_ends_it_and_closes_the_tab() {
    let (mut store, fakes) = store(true);
    store.send(A::Close(C::Ask { group: 0, index: 1 }), |_| {});
    store.send(A::Close(C::Choose(CloseChoice::EndForGood)), |_| {});
    store.receive_matching("ended", |a| matches!(a, A::Close(C::Ended(Ok(())))), |_| {});
    let ended = format!("end running holder {:?} on this-mac", HolderTab::Session { session: ledger(), index: 1 });
    assert!(fakes.daemon.calls().contains(&ended), "{:?}", fakes.daemon.calls());
    assert_eq!(tabs(&store), vec![Tab::Terminal(0)]);
}

#[test]
fn ending_the_agent_for_good_archives_it_and_undo_restores_it_stopped() {
    let (mut store, fakes) = store(true);
    store.send(A::Close(C::Ask { group: 0, index: 0 }), |_| {});
    store.send(A::Close(C::Choose(CloseChoice::EndForGood)), |_| {});
    store.receive_matching(
        "End for good's toast",
        |a| matches!(a, A::Delegate(Delegate::EndedForGood(id, _)) if *id == ledger()),
        |_| {},
    );
    assert!(fakes.daemon.workspace().session(&ledger()).is_some_and(|found| found.archived));
    store.send(A::Close(C::Then(CloseThen::RestoreStopped(ledger()))), |_| {});
    store.receive_matching("restored", |a| matches!(a, A::Close(C::ThenDone(Ok(())))), |_| {});
    let restored = fakes.daemon.workspace().session(&ledger()).map(|found| (found.archived, found.status));
    assert_eq!(restored, Some((false, Status::Stopped)));
}

#[test]
fn an_offline_machine_refuses_stop_and_end_without_asking_the_daemon() {
    let (mut store, fakes) = store(false);
    let before = fakes.daemon.calls().len();
    store.send(A::Close(C::Ask { group: 0, index: 0 }), |_| {});
    for choice in [CloseChoice::Stop, CloseChoice::EndForGood] {
        store.send(A::Close(C::Choose(choice)), |_| {});
        assert!(store.state().closing.as_ref().is_some_and(|confirm| confirm.error.is_some()));
    }
    assert_eq!(fakes.daemon.calls().len(), before, "nothing is sent or queued");
    assert_eq!(tabs(&store).len(), 2);
}

#[test]
fn a_failed_stop_keeps_the_dialog_open_with_the_reason() {
    let (mut store, fakes) = store(true);
    fakes.daemon.fail_next(Operation::StopSession, "the agent would not stop");
    store.send(A::Close(C::Ask { group: 0, index: 0 }), |_| {});
    store.send(A::Close(C::Choose(CloseChoice::Stop)), |_| {});
    store.receive_matching("stop failed", |a| matches!(a, A::Close(C::Stopped(Err(_)))), |_| {});
    let open = store.state().closing.as_ref().map(|confirm| (confirm.sending, confirm.error.is_some()));
    assert_eq!(open, Some((None, true)));
    assert_eq!(tabs(&store).len(), 2);
}

/// B1 (Fable, 2026-11-18): until an extra agent tab can be resumed in its
/// own conversation, its close offers Keep running and End for good, never
/// Stop.
#[test]
fn an_extra_agent_tab_offers_end_for_good_but_no_stop() {
    let (mut store, fakes) = store_with(true, Some(Agent::Copilot));
    store.send(A::Close(C::Ask { group: 0, index: 1 }), |_| {});
    let confirm = store.state().closing.clone().expect("the close asks");
    assert_eq!(confirm.kind, CloseKind::ExtraAgent);
    assert!(confirm.choices().contains(&CloseChoice::KeepRunning));
    assert!(confirm.choices().contains(&CloseChoice::EndForGood));
    assert!(!confirm.choices().contains(&CloseChoice::Stop));
    store.send(A::Close(C::Choose(CloseChoice::Stop)), |_| {});
    assert!(store.state().closing.is_some(), "Stop isn't offered, so it does nothing");
    store.send(A::Close(C::Choose(CloseChoice::EndForGood)), |_| {});
    store.receive_matching("ended", |a| matches!(a, A::Close(C::Ended(Ok(())))), |_| {});
    let ended = format!("end running holder {:?} on this-mac", HolderTab::Session { session: ledger(), index: 1 });
    assert!(fakes.daemon.calls().contains(&ended), "{:?}", fakes.daemon.calls());
    assert!(!fakes.daemon.calls().iter().any(|call| call.starts_with("archive")), "the session isn't archived");
    assert_eq!(tabs(&store), vec![Tab::Terminal(0)]);
}

#[test]
fn a_file_tab_closes_without_asking() {
    let (mut store, _) = store(true);
    store.send(A::OpenView("sample".into()), |_| {});
    store.skip_received();
    let at = tabs(&store).iter().position(|tab| matches!(tab, Tab::View(_)));
    assert!(at.is_some(), "the view opened in a tab");
    store.send(A::Close(C::Ask { group: 0, index: at.unwrap_or(0) }), |_| {});
    assert!(store.state().closing.is_none());
    assert!(!tabs(&store).iter().any(|tab| matches!(tab, Tab::View(_))));
}
