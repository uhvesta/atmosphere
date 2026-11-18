//! ⌘0, ⌘L and ⌥⌘M (NV-6, MV-1): a folded sidebar docks again when the
//! window has room and comes over the page only when it hasn't; ⌥⌘M with
//! no session open says why nothing moves; a Desk dialog or Add machine
//! keeps the keyboard from a terminal.
use std::sync::Arc;

use humata2_app::layout::{Layout, PaneId, Reveal};
use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::{Clients, fixture};
use humata2_palette::{Command, Delegate, PaletteAction};
use tca::{TestClock, TestStore};

const WIDE: f32 = 1600.;
const NARROW: f32 = 500.;

#[test]
fn a_shown_sidebar_needs_no_revealing() {
    let layout = Layout::default();
    assert_eq!(layout.reveal_sidebar(WIDE, true, true), None);
}

#[test]
fn a_sidebar_folded_in_a_wide_window_docks_again() {
    let mut layout = Layout::default();
    layout.toggle(PaneId::Sidebar);
    assert_eq!(layout.reveal_sidebar(WIDE, true, true), Some(Reveal::Dock));
    assert_eq!(layout.reveal_sidebar(WIDE, false, false), Some(Reveal::Dock));
}

#[test]
fn a_window_too_narrow_for_the_sidebar_shows_it_over_the_page() {
    let layout = Layout::default();
    assert_eq!(layout.reveal_sidebar(NARROW, true, true), Some(Reveal::Overlay));
    let mut folded = Layout::default();
    folded.toggle(PaneId::Sidebar);
    assert_eq!(folded.reveal_sidebar(NARROW, true, true), Some(Reveal::Overlay));
}

#[test]
fn move_with_no_session_open_says_so_and_moves_nothing() {
    let workspace = fixture::design();
    let (clients, fakes) = Clients::fake(workspace, Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    store.send(A::Loaded(Ok(fakes.daemon.workspace())), |_| {});
    store.send(A::Switch(Route::Home), |_| {});
    store.settle();
    let before = store.state().toasts.len();
    let run = PaletteAction::Delegate(Delegate::Run(Command::MoveSession));
    store.send(A::Palette(run), |_| {});
    store.settle();
    assert_eq!(store.state().toasts.len(), before + 1, "a toast says why nothing moved");
    assert_eq!(store.state().route, Route::Home);
}

#[test]
fn a_desk_dialog_or_add_machine_holds_the_keys_from_a_terminal() {
    let (clients, _fakes) = Clients::fake(fixture::empty(), Arc::new(TestClock::new()));
    let state = AppState::launch(&clients);
    let mut store = TestStore::new(state, App::new(clients)).non_exhaustive();
    assert!(!store.state().dialog_holds_keys(), "nothing is open at launch");
    store.send(A::StartNewDeskSession, |_| {});
    assert!(store.state().dialog_holds_keys(), "the Desk's New form holds the keys");
    store.send(A::Escape, |_| {});
    store.skip_received();
    assert!(!store.state().dialog_holds_keys(), "Escape gives them back");
    let mut state = AppState::launch(&Clients::fake(fixture::empty(), Arc::new(TestClock::new())).0);
    state.add_machine.open = true;
    assert!(state.dialog_holds_keys(), "the Add machine dialog holds the keys");
}
