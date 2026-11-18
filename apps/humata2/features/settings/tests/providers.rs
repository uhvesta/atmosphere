//! Settings › Machines › Providers (PROVIDERS): one row per provider with
//! Commands that open in place one at a time, Remove behind a confirm that
//! refuses while machines still use the provider, a pasted provider file
//! reviewed before it is imported, and host variables whose values leave
//! the state as they are sent.

use std::sync::Arc;

use humata2_clients::machines::{
    Call, FakeMachines, ProviderCommand, ProviderEntry, ProviderImportPlan, ProviderProblem, ProviderVariable,
};
use humata2_clients::{Clients, DaemonError, fixture};
use humata2_settings::{
    ProvidersAction as P, Secret, Section, Settings, SettingsAction as A, SettingsState,
};
use tca::{TestClock, TestStore};

fn setup(providers: Vec<ProviderEntry>) -> (TestStore<Settings>, FakeMachines) {
    let (clients, fakes) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    fakes.machines.set_providers(providers);
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients)).non_exhaustive();
    store.send(A::Section(Section::Machines), |_| {});
    store.receive_matching("providers listed", |a| matches!(a, A::Providers(P::Loaded(Ok(_)))), |_| {});
    (store, fakes.machines)
}

fn provider(id: &str, built_in: bool, machines: &[&str]) -> ProviderEntry {
    ProviderEntry {
        id: id.to_owned(),
        label: format!("{id} label"),
        built_in,
        actions: vec!["create".to_owned(), "delete".to_owned()],
        commands: vec![ProviderCommand { action: "create".to_owned(), command: format!("{id} create") }],
        params: vec![],
        variables: vec![ProviderVariable { name: "HCLOUD_TOKEN".to_owned(), set: false }],
        machines: machines.iter().map(|name| (*name).to_owned()).collect(),
    }
}

fn listed(store: &TestStore<Settings>) -> Vec<String> {
    let list = store.state().providers.list.as_ref().expect("the providers were listed");
    list.iter().map(|provider| provider.id.clone()).collect()
}

fn reloaded(store: &mut TestStore<Settings>) {
    store.receive_matching("providers listed again", |a| matches!(a, A::Providers(P::Loaded(_))), |_| {});
}

#[test]
fn opening_machines_lists_the_providers() {
    let (store, _) = setup(vec![provider("lima", true, &[]), provider("hcloud", false, &[])]);

    assert_eq!(listed(&store), ["lima", "hcloud"]);
}

#[test]
fn commands_open_one_row_at_a_time_and_close_again() {
    let (mut store, _) = setup(vec![provider("lima", true, &[]), provider("hcloud", false, &[])]);

    store.send(A::Providers(P::ToggleCommands("lima".to_owned())), |_| {});
    assert_eq!(store.state().providers.open.as_deref(), Some("lima"));

    store.send(A::Providers(P::ToggleCommands("hcloud".to_owned())), |_| {});
    assert_eq!(store.state().providers.open.as_deref(), Some("hcloud"));

    store.send(A::Providers(P::ToggleCommands("hcloud".to_owned())), |_| {});
    assert_eq!(store.state().providers.open, None);
}

#[test]
fn a_reviewed_file_imports_exactly_what_the_review_showed() {
    let (mut store, machines) = setup(vec![provider("lima", true, &[])]);
    machines.set_import_plan(Ok(ProviderImportPlan {
        providers: vec![provider("hcloud", false, &[])],
        replaces: vec![],
        problems: vec![],
        token: Some("pl_1".to_owned()),
    }));

    store.send(A::Providers(P::OpenImport), |_| {});
    store.send(A::Providers(P::ImportText("[providers.hcloud]".to_owned())), |_| {});
    store.send(A::Providers(P::Review), |_| {});
    store.receive_matching("reviewed", |a| matches!(a, A::Providers(P::Reviewed(Ok(_)))), |_| {});
    let review = store.state().providers.import.as_ref().and_then(|draft| draft.review.as_ref());
    assert_eq!(review.map(|plan| plan.providers[0].id.as_str()), Some("hcloud"));
    assert!(!machines.calls().iter().any(|call| matches!(call, Call::ImportProvider(_))), "a review adds nothing");

    store.send(A::Providers(P::Import), |_| {});
    store.receive_matching("imported", |a| matches!(a, A::Providers(P::Imported(Ok(())))), |_| {});
    reloaded(&mut store);

    assert!(machines.calls().contains(&Call::PreviewProviderImport("[providers.hcloud]".to_owned())));
    assert!(machines.calls().contains(&Call::ImportProvider(Some("pl_1".to_owned()))));
    assert!(store.state().providers.import.is_none());
    assert_eq!(listed(&store), ["lima", "hcloud"]);
}

