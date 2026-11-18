//! TRACE.md §2, "Who may do what": every actor against every tier.
//!
//! Each tier is stood for by one call:
//!
//! - No rule: reading a file in session `a`'s clone (for agents, their own).
//! - Default Allow: adding an item to the Payments channel board.
//! - Ask me: a note to another session.
//! - Never by default: deleting another session.
//! - User-only: changing access rules.
//! - Peer: a machine's report to the Mac.
//!
//! The row "CLI outside a session, no credential" isn't here: such a call
//! never reaches the policy, because the daemon refuses it everything but
//! `WhoAmI` and starting a CLI login (SEC-9). humatad2's tests/auth.rs
//! covers it.

mod support;

use humata2_access::{
    AnswerAt, Call, Capability, Context, Effect, PresenceMethod, Principal, Refusal, ScriptToken,
    Source, Target,
};
use support::*;

struct Cell {
    actor: &'static str,
    tier: &'static str,
    principal: Principal,
    context: Context,
    call: Call,
    expect: Expect,
}

fn stop(session: humata2_access::SessionRef) -> Call {
    use_on(Capability::SessionDeleteOther, Target::Session(session))
}

fn change_access() -> Call {
    use_on(Capability::AccessChange, Target::Channel(payments()))
}

fn peer_report() -> Call {
    use_on(Capability::PeerReport, Target::Global)
}

fn user_only(refusal: &Refusal) -> bool {
    matches!(refusal, Refusal::UserOnly { .. })
}

fn never_rule(refusal: &Refusal) -> bool {
    matches!(refusal, Refusal::NeverRule { .. })
}

fn peer_only(refusal: &Refusal) -> bool {
    matches!(refusal, Refusal::PeerOnly { .. })
}

fn presence(refusal: &Refusal) -> bool {
    matches!(refusal, Refusal::PresenceRequired { .. })
}

fn token_lacks(refusal: &Refusal) -> bool {
    matches!(refusal, Refusal::ScriptTokenLacks { .. })
}

fn peer_cannot_call(refusal: &Refusal) -> bool {
    matches!(refusal, Refusal::PeerCannotCall)
}

/// The six tier calls for one actor.
struct Row {
    actor: &'static str,
    principal: Principal,
    context: Context,
    no_rule: (Call, Expect),
    default_allow: (Call, Expect),
    ask_me: (Call, Expect),
    never: (Call, Expect),
    user_only: (Call, Expect),
    peer: (Call, Expect),
}

impl Row {
    fn cells(self) -> Vec<Cell> {
        let Row {
            actor,
            principal,
            context,
            no_rule,
            default_allow,
            ask_me,
            never,
            user_only,
            peer,
        } = self;
        [
            ("no rule", no_rule),
            ("default allow", default_allow),
            ("ask me", ask_me),
            ("never by default", never),
            ("user-only", user_only),
            ("peer", peer),
        ]
        .into_iter()
        .map(|(tier, (call, expect))| Cell {
            actor,
            tier,
            principal: principal.clone(),
            context: context.clone(),
            call,
            expect,
        })
        .collect()
    }
}

fn token() -> ScriptToken {
    ScriptToken {
        capabilities: [Capability::ChannelBoardAdd].into_iter().collect(),
        channels: [payments()].into_iter().collect(),
    }
}

