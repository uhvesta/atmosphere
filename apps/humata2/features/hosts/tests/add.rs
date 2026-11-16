//! The add-machine dialog against a scripted fake: what each outcome of an
//! add leaves in the state and what the dialog tells its parent.
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    AddEvent, AddFailure, AddOutcome, AddRequest, AddTarget, Call, Choices, FakeMachines, Fix,
    LoginEvent, LoginTarget, OperationId, Provider, ProviderParam, SshHost, StepKind, StepStatus,
    StepUpdate, UnattendedMethod,
};
use humata2_clients::model::MachineId;
use humata2_hosts::add::{Choice, Delegate};
use humata2_hosts::{AddMachine, AddMachineAction as A, AddMachineState};
use tca::{StreamEnd, TestStore};

fn store(state: AddMachineState) -> (TestStore<AddMachine>, FakeMachines) {
    let fake = FakeMachines::new();
    let store = TestStore::new(state, AddMachine::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn open_with_host(host: &str) -> AddMachineState {
    AddMachineState {
        open: true,
        choice: Some(Choice::Host(host.to_owned())),
        ..AddMachineState::default()
    }
}

fn op() -> OperationId {
    OperationId::new("op-1")
}

fn started() -> AddEvent {
    AddEvent::Started {
        operation: op(),
        steps: vec![StepKind::Connect, StepKind::Check, StepKind::Install, StepKind::Start, StepKind::FindAgents],
    }
}

fn step(kind: StepKind, status: StepStatus, detail: &str) -> AddEvent {
    AddEvent::Step(StepUpdate { step: kind, status, detail: detail.to_owned() })
}

fn ssh_request(host: &str, trusted: Option<&str>) -> AddRequest {
    AddRequest {
        target: AddTarget::Ssh { host: host.to_owned() },
        name: None,
        trusted_fingerprint: trusted.map(str::to_owned),
    }
}

fn failure() -> AddFailure {
    AddFailure {
        step: StepKind::Install,
        title: "Install humatad failed".to_owned(),
        what: "permission denied".to_owned(),
        fix: Some(Fix { command: "sudo chown -R $USER ~/.local/bin".to_owned(), where_to_run: "devbox-2".to_owned() }),
    }
}

fn added_events(id: &str) -> Vec<AddEvent> {
    vec![started(), AddEvent::Outcome(AddOutcome::Added(MachineId::new(id)))]
}

fn receive_added(store: &mut TestStore<AddMachine>, id: &str) {
    store.receive(A::Delegate(Delegate::Added(MachineId::new(id))), |_| {});
    store.receive_matching("a toast", |a| matches!(a, A::Delegate(Delegate::Toast(_))), |_| {});
}

#[test]
fn open_loads_the_choices_and_picks_the_first_host() {
    let (mut store, fake) = store(AddMachineState::default());
    fake.set_choices(Choices {
        hosts: vec![
            SshHost { name: "devbox-2".to_owned(), address: "avesta@10.0.4.22".to_owned() },
            SshHost { name: "corp-vm".to_owned(), address: "corp-vm".to_owned() },
        ],
        providers: vec![Provider {
            id: "hcloud".to_owned(),
            label: "Cloud VM".to_owned(),
            interactive: false,
            params: vec![ProviderParam {
                name: "size".to_owned(),
                label: "Size".to_owned(),
                default: "cx22".to_owned(),
                required: true,
            }],
        }],
    });
    store.send(A::Open, |_| {});
    assert!(store.state().open && store.state().loading);
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_)), |_| {});
    let state = store.state();
    assert!(!state.loading);
    assert_eq!(state.hosts.len(), 2);
    assert_eq!(state.choice, Some(Choice::Host("devbox-2".to_owned())));
    assert_eq!(state.display_name(), Some("devbox-2"));

    store.send(A::SelectProvider("hcloud".to_owned()), |_| {});
    store.send(A::SetParam("size".to_owned(), "cx32".to_owned()), |_| {});
    store.send(A::SetName("build-box".to_owned()), |_| {});
    let Some(Choice::Provider { id, params }) = &store.state().choice else {
        panic!("a provider should be chosen");
    };
    assert_eq!(id, "hcloud");
    assert_eq!(params.get("size").map(String::as_str), Some("cx32"));
    assert_eq!(store.state().display_name(), Some("build-box"));
}

