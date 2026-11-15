use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tca::{Effect, Reducer, Shared, TestStore};

#[derive(Clone, Debug, Default, PartialEq)]
struct Settings {
    theme: String,
    font_size: u32,
}

/// A feature that shows the theme it's told of, and can change it.
#[derive(Clone, Debug, PartialEq)]
struct Panel {
    settings: Shared<Settings>,
    shown: String,
}

#[derive(Debug, PartialEq)]
enum PanelAction {
    Appeared,
    Disappeared,
    SetTheme(String),
    Changed(String),
}

struct Paneling;

impl Reducer for Paneling {
    type State = Panel;
    type Action = PanelAction;

    fn reduce(&self, panel: &mut Panel, action: PanelAction) -> Effect<PanelAction> {
        match action {
            PanelAction::Appeared => panel
                .settings
                .changes(|settings| PanelAction::Changed(settings.theme.clone()))
                .cancellable("settings", true),
            PanelAction::Disappeared => Effect::cancel("settings"),
            PanelAction::SetTheme(theme) => {
                panel.settings.update(|settings| settings.theme = theme);
                Effect::none()
            }
            PanelAction::Changed(theme) => {
                panel.shown = theme;
                Effect::none()
            }
        }
    }
}

fn panel(settings: &Shared<Settings>) -> TestStore<Paneling> {
    TestStore::new(
        Panel {
            settings: settings.clone(),
            shown: String::new(),
        },
        Paneling,
    )
}

#[test]
fn every_reader_sees_a_change_made_through_any_of_them() {
    let settings = Shared::new(Settings::default());
    let readers = [settings.clone(), settings.clone(), settings.clone()];
    readers[1].set(Settings {
        theme: "dark".into(),
        font_size: 13,
    });
    for reader in &readers {
        assert_eq!(reader.get().theme, "dark");
        assert_eq!(reader.version(), 1);
    }
    assert_eq!(settings.with(|s| s.font_size), 13);
}

#[test]
fn readers_share_one_value_rather_than_copies() {
    let settings = Shared::new(Settings::default());
    let other = settings.clone();
    assert!(Arc::ptr_eq(&settings.get(), &other.get()));
}

#[test]
fn a_reader_holding_the_old_value_keeps_it_while_others_see_the_new() {
    let settings = Shared::new(Settings {
        theme: "light".into(),
        font_size: 12,
    });
    let before = settings.get();
    settings.update(|s| s.font_size = 14);
    assert_eq!(before.font_size, 12);
    assert_eq!(settings.get().font_size, 14);
    assert_eq!(settings.get().theme, "light");
}

#[test]
fn every_subscriber_is_told_each_change_until_dropped() {
    let settings = Shared::new(Settings::default());
    let told = Arc::new(Mutex::new(Vec::new()));
    let first = {
        let told = told.clone();
        settings.subscribe(move |s| told.lock().unwrap().push(("first", s.font_size)))
    };
    let _second = {
        let told = told.clone();
        settings
            .clone()
            .subscribe(move |s| told.lock().unwrap().push(("second", s.font_size)))
    };
    settings.update(|s| s.font_size = 1);
    drop(first);
    settings.update(|s| s.font_size = 2);
    let mut told = told.lock().unwrap().clone();
    told.sort();
    assert_eq!(told, [("first", 1), ("second", 1), ("second", 2)]);
}

#[test]
fn a_subscriber_may_read_the_value_it_is_told_of() {
    let settings = Shared::new(Settings::default());
    let reader = settings.clone();
    let read = Arc::new(Mutex::new(None));
    let seen = read.clone();
    let _subscribed = settings.subscribe(move |_| *seen.lock().unwrap() = Some(reader.get().theme.clone()));
    settings.update(|s| s.theme = "dark".into());
    assert_eq!(read.lock().unwrap().as_deref(), Some("dark"));
}

#[test]
fn changes_from_many_threads_are_all_counted() {
    let settings = Shared::new(Settings::default());
    let told = Arc::new(AtomicUsize::new(0));
    let counted = told.clone();
    let _subscribed = settings.subscribe(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let settings = settings.clone();
            std::thread::spawn(move || {
                for _ in 0..100 {
                    settings.update(|s| s.font_size += 1);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(settings.get().font_size, 800);
    assert_eq!(settings.version(), 800);
    assert_eq!(told.load(Ordering::SeqCst), 800);
}

#[test]
fn a_feature_is_told_of_a_change_another_feature_makes() {
    let settings = Shared::new(Settings::default());
    let mut changer = panel(&settings);
    let mut watcher = panel(&settings);
    watcher.send(PanelAction::Appeared, |_| {});
    changer.send(PanelAction::SetTheme("dark".into()), |_| {});
    watcher.receive(PanelAction::Changed("dark".into()), |p| {
        p.shown = "dark".into()
    });
    assert_eq!(changer.state().settings.get().theme, "dark");
    watcher.send(PanelAction::Disappeared, |_| {});
    // Told nothing once it stopped watching.
    settings.update(|s| s.theme = "light".into());
    watcher.finish();
    changer.finish();
}
