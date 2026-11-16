//! A file browser that drills down into folders. The top folder is a plain
//! field; each folder opened from it is a screen on a `StackState`, run
//! with `for_each_in_stack`. Every screen loads its own listing. Popping a
//! screen pops those above it too and cancels their loads, so a listing
//! that comes back late never lands anywhere. A screen opens a subfolder
//! by telling the browser through a delegate action; it can't push onto a
//! stack it doesn't hold.
//!
//! Patterns: `StackState`, `StackAction` and `for_each_in_stack`; popping
//! cancels the effects of the screens above; `Effect::dismiss` from a
//! screen pops it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tca::{
    Clock, Effect, Reducer, StackAction, StackElementId, StackState, TestClock, TestStore,
    for_each_in_stack,
};

#[derive(Clone, Debug, PartialEq)]
enum Entry {
    Folder(String),
    File(String),
}

/// One folder's screen.
#[derive(Clone, Debug, PartialEq)]
struct FolderScreen {
    path: String,
    /// None while loading.
    entries: Option<Vec<Entry>>,
}

impl FolderScreen {
    fn new(path: &str) -> Self {
        Self {
            path: path.into(),
            entries: None,
        }
    }

    fn child(&self, name: &str) -> String {
        match self.path.as_str() {
            "/" => format!("/{name}"),
            path => format!("{path}/{name}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum ScreenAction {
    Load,
    Loaded(Vec<Entry>),
    OpenFolder(String),
    Back,
    Delegate(ScreenDelegate),
}

#[derive(Clone, Debug, PartialEq)]
enum ScreenDelegate {
    Open(String),
}

/// The disk is a map from folder to what's in it; loading takes a moment.
struct Listing {
    disk: Arc<HashMap<String, Vec<Entry>>>,
    clock: Arc<dyn Clock>,
}

impl Reducer for Listing {
    type State = FolderScreen;
    type Action = ScreenAction;

    fn reduce(&self, screen: &mut FolderScreen, action: ScreenAction) -> Effect<ScreenAction> {
        match action {
            ScreenAction::Load => {
                let entries = self.disk.get(&screen.path).cloned().unwrap_or_default();
                let clock = self.clock.clone();
                // The same id on every screen: each stack screen's ids are
                // kept apart, and the root's are its own.
                Effect::future(async move {
                    clock.sleep(Duration::from_secs(1)).await;
                    ScreenAction::Loaded(entries)
                })
                .cancellable("load", true)
            }
            ScreenAction::Loaded(entries) => {
                screen.entries = Some(entries);
                Effect::none()
            }
            ScreenAction::OpenFolder(name) => {
                Effect::send(ScreenAction::Delegate(ScreenDelegate::Open(screen.child(&name))))
            }
            // On a stack this pops the screen. On the root, which nothing
            // presented, it's ignored: there's nowhere to go back to.
            ScreenAction::Back => Effect::dismiss(),
            ScreenAction::Delegate(_) => Effect::none(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Browser {
    root: FolderScreen,
    path: StackState<FolderScreen>,
}

#[derive(Debug, PartialEq)]
enum BrowserAction {
    Root(ScreenAction),
    Path(StackAction<FolderScreen, ScreenAction>),
}

struct Browsing {
    listing: Listing,
}

impl Browsing {
    /// Pushes the folder at `path` and starts its load. The load goes
    /// through `for_each_in_stack`, so it's the new screen's and is
    /// cancelled when that screen is popped.
    fn open(&self, browser: &mut Browser, path: &str) -> Effect<BrowserAction> {
        let id = browser.path.push(FolderScreen::new(path));
        for_each_in_stack(
            &self.listing,
            &mut browser.path,
            StackAction::Element(id, ScreenAction::Load),
        )
        .map(BrowserAction::Path)
    }
}

impl Reducer for Browsing {
    type State = Browser;
    type Action = BrowserAction;

    fn reduce(&self, browser: &mut Browser, action: BrowserAction) -> Effect<BrowserAction> {
        match action {
            BrowserAction::Root(ScreenAction::Delegate(ScreenDelegate::Open(path))) => {
                self.open(browser, &path)
            }
            BrowserAction::Root(action) => self
                .listing
                .reduce(&mut browser.root, action)
                .map(BrowserAction::Root),
            BrowserAction::Path(StackAction::Element(
                _,
                ScreenAction::Delegate(ScreenDelegate::Open(path)),
            )) => self.open(browser, &path),
            BrowserAction::Path(action) => {
                for_each_in_stack(&self.listing, &mut browser.path, action).map(BrowserAction::Path)
            }
        }
    }
}

fn disk() -> HashMap<String, Vec<Entry>> {
    HashMap::from([
        (
            "/".to_owned(),
            vec![Entry::Folder("photos".into()), Entry::File("todo.txt".into())],
        ),
        ("/photos".to_owned(), vec![Entry::Folder("2026".into())]),
        ("/photos/2026".to_owned(), vec![Entry::File("lake.jpg".into())]),
    ])
}

fn browser() -> (TestStore<Browsing>, TestClock) {
    let clock = TestClock::new();
    let state = Browser {
        root: FolderScreen::new("/"),
        path: StackState::new(),
    };
    let reducer = Browsing {
        listing: Listing {
            disk: Arc::new(disk()),
            clock: Arc::new(clock.clone()),
        },
    };
    (TestStore::new(state, reducer), clock)
}

fn element(id: u64, action: ScreenAction) -> BrowserAction {
    BrowserAction::Path(StackAction::Element(StackElementId(id), action))
}

fn opens(path: &str) -> ScreenAction {
    ScreenAction::Delegate(ScreenDelegate::Open(path.into()))
}

fn listed(browser: &mut Browser, id: u64, entries: Vec<Entry>) {
    if let Some(screen) = browser.path.get_mut(StackElementId(id)) {
        screen.entries = Some(entries);
    }
}

#[test]
fn opening_folders_drills_down_and_each_screen_loads_its_own_listing() {
    let (mut store, clock) = browser();
    store.send(BrowserAction::Root(ScreenAction::Load), |_| {});
    clock.advance(Duration::from_secs(1));
    store.receive(BrowserAction::Root(ScreenAction::Loaded(disk()["/"].clone())), |s| {
        s.root.entries = Some(disk()["/"].clone());
    });
    store.send(BrowserAction::Root(ScreenAction::OpenFolder("photos".into())), |_| {});
    store.receive(BrowserAction::Root(opens("/photos")), |s| {
        s.path.push(FolderScreen::new("/photos"));
    });
    clock.advance(Duration::from_secs(1));
    store.receive(element(0, ScreenAction::Loaded(disk()["/photos"].clone())), |s| {
        listed(s, 0, disk()["/photos"].clone());
    });
    store.send(element(0, ScreenAction::OpenFolder("2026".into())), |_| {});
    store.receive(element(0, opens("/photos/2026")), |s| {
        s.path.push(FolderScreen::new("/photos/2026"));
    });
    clock.advance(Duration::from_secs(1));
    store.receive(element(1, ScreenAction::Loaded(disk()["/photos/2026"].clone())), |s| {
        listed(s, 1, disk()["/photos/2026"].clone());
    });
    let shown: Vec<&str> = store.state().path.iter().map(|(_, s)| s.path.as_str()).collect();
    assert_eq!(shown, ["/photos", "/photos/2026"]);
}

#[test]
fn popping_a_screen_pops_those_above_and_cancels_their_loads() {
    let (mut store, clock) = browser();
    // Two screens opened before either has loaded.
    store.send(BrowserAction::Root(opens("/photos")), |s| {
        s.path.push(FolderScreen::new("/photos"));
    });
    store.send(element(0, opens("/photos/2026")), |s| {
        s.path.push(FolderScreen::new("/photos/2026"));
    });
    store.send(
        BrowserAction::Path(StackAction::PopFrom(StackElementId(0))),
        |s| s.path = StackState::new(),
    );
    // Neither load lands: finishing checks nothing came and nothing runs.
    clock.advance(Duration::from_secs(2));
    store.finish();
}

#[test]
fn going_back_from_a_screen_pops_it_and_leaves_the_one_below_loading() {
    let (mut store, clock) = browser();
    store.send(BrowserAction::Root(opens("/photos")), |s| {
        s.path.push(FolderScreen::new("/photos"));
    });
    store.send(element(0, opens("/photos/2026")), |s| {
        s.path.push(FolderScreen::new("/photos/2026"));
    });
    store.send(element(1, ScreenAction::Back), |_| {});
    store.receive(BrowserAction::Path(StackAction::PopFrom(StackElementId(1))), |s| {
        s.path = [FolderScreen::new("/photos")].into_iter().collect();
    });
    clock.advance(Duration::from_secs(1));
    // Only the screen still there gets its listing.
    store.receive(element(0, ScreenAction::Loaded(disk()["/photos"].clone())), |s| {
        listed(s, 0, disk()["/photos"].clone());
    });
    store.finish();
}

#[test]
fn back_on_the_root_does_nothing() {
    let (mut store, _clock) = browser();
    store.send(BrowserAction::Root(ScreenAction::Back), |_| {});
    store.finish();
}
