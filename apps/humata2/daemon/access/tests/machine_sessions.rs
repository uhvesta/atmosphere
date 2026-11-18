//! Stopping and starting all of a machine's sessions (STOP, D263): a rule
//! of its own, Never by default, so an agent can never stop or start a
//! machine unless its channel's rule allows it; the user always may, on
//! the machine itself even while the Mac is away.

mod support;

use humata2_access::{
    AwayRule, Call, Capability, Decision, Effect, GrantTarget, MachineAction, MachineActionKind,
    MachineId, Policy, PolicyError, PresenceMethod, Principal, Refusal, Rules, ScriptToken, Source, Target,
    Verdict,
};
use support::*;

fn sessions(machine: MachineId, action: MachineActionKind) -> Call {
    let capability = action.capability();
    use_on(capability, Target::Machine(MachineAction { machine, action }))
}

fn stop_on(machine: MachineId) -> Call {
    sessions(machine, MachineActionKind::StopSessions)
}

fn start_on(machine: MachineId) -> Call {
    sessions(machine, MachineActionKind::StartSessions)
}

fn set(policy: &mut Policy, capability: Capability, decision: Decision) {
    policy
        .set_rule(&user(), &Rules::Channel(payments()), capability, decision)
        .expect("the rule takes this setting");
}

fn never_rule(verdict: &Verdict) -> bool {
    matches!(
        verdict,
        Verdict::Refused(Refusal::NeverRule { capability: Capability::MachineSessionsStop, .. })
    )
}

#[test]
fn stopping_and_starting_sessions_need_their_own_rule_not_the_provider_ones() {
    for action in [MachineActionKind::StopSessions, MachineActionKind::StartSessions] {
        assert_eq!(action.capability(), Capability::MachineSessionsStop, "{action:?}");
    }
    // A provider's power Stop and Start stay what they were.
    assert_eq!(MachineActionKind::Stop.capability(), Capability::MachineProvision);
    assert_eq!(MachineActionKind::Start.capability(), Capability::MachineProvision);
    assert!(Capability::ALL.contains(&Capability::MachineSessionsStop));
}

