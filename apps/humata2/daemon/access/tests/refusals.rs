//! Refusals carry the kind of reason and the fix, and the policy refuses
//! what it can't judge.

mod support;

use std::collections::BTreeSet;

use humata2_access::{
    Agent, BoardAct, BoardRef, Call, Capability, ChannelId, CheckError, Decision, DeniedSource,
    Entry, ErrorKind, Fix, GrantTarget, NetworkTarget, Place, Policy, PolicyError, PresenceMethod,
    Principal, Refusal, RepoBranch, RepoRef, Root, Rules, ScriptToken, SessionId, SessionKind,
    SessionRef, Source, Target, Tier, Verdict,
};
use support::*;

/// The refusal in a verdict; fails the test for any other verdict.
fn refusal_of(verdict: Verdict) -> Refusal {
    match verdict {
        Verdict::Refused(refusal) => refusal,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn refused(principal: &Principal, call: &Call) -> Refusal {
    refusal_of(check(&policy(), principal, call, &on_mac()))
}

fn main_clone(file: &str) -> Place {
    Place {
        machine: mac(),
        root: Root::MainClone {
            channel: payments(),
            repo: RepoRef::new("pay"),
        },
        path: path(file),
        entry: Entry::File,
    }
}

fn outside() -> Place {
    Place {
        machine: mac(),
        root: Root::Outside,
        path: path("etc/passwd"),
        entry: Entry::File,
    }
}

fn fork_of_a() -> Principal {
    Principal::Agent(Agent {
        session: SessionRef {
            id: SessionId::new("f"),
            ..a()
        },
        kind: SessionKind::ContextOnlyFork { parent: a().id },
    })
}

fn push() -> Call {
    use_on(
        Capability::GitPush,
        Target::Push(RepoBranch {
            repo: RepoRef::new("pay"),
            branch: "fix-refunds".to_owned(),
        }),
    )
}

fn review(push_allowed: bool) -> Principal {
    Principal::Agent(Agent {
        session: a(),
        kind: SessionKind::Review { push_allowed },
    })
}

#[test]
fn a_fork_reads_its_parent_but_does_not_write_it() {
    let parent_file = file_of(&a(), "src/lib.rs");
    assert!(allowed(Source::Owned).met_by(&check(
        &policy(),
        &fork_of_a(),
        &read(parent_file.clone()),
        &on_mac(),
    )));
    let refusal = refused(&fork_of_a(), &write(parent_file));
    assert_eq!(refusal.kind(), ErrorKind::ParentReadOnly);
    assert_eq!(refusal.fix(), Fix::UseOwnFolders);
}

#[test]
fn main_clones_are_read_only_for_everyone() {
    for principal in [as_agent(a()), Principal::App] {
        let refusal = refused(&principal, &write(main_clone("src/lib.rs")));
        assert_eq!(
            refusal.kind(),
            ErrorKind::MainCloneReadOnly,
            "{principal:?}"
        );
        assert_eq!(refusal.fix(), Fix::UseOwnFolders);
    }
    // Copying into one is a write to it.
    let refusal = refused(
        &Principal::App,
        &Call::CopyFiles {
            from: file_of(&a(), "src/lib.rs"),
            into: main_clone("src/lib.rs"),
        },
    );
    assert_eq!(refusal.kind(), ErrorKind::MainCloneReadOnly);
    // Reading one is a local fetch, allowed by default.
    assert!(allowed(Source::Rule).met_by(&check(
        &policy(),
        &as_agent(a()),
        &read(main_clone("src/lib.rs")),
        &on_mac(),
    )));
}

#[test]
fn nothing_outside_humatas_folders_for_anyone() {
    for principal in [
        as_agent(a()),
        Principal::App,
        Principal::Cli(PresenceMethod::CliLogin),
    ] {
        let refusal = refused(&principal, &read(outside()));
        assert_eq!(
            refusal.kind(),
            ErrorKind::OutsideHumataFolders,
            "{principal:?}"
        );
        assert_eq!(refusal.fix(), Fix::UseOwnFolders);
    }
}

#[test]
fn an_agent_writes_only_its_own_folders() {
    let channel_folder = Place {
        root: Root::ChannelFolder(payments()),
        ..file_of(&a(), "notes.md")
    };
    for place in [
        file_of(&b(), "src/lib.rs"),
        channel_folder,
        shared_on(the_box(), payments(), "notes.md"),
    ] {
        let refusal = refused(&as_agent(a()), &write(place.clone()));
        assert_eq!(
            refusal.kind(),
            ErrorKind::Denied(DeniedSource::HardLimit),
            "{place:?}"
        );
        assert_eq!(refusal.fix(), Fix::UseOwnFolders);
    }
    // Its own shared folder is allowed by default.
    assert!(allowed(Source::Rule).met_by(&check(
        &policy(),
        &as_agent(a()),
        &write(shared_on(mac(), payments(), "notes.md")),
        &on_mac(),
    )));
}

#[test]
fn a_review_session_pushes_only_when_the_user_lets_it() {
    let refusal = refused(&review(false), &push());
    assert_eq!(refusal.kind(), ErrorKind::Denied(DeniedSource::HardLimit));
    assert_eq!(refusal.fix(), Fix::UserAllowsReviewPush { session: a().id });
    assert!(allowed(Source::ReviewPushSetting).met_by(&check(
        &policy(),
        &review(true),
        &push(),
        &on_mac(),
    )));
    // An ordinary session asks.
    assert!(asks(Capability::GitPush).met_by(&check(
        &policy(),
        &as_agent(a()),
        &push(),
        &on_mac()
    )));
}

#[test]
fn user_only_and_peer_only_capabilities_are_not_an_agents() {
    let refusal = refused(
        &as_agent(a()),
        &use_on(Capability::AccessChange, Target::Channel(payments())),
    );
    assert_eq!(refusal.kind(), ErrorKind::UserOnly);
    assert_eq!(refusal.fix(), Fix::AskTheUser);

    let refusal = refused(
        &as_agent(a()),
        &use_on(Capability::PeerReport, Target::Global),
    );
    assert_eq!(refusal.kind(), ErrorKind::Denied(DeniedSource::PeerLink));
    assert_eq!(refusal.fix(), Fix::Nothing);
}

#[test]
fn a_never_rule_names_the_rule_to_change() {
    // Never by default.
    let refusal = refused(
        &as_agent(a()),
        &use_on(Capability::SessionStopOther, Target::Session(b())),
    );
    assert_eq!(refusal.kind(), ErrorKind::Denied(DeniedSource::NeverRule));
    assert_eq!(
        refusal.fix(),
        Fix::UserChangesRule {
            channel: payments(),
            capability: Capability::SessionStopOther,
        }
    );
}

#[test]
fn another_channel_is_read_with_global_read_and_otherwise_refused() {
    let agent = as_agent(a());
    let mut policy = policy();
    let reads = [
        Call::ReadSession(x()),
        Call::Board {
            board: BoardRef::Channel(search()),
            act: BoardAct::Read,
        },
        use_on(Capability::ChannelRead, Target::Channel(search())),
    ];
    // Reads are allowed by default (D127.4).
    for call in &reads {
        assert!(
            allowed(Source::Rule).met_by(&check(&policy, &agent, call, &on_mac())),
            "{call:?}"
        );
    }
    // The user sets Ask me: the same reads ask.
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::GlobalRead,
            Decision::Ask,
        )
        .expect("a rule the capability offers");
    for call in &reads {
        assert!(
            asks(Capability::GlobalRead).met_by(&check(&policy, &agent, call, &on_mac())),
            "{call:?}"
        );
    }
    for call in [
        note_to(x()),
        read(file_of(&x(), "src/lib.rs")),
        write(shared_on(mac(), search(), "notes.md")),
        use_on(Capability::InboxAsk, Target::Channel(search())),
    ] {
        let refusal = refusal_of(check(&policy, &agent, &call, &on_mac()));
        assert_eq!(
            refusal.kind(),
            ErrorKind::Denied(DeniedSource::OtherChannel),
            "{call:?}"
        );
        assert_eq!(refusal.fix(), Fix::AskTheUser);
    }
}

#[test]
fn board_items_an_agent_made_are_its_to_edit() {
    let policy = policy();
    let edit = |created_by: SessionId| Call::Board {
        board: BoardRef::Channel(payments()),
        act: BoardAct::Edit {
            created_by: Some(created_by),
        },
    };
    assert!(allowed(Source::Rule).met_by(&check(
        &policy,
        &as_agent(a()),
        &edit(a().id),
        &on_mac()
    )));
    let asked = request(check(&policy, &as_agent(a()), &edit(b().id), &on_mac()));
    assert_eq!(asked.capability, Capability::ChannelBoardEditOthers);
    assert_eq!(
        asked.always_let,
        Some(GrantTarget::Board(BoardRef::Channel(payments())))
    );
}

#[test]
fn answering_on_a_machine_with_the_mac_away_asks_for_the_passphrase() {
    let refusal = refusal_of(check(
        &policy(),
        &Principal::Cli(PresenceMethod::CliLogin),
        &use_on(Capability::InboxAnswer, Target::Channel(payments())),
        &on_box_away(),
    ));
    assert_eq!(refusal.kind(), ErrorKind::PresenceRequired);
    assert_eq!(
        refusal.fix(),
        Fix::ShowPresence {
            accepted: vec![PresenceMethod::ApprovalPassphrase],
        }
    );
}

#[test]
fn script_refusals_say_what_token_would_do() {
    let token = Principal::Script(ScriptToken {
        capabilities: [Capability::ChannelBoardAdd, Capability::NetworkReach]
            .into_iter()
            .collect(),
        channels: [payments()].into_iter().collect::<BTreeSet<_>>(),
    });
    let policy = policy();
    let cases = [
        (
            note_to(a()),
            Fix::TokenWithCapability {
                capability: Capability::SessionNote,
            },
        ),
        (
            Call::Board {
                board: BoardRef::Channel(search()),
                act: BoardAct::Add,
            },
            Fix::TokenForChannel { channel: search() },
        ),
        (read(file_of(&a(), "src/lib.rs")), Fix::NotFromAScript),
        // Listed, but a token never waits for an answer.
        (
            use_on(
                Capability::NetworkReach,
                Target::Network(NetworkTarget {
                    machine: mac(),
                    host: "localhost".to_owned(),
                    port: 5432,
                }),
            ),
            Fix::NotFromAScript,
        ),
    ];
    for (call, fix) in cases {
        let refusal = refusal_of(check(&policy, &token, &call, &on_mac()));
        assert_eq!(
            refusal.kind(),
            ErrorKind::Denied(DeniedSource::ScriptToken),
            "{call:?}"
        );
        assert_eq!(refusal.fix(), fix, "{call:?}");
    }
    assert!(allowed(Source::ScriptToken).met_by(&check(
        &policy,
        &token,
        &add_to_payments_board(),
        &on_mac(),
    )));
}

#[test]
fn asking_ahead_with_nothing_to_grant_says_to_make_the_call() {
    let refusal = refusal_of(
        policy()
            .check_ask_ahead(
                &agent(a()),
                &use_on(Capability::TrashRestore, Target::Channel(payments())),
                &on_mac(),
            )
            .expect("the call can be checked"),
    );
    assert_eq!(refusal.kind(), ErrorKind::InvalidField);
    assert_eq!(refusal.fix(), Fix::MakeTheCall);
}

#[test]
fn calls_the_policy_cannot_judge_are_errors_not_refusals() {
    let policy = policy();
    // A target the capability doesn't take.
    assert_eq!(
        policy.check(
            &as_agent(a()),
            &use_on(Capability::SessionNote, Target::Global),
            &on_mac(),
        ),
        Err(CheckError::TargetDoesNotFit {
            capability: Capability::SessionNote,
        })
    );
    // A forward from another session's machine (HO-11).
    assert_eq!(
        policy.check(
            &as_agent(a()),
            &use_on(
                Capability::NetworkReach,
                Target::Network(NetworkTarget {
                    machine: the_box(),
                    host: "localhost".to_owned(),
                    port: 5432,
                }),
            ),
            &on_mac(),
        ),
        Err(CheckError::TargetDoesNotFit {
            capability: Capability::NetworkReach,
        })
    );
    // A session in a channel the policy doesn't know.
    let gone = SessionRef {
        channel: ChannelId::new("gone"),
        ..a()
    };
    assert_eq!(
        policy.check(&as_agent(gone.clone()), &note_to(gone), &on_mac()),
        Err(CheckError::UnknownChannel {
            channel: ChannelId::new("gone"),
        })
    );
}

#[test]
fn rule_changes_offer_only_what_the_rule_has() {
    let mut policy = policy();
    let rules = Rules::Channel(payments());
    assert_eq!(
        policy.set_rule(&user(), &rules, Capability::NetworkReach, Decision::Allow),
        Err(PolicyError::SettingNotOffered {
            capability: Capability::NetworkReach,
            decision: Decision::Allow,
        })
    );
    assert_eq!(
        policy.set_rule(&user(), &rules, Capability::AccessChange, Decision::Allow),
        Err(PolicyError::NotARule {
            capability: Capability::AccessChange,
        })
    );
    assert_eq!(
        policy.set_rule(
            &user(),
            &Rules::Channel(ChannelId::new("gone")),
            Capability::SessionNote,
            Decision::Allow,
        ),
        Err(PolicyError::UnknownChannel {
            channel: ChannelId::new("gone"),
        })
    );
    assert_eq!(
        policy.set_rule(&user(), &rules, Capability::NetworkReach, Decision::Never),
        Ok(Decision::Ask)
    );
    assert_eq!(
        policy.add_channel(payments()),
        Err(PolicyError::ChannelExists {
            channel: payments(),
        })
    );
}

#[test]
fn a_deleted_channel_takes_its_rules_and_grants_with_it() {
    let mut policy = policy();
    grant_for(&mut policy, &as_agent(b()), &note_to(a()));
    let ended = policy.remove_channel(&payments()).expect("payments exists");
    assert_eq!(ended.len(), 1);
    assert_eq!(policy.grants().count(), 0);
    assert!(matches!(
        policy.check(&as_agent(a()), &note_to(b()), &on_mac()),
        Err(CheckError::UnknownChannel { .. })
    ));
    // A fresh policy has no channels to check against.
    assert!(Policy::new().channel_rules(&payments()).is_none());
}

#[test]
fn the_capability_list_matches_the_trace_matrix() {
    let all: BTreeSet<Capability> = Capability::ALL.into_iter().collect();
    assert_eq!(
        all.len(),
        Capability::ALL.len(),
        "no capability is listed twice"
    );
    let count = |tier: Tier| all.iter().filter(|c| c.tier() == tier).count();
    // TRACE.md §2's rows.
    assert_eq!(count(Tier::DefaultAllow), 13);
    assert_eq!(count(Tier::AskMe), 16);
    assert_eq!(count(Tier::AskNoAllow), 1);
    assert_eq!(count(Tier::NeverByDefault), 3);
    assert_eq!(count(Tier::UserOnly), 11);
    assert_eq!(count(Tier::Peer), 2);
    // Rules exist for the agent tiers only.
    for capability in Capability::ALL {
        let agent_tier = !matches!(capability.tier(), Tier::UserOnly | Tier::Peer);
        assert_eq!(capability.has_rule(), agent_tier, "{capability:?}");
    }
}
