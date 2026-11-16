//! A preferences screen whose choices outlive the app, while the screen's
//! own state doesn't. `Persisting` wraps the screen's reducer and saves
//! only the `Preferences` part, once changes pause. Which section is open
//! and what's being typed are never saved. `MemoryStorage` stands in for
//! the disk in tests: a second store made from a clone of it restores what
//! the first one saved, as after a restart.
//!
//! Patterns: `Persisting` saving part of the state; `Persistence::restore`
//! at startup; a `Codec`; a failed save as an action.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use tca::{
    Codec, Effect, MemoryStorage, PersistError, Persistence, Persisting, Reducer, Storage,
    StorageError, TestClock, TestStore,
};

/// A burst of changes, such as dragging a slider, is written once.
const SAVE_AFTER: Duration = Duration::from_millis(500);
const KEY: &str = "preferences";

/// What's saved. Kept small, because `Persisting` compares it before and
/// after every action.
#[derive(Clone, Debug, PartialEq)]
struct Preferences {
    font_size: u32,
    sync_on_cellular: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            font_size: 14,
            sync_on_cellular: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Section {
    #[default]
    Reading,
    Sync,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct PreferencesScreen {
    preferences: Preferences,
    /// The screen's own state: gone when it closes, so not saved.
    section: Section,
    /// The last save that failed, for a banner.
    save_failed: Option<SaveFailure>,
}

/// Why a save failed, as data a view can show. The error itself isn't
/// kept in the action: it can't be compared, and the view needs only the
/// kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveFailure {
    Encode,
    Storage,
}

impl<E: std::error::Error + Send + Sync + 'static> From<PersistError<E>> for SaveFailure {
    fn from(error: PersistError<E>) -> Self {
        match error {
            PersistError::Encode { .. } => Self::Encode,
            _ => Self::Storage,
        }
    }
}

#[derive(Debug, PartialEq)]
enum PreferencesAction {
    Open(Section),
    FontSize(u32),
    SyncOnCellular(bool),
    SaveFailed(SaveFailure),
}

/// The screen's reducer knows nothing of saving.
struct Editing;

impl Reducer for Editing {
    type State = PreferencesScreen;
    type Action = PreferencesAction;

    fn reduce(&self, screen: &mut PreferencesScreen, action: PreferencesAction) -> Effect<PreferencesAction> {
        match action {
            PreferencesAction::Open(section) => screen.section = section,
            PreferencesAction::FontSize(size) => screen.preferences.font_size = size,
            PreferencesAction::SyncOnCellular(on) => screen.preferences.sync_on_cellular = on,
            PreferencesAction::SaveFailed(failure) => screen.save_failed = Some(failure),
        }
        Effect::none()
    }
}

/// Saves preferences as `key=value` lines. Unknown keys are skipped, so a
/// newer version's file still loads; a bad value fails the whole read.
struct PreferencesCodec;

#[derive(Debug)]
struct NotPreferences {
    line: String,
}

impl fmt::Display for NotPreferences {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} isn't a saved preference", self.line)
    }
}

impl std::error::Error for NotPreferences {}

impl Codec<Preferences> for PreferencesCodec {
    type Error = NotPreferences;

    fn encode(&self, preferences: &Preferences) -> Result<Vec<u8>, NotPreferences> {
        Ok(format!(
            "font_size={}\nsync_on_cellular={}\n",
            preferences.font_size, preferences.sync_on_cellular
        )
        .into_bytes())
    }

    fn decode(&self, bytes: &[u8]) -> Result<Preferences, NotPreferences> {
        let text = String::from_utf8_lossy(bytes);
        let mut preferences = Preferences::default();
        for line in text.lines().filter(|line| !line.is_empty()) {
            let bad = || NotPreferences { line: line.into() };
            let (key, value) = line.split_once('=').ok_or_else(bad)?;
            match key {
                "font_size" => preferences.font_size = value.parse().map_err(|_| bad())?,
                "sync_on_cellular" => {
                    preferences.sync_on_cellular = value.parse().map_err(|_| bad())?;
                }
                _ => {}
            }
        }
        Ok(preferences)
    }
}

type Screen = Persisting<Editing, Preferences, PreferencesCodec>;

