//! The add-machine dialog against a scripted fake: what each outcome of an
//! add leaves in the state and what the dialog tells its parent.
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    AddEvent, AddFailure, AddOutcome, AddRequest, AddTarget, Call, Choices, FakeMachines, Fix,
    LoginEvent, LoginReason, LoginTarget, OperationId, PromptKind, Provider, ProviderParam, SshHost,
    StepKind, StepStatus, StepUpdate, UnattendedMethod,
};
use humata2_clients::model::MachineId;
use humata2_hosts::add::{Choice, Delegate, LoginWhy, login_detail, login_intro};
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
            unset: Vec::new(),
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
            unset: Vec::new(),
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

fn open_on_cloud(unset: &[&str]) -> AddMachineState {
    AddMachineState {
        open: true,
        providers: vec![Provider {
            id: "hcloud".to_owned(),
            label: "Cloud VM".to_owned(),
            interactive: false,
            params: vec![],
            unset: unset.iter().map(|name| (*name).to_owned()).collect(),
        }],
        choice: Some(Choice::Provider { id: "hcloud".to_owned(), params: std::collections::BTreeMap::new() }),
        ..AddMachineState::default()
    }
}

#[test]
fn a_provider_with_unset_variables_names_them_and_does_not_start() {
    let (mut store, fake) = store(open_on_cloud(&["HCLOUD_TOKEN", "SSH_KEY"]));
    let needs = store.state().needs_variables().unwrap_or_default();
    assert!(needs.contains("HCLOUD_TOKEN") && needs.contains("SSH_KEY"), "{needs}");
    store.send(A::Start, |_| {});
    assert!(!store.state().running);
    assert!(!fake.calls().iter().any(|call| matches!(call, Call::Add(_))));
    assert_eq!(open_on_cloud(&[]).needs_variables(), None);
}

#[test]
fn manage_providers_closes_the_dialog_and_asks_for_settings() {
    let (mut store, _fake) = store(open_on_cloud(&["HCLOUD_TOKEN"]));
    store.send(A::ManageProviders, |state| *state = AddMachineState::default());
    store.receive(A::Delegate(Delegate::ManageProviders), |_| {});
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
    // The add stays open, as a running one does, until Cancel reaches it.
    fake.script_add_held(vec![started()]);
    store.send(A::Start, |_| {});
    store.receive(A::Progress(started()), |_| {});
    store.send(A::Cancel, |_| {});
    store.skip_received();
    assert_eq!(*store.state(), AddMachineState::default());
    assert!(fake.calls().contains(&Call::Cancel(op())));
    let adds = fake.calls().into_iter().filter(|c| matches!(c, Call::Add(_))).count();
    assert_eq!(adds, 1, "nothing is added or retried");
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

fn ready(can_make_unattended: bool) -> LoginEvent {
    LoginEvent::Ready {
        machine: MachineId::new("corp-vm"),
        can_make_unattended,
        daemon_version: Some("2.0.4".to_owned()),
    }
}

fn code_prompt() -> LoginReason {
    LoginReason::Prompted { kind: PromptKind::Code, line: "Verification code:".to_owned() }
}

fn ask_for_login(store: &mut TestStore<AddMachine>, fake: &FakeMachines) {
    ask_for_login_because(store, fake, code_prompt());
}

fn ask_for_login_because(store: &mut TestStore<AddMachine>, fake: &FakeMachines, reason: LoginReason) {
    fake.script_add(vec![started(), AddEvent::Outcome(AddOutcome::NeedsLogin { reason })]);
    store.send(A::Start, |_| {});
    store.receive(A::ProgressEnded(StreamEnd::Finished), |_| {});
}

/// The detail under the "Log in" step after the login worked.
fn login_step_detail(store: &TestStore<AddMachine>) -> Option<String> {
    store
        .state()
        .steps
        .iter()
        .find(|step| step.kind == StepKind::Login)
        .map(|step| step.detail.clone())
}

#[test]
fn the_login_offer_keeps_the_daemons_reason_and_opening_it_keeps_why() {
    let (mut store, fake) = store(provider_state());
    ask_for_login(&mut store, &fake);
    assert_eq!(store.state().login_offer, Some(code_prompt()));
    store.send(A::OpenLogin, |_| {});
    assert_eq!(
        store.state().login.as_ref().map(|login| login.why),
        Some(LoginWhy::Prompted(PromptKind::Code))
    );
    assert!(store.state().login_offer.is_none());
}

#[test]
fn a_hung_connect_is_always_interactive_for_an_interactive_provider_and_hung_for_others() {
    let (mut store, fake) = store(provider_state());
    ask_for_login_because(&mut store, &fake, LoginReason::Hung);
    store.send(A::OpenLogin, |_| {});
    assert_eq!(store.state().login.as_ref().map(|login| login.why), Some(LoginWhy::AlwaysInteractive));

    let (mut store, fake) = self::store(open_with_host("corp-vm"));
    ask_for_login_because(&mut store, &fake, LoginReason::Hung);
    store.send(A::OpenLogin, |_| {});
    assert_eq!(store.state().login.as_ref().map(|login| login.why), Some(LoginWhy::Hung));
}

#[test]
fn a_prompt_the_daemon_could_not_name_is_the_persons_own_choice_to_log_in() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    let other = LoginReason::Prompted { kind: PromptKind::Other, line: "Token?".to_owned() };
    ask_for_login_because(&mut store, &fake, other);
    store.send(A::OpenLogin, |_| {});
    assert_eq!(store.state().login.as_ref().map(|login| login.why), Some(LoginWhy::Asked));
}