#[test]
fn a_provider_add_sends_its_params_and_name() {
    let (mut store, fake) = store(AddMachineState::default());
    fake.set_choices(Choices {
        hosts: vec![],
        providers: vec![Provider {
            id: "hcloud".to_owned(),
            label: "Cloud VM".to_owned(),
            interactive: false,
            params: vec![ProviderParam {
                name: "size".to_owned(),
                label: "Size".to_owned(),
                default: "cx22".to_owned(),
                required: true,
            }],
        }],
    });
    fake.script_add(added_events("build-box"));
    store.send(A::Open, |_| {});
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_)), |_| {});
    store.send(A::SelectProvider("hcloud".to_owned()), |_| {});
    store.send(A::SetName("build-box".to_owned()), |_| {});
    store.send(A::Start, |_| {});
    receive_added(&mut store, "build-box");
    let calls = fake.calls();
    let Some(Call::Add(request)) = calls.iter().find(|c| matches!(c, Call::Add(_))) else {
        panic!("no add was made");
    };
    assert_eq!(request.name.as_deref(), Some("build-box"));
    assert!(matches!(&request.target, AddTarget::Provider { provider, params }
        if provider == "hcloud" && params.get("size").map(String::as_str) == Some("cx22")));
}

#[test]
fn a_successful_add_follows_the_steps_then_closes_and_delegates() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    fake.script_add(vec![
        started(),
        step(StepKind::Connect, StepStatus::Running, ""),
        step(StepKind::Connect, StepStatus::Done, "answered as avesta@10.0.4.22"),
        AddEvent::Outcome(AddOutcome::Added(MachineId::new("devbox-2"))),
    ]);
    store.send(A::Start, |_| {});
    assert!(store.state().running);
    store.receive_matching("connect done", |a| matches!(a, A::Progress(AddEvent::Step(u)) if u.status == StepStatus::Done), |_| {});
    let connect = &store.state().steps[0];
    assert_eq!(connect.status, StepStatus::Done);
    assert_eq!(connect.detail, "answered as avesta@10.0.4.22");
    assert_eq!(store.state().steps.len(), 5);
    receive_added(&mut store, "devbox-2");
    assert_eq!(*store.state(), AddMachineState::default());
    assert_eq!(fake.calls(), vec![Call::Add(ssh_request("devbox-2", None))]);
}

#[test]
fn a_second_start_while_one_runs_is_ignored() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    fake.script_add(added_events("devbox-2"));
    store.send(A::Start, |_| {});
    store.send(A::Start, |_| {});
    receive_added(&mut store, "devbox-2");
    let adds = fake.calls().into_iter().filter(|c| matches!(c, Call::Add(_))).count();
    assert_eq!(adds, 1);
}

#[test]
fn start_without_a_host_does_nothing() {
    let (mut store, fake) = store(AddMachineState { open: true, ..AddMachineState::default() });
    store.send(A::Start, |_| {});
    store.send(A::SelectHost("  ".to_owned()), |_| {});
    store.send(A::Start, |_| {});
    assert!(!store.state().running);
    assert!(fake.calls().is_empty());
}

#[test]
fn a_failure_stops_there_and_try_again_retries_the_same_operation() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    fake.script_add(vec![
        started(),
        step(StepKind::Connect, StepStatus::Done, "ok"),
        step(StepKind::Check, StepStatus::Done, "ok"),
        step(StepKind::Install, StepStatus::Running, ""),
        AddEvent::Outcome(AddOutcome::Failed(failure())),
    ]);
    fake.script_add(added_events("devbox-2"));
    store.send(A::Start, |_| {});
    store.receive(A::Progress(AddEvent::Outcome(AddOutcome::Failed(failure()))), |_| {});
    store.receive(A::ProgressEnded(StreamEnd::Finished), |_| {});
    let state = store.state();
    let statuses: Vec<_> = state.steps.iter().map(|s| s.status).collect();
    assert_eq!(
        statuses,
        vec![StepStatus::Done, StepStatus::Done, StepStatus::Failed, StepStatus::NotRun, StepStatus::NotRun]
    );
    assert_eq!(state.failure, Some(failure()));
    assert!(!state.running);

    store.send(A::TryAgain, |_| {});
    assert!(store.state().failure.is_none() && store.state().running);
    receive_added(&mut store, "devbox-2");
    assert!(fake.calls().contains(&Call::Retry(op())));
    let adds = fake.calls().into_iter().filter(|c| matches!(c, Call::Add(_))).count();
    assert_eq!(adds, 1);
}

