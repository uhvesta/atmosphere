//! A Desk session's agent asking to move its session into a channel
//! (DK-15, DK-19, D305): the target channel's rule decides, not the Desk's.
//! The rule is Ask me or Never, never Allow, never "Always allow" and never
//! approved while the user is away.

mod support;

use std::collections::BTreeSet;

use humata2_access::{
    Agent, Answer, AnswerError, AwayRule, Capability, ChannelId, Decision, DeskAccess, PolicyError,
    Principal, Refusal, RequestId, Rules, SessionId, SessionKind, SessionRef, Target, Verdict,
};
use support::*;

fn desk_agent() -> Principal {
    Principal::Agent(Agent {
        session: SessionRef {
            id: SessionId::new("dk"),
            channel: ChannelId::new("desk"),
            machine: mac(),
        },
        kind: SessionKind::Desk(DeskAccess {
            boards: false,
            channels_and_machines: false,
        }),
    })
}

fn move_into(channel: ChannelId) -> humata2_access::Call {
    use_on(Capability::DeskMoveToChannel, Target::Channel(channel))
}

#[test]
fn a_desk_agents_move_asks_under_the_target_channel_with_no_always_allow() {
    let policy = policy();

    let asked = request(check(&policy, &desk_agent(), &move_into(payments()), &on_mac()));

    assert_eq!(asked.capability, Capability::DeskMoveToChannel);
    assert_eq!(asked.channel, payments());
    assert_eq!(asked.always_let, None);
}

#[test]
fn a_channel_sessions_agent_can_not_ask_to_move() {
    let policy = policy();

    let verdict = check(&policy, &as_agent(a()), &move_into(search()), &on_mac());

    assert!(matches!(verdict, Verdict::Refused(_)), "{verdict:?}");
}

#[test]
fn the_move_rule_is_ask_me_or_never() {
    let mut policy = policy();
    let rules = Rules::Channel(payments());

    assert_eq!(
        policy.set_rule(&user(), &rules, Capability::DeskMoveToChannel, Decision::Allow),
        Err(PolicyError::SettingNotOffered {
            capability: Capability::DeskMoveToChannel,
            decision: Decision::Allow,
        })
    );
    assert!(
        policy
            .set_rule(&user(), &rules, Capability::DeskMoveToChannel, Decision::Never)
            .is_ok()
    );
}

#[test]
fn never_on_the_target_refuses_while_another_channel_still_asks() {
    let mut policy = policy();
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::DeskMoveToChannel,
            Decision::Never,
        )
        .expect("the user sets the rule");

    let into_payments = check(&policy, &desk_agent(), &move_into(payments()), &on_mac());
    let into_search = check(&policy, &desk_agent(), &move_into(search()), &on_mac());

    assert!(
        matches!(
            &into_payments,
            Verdict::Refused(Refusal::NeverRule { channel, capability: Capability::DeskMoveToChannel })
                if channel == &payments()
        ),
        "{into_payments:?}"
    );
    assert_eq!(request(into_search).channel, search());
}

#[test]
fn a_move_asked_before_the_target_turned_never_is_not_approved() {
    let mut policy = policy();
    let asked = request(check(&policy, &desk_agent(), &move_into(payments()), &on_mac()));
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::DeskMoveToChannel,
            Decision::Never,
        )
        .expect("the user sets the rule");

    let approved = policy.approve(&user(), &RequestId::new("r1"), &asked, &Answer::Once, answered_at());

    assert!(
        matches!(
            &approved,
            Err(AnswerError::RuleIsNever { channel, capability: Capability::DeskMoveToChannel })
                if channel == &payments()
        ),
        "{approved:?}"
    );
}

#[test]
fn a_move_is_never_approved_while_the_user_is_away() {
    let mut policy = policy();

    let refused = policy.set_away_rule(
        &user(),
        &Rules::Channel(payments()),
        AwayRule::AllowLowRisk(BTreeSet::from([Capability::DeskMoveToChannel])),
    );

    assert_eq!(
        refused,
        Err(PolicyError::NotLowRisk {
            capability: Capability::DeskMoveToChannel,
        })
    );
}