#[test]
fn a_new_channel_never_lets_an_agent_stop_or_start_a_machine() {
    let policy = policy();
    assert_eq!(
        policy
            .channel_rules(&payments())
            .and_then(|rules| rules.decision(Capability::MachineSessionsStop)),
        Some(Decision::Never),
        "a channel's rule starts at Never"
    );
    for call in [stop_on(mac()), start_on(mac()), stop_on(the_box())] {
        let verdict = check(&policy, &as_agent(a()), &call, &on_mac());
        assert!(never_rule(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn an_agent_whose_rule_allows_it_stops_and_starts_machines() {
    let mut policy = policy();
    set(&mut policy, Capability::MachineSessionsStop, Decision::Allow);
    for call in [stop_on(mac()), start_on(mac()), stop_on(the_box())] {
        assert!(
            allowed(Source::Rule).met_by(&check(&policy, &as_agent(a()), &call, &on_mac())),
            "{call:?}"
        );
    }
}

#[test]
fn an_agent_whose_rule_says_ask_puts_the_stop_in_the_inbox_offering_that_machine_only() {
    let mut policy = policy();
    set(&mut policy, Capability::MachineSessionsStop, Decision::Ask);
    let asked = request(check(&policy, &as_agent(a()), &stop_on(the_box()), &on_mac()));
    assert_eq!(asked.capability, Capability::MachineSessionsStop);
    assert_eq!(
        asked.always_let,
        Some(GrantTarget::MachineAction(MachineAction {
            machine: the_box(),
            action: MachineActionKind::StopSessions,
        }))
    );
}

#[test]
fn always_allowing_one_stop_lets_that_agent_stop_that_machine_and_nothing_more() {
    let mut policy = policy();
    set(&mut policy, Capability::MachineSessionsStop, Decision::Ask);
    let agent = as_agent(a());
    grant_for(&mut policy, &agent, &stop_on(the_box()));

    assert!(matches!(
        check(&policy, &agent, &stop_on(the_box()), &on_mac()),
        Verdict::Allowed { source: Source::Grant(_), .. }
    ));
    // Not Start, and not another machine: each is asked again.
    for call in [start_on(the_box()), stop_on(mac())] {
        assert!(matches!(check(&policy, &agent, &call, &on_mac()), Verdict::Ask(_)), "{call:?}");
    }
}

#[test]
fn never_wins_over_a_grant_made_before_it() {
    let mut policy = policy();
    set(&mut policy, Capability::MachineSessionsStop, Decision::Ask);
    let agent = as_agent(a());
    grant_for(&mut policy, &agent, &stop_on(the_box()));
    set(&mut policy, Capability::MachineSessionsStop, Decision::Never);
    assert!(never_rule(&check(&policy, &agent, &stop_on(the_box()), &on_mac())));
}

#[test]
fn allowing_provider_actions_does_not_let_an_agent_stop_sessions() {
    let mut policy = policy();
    set(&mut policy, Capability::MachineProvision, Decision::Allow);
    set(&mut policy, Capability::MachineDestroy, Decision::Allow);
    let verdict = check(&policy, &as_agent(a()), &stop_on(mac()), &on_mac());
    assert!(never_rule(&verdict), "{verdict:?}");
}

#[test]
fn one_channels_allow_does_not_reach_an_agent_in_another_channel() {
    let mut policy = policy();
    set(&mut policy, Capability::MachineSessionsStop, Decision::Allow);
    // x is in Search, whose rule is still Never.
    let verdict = check(&policy, &as_agent(x()), &stop_on(mac()), &on_mac());
    assert!(matches!(verdict, Verdict::Refused(Refusal::NeverRule { .. })), "{verdict:?}");
}

#[test]
fn a_script_token_stops_sessions_only_when_it_lists_the_capability() {
    let policy = policy();
    let lacking = Principal::Script(ScriptToken {
        capabilities: [Capability::ChannelBoardAdd].into_iter().collect(),
        channels: [payments()].into_iter().collect(),
    });
    for call in [stop_on(mac()), start_on(mac())] {
        let verdict = check(&policy, &lacking, &call, &on_mac());
        assert!(
            matches!(
                verdict,
                Verdict::Refused(Refusal::ScriptTokenLacks {
                    capability: Capability::MachineSessionsStop
                })
            ),
            "{call:?}: {verdict:?}"
        );
    }
}

#[test]
fn the_user_stops_and_starts_sessions_in_the_app_or_logged_in() {
    let policy = policy();
    for principal in [
        Principal::App,
        Principal::Cli(PresenceMethod::CliLogin),
        Principal::Cli(PresenceMethod::ApprovalPassphrase),
    ] {
        for call in [stop_on(mac()), start_on(mac()), stop_on(the_box())] {
            assert!(
                allowed(Source::User).met_by(&check(&policy, &principal, &call, &on_mac())),
                "{principal:?} {call:?}"
            );
        }
    }
}

#[test]
fn the_user_on_a_machine_stops_and_starts_its_sessions_while_the_mac_is_away() {
    let policy = policy();
    for principal in [
        Principal::Cli(PresenceMethod::CliLogin),
        Principal::Cli(PresenceMethod::ApprovalPassphrase),
    ] {
        for call in [stop_on(the_box()), start_on(the_box())] {
            let verdict = check(&policy, &principal, &call, &on_box_away());
            assert!(
                matches!(verdict, Verdict::Allowed { source: Source::User, effect: Effect::Apply }),
                "{principal:?} {call:?}: {verdict:?}"
            );
        }
    }
}

#[test]
fn a_machine_stopping_another_machine_while_the_mac_is_away_waits_for_the_mac() {
    let policy = policy();
    let login = Principal::Cli(PresenceMethod::CliLogin);
    // From the box, the Mac's sessions can only be reached through the Mac.
    let verdict = check(&policy, &login, &stop_on(mac()), &on_box_away());
    assert_eq!(verdict, Verdict::WaitForHost);
}

#[test]
fn stopping_sessions_is_never_a_low_risk_kind_while_the_user_is_away() {
    let mut policy = policy();
    let refused = policy
        .set_away_rule(
            &user(),
            &Rules::Channel(payments()),
            AwayRule::AllowLowRisk([Capability::MachineSessionsStop].into_iter().collect()),
        )
        .expect_err("stopping sessions can't be listed as low-risk");
    assert!(
        matches!(refused, PolicyError::NotLowRisk { capability: Capability::MachineSessionsStop }),
        "{refused:?}"
    );
}

#[test]
fn an_agent_on_a_machine_whose_rule_allows_it_stops_that_machine_while_the_mac_is_away() {
    let mut policy = policy();
    set(&mut policy, Capability::MachineSessionsStop, Decision::Allow);
    // c is on the box.
    let verdict = check(&policy, &as_agent(c()), &stop_on(the_box()), &on_box_away());
    assert!(
        matches!(verdict, Verdict::Allowed { source: Source::Rule, effect: Effect::Apply }),
        "{verdict:?}"
    );
}
