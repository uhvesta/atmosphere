//! A channel Desk session's agent (CD-3, D306): it uses its channel's rules
//! with the channel as its own, and its management switches allow more on
//! top, after Never. With a session as the target a switch covers that
//! session and its forks only, and "Start and fork sessions" covers only
//! forking them. A switch that is off leaves the channel's rule to decide,
//! a Never rule blocks a switch, and no switch answers Inbox requests.

mod support;

use std::collections::BTreeSet;

use humata2_access::{
    Agent, AgentId, BoardAct, BoardRef, Call, Capability, ChannelDeskAccess, ChannelDeskSwitch,
    ChannelDeskSwitches, ChannelId, Decision, PERMISSION_PROMPT_EXIT_CODE,
    PERMISSION_PROMPT_PROTECTED_CODE, Policy, Principal, Rules, SessionId, SessionKind, SessionRef,
    Source, Target, TerminalTypeRefusal, Verdict, check_terminal_type_safety,
    requires_explicit_consent,
};
use support::*;

/// The channel Desk session itself: filed under Payments, on the Mac.
fn desk_session() -> SessionRef {
    SessionRef {
        id: SessionId::new("cd"),
        channel: payments(),
        machine: mac(),
    }
}

/// A fork of `a`, made after the Desk session started.
fn a_fork() -> SessionRef {
    SessionRef {
        id: SessionId::new("a_fork"),
        ..a()
    }
}

fn all_on() -> ChannelDeskSwitches {
    ChannelDeskSwitches {
        read_files: true,
        send_notes: true,
        type_into_terminal: true,
        start_and_fork: true,
        stop_and_archive: true,
        change_board_items: true,
    }
}

fn all_off() -> ChannelDeskSwitches {
    ChannelDeskSwitches {
        read_files: false,
        send_notes: false,
        type_into_terminal: false,
        start_and_fork: false,
        stop_and_archive: false,
        change_board_items: false,
    }
}

/// A channel Desk agent looking after the whole channel.
fn for_channel(switches: ChannelDeskSwitches) -> Principal {
    desk_agent(ChannelDeskAccess {
        target: None,
        covers: None,
        switches,
    })
}

/// A channel Desk agent looking after `a`, whose fork `a_fork` it covers too.
fn for_a(switches: ChannelDeskSwitches) -> Principal {
    desk_agent(ChannelDeskAccess {
        target: Some(a().id),
        covers: Some(BTreeSet::from([a().id, a_fork().id])),
        switches,
    })
}

fn desk_agent(access: ChannelDeskAccess) -> Principal {
    Principal::Agent(Agent {
        session: desk_session(),
        kind: SessionKind::ChannelDesk(access),
    })
}

fn by_switch(verdict: &Verdict) -> bool {
    matches!(
        verdict,
        Verdict::Allowed {
            source: Source::DeskSwitch,
            ..
        }
    )
}

fn asks(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Ask(_))
}

fn refused(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Refused(_))
}

fn type_into(session: SessionRef) -> Call {
    use_on(Capability::SessionType, Target::Session(session))
}

fn stop(session: SessionRef) -> Call {
    use_on(Capability::SessionStopOther, Target::Session(session))
}

fn start(fork_of: Option<SessionId>) -> Call {
    use_on(
        Capability::SessionStartSameMachine,
        Target::Start {
            channel: payments(),
            machine: mac(),
            agent: AgentId::new("claude"),
            fork_of,
        },
    )
}

/// An edit of an item `b`'s agent made on `board`.
fn edit_bs_item(board: BoardRef) -> Call {
    Call::Board {
        board,
        act: BoardAct::Edit {
            created_by: Some(b().id),
        },
    }
}

fn with_rules(settings: &[(Capability, Decision)]) -> Policy {
    let mut policy = policy();
    for &(capability, decision) in settings {
        policy
            .set_rule(&user(), &Rules::Channel(payments()), capability, decision)
            .expect("the user sets the channel's rule");
    }
    policy
}

