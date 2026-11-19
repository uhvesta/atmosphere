//! The "Option acts as Alt" switch (D420): off at first, toggles, and is
//! found by the palette in Keyboard.
use humata2_clients::{Clients, fixture};
use humata2_settings::{Preferences, Row, Section, Settings, SettingsAction as A, SettingsState};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store() -> TestStore<Settings> {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    TestStore::new(SettingsState::default(), Settings::new(clients))
}

#[test]
fn option_types_the_layouts_characters_by_default() {
    assert!(!Preferences::default().option_as_alt);
}

#[test]
fn toggling_turns_it_on_and_back_off() {
    let mut store = store();
    store.send(A::ToggleOptionAsAlt, |s| s.preferences.option_as_alt = true);
    store.send(A::ToggleOptionAsAlt, |s| s.preferences.option_as_alt = false);
    store.finish();
}

#[test]
fn the_row_is_in_keyboard_and_reachable_from_the_palette() {
    assert_eq!(Row::OptionAsAlt.section(), Section::Keyboard);
    assert!(Row::ALL.contains(&Row::OptionAsAlt));
    let mut store = store();
    store.send(A::Reveal(Row::OptionAsAlt), |s| {
        s.section = Section::Keyboard;
        s.revealed = Some(Row::OptionAsAlt);
    });
    store.finish();
}
