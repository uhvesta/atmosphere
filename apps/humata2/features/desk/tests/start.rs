//! DK-14: New desk session starts a Desk session in a scratch or picked folder.
use std::sync::Arc;

use humata2_clients::desk::{self, DeskSession, Folder};
use humata2_clients::model::{Agent, Session};
use humata2_clients::{FakeDaemon, Operation, Workspace, fixture};
use humata2_desk::{Delegate, Desk, DeskAction as A, DeskState, NewAction as N};
use tca::{TestClock, TestStore};

fn store(workspace: Workspace) -> (TestStore<Desk>, FakeDaemon) {
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    let store = TestStore::new(state, Desk::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn started(store: &mut TestStore<Desk>) {
    store.receive_matching("Started(Ok(_))", |a| matches!(a, A::Started(Ok(_))), |_| {});
    store.receive_matching(
        "Delegate(OpenSession(_))",
        |a| matches!(a, A::Delegate(Delegate::OpenSession(_))),
        |_| {},
    );
}

fn find(fake: &FakeDaemon, title: &str) -> (Session, DeskSession) {
    let workspace = fake.workspace();
    let Some(session) = workspace.sessions.iter().find(|s| s.title == title) else {
        panic!("no session titled {title:?} was started");
    };
    let Some(record) = workspace.desk.sessions.get(&session.id) else {
        panic!("{title:?} has no Desk record");
    };
    (session.clone(), record.clone())
}

#[test]
fn a_titled_session_starts_in_a_scratch_folder_named_from_the_title() {
    let (mut store, fake) = store(fixture::empty());
    store.send(A::StartNew { agent: Agent::Claude }, |_| {});
    store.send(A::New(N::Title("Plan Q4 board".to_owned())), |_| {});
    store.send(A::New(N::Start), |_| {});
    started(&mut store);

    let (session, record) = find(&fake, "Plan Q4 board");
    assert_eq!(session.channel, desk::channel());
    assert_eq!(session.agent, Agent::Claude);
    assert_eq!(
        record.folder,
        Folder::Scratch {
            path: desk::scratch_path("plan-q4-board"),
            kept: false
        }
    );
    assert!(store.state().new.is_none());
}

#[test]
fn the_dialog_previews_the_path_the_folder_will_get() {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    let (mut store, _fake) = store(workspace);
    store.send(A::StartNew { agent: Agent::Claude }, |_| {});
    store.send(A::New(N::Title("Write notes".to_owned())), |_| {});
    assert_eq!(
        store.state().scratch_preview(),
        Some(desk::scratch_path("write-notes"))
    );
    // This Mac already has a Desk folder called plan-q4 (FC-14).
    store.send(A::New(N::Title("Plan Q4".to_owned())), |_| {});
    assert_eq!(
        store.state().scratch_preview(),
        Some(desk::scratch_path("plan-q4-2"))
    );
    // devbox-1 has none.
    store.send(
        A::New(N::Machine(humata2_clients::model::MachineId::new("devbox-1"))),
        |_| {},
    );
    assert_eq!(
        store.state().scratch_preview(),
        Some(desk::scratch_path("plan-q4"))
    );
}

#[test]
fn the_preview_counts_a_folder_the_daemon_reports_by_its_full_path() {
    // A live daemon names a Desk folder by its full path under the machine's
    // root, not the short ~/Humata form the dialog shows.
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    for record in workspace.desk.sessions.values_mut() {
        if let Folder::Scratch { path, .. } = &mut record.folder {
            if let Some(rest) = path.strip_prefix(desk::DEFAULT_ROOT) {
                *path = format!("/Users/me/Humata{rest}");
            }
        }
    }
    let (mut store, _fake) = store(workspace);
    store.send(A::StartNew { agent: Agent::Claude }, |_| {});
    store.send(A::New(N::Title("Plan Q4".to_owned())), |_| {});
    assert_eq!(
        store.state().scratch_preview(),
        Some(desk::scratch_path("plan-q4-2"))
    );
    store.send(A::New(N::Title("Write notes".to_owned())), |_| {});
    assert_eq!(
        store.state().scratch_preview(),
        Some(desk::scratch_path("write-notes"))
    );
}

#[test]
fn a_blank_title_names_the_folder_after_the_title_it_will_get() {
    let (mut store, fake) = store(fixture::empty());
    store.send(A::StartNew { agent: Agent::Codex }, |_| {});
    assert_eq!(
        store.state().scratch_preview(),
        Some(desk::scratch_path("new-codex-session"))
    );
    store.send(A::New(N::Start), |_| {});
    started(&mut store);
    let (_, record) = find(&fake, "New Codex session");
    assert_eq!(record.folder.path(), desk::scratch_path("new-codex-session"));
}

#[test]
fn with_no_online_machine_start_is_an_error() {
    let mut workspace = fixture::empty();
    for machine in &mut workspace.machines {
        machine.online = false;
    }
    let (mut store, fake) = store(workspace);
    store.send(A::StartNew { agent: Agent::Claude }, |_| {});
    store.send(A::New(N::Title("Plan".to_owned())), |_| {});
    store.send(A::New(N::Start), |_| {});
    store.skip_received();

    let Some(form) = store.state().new.as_ref() else {
        panic!("the form stays open");
    };
    assert_eq!(form.machine, None);
    assert!(form.error.is_some());
    assert!(fake.workspace().desk.sessions.is_empty());
}

#[test]
fn a_failed_start_keeps_the_form_with_the_error() {
    let (mut store, fake) = store(fixture::empty());
    fake.fail_next(Operation::Desk(desk::Operation::Start), "no luck");
    store.send(A::StartNew { agent: Agent::Claude }, |_| {});
    store.send(A::New(N::Title("Plan".to_owned())), |_| {});
    store.send(A::New(N::Start), |_| {});
    store.receive_matching("Started(Err(_))", |a| matches!(a, A::Started(Err(_))), |_| {});

    let Some(form) = store.state().new.as_ref() else {
        panic!("the form stays open");
    };
    assert!(form.error.is_some());
    assert!(!form.starting);
    assert_eq!(form.title, "Plan");
    assert!(fake.workspace().desk.sessions.is_empty());
}

#[test]
fn a_blank_title_gets_one_from_the_agent() {
    let (mut store, fake) = store(fixture::empty());
    store.send(A::StartNew { agent: Agent::Codex }, |_| {});
    store.send(A::New(N::Start), |_| {});
    started(&mut store);

    let workspace = fake.workspace();
    assert_eq!(workspace.desk.sessions.len(), 1);
    let (session, _) = find(&fake, "New Codex session");
    assert_eq!(session.agent, Agent::Codex);
}

#[test]
fn repos_are_trimmed_and_not_repeated() {
    let (mut store, _fake) = store(fixture::empty());
    store.send(A::StartNew { agent: Agent::Claude }, |_| {});
    store.send(A::New(N::AddRepo(" acme/ledger ".to_owned())), |_| {});
    store.send(A::New(N::AddRepo("acme/ledger".to_owned())), |_| {});
    store.send(A::New(N::AddRepo("  ".to_owned())), |_| {});
    let Some(form) = store.state().new.as_ref() else {
        panic!("the form is open");
    };
    assert_eq!(form.repos, vec!["acme/ledger".to_owned()]);
}

#[test]
fn a_late_reply_without_a_form_still_opens_the_session() {
    let (mut store, _fake) = store(fixture::empty());
    let id = humata2_clients::model::SessionId::new("late");
    store.send(A::Started(Ok(id.clone())), |_| {});
    store.receive(A::Delegate(Delegate::OpenSession(id)), |_| {});
}

#[test]
fn cancelling_during_start_surfaces_a_late_failure_in_desk_error() {
    // SMB-011: pressing Esc/Cancel while a Desk start is in flight must not
    // silently swallow a subsequent Started(Err) failure.
    let (mut store, _fake) = store(fixture::empty());
    store.send(A::StartNew { agent: Agent::Claude }, |_| {});
    store.send(A::New(N::Title("Plan".to_owned())), |_| {});
    store.send(A::CancelDialog, |_| {});
    assert!(store.state().new.is_none());
    store.send(
        A::Started(Err(humata2_clients::DaemonError::Refused {
            what: "start desk".to_owned(),
            reason: "disk full".to_owned(),
        })),
        |_| {},
    );
    assert!(
        store.state().error.as_deref().is_some_and(|e| e.contains("disk full")),
        "expected Desk state.error to surface the failure, got {:?}",
        store.state().error
    );
}
