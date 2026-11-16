//! Appearance settings that three features use: the settings screen that
//! changes them, an article reader that lays its text out from the font
//! size, and a sidebar that reads them only when asked. Each holds a clone
//! of one `Shared<Appearance>`, so a change made on the settings screen is
//! seen by the other two at once. The reader also needs to redo its
//! layout when the size changes, so it follows `changes` as an effect.
//!
//! Patterns: `Shared<T>` read by several features; a change seen by all;
//! `Shared::changes` as a cancellable effect.

use std::time::Duration;

use tca::{Effect, Reducer, Shared, TestStore};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Theme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Debug, PartialEq)]
struct Appearance {
    font_size: u32,
    theme: Theme,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            font_size: 14,
            theme: Theme::Light,
        }
    }
}

// The settings screen: it owns no copy of the settings, only the shared
// value, so there's nothing to keep in step.

#[derive(Clone, Debug, PartialEq)]
struct SettingsScreen {
    appearance: Shared<Appearance>,
}

#[derive(Debug, PartialEq)]
enum SettingsAction {
    FontSize(u32),
    Theme(Theme),
}

struct Settings;

impl Reducer for Settings {
    type State = SettingsScreen;
    type Action = SettingsAction;

    fn reduce(&self, screen: &mut SettingsScreen, action: SettingsAction) -> Effect<SettingsAction> {
        match action {
            SettingsAction::FontSize(size) => {
                screen.appearance.update(|a| a.font_size = size.clamp(10, 32));
            }
            SettingsAction::Theme(theme) => screen.appearance.update(|a| a.theme = theme),
        }
        Effect::none()
    }
}

// The reader keeps a layout worked out from the font size. It's state, not
// derived on demand, because working it out is the costly part.

/// How wide a line of text the reader lays out at a font size.
fn characters_per_line(font_size: u32) -> u32 {
    960 / font_size
}

#[derive(Clone, Debug, PartialEq)]
struct Reader {
    appearance: Shared<Appearance>,
    per_line: u32,
}

#[derive(Debug, PartialEq)]
enum ReaderAction {
    Appeared,
    Disappeared,
    FontSizeChanged(u32),
}

/// The reader follows the settings only while it's on screen.
const FOLLOW: &str = "follow-appearance";

struct Reading;

impl Reducer for Reading {
    type State = Reader;
    type Action = ReaderAction;

    fn reduce(&self, reader: &mut Reader, action: ReaderAction) -> Effect<ReaderAction> {
        match action {
            ReaderAction::Appeared => {
                // What it is now, then each change from here on. It never
                // ends of itself, so it's cancellable.
                reader.per_line = characters_per_line(reader.appearance.get().font_size);
                reader
                    .appearance
                    .changes(|a| ReaderAction::FontSizeChanged(a.font_size))
                    .cancellable(FOLLOW, true)
            }
            ReaderAction::Disappeared => Effect::cancel(FOLLOW),
            ReaderAction::FontSizeChanged(size) => {
                reader.per_line = characters_per_line(size);
                Effect::none()
            }
        }
    }
}

// The sidebar reads the theme when it needs it and keeps nothing derived.

#[derive(Clone, Debug, PartialEq)]
struct Sidebar {
    appearance: Shared<Appearance>,
}

impl Sidebar {
    fn dark(&self) -> bool {
        self.appearance.with(|a| a.theme == Theme::Dark)
    }
}

// The app holds the three. Building them from one shared value is what
// makes them see the same settings.

#[derive(Clone, Debug, PartialEq)]
struct App {
    settings: SettingsScreen,
    reader: Reader,
    sidebar: Sidebar,
}

impl App {
    fn new(appearance: &Shared<Appearance>) -> Self {
        Self {
            settings: SettingsScreen {
                appearance: appearance.clone(),
            },
            reader: Reader {
                appearance: appearance.clone(),
                per_line: 0,
            },
            sidebar: Sidebar {
                appearance: appearance.clone(),
            },
        }
    }
}

#[derive(Debug, PartialEq)]
enum AppAction {
    Settings(SettingsAction),
    Reader(ReaderAction),
}

struct Apping;

impl Reducer for Apping {
    type State = App;
    type Action = AppAction;

    fn reduce(&self, app: &mut App, action: AppAction) -> Effect<AppAction> {
        match action {
            AppAction::Settings(action) => Settings
                .reduce(&mut app.settings, action)
                .map(AppAction::Settings),
            AppAction::Reader(action) => Reading
                .reduce(&mut app.reader, action)
                .map(AppAction::Reader),
        }
    }
}

fn app() -> (TestStore<Apping>, Shared<Appearance>) {
    let appearance = Shared::new(Appearance::default());
    let store = TestStore::new(App::new(&appearance), Apping);
    (store, appearance)
}

#[test]
fn a_change_on_the_settings_screen_is_seen_by_every_feature() {
    let (mut store, appearance) = app();
    // The expected state shares the value with the real one, so a change
    // to it isn't a change of state. Check it by reading it.
    store.send(AppAction::Settings(SettingsAction::Theme(Theme::Dark)), |_| {});
    assert!(store.state().sidebar.dark());
    assert_eq!(store.state().reader.appearance.get().theme, Theme::Dark);
    assert_eq!(appearance.get().theme, Theme::Dark);
}

#[test]
fn the_reader_lays_out_again_when_the_font_size_changes() {
    let (mut store, _appearance) = app();
    store.send(AppAction::Reader(ReaderAction::Appeared), |s| {
        s.reader.per_line = 68;
    });
    store.send(AppAction::Settings(SettingsAction::FontSize(20)), |_| {});
    store.receive(AppAction::Reader(ReaderAction::FontSizeChanged(20)), |s| {
        s.reader.per_line = 48;
    });
    // Out of range sizes are kept in it, and the reader hears the kept one.
    store.send(AppAction::Settings(SettingsAction::FontSize(99)), |_| {});
    store.receive(AppAction::Reader(ReaderAction::FontSizeChanged(32)), |s| {
        s.reader.per_line = 30;
    });
    store.send(AppAction::Reader(ReaderAction::Disappeared), |_| {});
}

#[test]
fn a_change_made_elsewhere_reaches_the_reader_too() {
    let (mut store, appearance) = app();
    store.send(AppAction::Reader(ReaderAction::Appeared), |s| {
        s.reader.per_line = 68;
    });
    // Another window, say, holding its own clone.
    appearance.update(|a| a.font_size = 16);
    store.receive(AppAction::Reader(ReaderAction::FontSizeChanged(16)), |s| {
        s.reader.per_line = 60;
    });
    store.send(AppAction::Reader(ReaderAction::Disappeared), |_| {});
}

#[test]
fn a_reader_off_screen_follows_nothing() {
    let (mut store, appearance) = app();
    store.send(AppAction::Reader(ReaderAction::Appeared), |s| {
        s.reader.per_line = 68;
    });
    store.send(AppAction::Reader(ReaderAction::Disappeared), |_| {});
    appearance.update(|a| a.font_size = 24);
    // Nothing was received and nothing still runs.
    let store = store.with_timeout(Duration::from_millis(100));
    store.finish();
}
