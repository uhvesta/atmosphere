//! A Desk session's agent starts or forks sessions in any channel, on this
//! machine or another, when the owner approves (DK-10, D910): its
//! new-channel rule decides, Ask me by default, Never still wins, and no
//! Desk switch allows it. Every other session kind stays in its channel.

mod support;

use humata2_access::{
    AgentId, Answer, Call, Capability, ChannelId, Decision, DeskAccess, GrantTarget, MachineId,
    Principal, Refusal, RequestId, Rules, SessionId, SessionKind, SessionRef, Target, Verdict,
};
use support::*;

fn desk_agent(boards: bool, channels_and_machines: bool) -> humata2_access::Agent {
    humata2_access::Agent {
        session: SessionRef {
            id: SessionId::new("dk"),
            channel: ChannelId::new("desk"),
            machine: mac(),
        },
        kind: SessionKind::Desk(DeskAccess {
            boards,
            channels_and_machines,
        }),
    }
}

fn desk(boards: bool, channels_and_machines: bool) -> Principal {
    Principal::Agent(desk_agent(boards, channels_and_machines))
}

fn capability_for(machine: &MachineId) -> Capability {
    if machine == &mac() {
        Capability::SessionStartSameMachine
    } else {
        Capability::SessionStartOtherMachine
    }
}

fn start_in(channel: ChannelId, machine: MachineId) -> Call {
    use_on(
        capability_for(&machine),
        Target::Start {
            channel,
            machine,
            agent: AgentId::new("claude"),
            fork_of: None,
        },
    )
}

fn fork_in(channel: ChannelId, parent: &SessionRef) -> Call {
    use_on(
        Capability::SessionStartSameMachine,
        Target::Start {
            channel,
            machine: mac(),
            agent: AgentId::new("claude"),
            fork_of: Some(parent.id.clone()),
        },
    )
}

fn every_start() -> Vec<Call> {
    let mut calls = Vec::new();
    for channel in [payments(), search()] {
        for machine in [mac(), the_box()] {
            calls.push(start_in(channel.clone(), machine));
        }
    }
    calls
}

fn set_new_channel_rule(
    policy: &mut humata2_access::Policy,
    capability: Capability,
    decision: Decision,
) {
    policy
        .set_rule(&user(), &Rules::NewChannels, capability, decision)
        .expect("the user sets a default");
}

#[test]
fn a_desk_start_in_any_channel_asks_the_owner() {
    let policy = policy();
    for call in every_start() {
        let asked = request(check(&policy, &desk(false, false), &call, &on_mac()));
        let Call::Use {
            capability,
            target: Target::Start { machine, .. },
        } = &call
        else {
            unreachable!("every_start makes starts");
        };
        assert_eq!(asked.capability, *capability, "{call:?}");
        // The request carries the call, so the card names the channel and
        // machine; it is filed under the Desk channel id.
        assert_eq!(asked.call, call);
        assert_eq!(asked.channel, ChannelId::new("desk"));
        assert_eq!(
            asked.always_let,
            Some(GrantTarget::AgentOnMachine {
                machine: machine.clone(),
                agent: AgentId::new("claude"),
            }),
            "{call:?}"
        );
    }
}

#[test]
fn the_desk_switches_do_not_allow_a_start_without_asking() {
    let policy = policy();
    for call in every_start() {
        let verdict = check(&policy, &desk(true, true), &call, &on_mac());
        assert!(matches!(verdict, Verdict::Ask(_)), "{call:?}: {verdict:?}");
    }
}

#[test]
fn a_desk_start_is_allowed_when_the_rule_is_allow() {
    let mut policy = policy();
    set_new_channel_rule(&mut policy, Capability::SessionStartSameMachine, Decision::Allow);
    set_new_channel_rule(&mut policy, Capability::SessionStartOtherMachine, Decision::Allow);
    for call in every_start() {
        let verdict = check(&policy, &desk(false, false), &call, &on_mac());
        assert!(matches!(verdict, Verdict::Allowed { .. }), "{call:?}: {verdict:?}");
    }
}

