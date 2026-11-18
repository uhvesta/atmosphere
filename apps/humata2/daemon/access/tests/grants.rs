//! Grants: scoped to what was asked, owned by one session, removable, and
//! made only by the user approving a request.

mod support;

use humata2_access::{
    AgentId, Answer, AnswerError, BoardAct, BoardRef, Call, Capability, Decision, GrantTarget,
    PathScope, Policy, Principal, RequestId, Root, Rules, SessionKind, Source, Target, Verdict,
};
use support::*;

fn granted(verdict: &Verdict) -> bool {
    matches!(
        verdict,
        Verdict::Allowed {
            source: Source::Grant(_),
            ..
        }
    )
}

fn asks_again(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Ask(_))
}

#[test]
fn always_let_covers_the_folder_asked_for_and_nothing_beside_it() {
    let mut policy = policy();
    let b_agent = as_agent(b());
    let grant = grant_for(&mut policy, &b_agent, &read(file_of(&a(), "src/lib/mod.rs")));

    // A file read is granted on the folder holding it.
    assert_eq!(
        grant.target,
        GrantTarget::Path(PathScope {
            machine: mac(),
            root: Root::Clone(a()),
            prefix: path("src/lib"),
        })
    );
    assert_eq!(grant.requester, b().id);

    let check_b = |call: Call| check(&policy, &b_agent, &call, &on_mac());
    assert!(granted(&check_b(read(file_of(&a(), "src/lib/mod.rs")))));
    assert!(granted(&check_b(read(file_of(&a(), "src/lib/deep/x.rs")))));
    assert!(granted(&check_b(read(folder_of(&a(), "src/lib")))));
    // Beside it, above it, in another session: asked again.
    assert!(asks_again(&check_b(read(file_of(&a(), "src/library.rs")))));
    assert!(asks_again(&check_b(read(file_of(&a(), "src/main.rs")))));
    assert!(asks_again(&check_b(read(folder_of(&a(), "src")))));
    assert!(asks_again(&check_b(read(file_of(&c(), "src/lib/mod.rs")))));
    // A read grant is not a copy grant.
    assert!(asks_again(&check_b(Call::CopyFiles {
        from: file_of(&a(), "src/lib/mod.rs"),
        into: file_of(&b(), "vendor/mod.rs"),
    })));
}

#[test]
fn a_grant_belongs_to_one_session_and_forks_get_none() {
    let mut policy = policy();
    let call = note_to(a());
    grant_for(&mut policy, &as_agent(b()), &call);

    assert!(granted(&check(&policy, &as_agent(b()), &call, &on_mac())));
    // Another session in the channel asking the same.
    let other = humata2_access::SessionRef {
        id: humata2_access::SessionId::new("b2"),
        ..b()
    };
    assert!(asks_again(&check(&policy, &as_agent(other.clone()), &call, &on_mac())));
    // A context-only fork of b is a new session.
    let fork = Principal::Agent(humata2_access::Agent {
        session: other,
        kind: SessionKind::ContextOnlyFork { parent: b().id },
    });
    assert!(asks_again(&check(&policy, &fork, &call, &on_mac())));
}

#[test]
fn a_grant_can_not_be_wider_than_the_offer() {
    let mut policy = policy();
    let asked = request(check(
        &policy,
        &as_agent(b()),
        &read(file_of(&a(), "src/lib/mod.rs")),
        &on_mac(),
    ));
    let wider = [
        // The whole clone.
        GrantTarget::Path(PathScope {
            machine: mac(),
            root: Root::Clone(a()),
            prefix: humata2_access::RepoPath::root(),
        }),
        // Another session.
        GrantTarget::Path(PathScope {
            machine: mac(),
            root: Root::Clone(x()),
            prefix: path("src/lib"),
        }),
        // Another kind of target.
        GrantTarget::Session(a().id),
    ];
    for target in wider {
        assert_eq!(
            policy.approve(
                &user(),
                &RequestId::new("r1"),
                &asked,
                &Answer::AlwaysLet(target),
                answered_at(),
            ),
            Err(AnswerError::TargetNotOffered)
        );
    }
    assert_eq!(policy.grants().count(), 0);
}

#[test]
fn approving_once_makes_no_grant() {
    let mut policy = policy();
    let call = note_to(a());
    let asked = request(check(&policy, &as_agent(b()), &call, &on_mac()));
    let approval = policy
        .approve(&user(), &RequestId::new("r1"), &asked, &Answer::Once, answered_at())
        .expect("the user approves");
    assert_eq!(approval.grant, None);
    assert!(asks_again(&check(&policy, &as_agent(b()), &call, &on_mac())));
}

