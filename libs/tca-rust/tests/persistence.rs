use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tca::{
    Codec, Effect, FileStorage, MemoryStorage, PersistError, Persistence, Persisting, Reducer,
    Storage, StorageError, TestClock, TestStore,
};

const DEBOUNCE: Duration = Duration::from_millis(500);

/// What's saved: the sidebar's width and the folders folded.
#[derive(Clone, Debug, Default, PartialEq)]
struct Layout {
    sidebar_width: u32,
    folded: Vec<String>,
}

/// A window: its layout is saved, what's hovered isn't.
#[derive(Clone, Debug, Default, PartialEq)]
struct Window {
    layout: Layout,
    hovered: bool,
}

#[derive(Debug, PartialEq)]
enum WindowAction {
    Resize(u32),
    Fold(String),
    Hover(bool),
    SaveFailed(String),
}

struct Windowing;

impl Reducer for Windowing {
    type State = Window;
    type Action = WindowAction;

    fn reduce(&self, window: &mut Window, action: WindowAction) -> Effect<WindowAction> {
        match action {
            WindowAction::Resize(width) => window.layout.sidebar_width = width,
            WindowAction::Fold(folder) => window.layout.folded.push(folder),
            WindowAction::Hover(hovered) => window.hovered = hovered,
            WindowAction::SaveFailed(_) => {}
        }
        Effect::none()
    }
}

/// Saves a layout as text: `layout <width>`, then a folded folder a line.
struct LayoutCodec;

#[derive(Debug)]
struct NotALayout;

impl fmt::Display for NotALayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a saved layout")
    }
}

impl std::error::Error for NotALayout {}

impl Codec<Layout> for LayoutCodec {
    type Error = NotALayout;

    fn encode(&self, layout: &Layout) -> Result<Vec<u8>, NotALayout> {
        let mut text = format!("layout {}", layout.sidebar_width);
        for folder in &layout.folded {
            text.push('\n');
            text.push_str(folder);
        }
        Ok(text.into_bytes())
    }

    fn decode(&self, bytes: &[u8]) -> Result<Layout, NotALayout> {
        let text = std::str::from_utf8(bytes).map_err(|_| NotALayout)?;
        let mut lines = text.lines();
        let sidebar_width = lines
            .next()
            .and_then(|line| line.strip_prefix("layout "))
            .and_then(|width| width.parse().ok())
            .ok_or(NotALayout)?;
        Ok(Layout {
            sidebar_width,
            folded: lines.map(str::to_owned).collect(),
        })
    }
}

type Persisted = Persisting<Windowing, Layout, LayoutCodec>;

fn persistence(storage: Arc<dyn Storage>, clock: &TestClock) -> Persistence<Layout, LayoutCodec> {
    Persistence::new(
        "layout",
        storage,
        LayoutCodec,
        Arc::new(clock.clone()),
        DEBOUNCE,
    )
}

/// A window store as the app makes one at start: restored from storage.
fn window(
    storage: Arc<dyn Storage>,
    clock: &TestClock,
) -> (TestStore<Persisted>, Option<PersistError<NotALayout>>) {
    let persistence = persistence(storage, clock);
    let restored = persistence.restore();
    let reducer = Persisting::new(
        Windowing,
        persistence,
        |window: &Window| window.layout.clone(),
        |error| WindowAction::SaveFailed(error.to_string()),
    );
    let state = Window {
        layout: restored.value,
        hovered: false,
    };
    (TestStore::new(state, reducer), restored.error)
}

/// Counts the saves that reach storage.
#[derive(Clone, Default)]
struct CountingStorage {
    saved: MemoryStorage,
    saves: Arc<AtomicUsize>,
}

impl Storage for CountingStorage {
    fn load(&self, key: &str) -> Result<Option<Vec<u8>>, StorageError> {
        self.saved.load(key)
    }
    fn save(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        self.saved.save(key, bytes)
    }
    fn remove(&self, key: &str) -> Result<(), StorageError> {
        self.saved.remove(key)
    }
}

/// Storage whose disk is gone.
struct BrokenStorage;

impl Storage for BrokenStorage {
    fn load(&self, _: &str) -> Result<Option<Vec<u8>>, StorageError> {
        Err(StorageError::Read {
            path: "layout".into(),
            source: std::io::Error::other("the disk is gone"),
        })
    }
    fn save(&self, _: &str, _: &[u8]) -> Result<(), StorageError> {
        Err(StorageError::Write {
            path: "layout".into(),
            source: std::io::Error::other("the disk is gone"),
        })
    }
    fn remove(&self, _: &str) -> Result<(), StorageError> {
        Ok(())
    }
}

