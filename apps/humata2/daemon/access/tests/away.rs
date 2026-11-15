//! While the Mac is away: a machine judges calls against its copy of the
//! policy, the channel's While I'm away rule decides what would have asked,
//! and nothing that needs the Mac runs (AW-4, AW-7, AW-8, AW-11).

mod support;

use std::collections::BTreeSet;

use humata2_access::{
    Answer, AnswerAt, AnswerError, AwayRule, BoardAct, BoardRef, Call, Capability, Decision,
    Effect, ErrorKind, Policy, PolicyError, Refusal, RequestId, Rules, Source, Target, Verdict,
};
use support::*;

fn set_away(policy: &mut Policy, away: AwayRule) {
    policy
        .set_away_rule(&user(), &Rules::Channel(payments()), away)
        .expect("the rule is accepted");
}

fn low_risk(kinds: &[Capability]) -> AwayRule {
    AwayRule::AllowLowRisk(kinds.iter().copied().collect::<BTreeSet<_>>())
}

/// Session `c`, on the box, with the Mac away.
fn c_away(policy: &Policy, call: &Call) -> Verdict {
    check(policy, &as_agent(c()), call, &on_box_away())
}

fn asks_on_the_box(verdict: &Verdict) -> bool {
    matches!(
        verdict,
        Verdict::Ask(request) if request.answer_at == AnswerAt::Machine(the_box())
    )
}

#[test]
fn allowed_calls_run_journal_or_read_the_last_copy() {
    let policy = policy();
    let cases = [
        // Its own clone and the channel's shared folder on this machine.
        (read(file_of(&c(), "src/lib.rs")), Source::Owned, Effect::Apply),
        (write(file_of(&c(), "src/lib.rs")), Source::Owned, Effect::Apply),
        (
            read(shared_on(the_box(), payments(), "notes.md")),
            Source::Owned,
            Effect::Apply,
        ),
        // A session on this machine is read here; one on the Mac comes
        // from the last copy.
        (Call::ReadSession(d()), Source::Rule, Effect::Apply),
        (Call::ReadSession(b()), Source::Rule, Effect::LastCopy),
        (
            Call::Board {
                board: BoardRef::Channel(payments()),
                act: BoardAct::Read,
            },
            Source::Rule,
            Effect::LastCopy,
        ),
        // Channel writes go to the journal.
        (add_to_payments_board(), Source::Rule, Effect::Journal),
        (
            use_on(Capability::InboxAsk, Target::Channel(payments())),
            Source::Rule,
            Effect::Journal,
        ),
    ];
    for (call, source, effect) in cases {
        assert_eq!(
            c_away(&policy, &call),
            Verdict::Allowed { source, effect },
            "{call:?}"
        );
    }
}

