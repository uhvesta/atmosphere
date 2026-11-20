//! Settings › Sync (FUSEFS): the values load from the daemon, a toggle shows
//! at once and stays after the daemon saves it, a refusal puts it back, and
//! the master switch keeps the category values.
use humata2_clients::daemon::{FuseMountStatus, MachineSyncSettings, SyncSkillsMode, SyncSnapshot};
use humata2_clients::{FakeDaemon, Operation, fixture};
use humata2_settings::{Row, Section, Settings, SettingsAction as A, SettingsState, SyncAction, SyncCategory, SyncColumn};
use std::sync::Arc;
use tca::{TestClock, TestStore};

fn setup() -> (TestStore<Settings>, FakeDaemon) {
    let (clients, fakes) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let store = TestStore::new(SettingsState::default(), Settings::new(clients));
    (store, fakes.daemon)
}

/// What the daemon holds at the start: Sync on, skills and instructions on,
/// MCP and settings off, the mount off.
fn snapshot() -> SyncSnapshot {
    SyncSnapshot::default()
}

/// A store that has loaded the daemon's [`snapshot`].
fn loaded() -> (TestStore<Settings>, FakeDaemon) {
    let (mut store, daemon) = setup();
    store.send(A::Sync(SyncAction::Load), |_| {});
    store.receive(A::Sync(SyncAction::Loaded(Ok(snapshot()))), |s| {
        s.sync.snapshot = Some(snapshot());
    });
    (store, daemon)
}

/// Reads the daemon again and checks it holds `expected`.
fn assert_held(store: &mut TestStore<Settings>, expected: SyncSnapshot) {
    store.send(A::Sync(SyncAction::Load), |_| {});
    store.receive(A::Sync(SyncAction::Loaded(Ok(expected))), |_| {});
}

#[test]
fn the_values_load_from_the_daemon() {
    let (store, _) = loaded();
    assert_eq!(store.state().sync.snapshot, Some(snapshot()));
}

#[test]
fn values_the_daemon_holds_are_shown_as_they_are() {
    let (mut store, daemon) = setup();
    let mut held = snapshot();
    held.sync.sync_enabled = false;
    held.sync.mcp = true;
    held.fusefs_enabled = true;
    daemon.set_sync_snapshot(held.clone());
    store.send(A::Sync(SyncAction::Load), |_| {});
    let shown = held.clone();
    store.receive(A::Sync(SyncAction::Loaded(Ok(held))), move |s| s.sync.snapshot = Some(shown));
    store.finish();
}

#[test]
fn a_saved_change_is_what_the_daemon_then_holds() {
    let (mut store, _) = loaded();
    let mut changed = snapshot();
    changed.sync.mcp = true;
    store.send(A::Sync(SyncAction::ToggleCategory(SyncCategory::Mcp)), {
        let changed = changed.clone();
        move |s| s.sync.snapshot = Some(changed)
    });
    store.receive(A::Sync(SyncAction::Saved { before: snapshot(), result: Ok(changed.clone()) }), |_| {});
    assert_held(&mut store, changed);
    store.finish();
}

#[test]
fn opening_the_section_loads_it() {
    let (mut store, _) = setup();
    store.send(A::Section(Section::Sync), |s| s.section = Section::Sync);
    let defaults = SyncSnapshot::default();
    store.receive(A::Sync(SyncAction::Loaded(Ok(defaults.clone()))), |s| {
        s.sync.snapshot = Some(defaults);
    });
    store.finish();
}

#[test]
fn the_sync_rows_belong_to_the_sync_section() {
    assert_eq!(Row::SyncEnabled.section(), Section::Sync);
    assert_eq!(Row::FuseFsEnabled.section(), Section::Sync);
    assert!(Row::ALL.contains(&Row::SyncEnabled));
    assert!(Row::ALL.contains(&Row::FuseFsEnabled));
    assert!(Section::ALL.contains(&Section::Sync));
}

#[test]
fn a_category_toggle_shows_at_once_and_stays_after_it_is_saved() {
    let (mut store, _) = loaded();
    let mut after = snapshot();
    after.sync.skills = false;
    let shown = after.clone();
    store.send(A::Sync(SyncAction::ToggleCategory(SyncCategory::Skills)), move |s| {
        s.sync.snapshot = Some(shown);
    });
    store.receive(
        A::Sync(SyncAction::Saved { before: snapshot(), result: Ok(after.clone()) }),
        |_| {},
    );
    assert_eq!(store.state().sync.snapshot, Some(after.clone()));
    assert_held(&mut store, after.clone());
    store.finish();
}

