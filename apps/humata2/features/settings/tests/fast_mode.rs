//! FAST (D787, D887): the fast mode switch is on by default unless
//! `HUMATA2_FAST_MODE` turns it off, toggles, and is found by the palette
//! in General.
use humata2_clients::fixture;
use humata2_clients::daemon::TodoAutomation;
use humata2_settings::TodoAutomationAction;
use humata2_settings::{
    Preferences, Row, Section, Settings, SettingsAction as A, SettingsState, fast_mode_default,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store() -> TestStore<Settings> {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    TestStore::new(SettingsState::default(), Settings::new(clients))
}

#[test]
fn it_is_on_unless_the_environment_turns_it_off() {
    assert!(fast_mode_default(None));
    assert!(fast_mode_default(Some("")));
    assert!(fast_mode_default(Some("maybe")));
    for on in ["1", "true", "ON", " on "] {
        assert!(fast_mode_default(Some(on)), "{on:?} keeps it on");
    }
    for off in ["0", "false", "off"] {
        assert!(!fast_mode_default(Some(off)), "{off:?} turns it off");
    }
}

#[test]
fn deserialized_preferences_default_fast_mode_to_true_and_preserve_false() {
    let omitted: Preferences = serde_json::from_str(r#"{"font_size":14}"#).unwrap();
    assert_eq!(omitted.font_size, 14);
    assert!(omitted.fast_mode);

    let explicit_false: Preferences = serde_json::from_str(r#"{"fast_mode":false}"#).unwrap();
    assert!(!explicit_false.fast_mode);
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
    store.receive(A::Todo(TodoAutomationAction::Loaded(Ok(TodoAutomation::default()))), |s| {
        s.todo.section = Some(TodoAutomation::default());
    });
    store.receive_matching(
        "general loaded",
        |a| matches!(a, A::General(humata2_settings::GeneralAction::Loaded(Ok(_)))),
        |s| {
            s.general.settings = Some(humata2_clients::daemon::FullSettings::default());
            s.general.terminal_font_draft = "Menlo".to_owned();
            s.general.scrollback_draft = "10000".to_owned();
            s.general.archive_pr_days_draft = "0".to_owned();
            s.general.archive_idle_days_draft = "0".to_owned();
        },
    );
    store.finish();
}
