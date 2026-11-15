//! The pane workspace's behaviour (NV-28–NV-35): split, move, swap, close,
//! undo, persist and restore, Reset and Undo reset, and the layout policy.

use std::ops::Deref;
use std::sync::Arc;

use humata2_clients::fixture;
use humata2_clients::model::SessionId;
use humata2_layout::{
    Axis, Context, Delegate, Direction, Edge, Layout, LayoutAction, LayoutState, PaneId, PaneKind, Policy, Refusal,
    Source, Tree,
};
use tca::TestStore;

fn login() -> SessionId {
    SessionId::new("s-login")
}

/// A store whose state reads like the layout's own.
struct Harness(TestStore<Layout>);

impl Deref for Harness {
    type Target = LayoutState;

    fn deref(&self) -> &LayoutState {
        self.0.state()
    }
}

/// A layout on the standard fixture, on Login v2's page (which has
/// changes, so its default is the agent with Diff beside).
fn on_login(policy: Policy) -> Harness {
    let mut state = Harness(TestStore::new(LayoutState::default(), Layout).non_exhaustive());
    send(&mut state, LayoutAction::Show(Arc::new(fixture::standard())));
    send(&mut state, LayoutAction::SetPolicy(policy));
    send(&mut state, LayoutAction::Enter(Context::Session(login())));
    state
}

fn send(state: &mut Harness, action: LayoutAction) {
    state.0.send(action, |_| {});
}

/// Sends `action` and checks the layout hands `delegate` to the app.
fn expect(state: &mut Harness, action: LayoutAction, delegate: Delegate) {
    send(state, action);
    state.0.receive(LayoutAction::Delegate(delegate), |_| {});
}

fn kinds(state: &Harness) -> Vec<PaneKind> {
    state.tree().panes().into_iter().map(|pane| pane.kind.clone()).collect()
}

fn pane(state: &Harness, kind: &PaneKind) -> PaneId {
    state.tree().showing(kind).unwrap_or_else(|| panic!("{kind:?} isn't showing"))
}

#[test]
fn a_session_with_changes_opens_on_its_agent_with_diff_beside() {
    let state = on_login(Policy::Simple);
    assert_eq!(kinds(&state), [PaneKind::Agent(login()), PaneKind::Diff(login())]);
    assert!(!state.arranged());
}

#[test]
fn a_session_without_changes_opens_on_its_agent_alone() {
    let mut state = on_login(Policy::Simple);
    let copy = SessionId::new("s-copy");
    send(&mut state, LayoutAction::Enter(Context::Session(copy.clone())));
    assert_eq!(kinds(&state), [PaneKind::Agent(copy)]);
}

#[test]
fn splitting_a_terminal_opens_a_new_shell_beside_it_and_focuses_it() {
    let mut state = on_login(Policy::Free);
    let id = pane(&state, &PaneKind::Agent(login()));
    send(&mut state, LayoutAction::Focus(id));
    send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    let shown = kinds(&state);
    assert_eq!(shown.len(), 3);
    // Login v2 has two shells open already, so the new one is the third.
    assert_eq!(shown[1], PaneKind::Shell(login(), 2));
    assert_eq!(state.focus(), Some(pane(&state, &PaneKind::Shell(login(), 2))));
    assert!(state.arranged());
}

#[test]
fn splitting_something_else_shows_a_second_copy() {
    let mut state = on_login(Policy::Free);
    let id = pane(&state, &PaneKind::Diff(login()));
    send(&mut state, LayoutAction::Focus(id));
    send(&mut state, LayoutAction::SplitFocused(Axis::Vertical));
    let diffs = kinds(&state).into_iter().filter(|kind| *kind == PaneKind::Diff(login())).count();
    assert_eq!(diffs, 2);
    assert_eq!(state.tree().depth(), 2);
}

#[test]
fn dropping_a_pane_on_an_edge_moves_it_there() {
    let mut state = on_login(Policy::Free);
    let agent = pane(&state, &PaneKind::Agent(login()));
    let diff = pane(&state, &PaneKind::Diff(login()));
    send(
        &mut state,
        LayoutAction::Drop {
            source: Source::Pane(diff),
            target: agent,
            edge: Edge::Top,
        },
    );
    let Tree::Split(split) = state.tree() else {
        panic!("expected a split");
    };
    assert_eq!(split.axis, Axis::Vertical);
    assert_eq!(kinds(&state), [PaneKind::Diff(login()), PaneKind::Agent(login())]);
    assert_eq!(state.focus(), Some(diff));
}

#[test]
fn dropping_a_pane_on_the_middle_of_another_swaps_them() {
    let mut state = on_login(Policy::Simple);
    let agent = pane(&state, &PaneKind::Agent(login()));
    let diff = pane(&state, &PaneKind::Diff(login()));
    send(
        &mut state,
        LayoutAction::Drop {
            source: Source::Pane(agent),
            target: diff,
            edge: Edge::Centre,
        },
    );
    assert_eq!(kinds(&state), [PaneKind::Diff(login()), PaneKind::Agent(login())]);
    // Each pane keeps its id, so its view moves with it.
    assert_eq!(pane(&state, &PaneKind::Agent(login())), agent);
}

