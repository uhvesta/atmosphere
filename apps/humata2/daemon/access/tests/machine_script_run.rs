//! Running a machine script now (MACHINESCRIPTS, D688): a rule of its own,
//! Never by default, so an agent can't run the owner's scripts unless its
//! channel's rule allows it.

mod support;

use humata2_access::{
    Call, Capability, Decision, MachineAction, MachineActionKind, Policy, Refusal, Rules, Source, Target, Verdict,
};
use support::*;

fn run_now(target: Target) -> Call {
    use_on(Capability::MachineScriptRun, target)
}

fn set(policy: &mut Policy, decision: Decision) {
    policy
        .set_rule(&user(), &Rules::Channel(payments()), Capability::MachineScriptRun, decision)
        .expect("the rule takes this setting");
}

fn on_the_box() -> Target {
    Target::Machine(MachineAction {
        machine: the_box(),
        action: MachineActionKind::Start,
    })
}

#[test]
fn a_new_channel_never_lets_an_agent_run_a_machine_script() {
    let policy = policy();
    for target in [Target::Global, on_the_box()] {
        let verdict = check(&policy, &as_agent(a()), &run_now(target), &on_mac());
        assert!(
            matches!(
                verdict,
                Verdict::Refused(Refusal::NeverRule { capability: Capability::MachineScriptRun, .. })
            ),
            "{verdict:?}"
        );
    }
}

#[test]
fn a_channel_whose_rule_allows_it_lets_its_agents_run_one() {
    let mut policy = policy();
    set(&mut policy, Decision::Allow);
    for target in [Target::Global, on_the_box()] {
        assert!(
            allowed(Source::Rule).met_by(&check(&policy, &as_agent(a()), &run_now(target), &on_mac()))
        );
    }
}

#[test]
fn a_channel_whose_rule_says_ask_puts_the_run_in_the_inbox() {
    let mut policy = policy();
    set(&mut policy, Decision::Ask);
    let verdict = check(&policy, &as_agent(a()), &run_now(Target::Global), &on_mac());
    assert!(asks(Capability::MachineScriptRun).met_by(&verdict), "{verdict:?}");
}