#[test]
fn a_file_with_problems_shows_why_and_can_not_be_imported() {
    let (mut store, machines) = setup(vec![]);
    machines.set_import_plan(Ok(ProviderImportPlan {
        providers: vec![],
        replaces: vec![],
        problems: vec![ProviderProblem {
            provider: String::new(),
            field: String::new(),
            line: 2,
            message: "expected a table".to_owned(),
        }],
        token: None,
    }));

    store.send(A::Providers(P::OpenImport), |_| {});
    store.send(A::Providers(P::ImportText("not toml".to_owned())), |_| {});
    store.send(A::Providers(P::Review), |_| {});
    store.receive_matching("reviewed", |a| matches!(a, A::Providers(P::Reviewed(Ok(_)))), |_| {});

    let draft = store.state().providers.import.as_ref().expect("the field stays open");
    assert!(draft.problem.is_some());
    assert!(draft.review.is_none());

    store.send(A::Providers(P::Import), |_| {});
    store.skip_received();
    assert!(!machines.calls().iter().any(|call| matches!(call, Call::ImportProvider(_))));
}

#[test]
fn a_daemon_that_can_not_review_says_why_under_the_field() {
    let (mut store, machines) = setup(vec![]);
    machines.fail("preview_provider_import", DaemonError::Refused { what: "Reviewing".to_owned(), reason: "no".to_owned() });

    store.send(A::Providers(P::OpenImport), |_| {});
    store.send(A::Providers(P::ImportText("[providers.x]".to_owned())), |_| {});
    store.send(A::Providers(P::Review), |_| {});
    store.receive_matching("reviewed", |a| matches!(a, A::Providers(P::Reviewed(Err(_)))), |_| {});

    assert!(store.state().providers.import.as_ref().is_some_and(|draft| draft.problem.is_some() && !draft.busy));
}

#[test]
fn an_empty_paste_is_not_sent_for_review() {
    let (mut store, machines) = setup(vec![]);

    store.send(A::Providers(P::OpenImport), |_| {});
    store.send(A::Providers(P::Review), |_| {});

    assert!(store.state().providers.import.as_ref().is_some_and(|draft| draft.problem.is_some()));
    assert!(!machines.calls().iter().any(|call| matches!(call, Call::PreviewProviderImport(_))));
}

#[test]
fn remove_asks_first_then_removes_the_provider() {
    let (mut store, machines) = setup(vec![provider("lima", true, &[]), provider("hcloud", false, &[])]);

    store.send(A::Providers(P::Remove("hcloud".to_owned())), |_| {});
    assert_eq!(store.state().providers.removing.as_deref(), Some("hcloud"));
    assert!(!machines.calls().contains(&Call::RemoveProvider("hcloud".to_owned())), "asking removes nothing");

    store.send(A::Providers(P::ConfirmRemove("hcloud".to_owned())), |_| {});
    store.receive_matching("removed", |a| matches!(a, A::Providers(P::Removed(Ok(())))), |_| {});
    reloaded(&mut store);

    assert_eq!(listed(&store), ["lima"]);
    assert_eq!(store.state().providers.removing, None);
}