fn rows() -> Vec<Row> {
    vec![
        Row {
            actor: "user in the app",
            principal: Principal::App,
            context: on_mac(),
            no_rule: (read(file_of(&a(), "src/main.rs")), allowed(Source::User)),
            default_allow: (add_to_payments_board(), allowed(Source::User)),
            ask_me: (note_to(b()), allowed(Source::User)),
            never: (stop(b()), allowed(Source::User)),
            user_only: (change_access(), allowed(Source::User)),
            peer: (peer_report(), Expect::Refused(peer_only)),
        },
        Row {
            actor: "agent, its own session",
            principal: as_agent(a()),
            context: on_mac(),
            no_rule: (read(file_of(&a(), "src/main.rs")), allowed(Source::Owned)),
            default_allow: (add_to_payments_board(), allowed(Source::Rule)),
            ask_me: (note_to(b()), asks(Capability::SessionNote)),
            never: (stop(b()), Expect::Refused(never_rule)),
            user_only: (change_access(), Expect::Refused(user_only)),
            peer: (peer_report(), Expect::Refused(peer_only)),
        },
        Row {
            actor: "agent, another session on the same machine",
            principal: as_agent(b()),
            context: on_mac(),
            no_rule: (
                read(file_of(&a(), "src/main.rs")),
                asks(Capability::FilesReadOtherSession),
            ),
            default_allow: (add_to_payments_board(), allowed(Source::Rule)),
            ask_me: (note_to(a()), asks(Capability::SessionNote)),
            never: (stop(a()), Expect::Refused(never_rule)),
            user_only: (change_access(), Expect::Refused(user_only)),
            peer: (peer_report(), Expect::Refused(peer_only)),
        },
        Row {
            actor: "agent on another machine, checked on the Mac",
            principal: as_agent(c()),
            context: on_mac(),
            no_rule: (
                read(file_of(&a(), "src/main.rs")),
                asks(Capability::FilesReadOtherMachine),
            ),
            default_allow: (add_to_payments_board(), allowed(Source::Rule)),
            ask_me: (note_to(a()), asks(Capability::SessionNote)),
            never: (stop(a()), Expect::Refused(never_rule)),
            user_only: (change_access(), Expect::Refused(user_only)),
            peer: (peer_report(), Expect::Refused(peer_only)),
        },
        Row {
            // The CLI inside a session tree is that session's agent
            // (SEC-8); the daemon builds the same principal.
            actor: "CLI inside a session tree",
            principal: as_agent(a()),
            context: on_mac(),
            no_rule: (write(file_of(&a(), "src/main.rs")), allowed(Source::Owned)),
            default_allow: (add_to_payments_board(), allowed(Source::Rule)),
            ask_me: (note_to(b()), asks(Capability::SessionNote)),
            never: (stop(b()), Expect::Refused(never_rule)),
            user_only: (change_access(), Expect::Refused(user_only)),
            peer: (peer_report(), Expect::Refused(peer_only)),
        },
        Row {
            actor: "CLI outside a session, with presence",
            principal: Principal::Cli(PresenceMethod::CliLogin),
            context: on_mac(),
            no_rule: (read(file_of(&a(), "src/main.rs")), allowed(Source::User)),
            default_allow: (add_to_payments_board(), allowed(Source::User)),
            ask_me: (note_to(b()), allowed(Source::User)),
            never: (stop(b()), allowed(Source::User)),
            user_only: (change_access(), allowed(Source::User)),
            peer: (peer_report(), Expect::Refused(peer_only)),
        },
        Row {
            actor: "a machine while the Mac is away",
            principal: as_agent(c()),
            context: on_box_away(),
            no_rule: (read(file_of(&c(), "src/main.rs")), allowed(Source::Owned)),
            default_allow: (
                add_to_payments_board(),
                Expect::Allowed(Source::Rule, Effect::Journal),
            ),
            ask_me: (
                note_to(d()),
                Expect::Ask(Capability::SessionNote, AnswerAt::Machine(the_box())),
            ),
            never: (stop(d()), Expect::Refused(never_rule)),
            user_only: (change_access(), Expect::Refused(user_only)),
            peer: (peer_report(), Expect::Refused(peer_only)),
        },
        Row {
            actor: "script token",
            principal: Principal::Script(token()),
            context: on_mac(),
            no_rule: (
                read(file_of(&a(), "src/main.rs")),
                Expect::Refused(|r| matches!(r, Refusal::ScriptTokenHasNoSession)),
            ),
            default_allow: (add_to_payments_board(), allowed(Source::ScriptToken)),
            ask_me: (note_to(b()), Expect::Refused(token_lacks)),
            never: (stop(b()), Expect::Refused(token_lacks)),
            user_only: (change_access(), Expect::Refused(user_only)),
            peer: (peer_report(), Expect::Refused(peer_only)),
        },
        Row {
            actor: "peer daemon",
            principal: Principal::Peer(humata2_access::PeerLink::DaemonToHost {
                machine: the_box(),
            }),
            context: on_mac(),
            no_rule: (read(file_of(&a(), "src/main.rs")), Expect::Refused(peer_cannot_call)),
            default_allow: (add_to_payments_board(), Expect::Refused(peer_cannot_call)),
            ask_me: (note_to(b()), Expect::Refused(peer_cannot_call)),
            never: (stop(b()), Expect::Refused(peer_cannot_call)),
            user_only: (change_access(), Expect::Refused(peer_cannot_call)),
            peer: (peer_report(), allowed(Source::PeerLink)),
        },
    ]
}

#[test]
fn every_actor_against_every_tier() {
    let policy = policy();
    let cells: Vec<Cell> = rows().into_iter().flat_map(Row::cells).collect();
    assert_eq!(cells.len(), 54, "nine actors, six tiers");
    let wrong: Vec<String> = cells
        .iter()
        .filter_map(|cell| {
            let verdict = policy
                .check(&cell.principal, &cell.call, &cell.context)
                .expect("the call can be checked");
            (!cell.expect.met_by(&verdict)).then(|| {
                format!(
                    "{} / {}: expected {:?}, got {:?}",
                    cell.actor, cell.tier, cell.expect, verdict
                )
            })
        })
        .collect();
    assert!(wrong.is_empty(), "cells that disagree:\n{}", wrong.join("\n"));
}

// The cells whose wording in TRACE.md has a second half.

#[test]
fn an_agent_does_default_allow_things_unless_the_channel_set_ask_or_never() {
    let mut policy = policy();
    let rules = humata2_access::Rules::Channel(payments());
    let agent = as_agent(a());
    let call = add_to_payments_board();

    policy
        .set_rule(&user(), &rules, Capability::ChannelBoardAdd, humata2_access::Decision::Ask)
        .expect("Ask is offered");
    assert!(asks(Capability::ChannelBoardAdd).met_by(&check(&policy, &agent, &call, &on_mac())));

    policy
        .set_rule(&user(), &rules, Capability::ChannelBoardAdd, humata2_access::Decision::Never)
        .expect("Never is offered");
    assert!(Expect::Refused(never_rule).met_by(&check(&policy, &agent, &call, &on_mac())));
}

