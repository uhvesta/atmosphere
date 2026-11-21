//! D784: Escape cancels the close-tab dialog and the machine page's running
//! confirmations, leaving the page as it was.
use std::sync::Arc;

use humata2_app::{App, AppAction as A, AppState, Route};
use humata2_clients::model::{Agent, ChannelId, MachineId, SessionId};
use humata2_clients::{Clients, RunningHolder, fixture};
use humata2_session::{CloseConfirm, CloseKind};
use tca::{TestClock, TestStore};

fn store(route: Route, set: impl FnOnce(&mut AppState)) -> TestStore<App> {
    let (clients, _) = humata2_clients::fake_clients(fixture::design(), Arc::new(TestClock::new()));
    let mut state = AppState::launch(&clients);
    state.route = route;
    set(&mut state);
    TestStore::new(state, App::new(clients)).non_exhaustive()
}

fn holder() -> RunningHolder {
    RunningHolder {
        tab: Some("tab-1".to_owned()),
        session: None,
        title: String::new(),
        channel: None::<ChannelId>,
        desk: false,
        agent: Agent::Shell,
        pid: Some(1),
        folder: "~/scratch".to_owned(),
        started_unix_ms: None,
        attached: false,
        machine: MachineId::new("devbox-1"),
        stopped: false,
        agent_tab: false,
        archived: false,
        paused_for_move: false,
    }
}

fn store_adopting() -> TestStore<App> {
    store(Route::Machine(MachineId::new("devbox-1")), |state| {
        state.machine_page.running.adopting = Some(holder());
    })
}

#[test]
fn escape_cancels_the_close_dialog() {
    let id = SessionId::new("ledger");
    let mut store = store(Route::Session(id.clone()), |state| {
        state.session.closing = Some(CloseConfirm {
        session: id.clone(),
        group: 0,
        index: 0,
        tab: 0,
        kind: CloseKind::Agent,
        title: Some("Ledger".to_owned()),
        agent: "Claude Code".to_owned(),
        machine: "This Mac".to_owned(),
        always_keep_running: false,
        sending: None,
        error: None,
        });
    });
    store.send(A::Escape, |_| {});
    assert!(store.state().session.closing.is_none());
    assert_eq!(store.state().route, Route::Session(id));
}

#[test]
fn escape_cancels_end_for_good_and_adopt_on_the_machine_page() {
    let mut store = store(Route::Machine(MachineId::new("devbox-1")), |state| {
        state.machine_page.running.confirm_end = Some(holder());
    });
    store.send(A::Escape, |_| {});
    assert!(store.state().machine_page.running.confirm_end.is_none());
    let mut store = store_adopting();
    store.send(A::Escape, |_| {});
    assert!(store.state().machine_page.running.adopting.is_none());
}

#[test]
fn the_close_dialog_and_running_confirms_hold_the_keys_from_a_terminal() {
    let id = SessionId::new("ledger");
    let mut held = store(Route::Session(id.clone()), |state| {
        state.session.closing = Some(CloseConfirm {
            session: id.clone(),
            group: 0,
            index: 0,
            tab: 0,
            kind: CloseKind::Agent,
            title: Some("Ledger".to_owned()),
            agent: "Claude Code".to_owned(),
            machine: "This Mac".to_owned(),
            always_keep_running: false,
            sending: None,
            error: None,
        });
    });
    assert!(held.state().dialog_holds_keys(), "the close dialog holds the keys");
    held.send(A::Escape, |_| {});
    assert!(!held.state().dialog_holds_keys(), "Escape gives them back");

    let mut held = store(Route::Machine(MachineId::new("devbox-1")), |state| {
        state.machine_page.running.confirm_end = Some(holder());
    });
    assert!(held.state().dialog_holds_keys(), "End for good… holds the keys");
    held.send(A::Escape, |_| {});
    assert!(!held.state().dialog_holds_keys());

    let mut held = store_adopting();
    assert!(held.state().dialog_holds_keys(), "Adopt… holds the keys");
    held.send(A::Escape, |_| {});
    assert!(!held.state().dialog_holds_keys());
}

