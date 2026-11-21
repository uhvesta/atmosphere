//! PERMAUDIT policy audit (ATM-30, ATM-32, ATM-33, ATM-34):
//! - 10 safe local same-channel capabilities promoted to DefaultAllow
//! - Narrowed scope enforcement on both sides of the line
//! - Owner's Push Rule (non-default branch Allow vs default branch / PR Ask vs Never)
//! - One-time migration dry-run and apply semantics

mod support;

use std::collections::{BTreeMap, BTreeSet};

use humata2_access::{
    Agent, AgentId, BoardAct, BoardRef, Call, Capability, Decision, Effect, Policy, PolicyState,
    Principal, Refusal, RepoBranch, RepoRef, Rules, RulesState, SessionKind, Source, Target, Tier,
    Verdict,
};
use support::*;

fn is_allowed(verdict: &Verdict) -> bool {
    matches!(
        verdict,
        Verdict::Allowed {
            source: Source::Rule,
            effect: Effect::Apply
        }
    )
}

#[test]
fn promoted_capabilities_are_default_allow_and_outward_stay_guarded() {
    let promoted = Capability::permaudit_promoted();
    assert_eq!(promoted.len(), 10);
    for &cap in promoted {
        assert_eq!(cap.tier(), Tier::DefaultAllow, "{cap:?}");
        assert_eq!(cap.default_decision(), Some(Decision::Allow), "{cap:?}");
    }

    for cap in [
        Capability::GlobalBoardWrite,
        Capability::FilesReadOtherMachine,
        Capability::SessionType,
        Capability::SessionStartOtherMachine,
        Capability::SessionMove,
        Capability::GitPush,
        Capability::MachineProvision,
        Capability::ChannelCreate,
        Capability::ChannelRepoAdd,
        Capability::SessionStopOther,
    ] {
        assert_eq!(cap.tier(), Tier::AskMe, "{cap:?}");
    }

    for cap in [
        Capability::NetworkReach,
        Capability::DeskMoveToChannel,
        Capability::ProviderImport,
        Capability::SettingsChange,
    ] {
        assert_eq!(cap.tier(), Tier::AskNoAllow, "{cap:?}");
    }

    for cap in [
        Capability::SessionDeleteOther,
        Capability::SessionsDeleteMany,
        Capability::MachineSessionsStop,
        Capability::ChannelChange,
        Capability::MachineDestroy,
        Capability::MachineScriptRun,
    ] {
        assert_eq!(cap.tier(), Tier::NeverByDefault, "{cap:?}");
    }
}

#[test]
fn all_ten_promoted_capabilities_allow_in_scope_by_default() {
    let policy = policy();
    let a_agent = as_agent(a());

    let in_scope_calls = [
        // 1. TodoClose
        use_on(Capability::TodoClose, Target::Session(a())),
        // 2. ChannelBoardEditOthers (another agent's item in same channel)
        Call::Board {
            board: BoardRef::Channel(payments()),
            act: BoardAct::Edit {
                created_by: Some(b().id),
            },
        },
        // 3. FilesReadOtherSession (same machine, same channel)
        read(file_of(&b(), "src/lib.rs")),
        // 4. FilesCopyIn (same machine, same channel)
        Call::CopyFiles {
            from: file_of(&b(), "src/lib.rs"),
            into: file_of(&a(), "vendor/lib.rs"),
        },
        // 5. SessionNote (same channel)
        note_to(b()),
        // 6. SessionStartSameMachine (same channel, same machine)
        use_on(
            Capability::SessionStartSameMachine,
            Target::Start {
                channel: payments(),
                machine: mac(),
                agent: AgentId::new("claude"),
                fork_of: None,
            },
        ),
        // 7. PrReviewStart (own channel)
        use_on(Capability::PrReviewStart, Target::Channel(payments())),
        // 8. OwnSessionManage (own session)
        use_on(Capability::OwnSessionManage, Target::Session(a())),
        // 9. OwnSessionEnd (own session)
        use_on(Capability::OwnSessionEnd, Target::Session(a())),
        // 10. TrashRestore (own channel)
        use_on(Capability::TrashRestore, Target::Channel(payments())),
    ];

    for call in in_scope_calls {
        let verdict = check(&policy, &a_agent, &call, &on_mac());
        assert!(is_allowed(&verdict), "{call:?}: {verdict:?}");
    }
}

#[test]
fn narrowed_scope_files_copy_in_same_machine_vs_cross_machine() {
    let policy = policy();
    // Same machine, same channel -> Allow.
    let same_machine = Call::CopyFiles {
        from: file_of(&b(), "src/lib.rs"),
        into: file_of(&a(), "vendor/lib.rs"),
    };
    assert!(is_allowed(&check(&policy, &as_agent(a()), &same_machine, &on_mac())));

    // Cross machine, same channel -> Ask (FilesReadOtherMachine).
    let cross_machine = Call::CopyFiles {
        from: file_of(&c(), "src/lib.rs"),
        into: file_of(&a(), "vendor/lib.rs"),
    };
    assert!(asks(Capability::FilesReadOtherMachine).met_by(&check(
        &policy,
        &as_agent(a()),
        &cross_machine,
        &on_mac()
    )));
}