#[test]
fn a_grant_records_who_asked_which_request_where_and_when() {
    let mut policy = policy();
    let asked = request(check(&policy, &as_agent(b()), &note_to(a()), &on_mac()));
    let offered = asked.always_let.clone().expect("offered");
    let grant = policy
        .approve(
            &user(),
            &RequestId::new("r7"),
            &asked,
            &Answer::AlwaysLet(offered.clone()),
            answered_at(),
        )
        .expect("the user approves")
        .grant
        .expect("Always allow makes a grant");

    assert_eq!(grant.requester, b().id);
    assert_eq!(grant.channel, payments());
    assert_eq!(grant.capability, Capability::SessionNote);
    assert_eq!(grant.target, offered);
    assert_eq!(grant.granted_from, RequestId::new("r7"));
    assert_eq!(grant.granted_on, mac());
    assert_eq!(grant.granted_at, answered_at());
    // The app needs no presence method.
    assert_eq!(grant.granted_by, None);
    // It is listed, and found by its id.
    assert_eq!(policy.grants().cloned().collect::<Vec<_>>(), vec![grant.clone()]);
    assert_eq!(policy.grant(grant.id), Some(&grant));
}

#[test]
fn nothing_offered_means_no_always_let() {
    let mut policy = policy();
    // IN-22 names no target for moving a session.
    let asked = request(check(
        &policy,
        &as_agent(a()),
        &use_on(Capability::SessionMove, Target::Session(a())),
        &on_mac(),
    ));
    assert_eq!(asked.always_let, None);
    assert_eq!(
        policy.approve(
            &user(),
            &RequestId::new("r1"),
            &asked,
            &Answer::AlwaysLet(GrantTarget::Session(a().id)),
            answered_at(),
        ),
        Err(AnswerError::NoStandingApproval)
    );
}

#[test]
fn revoking_a_grant_takes_effect_on_the_next_call() {
    let mut policy = policy();
    let call = note_to(a());
    let grant = grant_for(&mut policy, &as_agent(b()), &call);
    assert!(granted(&check(&policy, &as_agent(b()), &call, &on_mac())));

    let revoked = policy.revoke_grant(&user(), grant.id).expect("the grant exists");
    assert_eq!(revoked.id, grant.id);
    assert!(asks_again(&check(&policy, &as_agent(b()), &call, &on_mac())));
    assert!(matches!(
        policy.revoke_grant(&user(), grant.id),
        Err(humata2_access::PolicyError::UnknownGrant { .. })
    ));
}

#[test]
fn a_never_rule_suspends_grants_until_it_changes_back() {
    let mut policy = policy();
    let call = note_to(a());
    let grant = grant_for(&mut policy, &as_agent(b()), &call);
    let rules = Rules::Channel(payments());

    policy
        .set_rule(&user(), &rules, Capability::SessionNote, Decision::Never)
        .expect("Never is offered");
    assert!(matches!(
        check(&policy, &as_agent(b()), &call, &on_mac()),
        Verdict::Refused(humata2_access::Refusal::NeverRule { .. })
    ));
    let listed = policy.grant(grant.id).expect("still listed").clone();
    assert!(policy.is_suspended(&listed));

    policy
        .set_rule(&user(), &rules, Capability::SessionNote, Decision::Ask)
        .expect("Ask is offered");
    assert!(!policy.is_suspended(&listed));
    assert!(granted(&check(&policy, &as_agent(b()), &call, &on_mac())));
}

#[test]
fn grants_end_with_their_session() {
    let mut policy = policy();
    let call = note_to(a());
    grant_for(&mut policy, &as_agent(b()), &call);
    let ended = policy.end_grants_of(&b().id);
    assert_eq!(ended.len(), 1);
    assert!(asks_again(&check(&policy, &as_agent(b()), &call, &on_mac())));
}

#[test]
fn a_moved_session_keeps_its_grants() {
    let mut policy = policy();
    let call = note_to(a());
    grant_for(&mut policy, &as_agent(b()), &call);
    // b moves to the box: same session, other machine.
    let moved = humata2_access::SessionRef {
        machine: the_box(),
        ..b()
    };
    assert!(granted(&check(&policy, &as_agent(moved), &call, &on_mac())));
}