fn with_rule(capability: Capability, decision: Decision) -> Policy {
    with_rules(&[(capability, decision)])
}

#[test]
fn a_new_desk_session_reads_files_and_starts_sessions_and_asks_for_notes() {
    let defaults = ChannelDeskSwitches::default();
    assert!(defaults.read_files && defaults.start_and_fork);
    assert!(
        !(defaults.send_notes
            || defaults.type_into_terminal
            || defaults.stop_and_archive
            || defaults.change_board_items)
    );
}

#[test]
fn the_default_switches_read_and_start_without_asking_and_ask_for_notes() {
    let policy = with_rule(Capability::SessionNote, Decision::Ask);
    let agent = for_a(ChannelDeskSwitches::default());
    for call in [read(file_of(&a(), "README.md")), start(Some(a().id))] {
        let verdict = check(&policy, &agent, &call, &on_mac());
        assert!(by_switch(&verdict), "{call:?}: {verdict:?}");
    }
    let note_verdict = check(&policy, &agent, &note_to(a()), &on_mac());
    assert!(asks(&note_verdict), "notes require explicit consent by default: {note_verdict:?}");
    let other_machine_start = use_on(
        Capability::SessionStartOtherMachine,
        Target::Start {
            channel: payments(),
            machine: humata2_access::MachineId::new("devbox"),
            agent: AgentId::new("claude"),
            fork_of: None,
        },
    );
    let other_machine_verdict = check(&policy, &for_channel(ChannelDeskSwitches::default()), &other_machine_start, &on_mac());
    assert!(asks(&other_machine_verdict), "other machine follows channel rule: {other_machine_verdict:?}");
}

#[test]
fn a_session_target_covers_its_forks() {
    let policy = policy();
    let agent = for_a(ChannelDeskSwitches::default());
    let verdict = check(&policy, &agent, &read(file_of(&a_fork(), "README.md")), &on_mac());
    assert!(by_switch(&verdict), "{verdict:?}");
    let with_notes = for_a(ChannelDeskSwitches {
        send_notes: true,
        ..ChannelDeskSwitches::default()
    });
    let verdict = check(&policy, &with_notes, &note_to(a_fork()), &on_mac());
    assert!(by_switch(&verdict), "{verdict:?}");
}