#[test]
fn the_first_prompt_of_the_session_says_what_the_login_accepted() {
    let (mut store, fake) = store(provider_state());
    ask_for_login_because(&mut store, &fake, LoginReason::Hung);
    let ready = ready(true);
    fake.script_login(vec![], vec![ready.clone()]);
    store.send(A::OpenLogin, |_| {});
    store.send(
        A::LoginProgress(LoginEvent::Prompt { kind: PromptKind::Password, line: "Password:".to_owned() }),
        |_| {},
    );
    // A later prompt doesn't replace the first.
    store.send(
        A::LoginProgress(LoginEvent::Prompt { kind: PromptKind::Code, line: "Code:".to_owned() }),
        |_| {},
    );
    assert_eq!(store.state().login.as_ref().and_then(|login| login.prompt), Some(PromptKind::Password));
    store.send(A::LoginDone, |_| {});
    store.receive(A::LoginProgress(ready), |_| {});
    assert_eq!(
        login_step_detail(&store),
        Some(login_detail("corp-ssh", LoginWhy::Prompted(PromptKind::Password), "corp-vm"))
    );
}

#[test]
fn without_a_prompt_the_detail_follows_the_reason_and_names_ssh_for_a_plain_host() {
    let (mut store, fake) = store(provider_state());
    log_in(&mut store, &fake, ready(true));
    assert_eq!(
        login_step_detail(&store),
        Some(login_detail("corp-ssh", LoginWhy::Prompted(PromptKind::Code), "corp-vm"))
    );

    let (mut store, fake) = self::store(open_with_host("corp-vm"));
    log_in(&mut store, &fake, ready(true));
    assert_eq!(
        login_step_detail(&store),
        Some(login_detail("ssh", LoginWhy::Prompted(PromptKind::Code), "corp-vm"))
    );
}