#[test]
fn narrowed_scope_board_edit_others_agent_vs_owner_vs_other_channel() {
    let mut policy = policy();
    let a_agent = as_agent(a());

    // Agent-created item in same channel -> Allow by default.
    let agent_item = Call::Board {
        board: BoardRef::Channel(payments()),
        act: BoardAct::Edit {
            created_by: Some(b().id),
        },
    };
    assert!(is_allowed(&check(&policy, &a_agent, &agent_item, &on_mac())));

    // Owner-created item in same channel -> Ask by default.
    let owner_item = Call::Board {
        board: BoardRef::Channel(payments()),
        act: BoardAct::Edit { created_by: None },
    };
    assert!(asks(Capability::ChannelBoardEditOthers).met_by(&check(
        &policy,
        &a_agent,
        &owner_item,
        &on_mac()
    )));

    // Adding to another session's board -> Ask by default.
    let add_other_session = Call::Board {
        board: BoardRef::Session(b()),
        act: BoardAct::Add,
    };
    assert!(asks(Capability::ChannelBoardEditOthers).met_by(&check(
        &policy,
        &a_agent,
        &add_other_session,
        &on_mac()
    )));

    // Other channel -> Ask (GlobalBoardWrite).
    let other_channel_item = Call::Board {
        board: BoardRef::Channel(search()),
        act: BoardAct::Edit {
            created_by: Some(x().id),
        },
    };
    assert!(asks(Capability::GlobalBoardWrite).met_by(&check(
        &policy,
        &a_agent,
        &other_channel_item,
        &on_mac()
    )));

    // Once owner explicitly sets ChannelBoardEditOthers to Allow, owner-created items are allowed.
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::ChannelBoardEditOthers,
            Decision::Allow,
        )
        .expect("Allow is offered");
    assert!(is_allowed(&check(&policy, &a_agent, &owner_item, &on_mac())));
}

#[test]
fn narrowed_scope_cross_channel_and_cross_machine_boundaries() {
    let policy = policy();
    let a_agent = as_agent(a());

    // SessionNote: other channel refused.
    assert!(matches!(
        check(&policy, &a_agent, &note_to(x()), &on_mac()),
        Verdict::Refused(Refusal::OtherChannel { .. })
    ));

    // SessionStartSameMachine: other machine asks (SessionStartOtherMachine), other channel refused.
    let other_machine_start = use_on(
        Capability::SessionStartSameMachine,
        Target::Start {
            channel: payments(),
            machine: the_box(),
            agent: AgentId::new("claude"),
            fork_of: None,
        },
    );
    assert!(asks(Capability::SessionStartOtherMachine).met_by(&check(
        &policy,
        &a_agent,
        &other_machine_start,
        &on_mac()
    )));

    let other_channel_start = use_on(
        Capability::SessionStartSameMachine,
        Target::Start {
            channel: search(),
            machine: mac(),
            agent: AgentId::new("claude"),
            fork_of: None,
        },
    );
    assert!(matches!(
        check(&policy, &a_agent, &other_channel_start, &on_mac()),
        Verdict::Refused(Refusal::OtherChannel { .. })
    ));

    // PrReviewStart & TrashRestore: other channel refused.
    for cap in [Capability::PrReviewStart, Capability::TrashRestore] {
        assert!(matches!(
            check(&policy, &a_agent, &use_on(cap, Target::Channel(search())), &on_mac()),
            Verdict::Refused(Refusal::OtherChannel { .. })
        ));
    }
}

#[test]
fn owners_push_rule_feature_branch_vs_default_vs_pr_vs_never() {
    let mut policy = policy();
    let a_agent = as_agent(a());

    let feature_branch = use_on(
        Capability::GitPush,
        Target::PushBranch(RepoBranch {
            repo: RepoRef::new("pay"),
            branch: "feat/permaudit".to_owned(),
        }),
    );
    assert!(is_allowed(&check(&policy, &a_agent, &feature_branch, &on_mac())));

    for default_branch in ["main", "master", "HEAD"] {
        let call = use_on(
            Capability::GitPush,
            Target::PushBranch(RepoBranch {
                repo: RepoRef::new("pay"),
                branch: default_branch.to_owned(),
            }),
        );
        assert!(
            asks(Capability::GitPush).met_by(&check(&policy, &a_agent, &call, &on_mac())),
            "{default_branch}"
        );
    }

    let pr_push = use_on(
        Capability::GitPush,
        Target::Push(RepoBranch {
            repo: RepoRef::new("pay"),
            branch: "feat/permaudit".to_owned(),
        }),
    );
    assert!(asks(Capability::GitPush).met_by(&check(&policy, &a_agent, &pr_push, &on_mac())));

    // Review { push_allowed: false } refuses even feature branch push.
    let review_blocked = Principal::Agent(Agent {
        session: a(),
        kind: SessionKind::Review {
            push_allowed: false,
        },
    });
    assert!(matches!(
        check(&policy, &review_blocked, &feature_branch, &on_mac()),
        Verdict::Refused(Refusal::ReviewSessionPush { .. })
    ));

    // GitPush == Never refuses feature branch push.
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::GitPush,
            Decision::Never,
        )
        .expect("Never is offered");
    assert!(matches!(
        check(&policy, &a_agent, &feature_branch, &on_mac()),
        Verdict::Refused(Refusal::NeverRule {
            capability: Capability::GitPush,
            ..
        })
    ));
}