#[test]
fn a_desk_start_is_refused_when_the_rule_is_never_even_with_switches_on() {
    let mut policy = policy();
    set_new_channel_rule(&mut policy, Capability::SessionStartSameMachine, Decision::Never);
    set_new_channel_rule(&mut policy, Capability::SessionStartOtherMachine, Decision::Never);
    for call in every_start() {
        let verdict = check(&policy, &desk(true, true), &call, &on_mac());
        assert!(
            matches!(verdict, Verdict::Refused(Refusal::NeverRule { .. })),
            "{call:?}: {verdict:?}"
        );
    }
}

#[test]
fn approving_a_desk_start_with_always_allow_lets_the_retry_through() {
    let mut policy = policy();
    let agent = desk(false, false);
    let call = start_in(search(), the_box());
    grant_for(&mut policy, &agent, &call);
    let retried = check(&policy, &agent, &call, &on_mac());
    assert!(matches!(retried, Verdict::Allowed { .. }), "{retried:?}");
}

#[test]
fn approving_a_desk_start_once_makes_no_standing_grant() {
    let mut policy = policy();
    let agent = desk(false, false);
    let call = start_in(payments(), mac());
    let asked = request(check(&policy, &agent, &call, &on_mac()));
    let approval = policy
        .approve(&user(), &RequestId::new("r1"), &asked, &Answer::Once, answered_at())
        .expect("the user approves once");
    assert!(approval.grant.is_none());
    let again = check(&policy, &agent, &call, &on_mac());
    assert!(matches!(again, Verdict::Ask(_)), "{again:?}");
}

#[test]
fn a_desk_agent_asks_ahead_for_a_start() {
    let policy = policy();
    let verdict = policy
        .check_ask_ahead(&desk_agent(false, false), &start_in(payments(), the_box()), &on_mac())
        .expect("the call can be checked");
    assert!(matches!(&verdict, Verdict::Ask(request) if request.ahead), "{verdict:?}");
}

#[test]
fn a_desk_fork_of_a_channel_session_asks_the_owner() {
    let policy = policy();
    let call = fork_in(payments(), &a());
    let asked = request(check(&policy, &desk(false, false), &call, &on_mac()));
    assert_eq!(asked.capability, Capability::SessionStartSameMachine);
}

#[test]
fn a_desk_fork_is_refused_when_its_global_read_rule_is_never() {
    // The fork carries the parent's conversation, a read of another
    // channel; the start rule alone would let it through.
    let mut policy = policy();
    set_new_channel_rule(&mut policy, Capability::SessionStartSameMachine, Decision::Allow);
    set_new_channel_rule(&mut policy, Capability::GlobalRead, Decision::Never);
    let fork = check(&policy, &desk(false, false), &fork_in(payments(), &a()), &on_mac());
    assert!(
        matches!(fork, Verdict::Refused(Refusal::NeverRule { .. })),
        "{fork:?}"
    );
    let ahead = policy
        .check_ask_ahead(&desk_agent(false, false), &fork_in(payments(), &a()), &on_mac())
        .expect("the call can be checked");
    assert!(matches!(ahead, Verdict::Refused(Refusal::NeverRule { .. })), "{ahead:?}");
    // A fresh start reads nothing, so it is still allowed.
    let start = check(&policy, &desk(false, false), &start_in(payments(), mac()), &on_mac());
    assert!(matches!(start, Verdict::Allowed { .. }), "{start:?}");
}

#[test]
fn other_session_kinds_are_still_refused_a_start_in_another_channel() {
    let policy = policy();
    for kind in [
        SessionKind::Ordinary,
        SessionKind::ContextOnlyFork {
            parent: SessionId::new("b"),
        },
        SessionKind::Review { push_allowed: false },
    ] {
        let agent = Principal::Agent(humata2_access::Agent {
            session: a(),
            kind: kind.clone(),
        });
        for machine in [mac(), the_box()] {
            let verdict = check(&policy, &agent, &start_in(search(), machine), &on_mac());
            assert!(
                matches!(verdict, Verdict::Refused(Refusal::OtherChannel { .. })),
                "{kind:?}: {verdict:?}"
            );
        }
        let fork = check(&policy, &agent, &fork_in(search(), &x()), &on_mac());
        assert!(
            matches!(fork, Verdict::Refused(Refusal::OtherChannel { .. })),
            "{kind:?}: {fork:?}"
        );
    }
}
