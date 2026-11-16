//! The bug-report commands follow whether a recording runs and whether a
//! report exists (D159).
use humata2_palette::{Command, Palette, PaletteAction as A, PaletteState, ReportStatus};
use tca::TestStore;

fn commands(report: ReportStatus) -> Vec<(Command, String)> {
    let state = PaletteState {
        report,
        ..Default::default()
    };
    let mut store = TestStore::new(state, Palette).non_exhaustive();
    store.send(A::Open { in_session: false }, |s| s.open = true);
    store
        .state()
        .entries
        .iter()
        .map(|entry| (entry.command.clone(), entry.title.clone()))
        .collect()
}

fn title(entries: &[(Command, String)], command: &Command) -> Option<String> {
    entries
        .iter()
        .find(|(c, _)| c == command)
        .map(|(_, title)| title.clone())
}

#[test]
fn recording_toggles_the_record_entry() {
    let idle = commands(ReportStatus::default());
    let running = commands(ReportStatus {
        recording: true,
        last: false,
    });
    let idle_title = title(&idle, &Command::Record);
    let running_title = title(&running, &Command::Record);
    assert!(idle_title.is_some());
    assert!(running_title.is_some());
    assert_ne!(idle_title, running_title);
    assert!(title(&idle, &Command::Snapshot).is_some());
    assert!(title(&idle, &Command::ReportBug).is_some());
}

#[test]
fn the_last_report_is_offered_only_once_one_exists() {
    let none = commands(ReportStatus::default());
    let some = commands(ReportStatus {
        recording: false,
        last: true,
    });
    assert!(title(&none, &Command::OpenLastReport).is_none());
    assert!(title(&some, &Command::OpenLastReport).is_some());
}

#[test]
fn picking_a_report_command_hands_it_to_the_app() {
    let state = PaletteState::default();
    let mut store = TestStore::new(state, Palette).non_exhaustive();
    store.send(A::Open { in_session: false }, |s| s.open = true);
    let index = store
        .state()
        .entries
        .iter()
        .position(|entry| entry.command == Command::Snapshot);
    let Some(index) = index else {
        panic!("the palette lists Take a snapshot");
    };
    store.send(A::Pick(index), |s| s.open = false);
    store.receive(
        A::Delegate(humata2_palette::Delegate::Closed),
        |_| {},
    );
    store.receive(
        A::Delegate(humata2_palette::Delegate::Run(Command::Snapshot)),
        |_| {},
    );
}