#[test]
fn files_modals_hold_keys_and_escape_dismisses_inner_before_outer() {
    let mut s = store(Route::Files, |state| {
        state.files.naming = Some(humata2_files::Naming {
            kind: humata2_files::NamingKind::NewFolder,
            initial: String::new(),
            error: None,
        });
        state.files.trash = Some(humata2_files::TrashState {
            machine: MachineId::new("local"),
            listing: None,
        });
        state.files.empty_plan = Some(humata2_clients::EmptyPlan {
            machine: MachineId::new("local"),
            items: 2,
            frees_bytes: 1024,
            token: "tok".to_owned(),
        });
    });

    assert!(s.state().dialog_holds_keys());
    // First Escape closes naming modal.
    s.send(A::Escape, |_| {});
    assert!(s.state().files.naming.is_none());
    assert!(s.state().files.empty_plan.is_some());
    assert!(s.state().dialog_holds_keys());

    // Second Escape closes empty-trash confirm while keeping trash open.
    s.send(A::Escape, |_| {});
    assert!(s.state().files.empty_plan.is_none());
    assert!(s.state().files.trash.is_some());
    assert!(s.state().dialog_holds_keys());

    // Third Escape closes trash sheet.
    s.send(A::Escape, |_| {});
    assert!(s.state().files.trash.is_none());
    assert!(!s.state().dialog_holds_keys());
}

#[test]
fn escape_closes_inner_pickers_before_popping_modal_sheets() {
    // NewSession: Escape closes picker before popping sheet and preserves title.
    let mut s = store(Route::Home, |state| {
        state.overlays.push(humata2_app::Overlay::NewSession(
            humata2_app::NewSession {
                channel: None,
                agent: Agent::Claude,
                machine: None,
                title: "Keep my title".to_owned(),
                first_message: String::new(),
                excluded_repos: Default::default(),
                item: None,
                picker: Some(humata2_app::NewSessionPicker::Channel),
                creating: false,
                progress: None,
                op: None,
                error: None,
                note: None,
            },
        ));
    });
    s.send(A::Escape, |_| {});
    match s.state().overlay() {
        Some(humata2_app::Overlay::NewSession(sheet)) => {
            assert_eq!(sheet.picker, None);
            assert_eq!(sheet.title, "Keep my title");
        }
        other => panic!("expected NewSession sheet to remain open, got {other:?}"),
    }
    s.send(A::Escape, |_| {});
    assert!(s.state().overlay().is_none());

    // NewChannel: Escape closes + Org before popping sheet and preserves name.
    let mut s = store(Route::Home, |state| {
        let mut sheet = humata2_app::NewChannel {
            name: "Payments".to_owned(),
            ..Default::default()
        };
        sheet.search.adding_owner = Some("acme".to_owned());
        state.overlays.push(humata2_app::Overlay::NewChannel(sheet));
    });
    s.send(A::Escape, |_| {});
    match s.state().overlay() {
        Some(humata2_app::Overlay::NewChannel(sheet)) => {
            assert_eq!(sheet.search.adding_owner, None);
            assert_eq!(sheet.name, "Payments");
        }
        other => panic!("expected NewChannel sheet to remain open, got {other:?}"),
    }
    s.send(A::Escape, |_| {});
    assert!(s.state().overlay().is_none());
}

#[test]
fn escape_closes_all_four_review_menus() {
    let id = SessionId::new("ledger");
    let mut s = store(Route::Session(id), |state| {
        state.review.base_menu = true;
        state.review.agent_tab_menu = true;
        state.review.repo_picker = Some(humata2_review::RepoPicker {
            repo: "acme/api".to_owned(),
            bases: None,
            filter: String::new(),
        });
        state.review.push_menu = true;
    });

    s.send(A::Escape, |_| {});
    assert!(!s.state().review.base_menu);

    s.send(A::Escape, |_| {});
    assert!(!s.state().review.agent_tab_menu);

    s.send(A::Escape, |_| {});
    assert!(s.state().review.repo_picker.is_none());

    s.send(A::Escape, |_| {});
    assert!(!s.state().review.push_menu);
}

#[test]
fn report_form_holds_keys_from_terminal() {
    let mut s = store(Route::Session(SessionId::new("ledger")), |state| {
        state.report.form = Some(humata2_app::ReportForm {
            folder: std::path::PathBuf::from("/tmp/report"),
            kind: humata2_app::ReportKind::Snapshot,
            happened: String::new(),
            expected: String::new(),
            elements: Vec::new(),
            query: String::new(),
            comments: Vec::new(),
            copied: false,
            error: None,
            drawing: false,
            strokes: Vec::new(),
            places: Vec::new(),
            window: (1200, 800),
        });
    });
    assert!(s.state().dialog_holds_keys());
    s.send(A::Escape, |_| {});
    assert!(s.state().report.form.is_none());
    assert!(!s.state().dialog_holds_keys());
}