#[test]
fn approving_checks_the_rule_as_it_is_now() {
    let mut policy = policy();
    let call = note_to(a());
    let asked = request(check(&policy, &as_agent(b()), &call, &on_mac()));
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::SessionNote,
            Decision::Never,
        )
        .expect("Never is offered");
    let offered = asked.always_let.clone().expect("offered");
    assert!(matches!(
        policy.approve(
            &user(),
            &RequestId::new("r1"),
            &asked,
            &Answer::AlwaysLet(offered),
            answered_at(),
        ),
        Err(AnswerError::RuleIsNever { .. })
    ));
    assert!(matches!(
        policy.approve(&user(), &RequestId::new("r1"), &asked, &Answer::Once, answered_at()),
        Err(AnswerError::RuleIsNever { .. })
    ));
    assert_eq!(policy.grants().count(), 0);
}

#[test]
fn asking_ahead_grants_what_always_let_would() {
    let mut policy = policy();
    let start = use_on(
        Capability::SessionStartOtherMachine,
        Target::Start {
            channel: payments(),
            machine: the_box(),
            agent: AgentId::new("claude"),
            fork_of: None,
        },
    );
    let a_agent = humata2_access::Agent {
        session: a(),
        kind: SessionKind::Ordinary,
    };
    let ahead = match policy
        .check_ask_ahead(&a_agent, &start, &on_mac())
        .expect("the call can be checked")
    {
        Verdict::Ask(request) => request,
        other => panic!("expected an ask-ahead request, got {other:?}"),
    };
    assert!(ahead.ahead);
    let approval = policy
        .approve(&user(), &RequestId::new("r1"), &ahead, &Answer::Once, answered_at())
        .expect("the user approves");
    let grant = approval.grant.expect("an approved ask-ahead makes a grant");
    assert_eq!(
        grant.target,
        GrantTarget::AgentOnMachine {
            machine: the_box(),
            agent: AgentId::new("claude"),
        }
    );
    assert!(granted(&check(&policy, &as_agent(a()), &start, &on_mac())));
    // Another agent on that machine is not covered.
    let other_agent = use_on(
        Capability::SessionStartOtherMachine,
        Target::Start {
            channel: payments(),
            machine: the_box(),
            agent: AgentId::new("codex"),
            fork_of: None,
        },
    );
    assert!(asks_again(&check(&policy, &as_agent(a()), &other_agent, &on_mac())));
}

#[test]
fn asking_ahead_for_something_with_no_always_let_is_refused() {
    let policy = policy();
    let a_agent = humata2_access::Agent {
        session: a(),
        kind: SessionKind::Ordinary,
    };
    let verdict = policy
        .check_ask_ahead(
            &a_agent,
            &use_on(Capability::SessionMove, Target::Session(a())),
            &on_mac(),
        )
        .expect("the call can be checked");
    assert!(matches!(
        verdict,
        Verdict::Refused(humata2_access::Refusal::NothingToAskAhead { .. })
    ));
}

/// Every call an agent can make, including the ones it is refused.
fn agent_calls() -> Vec<Call> {
    let mut calls = vec![
        read(file_of(&a(), "src/lib/mod.rs")),
        read(file_of(&a(), "src/main.rs")),
        read(folder_of(&a(), "")),
        write(file_of(&a(), "src/lib/mod.rs")),
        Call::CopyFiles {
            from: file_of(&a(), "src/lib/mod.rs"),
            into: file_of(&b(), "vendor/mod.rs"),
        },
        note_to(a()),
        note_to(x()),
        Call::Board {
            board: BoardRef::Channel(payments()),
            act: BoardAct::Edit { created_by: None },
        },
        Call::Board {
            board: BoardRef::Global,
            act: BoardAct::Add,
        },
        use_on(Capability::SessionType, Target::Session(a())),
        use_on(Capability::InboxAsk, Target::Channel(payments())),
    ];
    for capability in Capability::ALL {
        for target in [
            Target::Global,
            Target::Channel(payments()),
            Target::Session(a()),
        ] {
            calls.push(use_on(capability, target));
        }
    }
    calls
}