#[test]
fn a_refused_toggle_goes_back_and_shows_why() {
    let (store, daemon) = loaded();
    let mut store = store.non_exhaustive();
    daemon.fail_next(Operation::SetSyncSettings, "link unavailable");
    store.send(A::Sync(SyncAction::ToggleCategory(SyncCategory::Mcp)), |s| {
        if let Some(snapshot) = s.sync.snapshot.as_mut() {
            snapshot.sync.mcp = true;
        }
    });
    store.receive_matching(
        "failure",
        |a| matches!(a, A::Sync(SyncAction::Saved { result: Err(_), .. })),
        |_| {},
    );
    assert_eq!(store.state().sync.snapshot, Some(snapshot()));
    assert!(store.state().error.is_some());
    assert_held(&mut store, snapshot());
    store.finish();
}

#[test]
fn a_refused_mount_switch_goes_back() {
    let (store, daemon) = loaded();
    let mut store = store.non_exhaustive();
    daemon.fail_next(Operation::SetSyncSettings, "link unavailable");
    store.send(A::Sync(SyncAction::ToggleFuseFs), |s| {
        if let Some(snapshot) = s.sync.snapshot.as_mut() {
            snapshot.fusefs_enabled = true;
        }
    });
    store.receive_matching(
        "failure",
        |a| matches!(a, A::Sync(SyncAction::Saved { result: Err(_), .. })),
        |_| {},
    );
    assert_eq!(store.state().sync.snapshot, Some(snapshot()));
    store.finish();
}

#[test]
fn the_mount_switch_is_saved_without_touching_the_section() {
    let (mut store, _) = loaded();
    let mut after = snapshot();
    after.fusefs_enabled = true;
    let shown = after.clone();
    store.send(A::Sync(SyncAction::ToggleFuseFs), move |s| s.sync.snapshot = Some(shown));
    store.receive(
        A::Sync(SyncAction::Saved { before: snapshot(), result: Ok(after.clone()) }),
        |_| {},
    );
    assert_held(&mut store, after.clone());
    store.finish();
}

#[test]
fn the_master_switch_off_keeps_the_category_values() {
    let (mut store, _) = loaded();
    let mut after = snapshot();
    after.sync.sync_enabled = false;
    let shown = after.clone();
    store.send(A::Sync(SyncAction::ToggleEnabled), move |s| s.sync.snapshot = Some(shown));
    store.receive(
        A::Sync(SyncAction::Saved { before: snapshot(), result: Ok(after.clone()) }),
        |_| {},
    );
    let kept = &store.state().sync.snapshot.clone().unwrap_or_default().sync;
    assert!(!kept.sync_enabled);
    assert_eq!((kept.skills, kept.instructions, kept.mcp, kept.settings), (true, true, false, false));
    assert_held(&mut store, after);
    store.finish();
}

#[test]
fn a_machine_override_is_saved_and_clearing_it_removes_the_entry() {
    let (mut store, _) = loaded();
    let departures = MachineSyncSettings {
        skills_mode: Some(SyncSkillsMode::HostOnly),
        ..MachineSyncSettings::default()
    };
    let mut with = snapshot();
    with.sync.machines.insert("devbox-1".to_owned(), departures);
    let shown = with.clone();
    store.send(A::Sync(SyncAction::SetMachine("devbox-1".to_owned(), departures)), move |s| {
        s.sync.snapshot = Some(shown);
    });
    store.receive(A::Sync(SyncAction::Saved { before: snapshot(), result: Ok(with.clone()) }), |_| {});
    assert_held(&mut store, with.clone());

    store.send(
        A::Sync(SyncAction::SetMachine("devbox-1".to_owned(), MachineSyncSettings::default())),
        |s| s.sync.snapshot = Some(snapshot()),
    );
    store.receive(A::Sync(SyncAction::Saved { before: with, result: Ok(snapshot()) }), |_| {});
    assert_held(&mut store, snapshot());
    store.finish();
}

#[test]
fn a_toggle_before_the_values_load_does_nothing() {
    let (mut store, _) = setup();
    store.send(A::Sync(SyncAction::ToggleEnabled), |_| {});
    store.finish();
}

fn machine_pick(value: Option<bool>) -> A {
    A::Sync(SyncAction::PickMachine("devbox-1".to_owned(), SyncColumn::Mcp, value))
}

