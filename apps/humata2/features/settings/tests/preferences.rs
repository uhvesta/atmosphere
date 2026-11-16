//! Settings clamp usable sizes and retain choices independently of sections.
use humata2_clients::{Clients, fixture};
use humata2_settings::{Section, Settings, SettingsAction as A, SettingsState, Theme};
use std::sync::Arc;
use tca::{TestClock, TestStore};
#[test]
fn preferences_change_without_losing_other_choices() {
    let (clients, _) = Clients::fake(fixture::design(), Arc::new(TestClock::new()));
    let mut store = TestStore::new(SettingsState::default(), Settings::new(clients));
    store.send(A::SetTheme(Theme::Light), |s| {
        s.preferences.theme = Theme::Light
    });
    store.send(A::FontSize(100), |s| s.preferences.font_size = 20);
    store.send(A::FontSize(-100), |s| s.preferences.font_size = 10);
    store.send(A::TogglePinned, |s| s.preferences.show_pinned = false);
    store.send(A::SplitGroups(false), |s| {
        s.preferences.split_groups = false
    });
    store.send(A::Section(Section::Agents), |s| s.section = Section::Agents);
    store.send(A::ToggleWrap, |s| s.preferences.wrap_diffs = true);
    store.finish();
}