#[test]
fn the_login_texts_for_every_reason_are_the_decisions_wording() {
    let rows = [
        (LoginWhy::Prompted(PromptKind::Code), "corp-ssh is asking for a verification code, so Humata opened it here. Finish logging in, then press Done.", "Finished in the terminal; corp-ssh accepted the code"),
        (LoginWhy::Prompted(PromptKind::Password), "corp-ssh is asking for a password, so Humata opened it here. Finish logging in, then press Done.", "Finished in the terminal; corp-ssh accepted the password"),
        (LoginWhy::Prompted(PromptKind::Passphrase), "corp-ssh is asking for a passphrase, so Humata opened it here. Finish logging in, then press Done.", "Finished in the terminal; corp-ssh accepted the passphrase"),
        (LoginWhy::Prompted(PromptKind::Confirm), "corp-ssh is asking a yes or no question, so Humata opened it here. Finish logging in, then press Done.", "Finished in the terminal; corp-vm answered"),
        (LoginWhy::Prompted(PromptKind::Device), "corp-ssh is asking you to sign in on a web page, so Humata opened it here. Finish logging in, then press Done.", "Finished in the terminal; corp-ssh accepted the sign-in"),
        (LoginWhy::Hung, "corp-ssh didn't finish connecting on its own, so Humata opened it here. Finish logging in, then press Done.", "Finished in the terminal; corp-vm answered"),
        (LoginWhy::AlwaysInteractive, "corp-ssh is set up to log in by hand, so Humata opened it here. Finish logging in, then press Done.", "Finished in the terminal; corp-vm answered"),
        (LoginWhy::Asked, "Humata opened the login to corp-vm here. Finish logging in, then press Done.", "Finished in the terminal; corp-vm answered"),
        (LoginWhy::Prompted(PromptKind::Other), "Humata opened the login to corp-vm here. Finish logging in, then press Done.", "Finished in the terminal; corp-vm answered"),
    ];
    for (why, intro, detail) in rows {
        assert_eq!(login_intro("corp-ssh", why, "corp-vm"), intro, "{why:?}");
        assert_eq!(login_detail("corp-ssh", why, "corp-vm"), detail, "{why:?}");
    }
    // A plain SSH host is `ssh`, the program Humata ran.
    assert_eq!(
        login_detail("ssh", LoginWhy::Prompted(PromptKind::Password), "corp-vm"),
        "Finished in the terminal; ssh accepted the password"
    );
    assert_eq!(
        login_intro("ssh", LoginWhy::Prompted(PromptKind::Password), "corp-vm"),
        "ssh is asking for a password, so Humata opened it here. Finish logging in, then press Done."
    );
}

#[test]
fn a_login_flows_from_the_offer_through_done_to_the_unattended_offer() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    ask_for_login(&mut store, &fake);
    assert_eq!(store.state().login_offer, Some(code_prompt()));

    let ready = ready(true);
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

    // A plain SSH host has no provider's token to keep (MP-17), so the
    // offer's method stays the key; the provider case has its own test.
    store.send(A::MakeUnattended, |_| {});
    store.receive_matching("finished", |a| matches!(a, A::UnattendedFinished(Ok(()))), |_| {});
    receive_added(&mut store, "corp-vm");
    let calls = fake.calls();
    assert!(calls.contains(&Call::LoginInput(login_target(), b"123456\r".to_vec())));
    assert!(calls.contains(&Call::LoginResize(login_target(), 90, 20)));
    assert!(calls.contains(&Call::LoginDone(login_target())));
    assert!(calls.contains(&Call::MakeUnattended(MachineId::new("corp-vm"), UnattendedMethod::AddKey)));
}

#[test]
fn skipping_the_unattended_offer_keeps_the_machine_interactive() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    ask_for_login(&mut store, &fake);
    let ready = ready(true);
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
    let ready = ready(false);
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

fn provider_state() -> AddMachineState {
    AddMachineState {
        open: true,
        providers: vec![Provider {
            id: "corp-ssh".to_owned(),
            label: "Corporate SSH".to_owned(),
            interactive: true,
            params: vec![],
            unset: Vec::new(),
        }],
        choice: Some(Choice::Provider {
            id: "corp-ssh".to_owned(),
            params: std::collections::BTreeMap::new(),
        }),
        ..AddMachineState::default()
    }
}

/// Runs an add that asks for a login through to the login's ready event.
fn log_in(store: &mut TestStore<AddMachine>, fake: &FakeMachines, ready: LoginEvent) {
    ask_for_login(store, fake);
    fake.script_login(vec![], vec![ready.clone()]);
    store.send(A::OpenLogin, |_| {});
    store.send(A::LoginDone, |_| {});
    store.receive(A::LoginProgress(ready), |_| {});
}

#[test]
fn a_login_shows_as_its_own_done_step_in_place_of_the_connect_attempt() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    log_in(&mut store, &fake, ready(true));
    let kinds: Vec<_> = store.state().steps.iter().map(|s| (s.kind, s.status)).collect();
    assert_eq!(
        kinds,
        vec![
            (StepKind::Login, StepStatus::Done),
            (StepKind::Check, StepStatus::NotRun),
            (StepKind::Install, StepStatus::NotRun),
            (StepKind::Start, StepStatus::NotRun),
            (StepKind::FindAgents, StepStatus::NotRun),
        ]
    );
}