#[test]
fn what_needs_the_mac_waits_or_under_skip_is_refused() {
    let mut policy = policy();
    // Reading a session's files on the Mac, the rule set to Allow.
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::FilesReadOtherMachine,
            Decision::Allow,
        )
        .expect("Allow is offered");
    let call = read(file_of(&a(), "src/lib.rs"));

    assert_eq!(
        check(&policy, &as_agent(c()), &call, &on_box_reachable()),
        Verdict::Allowed {
            source: Source::Rule,
            effect: Effect::Apply,
        }
    );
    assert_eq!(c_away(&policy, &call), Verdict::WaitForHost);

    set_away(&mut policy, AwayRule::Skip);
    match c_away(&policy, &call) {
        Verdict::Refused(refusal) => assert_eq!(refusal.kind(), ErrorKind::HostUnreachable),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn asking_for_what_needs_the_mac_waits_whatever_the_away_rule() {
    let mut policy = policy();
    // Asks by default, and needs the Mac.
    let call = read(file_of(&a(), "src/lib.rs"));
    assert_eq!(c_away(&policy, &call), Verdict::WaitForHost);

    set_away(&mut policy, AwayRule::Skip);
    assert!(matches!(
        c_away(&policy, &call),
        Verdict::Refused(Refusal::HostUnreachable)
    ));

    // A low-risk kind aimed at a session on the Mac still needs the Mac.
    set_away(&mut policy, low_risk(&[Capability::SessionNote]));
    assert_eq!(c_away(&policy, &note_to(a())), Verdict::WaitForHost);
}

#[test]
fn wait_holds_the_request_for_the_passphrase_on_the_machine() {
    let policy = policy();
    assert!(asks_on_the_box(&c_away(&policy, &note_to(d()))));
    // With the Mac reachable it goes to the Mac's Inbox.
    assert_eq!(
        request(check(&policy, &as_agent(c()), &note_to(d()), &on_box_reachable())).answer_at,
        AnswerAt::Host
    );
}

#[test]
fn skip_refuses_what_would_ask_and_leaves_the_rest() {
    let mut policy = policy();
    set_away(&mut policy, AwayRule::Skip);
    match c_away(&policy, &note_to(d())) {
        Verdict::Refused(refusal) => assert_eq!(refusal.kind(), ErrorKind::UserAway),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(
        c_away(&policy, &write(file_of(&c(), "src/lib.rs"))),
        Verdict::Allowed {
            source: Source::Owned,
            effect: Effect::Apply,
        }
    );
    // The Mac reachable: Skip does nothing.
    assert!(matches!(
        check(&policy, &as_agent(c()), &note_to(d()), &on_box_reachable()),
        Verdict::Ask(_)
    ));
}

#[test]
fn allow_low_risk_runs_only_the_kinds_it_lists() {
    let mut policy = policy();
    set_away(
        &mut policy,
        low_risk(&[Capability::SessionNote, Capability::ChannelBoardEditOthers]),
    );
    assert_eq!(
        c_away(&policy, &note_to(d())),
        Verdict::Allowed {
            source: Source::AwayRule,
            effect: Effect::Apply,
        }
    );
    assert_eq!(
        c_away(
            &policy,
            &Call::Board {
                board: BoardRef::Channel(payments()),
                act: BoardAct::Edit { created_by: None },
            },
        ),
        Verdict::Allowed {
            source: Source::AwayRule,
            effect: Effect::Journal,
        }
    );
    // Not listed: held for the passphrase.
    assert!(asks_on_the_box(&c_away(
        &policy,
        &read(file_of(&d(), "src/lib.rs"))
    )));
    // Only while the Mac is away.
    assert!(matches!(
        check(&policy, &as_agent(c()), &note_to(d()), &on_box_reachable()),
        Verdict::Ask(_)
    ));
    // Another channel keeps its own rule.
    let mut search_box = humata2_access::SessionRef {
        channel: search(),
        ..c()
    };
    search_box.id = humata2_access::SessionId::new("y");
    let y_to_x = humata2_access::SessionRef {
        machine: the_box(),
        ..x()
    };
    assert!(asks_on_the_box(&check(
        &policy,
        &as_agent(search_box),
        &note_to(y_to_x),
        &on_box_away(),
    )));
}

#[test]
fn allow_low_risk_never_approves_an_ask_ahead() {
    let mut policy = policy();
    set_away(&mut policy, low_risk(&[Capability::SessionNote]));
    let verdict = policy
        .check_ask_ahead(&agent(c()), &note_to(d()), &on_box_away())
        .expect("the call can be checked");
    match verdict {
        Verdict::Ask(request) => {
            assert!(request.ahead);
            assert_eq!(request.answer_at, AnswerAt::Machine(the_box()));
        }
        other => panic!("expected an ask-ahead request, got {other:?}"),
    }
}

#[test]
fn never_stays_never_while_away() {
    let mut policy = policy();
    set_away(&mut policy, low_risk(&[Capability::SessionNote]));
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::SessionNote,
            Decision::Never,
        )
        .expect("Never is offered");
    assert!(matches!(
        c_away(&policy, &note_to(d())),
        Verdict::Refused(Refusal::NeverRule { .. })
    ));
}

#[test]
fn some_kinds_can_never_be_low_risk() {
    let mut policy = policy();
    for capability in [
        // Ask only, never allowed.
        Capability::NetworkReach,
        // Typing into a session runs anything.
        Capability::SessionType,
        // Never by default.
        Capability::SessionStopOther,
        // Needs the Mac.
        Capability::FilesReadOtherMachine,
        Capability::GitPush,
        // Not an agent's at all.
        Capability::InboxAnswer,
    ] {
        assert_eq!(
            policy.set_away_rule(
                &user(),
                &Rules::Channel(payments()),
                low_risk(&[Capability::SessionNote, capability]),
            ),
            Err(PolicyError::NotLowRisk { capability }),
            "{capability:?}"
        );
    }
    // Nothing changed.
    assert_eq!(
        policy.channel_rules(&payments()).expect("payments").away(),
        &AwayRule::Wait
    );
    for capability in [
        Capability::SessionNote,
        Capability::ChannelBoardEditOthers,
        Capability::FilesReadOtherSession,
        Capability::SessionStartSameMachine,
    ] {
        assert!(
            policy
                .set_away_rule(&user(), &Rules::Channel(payments()), low_risk(&[capability]))
                .is_ok(),
            "{capability:?}"
        );
    }
}

#[test]
fn new_channels_start_with_the_away_rule_set_for_them() {
    let mut policy = policy();
    policy
        .set_away_rule(&user(), &Rules::NewChannels, AwayRule::Skip)
        .expect("accepted");
    policy
        .add_channel(humata2_access::ChannelId::new("docs"))
        .expect("a new channel");
    assert_eq!(
        policy
            .channel_rules(&humata2_access::ChannelId::new("docs"))
            .expect("docs")
            .away(),
        &AwayRule::Skip
    );
    // Channels made before keep theirs.
    assert_eq!(
        policy.channel_rules(&payments()).expect("payments").away(),
        &AwayRule::Wait
    );
}

#[test]
fn on_a_machine_the_user_answers_but_does_not_change_rules() {
    let mut policy = policy();
    let grant = grant_for(&mut policy, &as_agent(b()), &note_to(a()));
    let rules = Rules::Channel(payments());
    assert_eq!(
        policy.set_rule(&user_on_box(), &rules, Capability::SessionNote, Decision::Allow),
        Err(PolicyError::NeedsHost)
    );
    assert_eq!(
        policy.set_away_rule(&user_on_box(), &rules, AwayRule::Skip),
        Err(PolicyError::NeedsHost)
    );
    assert_eq!(
        policy.revoke_grant(&user_on_box(), grant.id),
        Err(PolicyError::NeedsHost)
    );
    assert!(policy.grant(grant.id).is_some());
}

#[test]
fn a_request_for_the_mac_can_not_be_answered_on_a_machine() {
    let mut policy = policy();
    let asked = request(check(&policy, &as_agent(b()), &note_to(a()), &on_mac()));
    assert_eq!(
        policy.approve(&user_on_box(), &RequestId::new("r1"), &asked, &Answer::Once),
        Err(AnswerError::NeedsHost)
    );
}
