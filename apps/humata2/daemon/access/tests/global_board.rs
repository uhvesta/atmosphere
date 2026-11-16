//! Writes to the Global board and to other channels' boards ask with their
//! own capability; reads there stay Default Allow (BD-9, D155).

mod support;

use humata2_access::{BoardAct, BoardRef, Call, Capability, GrantTarget, Source, Tier, Verdict};
use support::*;

fn on(board: BoardRef, act: BoardAct) -> Call {
    Call::Board { board, act }
}

fn elsewhere() -> [BoardRef; 3] {
    [
        BoardRef::Global,
        BoardRef::Channel(search()),
        BoardRef::Session(x()),
    ]
}

#[test]
fn reading_the_global_board_or_another_channels_is_allowed() {
    let policy = policy();
    for board in elsewhere() {
        let call = on(board, BoardAct::Read);
        assert!(
            allowed(Source::Rule).met_by(&check(&policy, &as_agent(a()), &call, &on_mac())),
            "{call:?}"
        );
    }
}

#[test]
fn adding_or_editing_there_asks_and_offers_always_on_that_board() {
    let policy = policy();
    for board in elsewhere() {
        for act in [
            BoardAct::Add,
            BoardAct::Edit { created_by: None },
            // Even an item the agent made itself.
            BoardAct::Edit {
                created_by: Some(a().id),
            },
        ] {
            let call = on(board.clone(), act);
            let asked = request(check(&policy, &as_agent(a()), &call, &on_mac()));
            assert_eq!(asked.capability, Capability::GlobalBoardWrite, "{call:?}");
            assert_eq!(asked.always_let, Some(GrantTarget::Board(board.clone())));
        }
    }
}

#[test]
fn the_capability_is_ask_me_and_can_be_granted() {
    assert_eq!(Capability::GlobalBoardWrite.tier(), Tier::AskMe);
    assert!(Capability::GlobalBoardWrite.offers_always_let());
    assert!(Capability::ALL.contains(&Capability::GlobalBoardWrite));
}

#[test]
fn an_always_allow_covers_later_writes_on_that_board_only() {
    let mut policy = policy();
    let agent = as_agent(a());
    let global_add = on(BoardRef::Global, BoardAct::Add);
    grant_for(&mut policy, &agent, &global_add);
    let later = check(
        &policy,
        &agent,
        &on(BoardRef::Global, BoardAct::Edit { created_by: None }),
        &on_mac(),
    );
    assert!(
        matches!(later, Verdict::Allowed { source: Source::Grant(_), .. }),
        "{later:?}"
    );
    let other = on(BoardRef::Channel(search()), BoardAct::Add);
    assert_eq!(
        request(check(&policy, &agent, &other, &on_mac())).capability,
        Capability::GlobalBoardWrite
    );
}
