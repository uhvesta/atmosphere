//! Stopping or archiving another session asks by default, and deleting
//! another session is its own rule, Never by default (D306).

mod support;

use humata2_access::{
    AwayRule, Capability, Decision, Effect, PolicyError, Refusal, Rules, Source, Target, Verdict,
};
use support::*;

fn set(policy: &mut humata2_access::Policy, capability: Capability, decision: Decision) {
    policy
        .set_rule(&user(), &Rules::Channel(payments()), capability, decision)
        .expect("the rule takes this setting");
}

fn delete(session: humata2_access::SessionRef) -> humata2_access::Call {
    use_on(Capability::SessionDeleteOther, Target::Session(session))
}

fn stop(session: humata2_access::SessionRef) -> humata2_access::Call {
    use_on(Capability::SessionStopOther, Target::Session(session))
}

#[test]
fn stopping_asks_by_default_and_deleting_is_never() {
    assert_eq!(Capability::SessionStopOther.default_decision(), Some(Decision::Ask));
    assert_eq!(Capability::SessionDeleteOther.default_decision(), Some(Decision::Never));
    for capability in [Capability::SessionStopOther, Capability::SessionDeleteOther] {
        assert!(Capability::ALL.contains(&capability), "{capability:?}");
        assert!(capability.has_rule(), "{capability:?}");
        assert!(capability.settings().contains(&Decision::Allow), "{capability:?}");
    }
}

#[test]
fn an_agent_stopping_or_archiving_another_session_asks() {
    let verdict = check(&policy(), &as_agent(a()), &stop(b()), &on_mac());
    assert!(asks(Capability::SessionStopOther).met_by(&verdict), "{verdict:?}");
}

#[test]
fn an_agent_deleting_another_session_is_refused_by_the_never_rule() {
    let verdict = check(&policy(), &as_agent(a()), &delete(b()), &on_mac());
    assert!(
        matches!(
            verdict,
            Verdict::Refused(Refusal::NeverRule { capability: Capability::SessionDeleteOther, .. })
        ),
        "{verdict:?}"
    );
}

#[test]
fn an_agent_ending_its_own_session_goes_by_its_own_rule_whichever_it_names() {
    for call in [stop(a()), delete(a())] {
        let verdict = check(&policy(), &as_agent(a()), &call, &on_mac());
        assert!(asks(Capability::OwnSessionEnd).met_by(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn the_two_rules_are_set_separately() {
    let mut policy = policy();
    set(&mut policy, Capability::SessionDeleteOther, Decision::Ask);
    let verdict = check(&policy, &as_agent(a()), &delete(b()), &on_mac());
    assert!(asks(Capability::SessionDeleteOther).met_by(&verdict), "{verdict:?}");

    set(&mut policy, Capability::SessionStopOther, Decision::Never);
    let verdict = check(&policy, &as_agent(a()), &stop(b()), &on_mac());
    assert!(
        matches!(
            verdict,
            Verdict::Refused(Refusal::NeverRule { capability: Capability::SessionStopOther, .. })
        ),
        "{verdict:?}"
    );
    // Stopping being Never leaves the delete rule where the user put it.
    let verdict = check(&policy, &as_agent(a()), &delete(b()), &on_mac());
    assert!(asks(Capability::SessionDeleteOther).met_by(&verdict), "{verdict:?}");
}

#[test]
fn neither_can_be_listed_as_low_risk_while_the_mac_is_away() {
    let mut policy = policy();
    for capability in [Capability::SessionStopOther, Capability::SessionDeleteOther] {
        assert_eq!(
            policy.set_away_rule(
                &user(),
                &Rules::Channel(payments()),
                AwayRule::AllowLowRisk([capability].into_iter().collect()),
            ),
            Err(PolicyError::NotLowRisk { capability }),
            "{capability:?}"
        );
    }
}

#[test]
fn an_allow_rule_stops_another_session_without_asking() {
    let mut policy = policy();
    set(&mut policy, Capability::SessionStopOther, Decision::Allow);
    let verdict = check(&policy, &as_agent(a()), &stop(b()), &on_mac());
    assert!(
        matches!(verdict, Verdict::Allowed { source: Source::Rule, effect: Effect::Apply }),
        "{verdict:?}"
    );
}
