//! MACHINESCRIPTS (D686): the host's feed says a script run changed. The
//! open machine page reads its runs again; Settings › Machines, when it is
//! the page shown, reads the rules again; nothing else is read.
use std::sync::Arc;

use humata2_app::{App, AppAction, AppState, Route};
use humata2_clients::machines::{Call, FakeMachines};
use humata2_clients::model::MachineId;
use humata2_clients::{Clients, Event, fixture};
use humata2_hosts::MachinePageAction;
use humata2_settings::{Section, SettingsAction};
use tca::{TestClock, TestStore};

fn launch(route: Route) -> (TestStore<App>, FakeMachines) {
    let (mut clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let machines = FakeMachines::new();
    clients.machines = Arc::new(machines.clone());
    let mut state = AppState::launch(&clients);
    state.route = route;
    (TestStore::new(state, App::new(clients)).non_exhaustive(), machines)
}

fn reads(machines: &FakeMachines, of: Option<&MachineId>) -> usize {
    machines
        .calls()
        .iter()
        .filter(|call| matches!(call, Call::Scripts(asked) if asked.as_ref() == of))
        .count()
}

#[test]
fn a_changed_run_on_the_open_machine_reads_its_runs_again() {
    let devbox = MachineId::new("devbox-1");
    let (mut store, machines) = launch(Route::Machine(devbox.clone()));
    store.send(AppAction::MachinePage(MachinePageAction::Show(devbox.clone())), |_| {});
    store.settle();
    let before = reads(&machines, Some(&devbox));

    store.send(AppAction::Event(Event::MachineScriptRun(devbox.clone())), |_| {});
    store.settle();

    assert_eq!(reads(&machines, Some(&devbox)), before + 1);
}

#[test]
fn a_changed_run_on_another_machine_reads_nothing() {
    let devbox = MachineId::new("devbox-1");
    let (mut store, machines) = launch(Route::Machine(devbox.clone()));
    store.send(AppAction::MachinePage(MachinePageAction::Show(devbox.clone())), |_| {});
    store.settle();
    let before = machines.calls().len();

    store.send(AppAction::Event(Event::MachineScriptRun(MachineId::new("other"))), |_| {});
    store.settle();

    assert_eq!(machines.calls().len(), before);
}

#[test]
fn settings_machines_reads_the_rules_again_when_it_is_the_page_shown() {
    let (mut store, machines) = launch(Route::Settings);
    store.send(AppAction::Settings(SettingsAction::Section(Section::Machines)), |_| {});
    store.settle();
    let before = reads(&machines, None);

    store.send(AppAction::Event(Event::MachineScriptRun(MachineId::new("devbox-1"))), |_| {});
    store.settle();

    assert_eq!(reads(&machines, None), before + 1);
}

#[test]
fn settings_not_shown_reads_nothing_on_the_event() {
    let (mut store, machines) = launch(Route::Inbox);
    store.send(AppAction::Settings(SettingsAction::Section(Section::Machines)), |_| {});
    store.settle();
    let before = machines.calls().len();

    store.send(AppAction::Event(Event::MachineScriptRun(MachineId::new("devbox-1"))), |_| {});
    store.settle();

    assert_eq!(machines.calls().len(), before);
}
