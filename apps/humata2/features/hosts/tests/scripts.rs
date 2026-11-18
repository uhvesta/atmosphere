//! The Scripts section of a machine's page against the fake: the rules that
//! apply to the machine with their last run, Run now with its outcome, a
//! failed run's output, a rules file that can't be used, and the host's feed
//! saying a run changed.

use std::sync::Arc;

use humata2_clients::DaemonError;
use humata2_clients::machines::{
    Call, FakeMachines, GhLogin, MachineDetail, MachineHealth, MachineScripts, Reach, ScriptOutcome,
    ScriptRuleView, ScriptRun, ScriptStepView,
};
use humata2_clients::model::{MachineId, SessionId};
use humata2_hosts::{MachinePage, MachinePageAction as A, MachinePageState};
use tca::TestStore;

fn id() -> MachineId {
    MachineId::new("devbox-1")
}

fn detail() -> MachineDetail {
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
            variables: vec![],
            skills_problem: None,
            git_version: "2.43.0".to_owned(),
            gh_version: "2.83.0".to_owned(),
        },
        folders: vec![],
        sessions: vec![SessionId::new("s1")],
        rules: vec![],
        stop: None,
    }
}

fn rule(name: &str) -> ScriptRuleView {
    ScriptRuleView {
        name: name.to_owned(),
        triggers: vec!["added".to_owned()],
        machines: vec![],
        once: false,
        steps: vec![ScriptStepView { kind: "run".to_owned(), label: "setup.sh".to_owned() }],
    }
}

fn earlier_run(rule: &str, outcome: ScriptOutcome) -> ScriptRun {
    ScriptRun {
        id: "run-0".to_owned(),
        machine: id(),
        rule: rule.to_owned(),
        trigger: "added".to_owned(),
        session: None,
        at: "09:30".to_owned(),
        took_ms: 800,
        outcome,
        steps: vec![],
    }
}

fn scripts(rules: &[&str]) -> MachineScripts {
    MachineScripts {
        rules: rules.iter().map(|name| rule(name)).collect(),
        last_runs: vec![],
        script_names: vec!["setup.sh".to_owned()],
        problem: None,
        warnings: vec![],
    }
}

fn store(scripts: MachineScripts) -> (TestStore<MachinePage>, FakeMachines) {
    let fake = FakeMachines::new();
    fake.set_detail(Ok(detail()));
    fake.set_scripts(Ok(scripts));
    let store = TestStore::new(MachinePageState::default(), MachinePage::new(Arc::new(fake.clone())))
        .non_exhaustive();
    (store, fake)
}

fn show(store: &mut TestStore<MachinePage>) {
    store.send(A::Show(id()), |_| {});
    store.receive_matching("loaded", |a| matches!(a, A::Loaded(_, Ok(_))), |_| {});
    store.receive_matching("scripts", |a| matches!(a, A::ScriptsLoaded(_, Ok(_))), |_| {});
}

fn script_calls(fake: &FakeMachines) -> usize {
    fake.calls().iter().filter(|call| matches!(call, Call::Scripts(_))).count()
}

fn runs_sent(fake: &FakeMachines) -> usize {
    fake.calls().iter().filter(|call| matches!(call, Call::RunScript(..))).count()
}

#[test]
fn showing_the_page_lists_the_rules_with_their_last_run() {
    let mut loaded = scripts(&["set-up-tools", "after-a-move"]);
    loaded.last_runs.push(earlier_run("set-up-tools", ScriptOutcome::Passed));
    let (mut store, _) = store(loaded);
    show(&mut store);

    let state = store.state();
    let rows = state.scripts.rows();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].rule.name, "set-up-tools");
    assert_eq!(rows[0].last.map(|run| run.outcome), Some(ScriptOutcome::Passed));
    assert!(rows[1].last.is_none());
    assert!(state.scripts.visible());
}

#[test]
fn a_machine_with_no_rules_shows_no_scripts() {
    let (mut store, _) = store(scripts(&[]));
    show(&mut store);

    assert!(store.state().scripts.rows().is_empty());
    assert!(!store.state().scripts.visible());
}

#[test]
fn run_now_is_running_until_the_run_ends_and_then_shows_its_outcome() {
    let (mut store, fake) = store(scripts(&["set-up-tools"]));
    show(&mut store);

    store.send(A::RunScript("set-up-tools".to_owned()), |_| {});
    assert!(store.state().scripts.rows()[0].running);

    store.receive_matching("run ended", |a| matches!(a, A::ScriptRan(_, _, Ok(()))), |_| {});
    store.receive_matching("rules read again", |a| matches!(a, A::ScriptsLoaded(_, Ok(_))), |_| {});

    let state = store.state();
    let row = &state.scripts.rows()[0];
    assert!(!row.running);
    assert_eq!(row.last.map(|run| run.outcome), Some(ScriptOutcome::Passed));
    assert_eq!(
        fake.calls().iter().filter(|call| matches!(call, Call::RunScript(machine, rule, None) if machine == &id() && rule == "set-up-tools")).count(),
        1
    );
}