fn folder(name: &str) -> PathBuf {
    let base = std::env::var_os("TEST_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let folder = base.join(format!("tca-persistence-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

#[test]
fn a_new_store_from_the_same_storage_restores_what_the_last_one_saved() {
    let storage = MemoryStorage::new();
    let clock = TestClock::new();
    let (mut store, error) = window(Arc::new(storage.clone()), &clock);
    assert!(error.is_none());
    store.send(WindowAction::Resize(300), |w| w.layout.sidebar_width = 300);
    store.send(WindowAction::Fold("src".into()), |w| {
        w.layout.folded = vec!["src".into()]
    });
    store.send(WindowAction::Hover(true), |w| w.hovered = true);
    clock.advance(DEBOUNCE);
    store.finish();

    // The app starts again.
    let (store, error) = window(Arc::new(storage), &clock);
    assert!(error.is_none());
    assert_eq!(
        *store.state(),
        Window {
            layout: Layout {
                sidebar_width: 300,
                folded: vec!["src".into()],
            },
            hovered: false,
        }
    );
}

#[test]
fn nothing_saved_yet_restores_the_default_without_an_error() {
    let clock = TestClock::new();
    let (store, error) = window(Arc::new(MemoryStorage::new()), &clock);
    assert!(error.is_none());
    assert_eq!(*store.state(), Window::default());
}

#[test]
fn saves_wait_for_changes_to_pause_and_write_once() {
    let storage = CountingStorage::default();
    let clock = TestClock::new();
    let (mut store, _) = window(Arc::new(storage.clone()), &clock);
    store.send(WindowAction::Resize(1), |w| w.layout.sidebar_width = 1);
    clock.advance(Duration::from_millis(300));
    store.send(WindowAction::Resize(2), |w| w.layout.sidebar_width = 2);
    clock.advance(Duration::from_millis(300));
    store.settle();
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    clock.advance(Duration::from_millis(200));
    store.settle();
    assert_eq!(storage.saves.load(Ordering::SeqCst), 1);
    let saved = LayoutCodec.decode(&storage.load("layout").unwrap().unwrap()).unwrap();
    assert_eq!(saved.sidebar_width, 2);
    store.finish();
}

#[test]
fn a_change_to_what_isnt_saved_doesnt_save() {
    let storage = CountingStorage::default();
    let clock = TestClock::new();
    let (mut store, _) = window(Arc::new(storage.clone()), &clock);
    store.send(WindowAction::Hover(true), |w| w.hovered = true);
    clock.advance(DEBOUNCE);
    store.settle();
    assert_eq!(storage.saves.load(Ordering::SeqCst), 0);
    // Nothing waits to save, so nothing is left running.
    store.finish();
}

#[test]
fn a_corrupt_file_restores_the_default_with_the_error_and_the_next_save_replaces_it() {
    let folder = folder("corrupt");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("layout"), b"\xff\x00 half a lay").unwrap();
    let storage = FileStorage::new(&folder);
    let clock = TestClock::new();

    let (mut store, error) = window(Arc::new(storage.clone()), &clock);
    assert!(matches!(error, Some(PersistError::Decode { .. })));
    assert_eq!(*store.state(), Window::default());

    store.send(WindowAction::Resize(240), |w| w.layout.sidebar_width = 240);
    clock.advance(DEBOUNCE);
    store.finish();

    let (store, error) = window(Arc::new(storage), &clock);
    assert!(error.is_none());
    assert_eq!(store.state().layout.sidebar_width, 240);
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn storage_that_cant_be_read_restores_the_default_with_the_error() {
    let clock = TestClock::new();
    let (store, error) = window(Arc::new(BrokenStorage), &clock);
    assert!(matches!(error, Some(PersistError::Load { .. })));
    assert_eq!(*store.state(), Window::default());
}

#[test]
fn a_save_that_fails_sends_why() {
    let clock = TestClock::new();
    let (mut store, _) = window(Arc::new(BrokenStorage), &clock);
    store.send(WindowAction::Resize(10), |w| w.layout.sidebar_width = 10);
    clock.advance(DEBOUNCE);
    store.receive_matching(
        "SaveFailed",
        |action| matches!(action, WindowAction::SaveFailed(_)),
        |_| {},
    );
}

#[test]
fn a_file_save_replaces_the_old_copy_whole_and_leaves_no_temporary_files() {
    let folder = folder("atomic");
    let storage = FileStorage::new(&folder);
    assert_eq!(storage.load("tabs").unwrap(), None);
    storage.save("tabs", b"one").unwrap();
    storage.save("tabs", b"two, longer").unwrap();
    assert_eq!(storage.load("tabs").unwrap().as_deref(), Some(&b"two, longer"[..]));
    let names: Vec<String> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["tabs"]);
    storage.remove("tabs").unwrap();
    assert_eq!(storage.load("tabs").unwrap(), None);
    // Removing what isn't there is no error.
    storage.remove("tabs").unwrap();
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn a_key_that_isnt_a_plain_file_name_is_refused() {
    let folder = folder("keys");
    let storage = FileStorage::new(&folder);
    for key in ["", "../escape", "a/b", ".hidden"] {
        assert!(matches!(
            storage.save(key, b"x"),
            Err(StorageError::InvalidKey { .. })
        ));
    }
    let _ = std::fs::remove_dir_all(&folder);
}

#[test]
fn saving_now_writes_at_once() {
    let storage = MemoryStorage::new();
    let clock = TestClock::new();
    let persistence = persistence(Arc::new(storage.clone()), &clock);
    let layout = Layout {
        sidebar_width: 9,
        folded: Vec::new(),
    };
    persistence.save_now(&layout).unwrap();
    assert_eq!(persistence.restore().value, layout);
}
