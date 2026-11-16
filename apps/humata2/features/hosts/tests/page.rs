//! A machine's page against a scripted fake: loading, keeping what was
//! loaded when a reload fails, the skills preview and host variables.
use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    Call, ClashChoice, FakeMachines, GhLogin, MachineDetail, MachineHealth, Reach, SkillClash,
    SkillsPreview,
};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_hosts::page::{Delegate, Tab};
use humata2_hosts::{MachinePage, MachinePageAction as A, MachinePageState};
use tca::TestStore;

fn id() -> MachineId {
    MachineId::new("devbox-1")
}

fn detail(variables: &[&str]) -> MachineDetail {
    MachineDetail {
        machine: id(),
        os: "Ubuntu 24.04".to_owned(),
        arch: "x86_64".to_owned(),
        address: "devbox-1.lan".to_owned(),
        reach: Reach::Ssh,
        daemon_version: "2.0.3".to_owned(),
        health: MachineHealth {
            gh: Some(GhLogin { user: "uhvesta".to_owned(), scopes: vec!["repo".to_owned()] }),
            agents: vec![],
            disk_free: 11,
            disk_total: 500,
            variables: variables.iter().map(|v| (*v).to_owned()).collect(),
            skills_problem: None,
        },
        folders: vec![],
        sessions: vec![SessionId::new("s1")],
        rules: vec![],
    }
}

fn store() -> (TestStore<MachinePage>, FakeMachines) {
    let fake = FakeMachines::new();
    fake.set_detail(Ok(detail(&["DATABASE_URL"])));
    let store = TestStore::new(MachinePageState::default(), MachinePage::new(Arc::new(fake.clone())))
        .non_exhaustive();
    (store, fake)
}

fn show(store: &mut TestStore<MachinePage>) {
    store.send(A::Show(id()), |_| {});
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
}

fn loads(fake: &FakeMachines) -> usize {
    fake.calls().iter().filter(|c| matches!(c, Call::Detail(_))).count()
}

#[test]
fn show_loads_the_detail() {
    let (mut store, _fake) = store();
    store.send(A::Show(id()), |_| {});
    assert!(store.state().loading);
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    assert!(!store.state().loading);
    assert_eq!(store.state().detail, Some(detail(&["DATABASE_URL"])));
    assert_eq!(store.state().tab, Tab::Overview);
    store.send(A::SelectTab(Tab::Storage), |_| {});
    assert_eq!(store.state().tab, Tab::Storage);
}

#[test]
fn a_failed_reload_keeps_the_last_detail() {
    let (mut store, fake) = store();
    show(&mut store);
    fake.set_detail(Err(DaemonError::Offline { machine: id() }));
    store.send(A::Reload, |_| {});
    store.receive_matching("failed load", |a| matches!(a, A::Loaded(_, Err(_))), |_| {});
    assert!(store.state().detail.is_some());
    assert!(store.state().load_error.is_some());
    fake.set_detail(Ok(detail(&[])));
    store.send(A::Reload, |_| {});
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    assert!(store.state().load_error.is_none());
    assert_eq!(store.state().detail, Some(detail(&[])));
}

#[test]
fn showing_another_machine_drops_the_old_detail_and_ignores_its_late_answer() {
    let (mut store, _fake) = store();
    show(&mut store);
    store.send(A::Show(MachineId::new("devbox-2")), |_| {});
    assert!(store.state().detail.is_none());
    store.send(A::Loaded(id(), Ok(detail(&[]))), |_| {});
    assert!(store.state().detail.is_none());
    store.skip_in_flight();
}

#[test]
fn actions_that_belong_to_the_parent_are_delegated() {
    let (mut store, _fake) = store();
    show(&mut store);
    let cases = [
        (A::ReconnectNow, Delegate::Reconnect(id())),
        (A::OpenShell, Delegate::OpenShell(id())),
        (A::NewSession, Delegate::NewSession(id())),
        (A::OpenSession(SessionId::new("s1")), Delegate::OpenSession(SessionId::new("s1"))),
        (
            A::OpenFolder(ChannelId::new("infra")),
            Delegate::OpenFolder { machine: id(), channel: ChannelId::new("infra") },
        ),
        (A::ChangeAgentAccess, Delegate::ChangeAgentAccess(id())),
    ];
    for (action, delegate) in cases {
        store.send(action, |_| {});
        store.receive(A::Delegate(delegate), |_| {});
    }
}

