//! After a restart: the report and toast (PER-29) and the failed-resume
//! actions (PER-15), on the fake daemon.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, RestartAction, restart_groups};
use humata2_clients::model::{
    MachineId, NotBroughtBack, ResumeFailure, Restart, SessionId, Status,
};
use humata2_clients::{Clients, Event, Fakes, Workspace, fixture};
use tca::{TestClock, TestStore};

fn setup(workspace: Workspace) -> (TestStore<App>, Fakes) {
    let (clients, fakes) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.workspace = Arc::new(workspace);
    (TestStore::new(state, App::new(clients)).non_exhaustive(), fakes)
}

fn restart(machine: &str, resumed: &[&str], lost: usize) -> Restart {
    Restart {
        machine: MachineId::new(machine),
        at_unix_ms: 1,
        report: None,
        resumed: resumed.iter().map(|id| SessionId::new(*id)).collect(),
        not_brought_back: (0..lost)
            .map(|_| NotBroughtBack {
                session: None,
                tab: Some("zsh".into()),
                problem: "gone".into(),
            })
            .collect(),
    }
}

fn first_session(workspace: &Workspace) -> SessionId {
    workspace.sessions[0].id.clone()
}


/// The daemon brought its sessions back before the app connected, so no
/// restart event comes: the first load tells it, and a later load of the
/// same restart doesn't tell it again (D390).
#[test]
fn a_restart_the_first_load_brings_is_told_once() {
    let mut workspace = fixture::design();
    workspace.restarts = vec![restart("devbox-1", &["a"], 1)];
    let (clients, _) = humata2_clients::fake_clients(workspace.clone(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    let quiet = {
        let mut quiet = workspace.clone();
        quiet.restarts.clear();
        let (clients, _) = humata2_clients::fake_clients(quiet.clone(), Arc::new(TestClock::new()));
        let mut control = TestStore::new(AppState::launch(&clients), App::new(clients)).non_exhaustive();
        control.send(A::Loaded(Ok(quiet)), |_| {});
        let told = control.state().toasts.len();
        control.skip_received();
        told
    };
    store.send(A::Loaded(Ok(workspace.clone())), |_| {});
    let told = store.state().toasts.len();
    assert_eq!(told, quiet + 1, "the loaded restart is told");
    store.send(A::Loaded(Ok(workspace)), |_| {});
    assert_eq!(store.state().toasts.len(), told, "a reload doesn't tell it again");
    store.skip_received();
}

/// A restart heard as an event first isn't told again when the load that
/// follows brings it.
#[test]
fn a_restart_heard_before_the_load_is_not_told_twice() {
    let mut workspace = fixture::design();
    workspace.restarts.clear();
    let (mut store, _) = setup(workspace.clone());
    let heard = restart("devbox-1", &["a"], 0);
    store.send(A::Event(Event::Restart(heard.clone())), |_| {});
    let told = store.state().toasts.len();
    workspace.restarts = vec![heard];
    store.send(A::Loaded(Ok(workspace)), |_| {});
    assert_eq!(store.state().toasts.len(), told);
    store.skip_received();
}

#[test]
fn hearing_a_restart_keeps_it_and_tells_the_person() {
    let (mut store, _) = setup(fixture::design());
    store.send(A::Event(Event::Restart(restart("devbox-1", &["a", "b", "c"], 1))), |_| {});
    let state = store.state();
    assert_eq!(state.workspace.restarts.len(), 1);
    let toast = state.toasts.last().expect("a toast");
    assert!(toast.text.contains("3 sessions"));
    assert!(toast.text.contains("1 couldn't"));
    store.skip_received();
}

#[test]
fn a_newer_restart_of_the_same_host_replaces_the_older_one() {
    let (mut store, _) = setup(fixture::design());
    store.send(A::Event(Event::Restart(restart("devbox-1", &["a"], 0))), |_| {});
    store.send(A::Event(Event::Restart(restart("gpu-box", &[], 0))), |_| {});
    store.send(A::Event(Event::Restart(restart("devbox-1", &["b"], 0))), |_| {});
    let restarts = &store.state().workspace.restarts;
    assert_eq!(restarts.len(), 2);
    assert_eq!(restarts[0].machine, MachineId::new("devbox-1"));
    assert_eq!(restarts[0].resumed, vec![SessionId::new("b")]);
    store.skip_received();
}

#[test]
fn try_again_resumes_the_session() {
    let mut workspace = fixture::design();
    let id = first_session(&workspace);
    workspace.sessions[0].status = Status::Exited { code: 1 };
    workspace.sessions[0].resume_failed = Some(ResumeFailure::AgentExited(1));
    let (mut store, fakes) = setup(workspace);
    store.send(A::Restart(RestartAction::TryAgain(id.clone())), |_| {});
    store.receive_matching(
        "the resume finished",
        |a| matches!(a, A::Restart(RestartAction::Finished(Ok(())))),
        |_| {},
    );
    let after = fakes.daemon.workspace();
    assert_eq!(after.session(&id).map(|s| s.status), Some(Status::Idle));
    store.skip_received();
}

#[test]
fn home_groups_split_failed_from_idle_after_the_restart() {
    let mut workspace = fixture::design();
    for session in &mut workspace.sessions {
        session.resume_failed = None;
        session.status = Status::Working;
    }
    workspace.sessions[0].resume_failed = Some(ResumeFailure::ConversationMissing);
    workspace.sessions[0].status = Status::Exited { code: 1 };
    workspace.sessions[1].status = Status::Idle;
    workspace.sessions[2].status = Status::Idle;
    let resumed = workspace.sessions[1].id.as_str().to_owned();
    workspace.restarts = vec![restart("devbox-1", &[&resumed], 1)];
    let groups = restart_groups(&workspace);
    assert_eq!(groups.couldnt_resume.len(), 1);
    assert_eq!(groups.couldnt_resume[0].id, workspace.sessions[0].id);
    assert_eq!(groups.idle.len(), 1);
    assert_eq!(groups.idle[0].id, workspace.sessions[1].id);
}
