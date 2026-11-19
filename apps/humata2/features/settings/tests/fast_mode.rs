//! FAST (D787): the fast mode switch is off unless `HUMATA2_FAST_MODE`
//! says otherwise, toggles, and is found by the palette in General.
use humata2_clients::{Clients, fixture};
use humata2_settings::{
    Row, Section, Settings, SettingsAction as A, SettingsState, fast_mode_default,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store() -> TestStore<Settings> {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    TestStore::new(SettingsState::default(), Settings::new(clients))
}

#[test]
fn it_is_off_unless_the_environment_turns_it_on() {
    assert!(!fast_mode_default(None));
    assert!(!fast_mode_default(Some("")));
    assert!(!fast_mode_default(Some("maybe")));
    for on in ["1", "true", "ON", " on "] {
        assert!(fast_mode_default(Some(on)), "{on:?} turns it on");
    }
    for off in ["0", "false", "off"] {
        assert!(!fast_mode_default(Some(off)), "{off:?} keeps it off");
    }
}

#[test]
fn toggling_flips_it_and_back() {
    let mut store = store();
    let was = SettingsState::default().preferences.fast_mode;
    store.send(A::ToggleFastMode, |s| s.preferences.fast_mode = !was);
    store.send(A::ToggleFastMode, |s| s.preferences.fast_mode = was);
    store.finish();
}

#[test]
fn the_row_is_in_general_and_reachable_from_the_palette() {
    assert_eq!(Row::FastMode.section(), Section::General);
    assert!(Row::ALL.contains(&Row::FastMode));
    let mut store = store();
    store.send(A::Reveal(Row::FastMode), |s| {
        s.section = Section::General;
        s.revealed = Some(Row::FastMode);
    });
    store.finish();
}
