//! A Desk session's agent (DK-4, DK-10, D180, D182): it belongs to no
//! channel, takes the new-channel defaults, and its two switches allow the
//! two board rows and the channel and machine rows on top of them, after
//! Never. Two Desk sessions are not channel-mates, and a grant to one
//! survives storing the policy.

mod support;

use humata2_access::{
    Agent, BoardAct, BoardRef, Call, Capability, ChannelId, Decision, DeskAccess, MachineAction,
    MachineActionKind, Policy, Principal, Refusal, Rules, SessionId, SessionKind, SessionRef, Target, Verdict,
};
use support::*;

fn desk_session(id: &str) -> SessionRef {
    SessionRef {
        id: SessionId::new(id),
        channel: ChannelId::new("desk"),
        machine: mac(),
    }
}

fn desk_agent(boards: bool, channels_and_machines: bool) -> Principal {
    Principal::Agent(Agent {
        session: desk_session("dk"),
        kind: SessionKind::Desk(DeskAccess {
            boards,
            channels_and_machines,
        }),
    })
}

fn on(board: BoardRef, act: BoardAct) -> Call {
    Call::Board { board, act }
}

fn board_calls() -> Vec<Call> {
    let mut calls = Vec::new();
    for board in [
        BoardRef::Global,
        BoardRef::Channel(payments()),
        BoardRef::Session(a()),
    ] {
        for act in [
            BoardAct::Read,
            BoardAct::Add,
            BoardAct::Edit { created_by: None },
        ] {
            calls.push(on(board.clone(), act));
        }
    }
    calls
}

fn create_channel() -> Call {
    use_on(Capability::ChannelCreate, Target::Global)
}

fn add_machine() -> Call {
    use_on(
        Capability::MachineProvision,
        Target::Machine(MachineAction {
            machine: the_box(),
            action: MachineActionKind::Add,
        }),
    )
}

fn is_allowed(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Allowed { .. })
}

#[test]
fn the_board_switch_on_reads_and_changes_every_board_without_asking() {
    let policy = policy();
    let agent = desk_agent(true, false);
    for call in board_calls() {
        let verdict = check(&policy, &agent, &call, &on_mac());
        assert!(is_allowed(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn the_board_switch_off_asks_before_changing_any_board() {
    let policy = policy();
    let agent = desk_agent(false, false);
    for board in [
        BoardRef::Global,
        BoardRef::Channel(payments()),
        BoardRef::Session(a()),
    ] {
        let call = on(board, BoardAct::Add);
        assert_eq!(
            request(check(&policy, &agent, &call, &on_mac())).capability,
            Capability::GlobalBoardWrite,
            "{call:?}"
        );
    }
}

#[test]
fn the_second_switch_on_makes_channels_and_adds_machines_without_asking() {
    let policy = policy();
    let agent = desk_agent(false, true);
    for call in [create_channel(), add_machine()] {
        let verdict = check(&policy, &agent, &call, &on_mac());
        assert!(is_allowed(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn the_second_switch_off_asks_for_each() {
    let policy = policy();
    let agent = desk_agent(true, false);
    assert_eq!(
        request(check(&policy, &agent, &create_channel(), &on_mac())).capability,
        Capability::ChannelCreate
    );
    assert_eq!(
        request(check(&policy, &agent, &add_machine(), &on_mac())).capability,
        Capability::MachineProvision
    );
}

#[test]
fn other_rules_follow_the_new_channel_defaults() {
    // With its switches off, creating a channel is decided by the
    // new-channel rule alone: Allow lets it through, Never refuses it.
    let mut policy = policy();
    let agent = desk_agent(false, false);
    for (decision, allowed) in [(Decision::Allow, true), (Decision::Never, false)] {
        policy
            .set_rule(&user(), &Rules::NewChannels, Capability::ChannelCreate, decision)
            .expect("the user sets a default");
        let verdict = check(&policy, &agent, &create_channel(), &on_mac());
        assert_eq!(
            matches!(verdict, Verdict::Allowed { .. }),
            allowed,
            "{decision:?}: {verdict:?}"
        );
        if !allowed {
            assert!(matches!(verdict, Verdict::Refused(_)), "{verdict:?}");
        }
    }
}

#[test]
fn a_desk_agent_needs_no_channel_in_the_policy() {
    // "desk" is never added as a channel, and the check still decides.
    let policy = policy();
    let verdict = policy.check(&desk_agent(false, false), &create_channel(), &on_mac());
    assert!(verdict.is_ok(), "{verdict:?}");
}

#[test]
fn no_channel_is_its_own() {
    let policy = policy();
    let agent = desk_agent(false, false);
    // Its folder is its own; a channel's shared/ is not.
    let own = check(&policy, &agent, &write(file_of_desk("notes.md")), &on_mac());
    assert!(is_allowed(&own), "{own:?}");
    let shared = check(
        &policy,
        &agent,
        &write(shared_on(mac(), payments(), "notes.md")),
        &on_mac(),
    );
    assert!(!is_allowed(&shared), "{shared:?}");
}

fn file_of_desk(file: &str) -> humata2_access::Place {
    file_of(&desk_session("dk"), file)
}

#[test]
fn never_wins_over_a_switch_that_is_on() {
    let mut policy = policy();
    policy
        .set_rule(
            &user(),
            &Rules::NewChannels,
            Capability::GlobalBoardWrite,
            Decision::Never,
        )
        .expect("the user sets a default");
    let agent = desk_agent(true, true);
    let verdict = check(&policy, &agent, &on(BoardRef::Global, BoardAct::Add), &on_mac());
    assert!(matches!(verdict, Verdict::Refused(_)), "{verdict:?}");
}

#[test]
fn two_desk_sessions_are_not_channel_mates() {
    let policy = policy();
    let agent = desk_agent(true, true);
    let other = file_of(&desk_session("dk_other"), "notes.md");
    let read = check(&policy, &agent, &read(other.clone()), &on_mac());
    assert!(
        matches!(read, Verdict::Refused(Refusal::OtherChannel { .. })),
        "{read:?}"
    );
    let written = check(&policy, &agent, &write(other), &on_mac());
    assert!(!is_allowed(&written), "{written:?}");
}

#[test]
fn a_grant_to_a_desk_agent_survives_storing_the_policy() {
    let mut policy = policy();
    let agent = desk_agent(false, false);
    let call = on(BoardRef::Global, BoardAct::Add);
    grant_for(&mut policy, &agent, &call);
    let reloaded = Policy::from_state(policy.to_state()).expect("the stored policy loads");
    let verdict = check(&reloaded, &agent, &call, &on_mac());
    assert!(is_allowed(&verdict), "{verdict:?}");
}