#[test]
fn dropping_a_sidebar_entry_on_an_edge_opens_it_there() {
    let mut state = on_login(Policy::Simple);
    let agent = pane(&state, &PaneKind::Agent(login()));
    send(
        &mut state,
        LayoutAction::Drop {
            source: Source::Kind(PaneKind::Inbox),
            target: agent,
            edge: Edge::Bottom,
        },
    );
    assert_eq!(
        kinds(&state),
        [PaneKind::Agent(login()), PaneKind::Inbox, PaneKind::Diff(login())]
    );
}

#[test]
fn opening_from_the_sidebar_replaces_the_focused_pane_or_focuses_one_showing_it() {
    let mut state = on_login(Policy::Simple);
    let diff = pane(&state, &PaneKind::Diff(login()));
    send(&mut state, LayoutAction::Focus(diff));
    send(&mut state, LayoutAction::Open(PaneKind::Files(Some(login()))));
    assert_eq!(kinds(&state), [PaneKind::Agent(login()), PaneKind::Files(Some(login()))]);

    send(&mut state, LayoutAction::Open(PaneKind::Agent(login())));
    assert_eq!(state.focus(), Some(pane(&state, &PaneKind::Agent(login()))));
    assert_eq!(kinds(&state).len(), 2);
}

#[test]
fn closing_a_pane_gives_its_room_and_focus_to_a_neighbour() {
    let mut state = on_login(Policy::Simple);
    let agent = pane(&state, &PaneKind::Agent(login()));
    let diff = pane(&state, &PaneKind::Diff(login()));
    send(&mut state, LayoutAction::Focus(diff));
    send(&mut state, LayoutAction::CloseFocused);
    assert_eq!(kinds(&state), [PaneKind::Agent(login())]);
    assert_eq!(state.focus(), Some(agent));
}

#[test]
fn the_last_pane_does_not_close() {
    let mut state = on_login(Policy::Simple);
    let id = pane(&state, &PaneKind::Diff(login()));
    send(&mut state, LayoutAction::Close(id));
    let agent = pane(&state, &PaneKind::Agent(login()));
    expect(&mut state, LayoutAction::Close(agent), Delegate::Refused(Refusal::LastPane));
    assert_eq!(kinds(&state), [PaneKind::Agent(login())]);
}

#[test]
fn focus_moves_toward_the_arrow() {
    let mut state = on_login(Policy::Free);
    let agent = pane(&state, &PaneKind::Agent(login()));
    let diff = pane(&state, &PaneKind::Diff(login()));
    send(&mut state, LayoutAction::Focus(agent));
    send(&mut state, LayoutAction::FocusToward(Direction::Right));
    assert_eq!(state.focus(), Some(diff));
    send(&mut state, LayoutAction::FocusToward(Direction::Right));
    assert_eq!(state.focus(), Some(diff));
    send(&mut state, LayoutAction::FocusToward(Direction::Left));
    assert_eq!(state.focus(), Some(agent));
}

#[test]
fn undo_steps_back_through_layout_changes() {
    let mut state = on_login(Policy::Free);
    let before = state.tree();
    send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    let split = state.tree();
    let id = pane(&state, &PaneKind::Diff(login()));
    send(&mut state, LayoutAction::Close(id));
    send(&mut state, LayoutAction::Undo);
    assert_eq!(state.tree(), split);
    send(&mut state, LayoutAction::Undo);
    assert_eq!(state.tree(), before);
    assert!(!state.can_undo());
}

#[test]
fn reset_shows_the_default_and_undo_reset_restores_the_exact_layout() {
    let mut state = on_login(Policy::Free);
    send(&mut state, LayoutAction::SplitFocused(Axis::Vertical));
    send(&mut state, LayoutAction::Open(PaneKind::Inbox));
    let arranged = state.tree();
    let focus = state.focus();

    expect(&mut state, LayoutAction::Reset, Delegate::WasReset);
    assert_eq!(state.tree(), state.default_tree(&Context::Session(login())));
    assert!(state.can_undo_reset());

    send(&mut state, LayoutAction::UndoReset);
    assert_eq!(state.tree(), arranged);
    assert_eq!(state.focus(), focus);
}

#[test]
fn reset_on_the_default_layout_does_nothing() {
    let mut state = on_login(Policy::Simple);
    send(&mut state, LayoutAction::Reset);
    assert!(!state.can_undo());
}

#[test]
fn undo_reset_after_another_change_does_nothing() {
    let mut state = on_login(Policy::Free);
    send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    send(&mut state, LayoutAction::Reset);
    send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    let now = state.tree();
    send(&mut state, LayoutAction::UndoReset);
    assert_eq!(state.tree(), now);
}