#[test]
fn the_offer_carries_the_daemons_version_when_it_was_known() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    log_in(&mut store, &fake, ready(true));
    assert_eq!(store.state().unattended.as_ref().and_then(|o| o.version.clone()).as_deref(), Some("2.0.4"));

    let (mut store, fake) = self::store(open_with_host("corp-vm"));
    let unknown = LoginEvent::Ready { machine: MachineId::new("corp-vm"), can_make_unattended: true, daemon_version: None };
    log_in(&mut store, &fake, unknown);
    assert_eq!(store.state().unattended.as_ref().map(|o| o.version.clone()), Some(None));
}

#[test]
fn a_plain_ssh_login_has_no_token_to_keep() {
    let (mut store, fake) = store(open_with_host("corp-vm"));
    log_in(&mut store, &fake, ready(true));
    assert_eq!(store.state().unattended.as_ref().map(|o| o.provider.clone()), Some(None));
    store.send(A::ChooseMethod(UnattendedMethod::KeepToken), |_| {});
    assert_eq!(store.state().unattended.as_ref().map(|o| o.method), Some(UnattendedMethod::AddKey));
}

#[test]
fn a_provider_login_offers_its_own_cached_token() {
    let (mut store, fake) = store(provider_state());
    log_in(&mut store, &fake, ready(true));
    assert_eq!(
        store.state().unattended.as_ref().and_then(|o| o.provider.clone()).as_deref(),
        Some("corp-ssh")
    );
    store.send(A::ChooseMethod(UnattendedMethod::KeepToken), |_| {});
    assert_eq!(store.state().unattended.as_ref().map(|o| o.method), Some(UnattendedMethod::KeepToken));
}

#[test]
fn a_typed_host_wins_over_the_popups_pick() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    fake.script_add(added_events("gpu-9"));
    store.send(A::TypeHost("gpu-9".to_owned()), |_| {});
    assert_eq!(store.state().display_name(), Some("gpu-9"));
    store.send(A::Start, |_| {});
    receive_added(&mut store, "gpu-9");
    assert_eq!(fake.calls(), vec![Call::Add(ssh_request("gpu-9", None))]);
}

#[test]
fn emptying_the_typed_host_goes_back_to_the_pick_and_a_new_pick_clears_it() {
    let (mut store, _fake) = store(open_with_host("devbox-2"));
    store.send(A::TypeHost("gpu-9".to_owned()), |_| {});
    store.send(A::TypeHost("  ".to_owned()), |_| {});
    assert_eq!(store.state().effective_choice(), Some(Choice::Host("devbox-2".to_owned())));

    store.send(A::TypeHost("gpu-9".to_owned()), |_| {});
    store.send(A::SelectHost("corp-vm".to_owned()), |_| {});
    assert!(store.state().typed.is_empty());
    assert_eq!(store.state().effective_choice(), Some(Choice::Host("corp-vm".to_owned())));
}

#[test]
fn the_host_menu_opens_and_closes_on_a_pick_or_cancel_and_not_while_running() {
    let (mut store, fake) = store(open_with_host("devbox-2"));
    store.send(A::HostMenu(true), |_| {});
    assert!(store.state().host_menu);
    store.send(A::SelectHost("corp-vm".to_owned()), |_| {});
    assert!(!store.state().host_menu);

    store.send(A::HostMenu(true), |_| {});
    store.send(A::Cancel, |_| {});
    assert_eq!(*store.state(), AddMachineState::default());
    store.send(A::HostMenu(true), |_| {});
    assert!(!store.state().host_menu, "a closed dialog has no menu");

    let (mut store, fake2) = self::store(open_with_host("devbox-2"));
    fake2.script_add_held(vec![started()]);
    store.send(A::Start, |_| {});
    store.send(A::HostMenu(true), |_| {});
    assert!(!store.state().host_menu, "a running add can't change its target");
    store.send(A::Cancel, |_| {});
    store.skip_received();
    assert!(fake.calls().is_empty());
}