#[test]
fn no_rule_stops_an_agent_on_its_own_things() {
    let mut policy = policy();
    let rules = humata2_access::Rules::Channel(payments());
    for capability in Capability::ALL {
        if capability.has_rule() {
            policy
                .set_rule(&user(), &rules, capability, humata2_access::Decision::Never)
                .expect("Never is offered for every rule");
        }
    }
    let agent = as_agent(a());
    for call in [
        read(file_of(&a(), "src/main.rs")),
        write(file_of(&a(), "src/main.rs")),
        read(shared_on(mac(), payments(), "notes.md")),
        Call::ReadSession(a()),
    ] {
        assert!(
            allowed(Source::Owned).met_by(&check(&policy, &agent, &call, &on_mac())),
            "{call:?}"
        );
    }
}

#[test]
fn a_machine_while_away_waits_for_what_needs_another_machine() {
    let policy = policy();
    // c is on the box; a is on the Mac.
    let verdict = check(&policy, &as_agent(c()), &note_to(a()), &on_box_away());
    assert_eq!(verdict, humata2_access::Verdict::WaitForHost);
}

#[test]
fn a_machine_while_away_answers_only_requests_it_holds_with_the_passphrase() {
    let policy = policy();
    let answer = use_on(Capability::InboxAnswer, Target::Channel(payments()));
    let passphrase = Principal::Cli(PresenceMethod::ApprovalPassphrase);
    assert!(allowed(Source::User).met_by(&check(&policy, &passphrase, &answer, &on_box_away())));
    let login = Principal::Cli(PresenceMethod::CliLogin);
    assert!(Expect::Refused(presence).met_by(&check(&policy, &login, &answer, &on_box_away())));
    assert!(
        Expect::Refused(|r| matches!(r, Refusal::HostUnreachable))
            .met_by(&check(&policy, &passphrase, &change_access(), &on_box_away()))
    );
}

#[test]
fn a_script_token_does_only_what_it_lists_in_its_channels() {
    let policy = policy();
    let edit_others = || Call::Board {
        board: humata2_access::BoardRef::Channel(payments()),
        act: humata2_access::BoardAct::Edit { created_by: None },
    };
    let stop_b = stop(b());
    let listing = |capabilities: &[Capability]| {
        Principal::Script(ScriptToken {
            capabilities: capabilities.iter().copied().collect(),
            channels: [payments()].into_iter().collect(),
        })
    };

    // Listed, whatever its tier (CLI.md's triage token lists
    // channel_board_edit_others).
    let triage = listing(&[Capability::ChannelBoardEditOthers, Capability::SessionDeleteOther]);
    assert!(allowed(Source::ScriptToken).met_by(&check(&policy, &triage, &edit_others(), &on_mac())));
    assert!(allowed(Source::ScriptToken).met_by(&check(&policy, &triage, &stop_b, &on_mac())));

    // Not in its channels.
    let other_channel = Call::Board {
        board: humata2_access::BoardRef::Channel(search()),
        act: humata2_access::BoardAct::Edit { created_by: None },
    };
    assert!(
        Expect::Refused(|r| matches!(r, Refusal::ScriptTokenOtherChannel { .. }))
            .met_by(&check(&policy, &triage, &other_channel, &on_mac()))
    );

    // A network connection always needs the user's approval; listing it
    // doesn't help.
    let network = listing(&[Capability::NetworkReach]);
    let connect = use_on(
        Capability::NetworkReach,
        Target::Network(humata2_access::NetworkTarget {
            machine: mac(),
            host: "staging-db".to_owned(),
            port: 5432,
        }),
    );
    assert!(
        Expect::Refused(|r| matches!(r, Refusal::ScriptTokenCannotAsk { .. }))
            .met_by(&check(&policy, &network, &connect, &on_mac()))
    );

    // User-only, even if listed.
    let greedy = listing(&[Capability::AccessChange]);
    assert!(Expect::Refused(user_only).met_by(&check(&policy, &greedy, &change_access(), &on_mac())));
}

#[test]
fn the_peer_link_goes_one_way_each() {
    let policy = policy();
    let control = use_on(Capability::HostControl, Target::Global);
    let host = Principal::Peer(humata2_access::PeerLink::HostToDaemon);
    let machine = Principal::Peer(humata2_access::PeerLink::DaemonToHost { machine: the_box() });
    assert!(allowed(Source::PeerLink).met_by(&check(&policy, &host, &control, &on_box_reachable())));
    assert!(
        Expect::Refused(peer_cannot_call).met_by(&check(&policy, &machine, &control, &on_mac()))
    );
    assert!(
        Expect::Refused(peer_cannot_call)
            .met_by(&check(&policy, &host, &peer_report(), &on_box_reachable()))
    );
}
