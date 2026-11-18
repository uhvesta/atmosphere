//! An agent asking to import a machine provider (MP-23, D792): a rule of its
//! own that starts at Ask me, offers only Ask me and Never, never offers
//! "Always allow …", and no Desk switch lets it through.

mod support;

use humata2_access::{
    Agent, Call, Capability, ChannelId, Decision, DeskAccess, PolicyError, Principal, Refusal, Rules, SessionId,
    SessionKind, SessionRef, Target, Verdict,
};
use support::*;

fn import() -> Call {
    use_on(Capability::ProviderImport, Target::Global)
}

#[test]
fn a_new_channel_asks_the_user_before_an_agent_imports_a_provider() {
    let policy = policy();

    let asked = request(check(&policy, &as_agent(a()), &import(), &on_mac()));

    assert_eq!(asked.capability, Capability::ProviderImport);
    assert_eq!(asked.always_let, None, "no Always allow for a provider import");
}

#[test]
fn the_import_rule_is_ask_me_or_never() {
    let mut policy = policy();
    let rules = Rules::Channel(payments());

    assert_eq!(
        policy.set_rule(&user(), &rules, Capability::ProviderImport, Decision::Allow),
        Err(PolicyError::SettingNotOffered {
            capability: Capability::ProviderImport,
            decision: Decision::Allow,
        })
    );
    assert!(policy.set_rule(&user(), &rules, Capability::ProviderImport, Decision::Never).is_ok());
}

#[test]
fn never_refuses_an_agents_import() {
    let mut policy = policy();
    policy
        .set_rule(&user(), &Rules::Channel(payments()), Capability::ProviderImport, Decision::Never)
        .expect("Never is offered");

    let verdict = check(&policy, &as_agent(a()), &import(), &on_mac());

    assert!(
        matches!(verdict, Verdict::Refused(Refusal::NeverRule { capability: Capability::ProviderImport, .. })),
        "{verdict:?}"
    );
}

#[test]
fn a_desk_agent_with_every_switch_on_still_asks() {
    let policy = policy();
    let desk = Principal::Agent(Agent {
        session: SessionRef {
            id: SessionId::new("dk"),
            channel: ChannelId::new("desk"),
            machine: mac(),
        },
        kind: SessionKind::Desk(DeskAccess {
            boards: true,
            channels_and_machines: true,
        }),
    });

    let asked = request(check(&policy, &desk, &import(), &on_mac()));

    assert_eq!(asked.capability, Capability::ProviderImport);
}

#[test]
fn an_import_is_never_low_risk_and_always_needs_the_mac() {
    assert!(!Capability::ProviderImport.may_be_low_risk());
    assert!(Capability::ProviderImport.needs_host());
    assert!(!Capability::ProviderImport.offers_always_let());
}