/// The snapshot with devbox-1 departing as `departures`.
fn with_machine(departures: MachineSyncSettings) -> SyncSnapshot {
    let mut held = snapshot();
    held.sync.machines.insert("devbox-1".to_owned(), departures);
    held
}

#[test]
fn a_pick_sets_only_that_field_and_default_unsets_it() {
    let (mut store, _) = loaded();
    let first = MachineSyncSettings { skills: Some(false), ..MachineSyncSettings::default() };
    store.send(A::Sync(SyncAction::PickMachine("devbox-1".to_owned(), SyncColumn::Skills, Some(false))), {
        let shown = with_machine(first);
        move |s| s.sync.snapshot = Some(shown)
    });
    store.receive(A::Sync(SyncAction::Saved { before: snapshot(), result: Ok(with_machine(first)) }), |_| {});

    let both = MachineSyncSettings { mcp: Some(true), ..first };
    store.send(machine_pick(Some(true)), {
        let shown = with_machine(both);
        move |s| s.sync.snapshot = Some(shown)
    });
    store.receive(
        A::Sync(SyncAction::Saved { before: with_machine(first), result: Ok(with_machine(both)) }),
        |_| {},
    );

    // Default unsets that field and leaves the other pick alone.
    store.send(machine_pick(None), {
        let shown = with_machine(first);
        move |s| s.sync.snapshot = Some(shown)
    });
    store.receive(
        A::Sync(SyncAction::Saved { before: with_machine(both), result: Ok(with_machine(first)) }),
        |_| {},
    );
    store.finish();
}

#[test]
fn a_pick_closes_the_menu() {
    let (mut store, _) = loaded();
    let open = ("devbox-1".to_owned(), SyncColumn::Mcp);
    store.send(A::Sync(SyncAction::OpenMachineMenu(open.0.clone(), open.1)), {
        let open = open.clone();
        move |s| s.sync.machine_menu = Some(open)
    });
    let picked = with_machine(MachineSyncSettings { mcp: Some(true), ..MachineSyncSettings::default() });
    store.send(machine_pick(Some(true)), {
        let picked = picked.clone();
        move |s| {
            s.sync.machine_menu = None;
            s.sync.snapshot = Some(picked);
        }
    });
    store.receive(A::Sync(SyncAction::Saved { before: snapshot(), result: Ok(picked) }), |_| {});
    store.finish();
}

#[test]
fn opening_the_open_menu_again_closes_it() {
    let (mut store, _) = loaded();
    let id = "devbox-1".to_owned();
    store.send(A::Sync(SyncAction::OpenMachineMenu(id.clone(), SyncColumn::Sync)), {
        let id = id.clone();
        move |s| s.sync.machine_menu = Some((id, SyncColumn::Sync))
    });
    store.send(A::Sync(SyncAction::OpenMachineMenu(id, SyncColumn::Sync)), |s| s.sync.machine_menu = None);
    store.finish();
}

#[test]
fn a_refused_pick_goes_back() {
    let (store, daemon) = loaded();
    let mut store = store.non_exhaustive();
    daemon.fail_next(Operation::SetSyncSettings, "link unavailable");
    let shown = with_machine(MachineSyncSettings { mcp: Some(false), ..MachineSyncSettings::default() });
    store.send(machine_pick(Some(false)), move |s| s.sync.snapshot = Some(shown));
    store.receive_matching(
        "failure",
        |a| matches!(a, A::Sync(SyncAction::Saved { result: Err(_), .. })),
        |_| {},
    );
    assert_eq!(store.state().sync.snapshot, Some(snapshot()));
    assert!(store.state().error.is_some());
}

#[test]
fn a_mount_state_change_from_the_daemon_shows_when_read_again() {
    let (mut store, daemon) = loaded();
    for status in [
        FuseMountStatus::Starting,
        FuseMountStatus::Mounted { path: "/Users/me/Humata".to_owned() },
        FuseMountStatus::CannotSetUp { reason: "No FUSE here.".to_owned() },
    ] {
        daemon.set_fusefs_status(status.clone());
        let mut held = snapshot();
        held.fusefs_status = status;
        let shown = held.clone();
        store.send(A::Sync(SyncAction::Load), |_| {});
        store.receive(A::Sync(SyncAction::Loaded(Ok(held))), move |s| s.sync.snapshot = Some(shown));
    }
    store.finish();
}