#[test]
fn run_now_while_the_rule_runs_sends_nothing_more() {
    let (mut store, fake) = store(scripts(&["set-up-tools"]));
    show(&mut store);

    store.send(A::RunScript("set-up-tools".to_owned()), |_| {});
    store.send(A::RunScript("set-up-tools".to_owned()), |_| {});

    assert_eq!(runs_sent(&fake), 1);
}

#[test]
fn run_now_of_a_rule_the_machine_does_not_have_sends_nothing() {
    let (mut store, fake) = store(scripts(&["set-up-tools"]));
    show(&mut store);

    store.send(A::RunScript("someone-elses-rule".to_owned()), |_| {});

    assert_eq!(runs_sent(&fake), 0);
}

#[test]
fn a_failed_run_keeps_the_end_of_the_failed_steps_output() {
    let (mut store, fake) = store(scripts(&["set-up-tools"]));
    fake.set_script_outcome(
        "set-up-tools",
        ScriptOutcome::Failed,
        vec!["installing".to_owned(), "E: no space left on device".to_owned()],
    );
    show(&mut store);

    store.send(A::RunScript("set-up-tools".to_owned()), |_| {});
    store.receive_matching("run ended", |a| matches!(a, A::ScriptRan(_, _, Err(_))), |_| {});
    store.receive_matching("rules read again", |a| matches!(a, A::ScriptsLoaded(_, Ok(_))), |_| {});

    let state = store.state();
    let row = &state.scripts.rows()[0];
    assert!(!row.running);
    let last = row.last.expect("the failed run is the last run");
    assert_eq!(last.outcome, ScriptOutcome::Failed);
    let (step, tail) = last.failure_tail().expect("a failed step with output");
    assert_eq!(step.index, 0);
    assert_eq!(tail, ["installing".to_owned(), "E: no space left on device".to_owned()]);
    // The run explains itself, so no second message is shown for the row.
    assert!(row.refused.is_none());
}

#[test]
fn a_run_the_host_refuses_says_why_on_its_row() {
    let (mut store, fake) = store(scripts(&["set-up-tools"]));
    fake.fail(
        "run_script",
        DaemonError::Refused { what: "Running set-up-tools".to_owned(), reason: "no".to_owned() },
    );
    show(&mut store);

    store.send(A::RunScript("set-up-tools".to_owned()), |_| {});
    store.receive_matching("run refused", |a| matches!(a, A::ScriptRan(_, _, Err(_))), |_| {});

    let state = store.state();
    let row = &state.scripts.rows()[0];
    assert!(!row.running);
    assert!(row.refused.is_some());
}

#[test]
fn a_rules_file_that_cannot_be_used_shows_its_problem_and_no_rules() {
    let mut broken = scripts(&[]);
    broken.problem = Some("rule 2: the step names a script that isn't there".to_owned());
    let (mut store, _) = store(broken);
    show(&mut store);

    let state = store.state();
    assert!(state.scripts.rows().is_empty());
    assert!(state.scripts.problem().is_some());
    assert!(state.scripts.visible());
}

#[test]
fn a_failed_read_keeps_the_rules_last_read() {
    let (mut store, fake) = store(scripts(&["set-up-tools"]));
    show(&mut store);

    fake.fail("scripts", DaemonError::Refused { what: "Reading scripts".to_owned(), reason: "no".to_owned() });
    store.send(A::ScriptRunChanged(id()), |_| {});
    store.receive_matching("failed read", |a| matches!(a, A::ScriptsLoaded(_, Err(_))), |_| {});

    let state = store.state();
    assert_eq!(state.scripts.rows().len(), 1);
    assert!(state.scripts.error.is_some());
}

#[test]
fn the_feed_reads_the_runs_again_and_updates_the_row() {
    let (mut store, fake) = store(scripts(&["set-up-tools"]));
    show(&mut store);
    assert!(store.state().scripts.rows()[0].last.is_none());

    let mut changed = scripts(&["set-up-tools"]);
    changed.last_runs.push(earlier_run("set-up-tools", ScriptOutcome::Running));
    fake.set_scripts(Ok(changed));
    store.send(A::ScriptRunChanged(id()), |_| {});
    store.receive_matching("read again", |a| matches!(a, A::ScriptsLoaded(_, Ok(_))), |_| {});

    // The host says it is running, though this page sent nothing.
    assert!(store.state().scripts.rows()[0].running);
}

#[test]
fn the_feed_about_another_machine_changes_nothing() {
    let (mut store, fake) = store(scripts(&["set-up-tools"]));
    show(&mut store);
    let before = script_calls(&fake);

    store.send(A::ScriptRunChanged(MachineId::new("other")), |_| {});

    assert_eq!(script_calls(&fake), before);
}

#[test]
fn opening_a_rule_toggles_its_steps() {
    let (mut store, _) = store(scripts(&["set-up-tools"]));
    show(&mut store);

    store.send(A::ToggleScript("set-up-tools".to_owned()), |_| {});
    assert!(store.state().scripts.rows()[0].open);
    store.send(A::ToggleScript("set-up-tools".to_owned()), |_| {});
    assert!(!store.state().scripts.rows()[0].open);
}