#[test]
fn a_session_outside_the_target_is_left_to_the_channel_rule() {
    let policy = with_rules(&[
        (Capability::FilesReadOtherSession, Decision::Ask),
        (Capability::SessionNote, Decision::Ask),
        (Capability::SessionType, Decision::Ask),
        (Capability::ChannelBoardEditOthers, Decision::Ask),
        (Capability::SessionStopOther, Decision::Ask),
    ]);
    let agent = for_a(all_on());
    for call in [
        read(file_of(&b(), "README.md")),
        note_to(b()),
        type_into(b()),
        edit_bs_item(BoardRef::Session(b())),
        stop(b()),
    ] {
        let verdict = check(&policy, &agent, &call, &on_mac());
        assert!(asks(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn the_channel_as_target_covers_every_session_of_the_channel() {
    let policy = with_rule(Capability::SessionNote, Decision::Ask);
    let agent = for_channel(ChannelDeskSwitches::default());
    for session in [a(), b()] {
        let verdict = check(&policy, &agent, &read(file_of(&session, "README.md")), &on_mac());
        assert!(by_switch(&verdict), "{session:?}: {verdict:?}");
        let note_verdict = check(&policy, &agent, &note_to(session.clone()), &on_mac());
        assert!(asks(&note_verdict), "{session:?}: {note_verdict:?}");
    }
    let with_notes = for_channel(ChannelDeskSwitches {
        send_notes: true,
        ..ChannelDeskSwitches::default()
    });
    for session in [a(), b()] {
        let verdict = check(&policy, &with_notes, &note_to(session.clone()), &on_mac());
        assert!(by_switch(&verdict), "{session:?}: {verdict:?}");
    }
}

#[test]
fn another_channel_is_never_covered() {
    let policy = policy();
    let agent = for_channel(all_on());
    for call in [read(file_of(&x(), "README.md")), note_to(x())] {
        let verdict = check(&policy, &agent, &call, &on_mac());
        assert!(!by_switch(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn a_channel_desk_session_is_not_a_session_of_the_channel() {
    // Another channel Desk session is filed under the Desk's channel id,
    // not Payments, so "any session of the channel" never reaches it.
    let policy = policy();
    let agent = for_channel(all_on());
    let other_desk = SessionRef {
        id: SessionId::new("cd_other"),
        channel: ChannelId::new("desk"),
        machine: mac(),
    };
    let verdict = check(&policy, &agent, &note_to(other_desk), &on_mac());
    assert!(!by_switch(&verdict), "{verdict:?}");
}

#[test]
fn a_switch_that_is_off_leaves_it_to_the_channel_rule() {
    let policy = with_rules(&[
        (Capability::FilesReadOtherSession, Decision::Ask),
        (Capability::SessionNote, Decision::Ask),
        (Capability::SessionType, Decision::Ask),
    ]);
    let agent = for_a(all_off());
    for call in [read(file_of(&a(), "README.md")), note_to(a()), type_into(a())] {
        let verdict = check(&policy, &agent, &call, &on_mac());
        assert!(asks(&verdict), "{call:?}: {verdict:?}");
    }
    let policy = with_rule(Capability::FilesReadOtherSession, Decision::Allow);
    let verdict = check(&policy, &agent, &read(file_of(&a(), "README.md")), &on_mac());
    assert!(
        matches!(
            verdict,
            Verdict::Allowed {
                source: Source::Rule,
                ..
            }
        ),
        "{verdict:?}"
    );
}

#[test]
fn never_wins_over_a_switch_that_is_on() {
    let policy = with_rule(Capability::SessionNote, Decision::Never);
    let agent = for_a(all_on());
    let verdict = check(&policy, &agent, &note_to(a()), &on_mac());
    assert!(refused(&verdict), "{verdict:?}");
}

#[test]
fn a_never_rule_blocks_its_switch_whether_on_or_off() {
    let open = policy();
    let rules = open.channel_rules(&payments()).expect("Payments has rules");
    assert!(
        !ChannelDeskSwitch::blocked_by(rules).contains(&ChannelDeskSwitch::SendNotes),
        "nothing blocks notes under the defaults"
    );
    let closed = with_rule(Capability::SessionNote, Decision::Never);
    let rules = closed.channel_rules(&payments()).expect("Payments has rules");
    let blocked = ChannelDeskSwitch::blocked_by(rules);
    assert!(blocked.contains(&ChannelDeskSwitch::SendNotes), "{blocked:?}");
    assert!(!blocked.contains(&ChannelDeskSwitch::ReadFiles), "{blocked:?}");
}

#[test]
fn one_capability_set_to_never_blocks_a_switch_with_two() {
    let policy = with_rule(Capability::FilesReadOtherMachine, Decision::Never);
    let rules = policy.channel_rules(&payments()).expect("Payments has rules");
    assert!(ChannelDeskSwitch::blocked_by(rules).contains(&ChannelDeskSwitch::ReadFiles));
}

#[test]
fn typing_into_the_target_needs_its_switch() {
    let policy = policy();
    let verdict = check(&policy, &for_a(ChannelDeskSwitches::default()), &type_into(a()), &on_mac());
    assert!(asks(&verdict), "{verdict:?}");
    let on = ChannelDeskSwitches {
        type_into_terminal: true,
        ..ChannelDeskSwitches::default()
    };
    let verdict = check(&policy, &for_a(on), &type_into(a()), &on_mac());
    assert!(by_switch(&verdict), "{verdict:?}");
}

#[test]
fn the_stop_switch_stops_the_target_once_the_rule_is_not_never() {
    // Set explicitly, so this holds whatever the row's default is.
    let policy = with_rule(Capability::SessionStopOther, Decision::Ask);
    let on = ChannelDeskSwitches {
        stop_and_archive: true,
        ..ChannelDeskSwitches::default()
    };
    let verdict = check(&policy, &for_a(on), &stop(a()), &on_mac());
    assert!(by_switch(&verdict), "{verdict:?}");
    let verdict = check(&policy, &for_a(ChannelDeskSwitches::default()), &stop(a()), &on_mac());
    assert!(asks(&verdict), "{verdict:?}");
}

#[test]
fn with_a_session_target_the_start_switch_only_forks_it_and_its_forks() {
    let policy = with_rule(Capability::SessionStartSameMachine, Decision::Ask);
    let agent = for_a(ChannelDeskSwitches {
        start_and_fork: true,
        ..ChannelDeskSwitches::default()
    });
    for parent in [a().id, a_fork().id] {
        let verdict = check(&policy, &agent, &start(Some(parent.clone())), &on_mac());
        assert!(by_switch(&verdict), "fork of {parent:?}: {verdict:?}");
    }
    for call in [start(None), start(Some(b().id))] {
        let verdict = check(&policy, &agent, &call, &on_mac());
        assert!(asks(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn with_the_channel_as_target_the_start_switch_starts_and_forks_any() {
    let policy = policy();
    let agent = for_channel(ChannelDeskSwitches {
        start_and_fork: true,
        ..ChannelDeskSwitches::default()
    });
    for call in [start(None), start(Some(b().id))] {
        let verdict = check(&policy, &agent, &call, &on_mac());
        assert!(by_switch(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn the_board_switch_changes_others_items_where_it_covers() {
    let policy = with_rule(Capability::ChannelBoardEditOthers, Decision::Ask);
    let on = ChannelDeskSwitches {
        change_board_items: true,
        ..ChannelDeskSwitches::default()
    };
    let channel_board = edit_bs_item(BoardRef::Channel(payments()));
    let verdict = check(&policy, &for_channel(on), &channel_board, &on_mac());
    assert!(by_switch(&verdict), "{verdict:?}");
    let verdict = check(&policy, &for_a(on), &edit_bs_item(BoardRef::Session(a())), &on_mac());
    assert!(by_switch(&verdict), "{verdict:?}");
    // A session target doesn't reach the channel's own board.
    let verdict = check(&policy, &for_a(on), &channel_board, &on_mac());
    assert!(asks(&verdict), "{verdict:?}");
    let verdict = check(&policy, &for_channel(ChannelDeskSwitches::default()), &channel_board, &on_mac());
    assert!(asks(&verdict), "{verdict:?}");
}

#[test]
fn no_switch_answers_inbox_requests() {
    let policy = policy();
    let call = use_on(Capability::InboxAnswer, Target::Channel(payments()));
    let verdict = check(&policy, &for_channel(all_on()), &call, &on_mac());
    assert!(refused(&verdict), "{verdict:?}");
}

#[test]
fn typing_into_a_session_with_a_permission_prompt_is_hard_refused() {
    assert_eq!(PERMISSION_PROMPT_PROTECTED_CODE, "permission_prompt_protected");
    assert_eq!(PERMISSION_PROMPT_EXIT_CODE, 4);
    let refusal = check_terminal_type_safety(true, "Restart doctor").unwrap_err();
    assert_eq!(
        refusal,
        TerminalTypeRefusal::PermissionPromptProtected {
            target_title: "Restart doctor".to_owned(),
        }
    );
    assert!(refusal.to_string().contains("exit 4"));
    assert_eq!(check_terminal_type_safety(false, "Restart doctor"), Ok(()));
    let access = ChannelDeskAccess {
        target: None,
        covers: None,
        switches: ChannelDeskSwitches::default(),
    };
    assert!(requires_explicit_consent(&access, Capability::SessionNote));
    assert!(requires_explicit_consent(&access, Capability::SessionType));
    assert!(!requires_explicit_consent(&access, Capability::InboxAnswer));
}