#[test]
fn no_sequence_of_agent_calls_widens_a_grant() {
    let mut policy = policy();
    let b_agent = as_agent(b());
    let b_self = humata2_access::Agent {
        session: b(),
        kind: SessionKind::Ordinary,
    };
    grant_for(&mut policy, &b_agent, &read(file_of(&a(), "src/lib/mod.rs")));
    let before = policy.to_state();
    let calls = agent_calls();

    // A fixed pseudo-random walk through the calls, checked as calls and
    // as ask-aheads, on the Mac and on a machine with the Mac away.
    let mut seed: u64 = 0x5eed;
    for _ in 0..2_000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let call = &calls[(seed >> 33) as usize % calls.len()];
        let context = if seed & 1 == 0 { on_mac() } else { on_box_away() };
        // Unchecked calls are fine here: only the policy's state matters.
        let _ = policy.check(&b_agent, call, &context);
        let _ = policy.check_ask_ahead(&b_self, call, &context);
    }

    assert_eq!(policy.to_state(), before);
    // Still exactly the folder that was granted.
    assert!(granted(&check(&policy, &b_agent, &read(file_of(&a(), "src/lib/x.rs")), &on_mac())));
    assert!(asks_again(&check(&policy, &b_agent, &read(file_of(&a(), "src/main.rs")), &on_mac())));
}

#[test]
fn only_a_present_user_can_change_the_policy() {
    // Neither an agent nor a script nor a peer has a PresentUser to pass.
    for principal in [
        as_agent(a()),
        Principal::Script(humata2_access::ScriptToken::default()),
        Principal::Peer(humata2_access::PeerLink::HostToDaemon),
    ] {
        assert_eq!(principal.present_user(&on_mac()), None, "{principal:?}");
    }
}

#[test]
fn a_grant_made_on_a_machine_applies_there_at_once_and_replays_to_the_mac() {
    // The box's copy of the policy, the Mac away.
    let mut on_box = policy();
    let call = note_to(d());
    let asked = request(check(&on_box, &as_agent(c()), &call, &on_box_away()));
    assert_eq!(asked.answer_at, humata2_access::AnswerAt::Machine(the_box()));
    let offered = asked.always_let.clone().expect("offered");

    // Not without the passphrase.
    let app_like = Principal::Cli(humata2_access::PresenceMethod::CliLogin)
        .present_user(&on_box_away())
        .expect("present");
    assert_eq!(
        on_box.approve(
            &app_like,
            &RequestId::new("r1"),
            &asked,
            &Answer::AlwaysLet(offered.clone()),
            answered_at(),
        ),
        Err(AnswerError::PassphraseNeeded)
    );

    let grant = on_box
        .approve(
            &user_on_box(),
            &RequestId::new("r1"),
            &asked,
            &Answer::AlwaysLet(offered),
            answered_at(),
        )
        .expect("the passphrase answers on the machine")
        .grant
        .expect("a grant");
    assert!(!grant.on_host);
    assert!(granted(&check(&on_box, &as_agent(c()), &call, &on_box_away())));

    // At replay the Mac takes it in as the same grant.
    let mut mac_policy = policy();
    let replayed = mac_policy.replay_grant(&grant).expect("a machine grant replays");
    assert_eq!(replayed.target, grant.target);
    assert_eq!(replayed.requester, grant.requester);
    assert!(granted(&check(&mac_policy, &as_agent(c()), &call, &on_mac())));

    // A grant made on the Mac is not replayable: replay can't mint grants.
    let mut other = policy();
    let mac_grant = grant_for(&mut other, &as_agent(b()), &note_to(a()));
    assert!(matches!(
        mac_policy.replay_grant(&mac_grant),
        Err(humata2_access::PolicyError::NotAMachineGrant { .. })
    ));
}

#[test]
fn stored_policy_comes_back_the_same_and_bad_state_is_refused() {
    let mut policy = policy();
    grant_for(&mut policy, &as_agent(b()), &note_to(a()));
    let state = policy.to_state();
    assert_eq!(Policy::from_state(state.clone()).expect("valid"), policy);

    let mut bad = state.clone();
    bad.grants[0].target = GrantTarget::RepoBranch(humata2_access::RepoBranch {
        repo: humata2_access::RepoRef::new("pay"),
        branch: "main".to_owned(),
    });
    assert!(matches!(
        Policy::from_state(bad),
        Err(humata2_access::PolicyError::GrantDoesNotFit { .. })
    ));

    let mut bad = state.clone();
    bad.channels
        .get_mut(&payments())
        .expect("payments")
        .decisions
        .push((Capability::NetworkReach, Decision::Allow));
    assert!(matches!(
        Policy::from_state(bad),
        Err(humata2_access::PolicyError::SettingNotOffered { .. })
    ));

    let mut bad = state;
    bad.grants.push(bad.grants[0].clone());
    assert!(matches!(
        Policy::from_state(bad),
        Err(humata2_access::PolicyError::DuplicateGrant { .. })
    ));
}