#[test]
fn a_sync_shows_its_preview_and_needs_a_confirm() {
    let (mut store, fake) = store();
    show(&mut store);
    fake.set_preview(Ok(SkillsPreview {
        first_sync: true,
        added: vec!["review".to_owned()],
        ..SkillsPreview::default()
    }));
    store.send(A::SyncSkills, |_| {});
    store.receive_matching("preview", |a| matches!(a, A::Previewed(_, Ok(_))), |_| {});
    assert!(store.state().skills.as_ref().is_some_and(|d| d.preview.is_some()));
    assert!(!fake.calls().iter().any(|c| matches!(c, Call::SyncSkills(_))));

    store.send(A::ConfirmSync, |_| {});
    store.receive_matching("synced", |a| matches!(a, A::Synced(_, Ok(()))), |_| {});
    store.receive_matching("toast", |a| matches!(a, A::Delegate(Delegate::Toast(_))), |_| {});
    assert!(store.state().skills.is_none());
    assert_eq!(fake.calls().iter().filter(|c| matches!(c, Call::SyncSkills(_))).count(), 1);
    store.receive_matching("reloaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    assert_eq!(loads(&fake), 2);
}

#[test]
fn a_clash_blocks_the_sync_until_it_is_resolved() {
    let (mut store, fake) = store();
    show(&mut store);
    fake.set_preview(Ok(SkillsPreview {
        updated: vec!["review".to_owned()],
        clashes: vec![SkillClash { name: "lint".to_owned() }],
        ..SkillsPreview::default()
    }));
    store.send(A::SyncSkills, |_| {});
    store.receive_matching("preview", |a| matches!(a, A::Previewed(_, Ok(_))), |_| {});
    store.send(A::ConfirmSync, |_| {});
    assert!(!fake.calls().iter().any(|c| matches!(c, Call::SyncSkills(_))));

    store.send(A::ResolveClash("lint".to_owned(), ClashChoice::MachineLocal), |_| {});
    store.receive_matching("resolved", |a| matches!(a, A::ClashResolved(_, Ok(()))), |_| {});
    assert!(fake.calls().contains(&Call::ResolveClash(id(), "lint".to_owned(), ClashChoice::MachineLocal)));
    let clashes = store.state().skills.as_ref().and_then(|d| d.preview.as_ref()).map(|p| p.clashes.len());
    assert_eq!(clashes, Some(0));

    store.send(A::ConfirmSync, |_| {});
    store.receive_matching("synced", |a| matches!(a, A::Synced(_, Ok(()))), |_| {});
    store.skip_in_flight();
}

#[test]
fn a_failed_preview_is_kept_and_cancel_closes_the_dialog() {
    let (mut store, fake) = store();
    show(&mut store);
    fake.set_preview(Err(DaemonError::Offline { machine: id() }));
    store.send(A::SyncSkills, |_| {});
    store.receive_matching("preview", |a| matches!(a, A::Previewed(_, Err(_))), |_| {});
    assert!(store.state().skills.as_ref().is_some_and(|d| d.error.is_some() && d.preview.is_none()));
    store.send(A::ConfirmSync, |_| {});
    assert!(!fake.calls().iter().any(|c| matches!(c, Call::SyncSkills(_))));
    store.send(A::CancelSync, |_| {});
    assert!(store.state().skills.is_none());
}

#[test]
fn a_variable_value_is_dropped_from_state_as_it_is_sent() {
    let (mut store, fake) = store();
    show(&mut store);
    store.send(A::BeginVariable(None), |_| {});
    store.send(A::VariableName("STRIPE_KEY".to_owned()), |_| {});
    store.send(A::VariableValue("sk_live_secret".to_owned()), |_| {});
    store.send(A::SendVariable, |_| {});
    let edit = store.state().variable.clone();
    assert!(edit.as_ref().is_some_and(|e| e.sending && e.value.is_empty()));
    assert!(!format!("{:?}", store.state()).contains("sk_live_secret"));
    store.receive_matching("sent", |a| matches!(a, A::VariableSent(Ok(()))), |_| {});
    assert!(store.state().variable.is_none());
    assert!(fake.calls().contains(&Call::SetVariable(id(), "STRIPE_KEY".to_owned(), "sk_live_secret".to_owned())));
    store.skip_in_flight();
}

#[test]
fn an_empty_variable_is_not_sent_and_a_refused_one_says_so() {
    let (mut store, fake) = store();
    show(&mut store);
    store.send(A::BeginVariable(Some("STRIPE_KEY".to_owned())), |_| {});
    store.send(A::SendVariable, |_| {});
    assert!(fake.calls().iter().all(|c| !matches!(c, Call::SetVariable(..))));

    fake.fail("set_variable", DaemonError::Offline { machine: id() });
    store.send(A::VariableValue("x".to_owned()), |_| {});
    store.send(A::SendVariable, |_| {});
    store.receive_matching("refused", |a| matches!(a, A::VariableSent(Err(_))), |_| {});
    let edit = store.state().variable.clone();
    assert!(edit.is_some_and(|e| !e.sending && e.error.is_some() && e.value.is_empty()));
}

#[test]
fn removing_a_variable_calls_the_client_and_reloads() {
    let (mut store, fake) = store();
    show(&mut store);
    store.send(A::RemoveVariable("DATABASE_URL".to_owned()), |_| {});
    store.receive_matching("removed", |a| matches!(a, A::VariableRemoved(Ok(()))), |_| {});
    store.receive_matching("reloaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    assert!(fake.calls().contains(&Call::RemoveVariable(id(), "DATABASE_URL".to_owned())));
    assert_eq!(loads(&fake), 2);

    fake.fail("remove_variable", DaemonError::Offline { machine: id() });
    store.send(A::RemoveVariable("X".to_owned()), |_| {});
    store.receive_matching("refused", |a| matches!(a, A::VariableRemoved(Err(_))), |_| {});
    assert!(store.state().error.is_some());
}