#[test]
fn cancel_stops_a_running_add_and_closes() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    fake.script_add(vec![started()]);
    store.send(A::Start, |_| {});
    store.receive(A::Progress(started()), |_| {});
    store.send(A::Cancel, |_| {});
    store.skip_received();
    assert_eq!(*store.state(), AddMachineState::default());
    assert!(fake.calls().contains(&Call::Cancel(op())));
}

#[test]
fn cancel_on_an_idle_dialog_only_closes() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    store.send(A::Cancel, |_| {});
    assert!(!store.state().open);
    assert!(fake.calls().is_empty());
}

#[test]
fn a_host_key_request_waits_for_confirmation_and_then_trusts_it() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    fake.script_add(vec![
        started(),
        AddEvent::Outcome(AddOutcome::HostKey { fingerprint: "SHA256:abc".to_owned() }),
    ]);
    fake.script_add(added_events("devbox-2"));
    store.send(A::Start, |_| {});
    store.receive(A::ProgressEnded(StreamEnd::Finished), |_| {});
    assert_eq!(store.state().host_key.as_deref(), Some("SHA256:abc"));
    assert!(!store.state().running);

    store.send(A::ConfirmHostKey, |_| {});
    receive_added(&mut store, "devbox-2");
    assert_eq!(
        fake.calls(),
        vec![Call::Add(ssh_request("devbox-2", None)), Call::Add(ssh_request("devbox-2", Some("SHA256:abc")))]
    );
}

#[test]
fn declining_a_host_key_adds_nothing_and_keeps_the_dialog() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    fake.script_add(vec![
        started(),
        AddEvent::Outcome(AddOutcome::HostKey { fingerprint: "SHA256:abc".to_owned() }),
    ]);
    store.send(A::Start, |_| {});
    store.receive(A::ProgressEnded(StreamEnd::Finished), |_| {});
    store.send(A::DeclineHostKey, |_| {});
    assert!(store.state().open && store.state().host_key.is_none() && !store.state().running);
    store.send(A::ConfirmHostKey, |_| {});
    let adds = fake.calls().into_iter().filter(|c| matches!(c, Call::Add(_))).count();
    assert_eq!(adds, 1);
}

fn login_target() -> LoginTarget {
    LoginTarget::Operation(op())
}

fn ask_for_login(store: &mut TestStore<AddMachine>, fake: &FakeMachines) {
    fake.script_add(vec![
        started(),
        AddEvent::Outcome(AddOutcome::NeedsLogin { prompt: "Verification code:".to_owned() }),
    ]);
    store.send(A::Start, |_| {});
    store.receive(A::ProgressEnded(StreamEnd::Finished), |_| {});
}

#[test]
fn a_login_flows_from_the_offer_through_done_to_the_unattended_offer() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    ask_for_login(&mut store, &fake);
    assert_eq!(store.state().login_offer.as_deref(), Some("Verification code:"));

    let ready = LoginEvent::Ready { machine: MachineId::new("corp-vm"), can_make_unattended: true };
    fake.script_login(
        vec![LoginEvent::Output(b"Verification ".to_vec()), LoginEvent::Output(b"code: ".to_vec())],
        vec![ready.clone()],
    );
    store.send(A::OpenLogin, |_| {});
    store.receive(A::LoginProgress(LoginEvent::Output(b"code: ".to_vec())), |_| {});
    let login = store.state().login.as_ref().map(|l| l.bytes.clone());
    assert_eq!(login, Some(b"Verification code: ".to_vec()));
    assert!(store.state().login_offer.is_none());

    store.send(A::LoginInput(b"123456\r".to_vec()), |_| {});
    store.send(A::LoginResize { columns: 90, rows: 20 }, |_| {});
    store.send(A::LoginDone, |_| {});
    assert!(store.state().login.as_ref().is_some_and(|l| l.finishing));
    store.receive(A::LoginProgress(ready), |_| {});
    store.receive(A::LoginEnded(StreamEnd::Finished), |_| {});
    let offer = store.state().unattended.clone();
    assert_eq!(offer.map(|o| (o.machine, o.method)), Some((MachineId::new("corp-vm"), UnattendedMethod::AddKey)));
    assert!(store.state().login.is_none());

    store.send(A::ChooseMethod(UnattendedMethod::KeepToken), |_| {});
    store.send(A::MakeUnattended, |_| {});
    store.receive_matching("finished", |a| matches!(a, A::UnattendedFinished(Ok(()))), |_| {});
    receive_added(&mut store, "corp-vm");
    let calls = fake.calls();
    assert!(calls.contains(&Call::LoginInput(login_target(), b"123456\r".to_vec())));
    assert!(calls.contains(&Call::LoginResize(login_target(), 90, 20)));
    assert!(calls.contains(&Call::LoginDone(login_target())));
    assert!(calls.contains(&Call::MakeUnattended(MachineId::new("corp-vm"), UnattendedMethod::KeepToken)));
}

