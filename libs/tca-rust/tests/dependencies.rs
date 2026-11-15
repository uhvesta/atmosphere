use std::sync::{Arc, Mutex};
use std::time::Duration;

use tca::{
    Clock, Dependencies, DependencyError, Effect, Reducer, SystemClock, TestClock, TestStore,
};

/// A client the app would back with the system's clipboard.
trait Clipboard: Send + Sync {
    fn write(&self, text: &str);
    fn read(&self) -> Option<String>;
}

/// The fake: what's written is kept for the test to read.
#[derive(Default)]
struct FakeClipboard {
    text: Mutex<Option<String>>,
}

impl Clipboard for FakeClipboard {
    fn write(&self, text: &str) {
        *self.text.lock().unwrap() = Some(text.to_owned());
    }
    fn read(&self) -> Option<String> {
        self.text.lock().unwrap().clone()
    }
}

/// Stands in for the live client: a test must never reach it.
struct LiveClipboard;

impl Clipboard for LiveClipboard {
    fn write(&self, _: &str) {
        panic!("the test used the live clipboard");
    }
    fn read(&self) -> Option<String> {
        panic!("the test used the live clipboard");
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Copier {
    copied: bool,
}

#[derive(Debug, PartialEq)]
enum CopyAction {
    Copy(String),
    HideCopied,
}

/// Copies, and shows "Copied" for two seconds.
struct Copying {
    clipboard: Arc<dyn Clipboard>,
    clock: Arc<dyn Clock>,
}

impl Copying {
    fn new(dependencies: &Dependencies) -> Result<Self, DependencyError> {
        Ok(Self {
            clipboard: dependencies.get()?,
            clock: dependencies.get()?,
        })
    }
}

impl Reducer for Copying {
    type State = Copier;
    type Action = CopyAction;

    fn reduce(&self, copier: &mut Copier, action: CopyAction) -> Effect<CopyAction> {
        match action {
            CopyAction::Copy(text) => {
                self.clipboard.write(&text);
                copier.copied = true;
                let clock = self.clock.clone();
                Effect::future(async move {
                    clock.sleep(Duration::from_secs(2)).await;
                    CopyAction::HideCopied
                })
            }
            CopyAction::HideCopied => {
                copier.copied = false;
                Effect::none()
            }
        }
    }
}

fn live() -> Dependencies {
    Dependencies::new()
        .with::<Arc<dyn Clipboard>>(Arc::new(LiveClipboard))
        .with::<Arc<dyn Clock>>(Arc::new(SystemClock::new()))
}

#[test]
fn a_feature_runs_on_the_fakes_a_test_overrides_the_live_clients_with() {
    let clipboard = Arc::new(FakeClipboard::default());
    let clock = TestClock::new();
    let dependencies = live()
        .with::<Arc<dyn Clipboard>>(clipboard.clone())
        .with::<Arc<dyn Clock>>(Arc::new(clock.clone()));
    let mut store = TestStore::new(Copier::default(), Copying::new(&dependencies).unwrap());
    store.send(CopyAction::Copy("main.rs".into()), |c| c.copied = true);
    assert_eq!(clipboard.read().as_deref(), Some("main.rs"));
    clock.advance(Duration::from_secs(2));
    store.receive(CopyAction::HideCopied, |c| c.copied = false);
}

#[test]
fn overriding_a_client_leaves_the_original_dependencies_as_they_were() {
    let live = Dependencies::new().with(1_u32).with("live");
    let test = live.clone().with(2_u32);
    assert_eq!(live.get::<u32>().unwrap(), 1);
    assert_eq!(test.get::<u32>().unwrap(), 2);
    // What wasn't overridden is the live one's.
    assert_eq!(test.get::<&str>().unwrap(), "live");
}

#[test]
fn a_client_not_given_is_an_error_when_the_feature_is_made() {
    let only_a_clock = Dependencies::new().with::<Arc<dyn Clock>>(Arc::new(TestClock::new()));
    assert!(matches!(
        Copying::new(&only_a_clock),
        Err(DependencyError::Missing { .. })
    ));
    assert!(only_a_clock.has::<Arc<dyn Clock>>());
    assert!(!only_a_clock.has::<Arc<dyn Clipboard>>());
}

#[test]
fn a_client_is_found_by_the_type_it_was_given_as() {
    let dependencies = Dependencies::new().with(42_u32);
    assert_eq!(dependencies.get::<u32>().unwrap(), 42);
    assert!(dependencies.get::<u64>().is_err());
}
