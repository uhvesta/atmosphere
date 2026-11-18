//! D793: Settings › Layout › Desk sessions in sidebar caps the rows under
//! the sidebar's Desk. It is 5 unless changed, its pop-up picks 3, 4, 5, 8
//! or All and closes, and a saved value out of range still gives a usable cap.
use humata2_clients::{Clients, fixture};
use humata2_settings::{
    DeskRows, Preferences, Row, Section, Settings, SettingsAction as A, SettingsState,
};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn store() -> TestStore<Settings> {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    TestStore::new(SettingsState::default(), Settings::new(clients))
}

#[test]
fn the_sidebar_lists_five_desk_sessions_unless_changed() {
    assert_eq!(Preferences::default().desk_rows.limit(), Some(5));
}

#[test]
fn the_pop_up_offers_three_four_five_eight_and_all() {
    let limits: Vec<Option<usize>> = DeskRows::OPTIONS.iter().map(|rows| rows.limit()).collect();
    assert_eq!(limits, [Some(3), Some(4), Some(5), Some(8), None]);
}

#[test]
fn picking_from_the_pop_up_sets_the_cap_and_closes_it() {
    let mut store = store();
    for rows in DeskRows::OPTIONS {
        store.send(A::DeskRowsMenu, |s| s.desk_rows_menu = true);
        store.send(A::SetDeskRows(rows), |s| {
            s.preferences.desk_rows = rows;
            s.desk_rows_menu = false;
        });
    }
    store.finish();
}

#[test]
fn switching_section_closes_the_pop_up() {
    let mut store = store();
    store.send(A::DeskRowsMenu, |s| s.desk_rows_menu = true);
    store.send(A::Section(Section::General), |s| {
        s.section = Section::General;
        s.desk_rows_menu = false;
    });
    store.finish();
}

#[test]
fn a_saved_cap_out_of_range_is_brought_into_one_to_twenty() {
    assert_eq!(DeskRows::Count(0).limit(), Some(1));
    assert_eq!(DeskRows::Count(99).limit(), Some(20));
}

#[test]
fn preferences_saved_before_the_setting_existed_read_as_five() {
    let preferences: Preferences =
        serde_json::from_str(r#"{"font_size":14}"#).expect("older preferences still read");
    assert_eq!(preferences.desk_rows.limit(), Some(5));
    assert_eq!(preferences.font_size, 14);
}

#[test]
fn the_row_is_in_layout_and_reachable_from_the_palette() {
    assert_eq!(Row::DeskRowsInSidebar.section(), Section::Layout);
    assert!(Row::ALL.contains(&Row::DeskRowsInSidebar));
    let mut store = store();
    store.send(A::Reveal(Row::DeskRowsInSidebar), |s| {
        s.section = Section::Layout;
        s.revealed = Some(Row::DeskRowsInSidebar);
    });
    store.finish();
}