#[test]
fn permaudit_migration_dry_run_and_apply() {
    // Simulate a persisted pre-PERMAUDIT policy where all 10 promoted capabilities
    // were stored at their old default (Decision::Ask), plus:
    // - TodoClose is Never in payments (must NEVER loosen)
    // - SessionNote is owner-set to Ask in payments via set_rule (must stay Ask)
    // - FilesCopyIn is passed in owner_set_channels for payments (must stay Ask)
    // - OwnSessionEnd is passed in owner_set_defaults (must stay Ask in new_channels)
    // - GitPush is Ask (non-promoted, must stay Ask)
    let old_decisions = |extra: &[(Capability, Decision)]| -> Vec<(Capability, Decision)> {
        let mut map: BTreeMap<Capability, Decision> = Capability::ALL
            .iter()
            .filter_map(|&c| c.default_decision().map(|d| (c, d)))
            .collect();
        for &cap in Capability::permaudit_promoted() {
            map.insert(cap, Decision::Ask);
        }
        for &(cap, dec) in extra {
            map.insert(cap, dec);
        }
        map.into_iter().collect()
    };

    let state = PolicyState {
        new_channels: RulesState {
            decisions: old_decisions(&[]),
            away: humata2_access::AwayRule::Wait,
        },
        channels: BTreeMap::from([(
            payments(),
            RulesState {
                decisions: old_decisions(&[(Capability::TodoClose, Decision::Never)]),
                away: humata2_access::AwayRule::Wait,
            },
        )]),
        grants: vec![],
        next_grant: 1,
    };

    let mut policy = Policy::from_state(state).expect("valid state");
    // Owner explicitly sets SessionNote to Ask in payments.
    policy
        .set_rule(
            &user(),
            &Rules::Channel(payments()),
            Capability::SessionNote,
            Decision::Ask,
        )
        .expect("Ask is offered");

    let owner_set_channels =
        BTreeMap::from([(payments(), BTreeSet::from([Capability::FilesCopyIn]))]);
    let owner_set_defaults = BTreeSet::from([Capability::OwnSessionEnd]);

    let before = policy.to_state();
    let planned = policy.plan_permaudit_migration(&owner_set_channels, &owner_set_defaults);
    // Dry run does not mutate policy.
    assert_eq!(policy.to_state(), before);

    // new_channels: 9 of 10 promoted (OwnSessionEnd skipped).
    let default_caps: BTreeSet<Capability> = planned
        .iter()
        .filter(|(scope, _, _, _)| scope.is_none())
        .map(|(_, cap, _, _)| *cap)
        .collect();
    assert_eq!(default_caps.len(), 9);
    assert!(!default_caps.contains(&Capability::OwnSessionEnd));

    // payments: 7 of 10 promoted (TodoClose=Never, SessionNote=owner_set, FilesCopyIn=external owner_set).
    let payments_caps: BTreeSet<Capability> = planned
        .iter()
        .filter(|(scope, _, _, _)| scope.as_ref() == Some(&payments()))
        .map(|(_, cap, _, _)| *cap)
        .collect();
    assert_eq!(payments_caps.len(), 7);
    assert!(!payments_caps.contains(&Capability::TodoClose));
    assert!(!payments_caps.contains(&Capability::SessionNote));
    assert!(!payments_caps.contains(&Capability::FilesCopyIn));

    // Apply produces the exact same list.
    let applied = policy.apply_permaudit_migration(&owner_set_channels, &owner_set_defaults);
    assert_eq!(applied, planned);

    // Verify resulting decisions.
    let p_rules = policy.channel_rules(&payments()).expect("payments");
    assert_eq!(p_rules.decision(Capability::TodoClose), Some(Decision::Never));
    assert_eq!(p_rules.decision(Capability::SessionNote), Some(Decision::Ask));
    assert_eq!(p_rules.decision(Capability::FilesCopyIn), Some(Decision::Ask));
    assert_eq!(p_rules.decision(Capability::GitPush), Some(Decision::Ask));
    assert_eq!(p_rules.decision(Capability::FilesReadOtherSession), Some(Decision::Allow));
    assert_eq!(p_rules.decision(Capability::TrashRestore), Some(Decision::Allow));

    let d_rules = policy.new_channel_rules();
    assert_eq!(d_rules.decision(Capability::OwnSessionEnd), Some(Decision::Ask));
    assert_eq!(d_rules.decision(Capability::TodoClose), Some(Decision::Allow));
    assert_eq!(d_rules.decision(Capability::GitPush), Some(Decision::Ask));

    // Running migration a second time is a no-op.
    assert!(policy
        .plan_permaudit_migration(&owner_set_channels, &owner_set_defaults)
        .is_empty());
}
