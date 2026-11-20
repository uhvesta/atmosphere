//! Deleting many sessions at once is its own rule, Never by default, and it
//! needs the Mac. An agent never deletes its own session through it: its own
//! end is `OwnSessionEnd`.

mod support;

use humata2_access::{Capability, Decision, Refusal, Rules, Target, Verdict};
use support::*;

fn set(policy: &mut humata2_access::Policy, capability: Capability, decision: Decision) {
    policy
        .set_rule(&user(), &Rules::Channel(payments()), capability, decision)
        .expect("the rule takes this setting");
}

fn delete_many(session: humata2_access::SessionRef) -> humata2_access::Call {
    use_on(Capability::SessionsDeleteMany, Target::Session(session))
}

#[test]
fn deleting_many_sessions_is_never_by_default_and_needs_the_mac() {
    assert!(Capability::ALL.contains(&Capability::SessionsDeleteMany));
    assert_eq!(Capability::SessionsDeleteMany.default_decision(), Some(Decision::Never));
    assert!(Capability::SessionsDeleteMany.has_rule());
    assert!(Capability::SessionsDeleteMany.needs_host());
}

#[test]
fn an_agent_deleting_another_session_in_a_batch_is_refused_by_the_never_rule() {
    let verdict = check(&policy(), &as_agent(a()), &delete_many(b()), &on_mac());
    assert!(
        matches!(
            verdict,
            Verdict::Refused(Refusal::NeverRule { capability: Capability::SessionsDeleteMany, .. })
        ),
        "{verdict:?}"
    );
}

#[test]
fn an_agent_never_deletes_its_own_session_through_the_batch_even_when_allowed() {
    let mut policy = policy();
    set(&mut policy, Capability::SessionsDeleteMany, Decision::Allow);
    let verdict = check(&policy, &as_agent(a()), &delete_many(a()), &on_mac());
    assert!(matches!(verdict, Verdict::Refused(Refusal::UserOnly { .. })), "{verdict:?}");
}

#[test]
fn the_user_can_set_a_batch_delete_of_other_sessions_to_ask() {
    let mut policy = policy();
    set(&mut policy, Capability::SessionsDeleteMany, Decision::Ask);
    let verdict = check(&policy, &as_agent(a()), &delete_many(b()), &on_mac());
    assert!(asks(Capability::SessionsDeleteMany).met_by(&verdict), "{verdict:?}");
}
