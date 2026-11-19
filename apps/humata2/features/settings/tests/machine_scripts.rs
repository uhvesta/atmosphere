//! Settings › Machines › Scripts (MACHINESCRIPTS, D680): opening the section
//! reads the owner's rules and script names; Edit rules opens the rules file
//! in the editor and Reveal scripts shows the scripts folder. Nothing here
//! writes a rule or a script.

use std::sync::Arc;

use humata2_clients::machines::{
    FakeMachines, MachineScripts, SCRIPT_RULES_PATH, SCRIPTS_FOLDER, ScriptRuleView, ScriptStepView,
};
use humata2_clients::{Clients, DaemonError, FakeReveal, fixture};
use humata2_settings::{Section, Settings, SettingsAction as A, SettingsState};
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeMachines, FakeReveal) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let store = TestStore::new(SettingsState::default(), Settings::new(clients)).non_exhaustive();
    (store, fakes.machines, fakes.reveal)
}

fn scripts() -> MachineScripts {
    MachineScripts {
        rules: vec![ScriptRuleView {
            name: "set-up-tools".to_owned(),
            triggers: vec!["added".to_owned()],
            machines: vec![],
            once: true,
            steps: vec![ScriptStepView { kind: "run".to_owned(), label: "setup.sh".to_owned() }],
        }],
        last_runs: vec![],
        script_names: vec!["setup.sh".to_owned()],
        problem: None,
        warnings: vec![],
    }
}

#[test]
fn opening_machines_reads_the_rules_and_the_script_names() {
    let (mut store, machines, _) = setup();
    machines.set_scripts(Ok(scripts()));

    store.send(A::Section(Section::Machines), |_| {});
    store.receive_matching("scripts read", |a| matches!(a, A::ScriptsShown(Ok(_))), |_| {});

    let shown = store.state().scripts.as_ref().expect("the scripts were read");
    assert_eq!(shown.rules.len(), 1);
    assert_eq!(shown.script_names, ["setup.sh"]);
    assert!(store.state().scripts_error.is_none());
}

#[test]
fn a_failed_read_keeps_what_was_read_and_says_why() {
    let (mut store, machines, _) = setup();
    machines.set_scripts(Ok(scripts()));
    store.send(A::Section(Section::Machines), |_| {});
    store.receive_matching("scripts read", |a| matches!(a, A::ScriptsShown(Ok(_))), |_| {});

    machines.fail("scripts", DaemonError::Refused { what: "Reading".to_owned(), reason: "no".to_owned() });
    store.send(A::Section(Section::Machines), |_| {});
    store.receive_matching("scripts failed", |a| matches!(a, A::ScriptsShown(Err(_))), |_| {});

    assert_eq!(store.state().scripts.as_ref().map(|shown| shown.rules.len()), Some(1));
    assert!(store.state().scripts_error.is_some());
}

#[test]
fn edit_rules_opens_the_rules_file_in_the_editor() {
    let (mut store, _, reveal) = setup();

    store.send(A::EditScriptRules, |_| {});

    assert_eq!(reveal.edited(), [SCRIPT_RULES_PATH]);
    assert!(store.state().scripts_error.is_none());
}

#[test]
fn reveal_scripts_shows_the_scripts_folder() {
    let (mut store, _, reveal) = setup();

    store.send(A::RevealScripts, |_| {});

    assert_eq!(reveal.folders(), [SCRIPTS_FOLDER]);
}