#[test]
fn fixed_refuses_split_move_swap_and_close() {
    let mut state = on_login(Policy::Fixed);
    let before = state.tree();
    let agent = pane(&state, &PaneKind::Agent(login()));
    let diff = pane(&state, &PaneKind::Diff(login()));
    let refused = Delegate::Refused(Refusal::Fixed);
    expect(&mut state, LayoutAction::SplitFocused(Axis::Horizontal), refused.clone());
    for edge in [Edge::Top, Edge::Centre] {
        let action = LayoutAction::Drop {
            source: Source::Pane(diff),
            target: agent,
            edge,
        };
        expect(&mut state, action, refused.clone());
    }
    expect(&mut state, LayoutAction::Close(diff), refused);
    assert_eq!(state.tree(), before);
}

#[test]
fn fixed_still_lets_a_pane_show_something_else() {
    let mut state = on_login(Policy::Fixed);
    let id = pane(&state, &PaneKind::Diff(login()));
    send(&mut state, LayoutAction::Focus(id));
    send(&mut state, LayoutAction::Open(PaneKind::Inbox));
    assert_eq!(kinds(&state), [PaneKind::Agent(login()), PaneKind::Inbox]);
}

#[test]
fn simple_refuses_a_fifth_pane() {
    let mut state = on_login(Policy::Simple);
    send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    assert_eq!(state.tree().count(), 4);
    expect(
        &mut state,
        LayoutAction::SplitFocused(Axis::Horizontal),
        Delegate::Refused(Refusal::TooMany),
    );
    assert_eq!(state.tree().count(), 4);
}

#[test]
fn simple_refuses_a_third_level() {
    let mut state = on_login(Policy::Simple);
    send(&mut state, LayoutAction::SplitFocused(Axis::Vertical));
    assert_eq!(state.tree().depth(), 2);
    expect(
        &mut state,
        LayoutAction::SplitFocused(Axis::Horizontal),
        Delegate::Refused(Refusal::TooDeep),
    );
    assert_eq!(state.tree().depth(), 2);
}

#[test]
fn narrowing_the_policy_shows_the_default_and_widening_brings_the_layout_back() {
    let mut state = on_login(Policy::Free);
    for _ in 0..4 {
        send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    }
    let arranged = state.tree();
    assert_eq!(arranged.count(), 6);

    send(&mut state, LayoutAction::SetPolicy(Policy::Simple));
    assert_eq!(state.tree(), state.default_tree(&Context::Session(login())));
    send(&mut state, LayoutAction::SetPolicy(Policy::Fixed));
    assert_eq!(state.tree(), state.default_tree(&Context::Session(login())));
    send(&mut state, LayoutAction::SetPolicy(Policy::Free));
    assert_eq!(state.tree(), arranged);
}

#[test]
fn each_screen_keeps_its_own_layout() {
    let mut state = on_login(Policy::Simple);
    send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    let arranged = state.tree();
    send(&mut state, LayoutAction::Enter(Context::Home));
    assert_eq!(kinds(&state), [PaneKind::Home]);
    send(&mut state, LayoutAction::Enter(Context::Session(login())));
    assert_eq!(state.tree(), arranged);
}

#[test]
fn a_saved_layout_restores_with_its_focus_and_new_ids_stay_unique() {
    let mut state = on_login(Policy::Free);
    send(&mut state, LayoutAction::SplitFocused(Axis::Horizontal));
    send(&mut state, LayoutAction::Open(PaneKind::Journal));
    let arranged = state.tree();
    let focus = state.focus();
    let saved = state.saved();

    let mut restored = Harness(TestStore::new(LayoutState::default(), Layout).non_exhaustive());
    send(&mut restored, LayoutAction::Restore(saved));
    send(&mut restored, LayoutAction::Show(Arc::new(fixture::standard())));
    send(&mut restored, LayoutAction::Enter(Context::Session(login())));
    assert_eq!(restored.policy, Policy::Free);
    assert_eq!(restored.tree(), arranged);
    assert_eq!(restored.focus(), focus);

    send(&mut restored, LayoutAction::SplitFocused(Axis::Vertical));
    let mut ids: Vec<PaneId> = restored.tree().panes().iter().map(|pane| pane.id).collect();
    let count = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count);
}

#[test]
fn the_panes_sidebar_lists_the_session_first_then_everywhere_then_channels() {
    let state = on_login(Policy::Simple);
    let groups = state.catalogue();
    assert_eq!(groups[0].kinds[0], PaneKind::Agent(login()));
    assert!(groups[0].kinds.contains(&PaneKind::Diff(login())));
    assert!(groups[0].kinds.contains(&PaneKind::Shell(login(), 1)));
    assert!(groups[1].kinds.contains(&PaneKind::Inbox));
    assert!(groups.len() > 2);
}