#[test]
fn skipping_the_unattended_offer_keeps_the_machine_interactive() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    ask_for_login(&mut store, &fake);
    let ready = LoginEvent::Ready { machine: MachineId::new("corp-vm"), can_make_unattended: true };
    fake.script_login(vec![], vec![ready.clone()]);
    store.send(A::OpenLogin, |_| {});
    store.send(A::LoginDone, |_| {});
    store.receive(A::LoginProgress(ready), |_| {});
    store.send(A::SkipUnattended, |_| {});
    receive_added(&mut store, "corp-vm");
    assert!(!fake.calls().iter().any(|c| matches!(c, Call::MakeUnattended(..))));
}

#[test]
fn a_login_that_cannot_be_unattended_adds_the_machine_at_once() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    ask_for_login(&mut store, &fake);
    let ready = LoginEvent::Ready { machine: MachineId::new("corp-vm"), can_make_unattended: false };
    fake.script_login(vec![], vec![ready.clone()]);
    store.send(A::OpenLogin, |_| {});
    store.send(A::LoginDone, |_| {});
    store.receive(A::LoginProgress(ready), |_| {});
    receive_added(&mut store, "corp-vm");
}

#[test]
fn a_failed_login_says_why_and_can_be_finished_again() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    ask_for_login(&mut store, &fake);
    let failed = LoginEvent::Failed { reason: "the code was wrong".to_owned() };
    fake.script_login(vec![], vec![failed.clone()]);
    store.send(A::OpenLogin, |_| {});
    store.send(A::LoginDone, |_| {});
    store.receive(A::LoginProgress(failed), |_| {});
    let login = store.state().login.clone();
    assert!(login.as_ref().is_some_and(|l| !l.finishing && l.error.is_some()));
    store.send(A::LoginDone, |_| {});
    assert!(store.state().login.as_ref().is_some_and(|l| l.finishing && l.error.is_none()));
    store.skip_in_flight();
}

#[test]
fn cancelling_during_a_login_closes_it() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    ask_for_login(&mut store, &fake);
    fake.script_login(vec![], vec![]);
    store.send(A::OpenLogin, |_| {});
    store.send(A::Cancel, |_| {});
    store.skip_received();
    assert_eq!(*store.state(), AddMachineState::default());
    assert!(fake.calls().contains(&Call::LoginCancel(login_target())));
}

#[test]
fn a_failed_call_is_kept_as_text_and_out_of_order_actions_change_nothing() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    for action in [
        A::OpenLogin,
        A::LoginDone,
        A::LoginInput(vec![1]),
        A::LoginResize { columns: 1, rows: 1 },
        A::ConfirmHostKey,
        A::DeclineHostKey,
        A::TryAgain,
        A::MakeUnattended,
        A::SkipUnattended,
        A::ChooseMethod(UnattendedMethod::AddKey),
    ] {
        store.send(action, |_| {});
    }
    assert_eq!(*store.state(), open_with_host("corp-vm"));
    assert!(fake.calls().is_empty());

    store.send(
        A::CallFinished(Err(DaemonError::NotFound { what: "the login".to_owned() })),
        |_| {},
    );
    assert!(store.state().error.is_some());
}