fn persistence(storage: Arc<dyn Storage>, clock: &TestClock) -> Persistence<Preferences, PreferencesCodec> {
    Persistence::new(KEY, storage, PreferencesCodec, Arc::new(clock.clone()), SAVE_AFTER)
}

/// The screen as the app makes it: preferences restored, then the reducer
/// wrapped so it saves them.
fn screen(storage: Arc<dyn Storage>, clock: &TestClock) -> TestStore<Screen> {
    let persistence = persistence(storage, clock);
    let state = PreferencesScreen {
        preferences: persistence.restore().value,
        ..PreferencesScreen::default()
    };
    let reducer = Persisting::new(
        Editing,
        persistence,
        |screen: &PreferencesScreen| screen.preferences.clone(),
        |error| PreferencesAction::SaveFailed(error.into()),
    );
    TestStore::new(state, reducer)
}

fn saved(storage: &MemoryStorage) -> Option<String> {
    storage
        .load(KEY)
        .ok()
        .flatten()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn opening_a_section_saves_nothing() {
    let storage = MemoryStorage::new();
    let clock = TestClock::new();
    let mut store = screen(Arc::new(storage.clone()), &clock);
    store.send(PreferencesAction::Open(Section::Sync), |s| s.section = Section::Sync);
    clock.advance(SAVE_AFTER);
    store.settle();
    assert_eq!(saved(&storage), None);
}

#[test]
fn a_burst_of_changes_is_saved_once_after_it_pauses() {
    let storage = MemoryStorage::new();
    let clock = TestClock::new();
    let mut store = screen(Arc::new(storage.clone()), &clock);
    for size in [15, 16, 17, 18] {
        store.send(PreferencesAction::FontSize(size), |s| s.preferences.font_size = size);
        clock.advance(SAVE_AFTER / 4);
        store.settle();
        // Still changing: nothing written yet.
        assert_eq!(saved(&storage), None);
    }
    clock.advance(SAVE_AFTER);
    store.settle();
    assert_eq!(
        saved(&storage).as_deref(),
        Some("font_size=18\nsync_on_cellular=false\n")
    );
}

#[test]
fn preferences_saved_before_a_restart_are_restored_and_the_rest_starts_new() {
    let storage = MemoryStorage::new();
    let clock = TestClock::new();
    let mut store = screen(Arc::new(storage.clone()), &clock);
    store.send(PreferencesAction::SyncOnCellular(true), |s| {
        s.preferences.sync_on_cellular = true;
    });
    store.send(PreferencesAction::Open(Section::Sync), |s| s.section = Section::Sync);
    clock.advance(SAVE_AFTER);
    store.finish();

    let restarted = screen(Arc::new(storage), &clock);
    assert_eq!(
        *restarted.state(),
        PreferencesScreen {
            preferences: Preferences {
                font_size: 14,
                sync_on_cellular: true,
            },
            section: Section::Reading,
            save_failed: None,
        }
    );
}

#[test]
fn preferences_that_cannot_be_read_start_as_new_and_say_why() {
    let storage = MemoryStorage::new();
    storage.save(KEY, b"font_size=huge\n").unwrap();
    let restored = persistence(Arc::new(storage), &TestClock::new()).restore();
    assert_eq!(restored.value, Preferences::default());
    assert!(matches!(restored.error, Some(PersistError::Decode { .. })));
}

/// A disk that's full.
struct FullDisk;

impl Storage for FullDisk {
    fn load(&self, _: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Ok(None)
    }

    fn save(&self, key: &str, _: &[u8]) -> Result<(), StorageError> {
        Err(StorageError::Write {
            path: key.into(),
            source: std::io::Error::other("no space left on device"),
        })
    }

    fn remove(&self, _: &str) -> Result<(), StorageError> {
        Ok(())
    }
}

#[test]
fn a_save_that_fails_comes_back_as_an_action_for_the_screen_to_show() {
    let clock = TestClock::new();
    let mut store = screen(Arc::new(FullDisk), &clock);
    store.send(PreferencesAction::FontSize(20), |s| s.preferences.font_size = 20);
    clock.advance(SAVE_AFTER);
    store.receive(PreferencesAction::SaveFailed(SaveFailure::Storage), |s| {
        s.save_failed = Some(SaveFailure::Storage);
    });
}