#[test]
fn a_remove_the_daemon_refuses_says_why_and_keeps_the_provider() {
    let (mut store, machines) = setup(vec![provider("hcloud", false, &[])]);
    // A machine was made on it since the list was read.
    machines.fail(
        "remove_provider",
        DaemonError::Refused {
            what: "Removing the provider".to_owned(),
            reason: "machines still use it".to_owned(),
        },
    );
    store.send(A::Providers(P::Remove("hcloud".to_owned())), |_| {});
    store.send(A::Providers(P::ConfirmRemove("hcloud".to_owned())), |_| {});
    store.receive_matching("refused", |a| matches!(a, A::Providers(P::Removed(Err(_)))), |_| {});

    assert!(store.state().providers.error.is_some());
    assert_eq!(store.state().providers.removing, None);
    assert_eq!(listed(&store), ["hcloud"]);
}

#[test]
fn cancel_keeps_the_provider() {
    let (mut store, machines) = setup(vec![provider("hcloud", false, &[])]);

    store.send(A::Providers(P::Remove("hcloud".to_owned())), |_| {});
    store.send(A::Providers(P::CancelRemove), |_| {});
    store.send(A::Providers(P::ConfirmRemove("hcloud".to_owned())), |_| {});
    store.skip_received();

    assert!(!machines.calls().contains(&Call::RemoveProvider("hcloud".to_owned())));
    assert_eq!(listed(&store), ["hcloud"]);
}

#[test]
fn remove_is_blocked_while_machines_use_the_provider() {
    let (mut store, machines) = setup(vec![provider("hcloud", false, &["box-1", "box-2"])]);

    store.send(A::Providers(P::Remove("hcloud".to_owned())), |_| {});
    assert_eq!(store.state().providers.removing.as_deref(), Some("hcloud"));
    store.send(A::Providers(P::ConfirmRemove("hcloud".to_owned())), |_| {});
    store.skip_received();

    assert!(!machines.calls().contains(&Call::RemoveProvider("hcloud".to_owned())));
    assert_eq!(listed(&store), ["hcloud"]);
}

#[test]
fn a_built_in_provider_can_not_be_removed() {
    let (mut store, machines) = setup(vec![provider("lima", true, &[])]);

    store.send(A::Providers(P::Remove("lima".to_owned())), |_| {});
    store.send(A::Providers(P::ConfirmRemove("lima".to_owned())), |_| {});
    store.skip_received();

    assert_eq!(store.state().providers.removing, None);
    assert!(!machines.calls().contains(&Call::RemoveProvider("lima".to_owned())));
}

#[test]
fn a_variable_value_leaves_the_state_as_it_is_set() {
    let (mut store, machines) = setup(vec![provider("hcloud", false, &[])]);

    store.send(A::Providers(P::VariableValue("HCLOUD_TOKEN".to_owned(), Secret::new("s3cret-value"))), |_| {});
    assert!(!format!("{:?}", store.state()).contains("s3cret-value"), "a typed value never shows in Debug");

    store.send(A::Providers(P::SetVariable("HCLOUD_TOKEN".to_owned())), |_| {});
    assert!(store.state().providers.values.is_empty());
    store.receive_matching("saved", |a| matches!(a, A::Providers(P::VariableSaved(Ok(())))), |_| {});
    reloaded(&mut store);

    assert!(machines.calls().contains(&Call::SetProviderVariable("HCLOUD_TOKEN".to_owned(), "s3cret-value".to_owned())));
    let entry = store.state().providers.provider("hcloud").expect("still listed");
    assert_eq!(entry.unset().count(), 0);
}

#[test]
fn set_with_nothing_typed_sends_nothing() {
    let (mut store, machines) = setup(vec![provider("hcloud", false, &[])]);

    store.send(A::Providers(P::SetVariable("HCLOUD_TOKEN".to_owned())), |_| {});
    store.skip_received();

    assert!(!machines.calls().iter().any(|call| matches!(call, Call::SetProviderVariable(..))));
}

#[test]
fn clear_removes_the_variable() {
    let (mut store, machines) = setup(vec![provider("hcloud", false, &[])]);

    store.send(A::Providers(P::ClearVariable("HCLOUD_TOKEN".to_owned())), |_| {});
    store.receive_matching("cleared", |a| matches!(a, A::Providers(P::VariableSaved(Ok(())))), |_| {});

    assert!(machines.calls().contains(&Call::ClearProviderVariable("HCLOUD_TOKEN".to_owned())));
}
