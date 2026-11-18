//! A channel's own Desk (CD-1 to CD-6): New desk session with its switches,
//! Close asking first, and the switches' changes, on the fake daemon.
use std::sync::Arc;

use humata2_clients::desk::{Switch, Switches};
use humata2_clients::model::{Agent, ChannelId, RuleSetting, SessionId};
use humata2_clients::{FakeDaemon, Operation, Workspace, fixture};
use humata2_desk::{Change, ChannelNewAction as N, Delegate, Desk, DeskAction as A, DeskState};
use tca::{TestClock, TestStore};

fn payments() -> ChannelId {
    ChannelId::new("payments-rewrite")
}

fn store(workspace: Workspace) -> (TestStore<Desk>, FakeDaemon) {
    let fake = FakeDaemon::new(workspace.clone(), Arc::new(TestClock::new()));
    let state = DeskState {
        workspace: Arc::new(workspace),
        ..DeskState::default()
    };
    let store = TestStore::new(state, Desk::new(Arc::new(fake.clone()))).non_exhaustive();
    (store, fake)
}

fn show(store: &mut TestStore<Desk>, fake: &FakeDaemon) {
    store.send(A::Show(Arc::new(fake.workspace())), |_| {});
}

fn started(store: &mut TestStore<Desk>) {
    store.receive_matching("Started(Ok(_))", |a| matches!(a, A::Started(Ok(_))), |_| {});
    store.receive_matching(
        "Delegate(OpenSession(_))",
        |a| matches!(a, A::Delegate(Delegate::OpenSession(_))),
        |_| {},
    );
}

fn start(store: &mut TestStore<Desk>, target: Option<SessionId>, title: &str) {
    store.send(
        A::StartChannelNew {
            channel: payments(),
            target,
            agent: Agent::Claude,
        },
        |_| {},
    );
    store.send(A::ChannelNew(N::Title(title.to_owned())), |_| {});
}

#[test]
fn a_session_for_the_whole_channel_is_listed_on_its_desk_and_not_on_the_plain_one() {
    let (mut store, fake) = store(fixture::design());
    start(&mut store, None, "Triage flaky tests");
    store.send(A::ChannelNew(N::Start), |_| {});
    started(&mut store);
    show(&mut store, &fake);

    let state = store.state();
    let rows = state.channel_rows(&payments());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].session.title, "Triage flaky tests");
    assert_eq!(rows[0].desk.target, None);
    assert_eq!(rows[0].desk.switches, Switches::default());
    assert!(state.open().is_empty(), "the plain Desk doesn't list it");
    assert!(state.channel_new.is_none(), "the dialog closes once it starts");
}

#[test]
fn a_session_target_and_the_switches_reach_the_daemon() {
    let (mut store, fake) = store(fixture::design());
    let ledger = SessionId::new("ledger");
    start(&mut store, Some(ledger.clone()), "Watch the backfill");
    let Some(form) = store.state().channel_new.clone() else {
        panic!("the dialog is open")
    };
    assert!(form.on_session);
    assert_eq!(form.target(), Some(&ledger));
    store.send(A::ChannelNew(N::Switch(Switch::TypeIntoTerminal, true)), |_| {});
    store.send(A::ChannelNew(N::Switch(Switch::ReadFiles, false)), |_| {});
    store.send(A::ChannelNew(N::Start), |_| {});
    started(&mut store);

    let workspace = fake.workspace();
    let found: Vec<_> = workspace.desk.channel_sessions.values().collect();
    let [one] = found.as_slice() else {
        panic!("one channel Desk session started, got {}", found.len())
    };
    assert_eq!(one.target, Some(ledger));
    assert_eq!(one.channel, payments());
    assert!(one.switches.type_into_terminal);
    assert!(!one.switches.read_files);
    assert!(one.switches.send_notes);
}

#[test]
fn switching_to_a_session_target_picks_the_first_session_of_the_channel() {
    let (mut store, _fake) = store(fixture::design());
    start(&mut store, None, "Watch");
    store.send(A::ChannelNew(N::OnSession(true)), |_| {});
    let picked = store.state().channel_new.as_ref().and_then(|form| form.session.clone());
    assert!(picked.is_some_and(|id| {
        store
            .state()
            .channel_members(&payments())
            .iter()
            .any(|session| session.id == id)
    }));
    store.send(A::ChannelNew(N::OnSession(false)), |_| {});
    assert_eq!(
        store.state().channel_new.as_ref().and_then(|form| form.target().cloned()),
        None
    );
}

#[test]
fn a_switch_whose_channel_rule_is_never_is_blocked_whether_on_or_off() {
    let mut workspace = fixture::design();
    let rule = workspace
        .rules
        .get_mut(&payments())
        .and_then(|rules| {
            rules
                .iter_mut()
                .find(|rule| rule.capability.as_str() == "session_stop_other")
        })
        .expect("the design has the stop rule");
    rule.setting = Some(RuleSetting::Never);
    let (mut store, fake) = store(workspace);
    assert_eq!(store.state().blocked_for(&payments()), vec![Switch::StopAndArchive]);

    start(&mut store, None, "Tidy up");
    store.send(A::ChannelNew(N::Switch(Switch::StopAndArchive, true)), |_| {});
    store.send(A::ChannelNew(N::Start), |_| {});
    started(&mut store);
    let workspace = fake.workspace();
    let Some(record) = workspace.desk.channel_sessions.values().next() else {
        panic!("the session started")
    };
    assert_eq!(record.blocked, vec![Switch::StopAndArchive]);
    assert!(record.switches.stop_and_archive, "on, though the rule still refuses");
}

#[test]
fn a_blank_title_gets_one_from_the_agent_and_no_online_machine_says_so() {
    let mut workspace = fixture::design();
    for machine in &mut workspace.machines {
        machine.online = false;
    }
    let (mut store, fake) = store(workspace);
    start(&mut store, None, "");
    store.send(A::ChannelNew(N::Start), |_| {});
    let Some(form) = store.state().channel_new.as_ref() else {
        panic!("the dialog stays open")
    };
    assert!(form.error.is_some(), "it says why it didn't start");
    assert!(!form.starting);
    assert!(fake.workspace().desk.channel_sessions.is_empty());
}

#[test]
fn closing_asks_first_and_cancelling_leaves_the_session() {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    let (mut store, fake) = store(workspace);
    let id = SessionId::new("cdesk-triage");
    store.send(A::AskClose(id.clone()), |_| {});
    assert!(store.state().channel_close.is_some(), "it asks first");
    assert!(!store.state().pending.contains(&id), "nothing is closing yet");
    store.send(A::CancelDialog, |_| {});
    assert!(store.state().channel_close.is_none());
    assert!(fake.workspace().desk.channel_sessions.contains_key(&id));
}

#[test]
fn confirming_closes_the_session_and_it_is_gone_from_the_channels_desk() {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    let (mut store, fake) = store(workspace);
    let id = SessionId::new("cdesk-triage");
    store.send(A::AskClose(id.clone()), |_| {});
    store.send(A::ConfirmCloseChannel, |_| {});
    store.receive(
        A::Changed {
            session: id.clone(),
            change: Change::Close,
            result: Ok(()),
        },
        |_| {},
    );
    assert!(store.state().channel_close.is_none());
    assert!(!fake.workspace().desk.channel_sessions.contains_key(&id));
    show(&mut store, &fake);
    let titles: Vec<String> = store
        .state()
        .channel_rows(&payments())
        .iter()
        .map(|row| row.session.title.clone())
        .collect();
    assert_eq!(titles, vec!["Watch the backfill".to_owned()]);
}

#[test]
fn a_refused_close_keeps_the_sheet_with_what_went_wrong() {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    let (mut store, fake) = store(workspace);
    fake.fail_next(Operation::Desk(humata2_clients::desk::Operation::Close), "no luck");
    let id = SessionId::new("cdesk-triage");
    store.send(A::AskClose(id.clone()), |_| {});
    store.send(A::ConfirmCloseChannel, |_| {});
    store.receive_matching(
        "Changed(Err(_))",
        |a| matches!(a, A::Changed { result: Err(_), .. }),
        |_| {},
    );
    let Some(sheet) = store.state().channel_close.as_ref() else {
        panic!("the sheet stays")
    };
    assert!(sheet.error.is_some());
    assert!(!sheet.closing, "it can be tried again");
}

#[test]
fn a_switch_change_reaches_the_daemon() {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    let (mut store, fake) = store(workspace);
    let id = SessionId::new("cdesk-triage");
    store.send(A::SetSwitch(id.clone(), Switch::StartAndFork, true), |_| {});
    store.receive(
        A::Changed {
            session: id.clone(),
            change: Change::Switches,
            result: Ok(()),
        },
        |_| {},
    );
    let workspace = fake.workspace();
    let Some(record) = workspace.desk.channel_sessions.get(&id) else {
        panic!("the session is still there")
    };
    assert!(record.switches.start_and_fork);
    assert!(record.switches.read_files, "the others are as they were");
}

fn with_channel_desk() -> Workspace {
    let mut workspace = fixture::design();
    fixture::channel_desk(&mut workspace);
    workspace
}

#[test]
fn a_session_whose_target_went_is_closed_through_the_same_confirmation() {
    let mut workspace = with_channel_desk();
    let id = SessionId::new("cdesk-triage");
    if let Some(record) = workspace.desk.channel_sessions.get_mut(&id) {
        record.target_gone = true;
    }
    let (mut store, fake) = store(workspace);
    assert!(store.state().channel_record(&id).is_some_and(|record| record.target_gone));
    store.send(A::AskClose(id.clone()), |_| {});
    assert!(store.state().channel_close.is_some(), "it asks first, as any close does");
    assert!(fake.workspace().desk.channel_sessions.contains_key(&id));
    store.send(A::ConfirmCloseChannel, |_| {});
    store.receive(
        A::Changed {
            session: id.clone(),
            change: Change::Close,
            result: Ok(()),
        },
        |_| {},
    );
    assert!(!fake.workspace().desk.channel_sessions.contains_key(&id));
}

#[test]
fn every_listed_markdown_file_has_a_line_count_to_show_beside_its_name() {
    let (store, _fake) = store(with_channel_desk());
    let Some(record) = store.state().channel_record(&SessionId::new("cdesk-triage")) else {
        panic!("the fixture has the session")
    };
    assert!(!record.documents.is_empty());
    assert!(record
        .documents
        .iter()
        .all(|name| record.document_lines.contains_key(name)));
}

fn closed_in(workspace: &mut Workspace, id: &str, kept: bool, versions: u32) {
    if let Some(record) = workspace.desk.channel_sessions.get_mut(&SessionId::new(id)) {
        record.desk.closed = Some(humata2_clients::desk::Closed {
            when: "today".to_owned(),
            days_left: (!kept).then_some(7),
        });
        record.desk.folder = humata2_clients::desk::Folder::Scratch {
            path: format!("~/Humata/payments-rewrite/desk/{id}"),
            kept,
        };
        record.desk.version_count = versions;
    }
}

fn copy_into(workspace: &mut Workspace, from: &str, id: &str, channel: &str) {
    let Some(session) = workspace
        .sessions
        .iter()
        .find(|session| session.id.as_str() == from)
        .cloned()
    else {
        panic!("the fixture has {from}")
    };
    let Some(record) = workspace
        .desk
        .channel_sessions
        .get(&SessionId::new(from))
        .cloned()
    else {
        panic!("the fixture has a record for {from}")
    };
    let mut session = session;
    session.id = SessionId::new(id);
    let mut record = record;
    record.desk.session = SessionId::new(id);
    record.channel = ChannelId::new(channel);
    workspace.sessions.push(session);
    workspace
        .desk
        .channel_sessions
        .insert(SessionId::new(id), record);
}

#[test]
fn a_closed_kept_desk_folder_is_listed_under_closed_with_its_documents() {
    let mut workspace = with_channel_desk();
    closed_in(&mut workspace, "cdesk-triage", true, 2);
    let (store, _fake) = store(workspace);
    let state = store.state();
    let closed = state.channel_closed(&payments());
    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].session.id, SessionId::new("cdesk-triage"));
    assert!(state
        .channel_rows(&payments())
        .iter()
        .all(|row| row.session.id != SessionId::new("cdesk-triage")));
    let documents = state.channel_documents(&payments());
    let names: Vec<&str> = documents.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["flaky-report.md", "retry-plan.md"]);
}

#[test]
fn a_folder_with_a_version_is_kept_even_when_nobody_pressed_keep() {
    let mut workspace = with_channel_desk();
    closed_in(&mut workspace, "cdesk-triage", false, 1);
    let (store, _fake) = store(workspace);
    assert_eq!(store.state().channel_closed(&payments()).len(), 1);
}

#[test]
fn a_closed_unkept_desk_folder_is_not_listed_and_its_documents_are_not_either() {
    let mut workspace = with_channel_desk();
    closed_in(&mut workspace, "cdesk-triage", false, 0);
    let (store, _fake) = store(workspace);
    let state = store.state();
    assert!(state.channel_closed(&payments()).is_empty());
    assert!(state.channel_documents(&payments()).is_empty());
}

#[test]
fn another_channels_kept_folder_is_not_listed() {
    let mut workspace = with_channel_desk();
    copy_into(&mut workspace, "cdesk-triage", "cdesk-other", "other-channel");
    closed_in(&mut workspace, "cdesk-other", true, 3);
    let (store, _fake) = store(workspace);
    let state = store.state();
    assert!(state.channel_closed(&payments()).is_empty());
    // Payments still lists its own open sessions' files, but none of the
    // other channel's kept folder.
    assert!(state
        .channel_documents(&payments())
        .iter()
        .all(|(_, row)| row.session.id != SessionId::new("cdesk-other")));
    assert_eq!(state.channel_closed(&ChannelId::new("other-channel")).len(), 1);
}

#[test]
fn open_documents_are_listed_before_closed_kept_ones() {
    let mut workspace = with_channel_desk();
    copy_into(&mut workspace, "cdesk-triage", "cdesk-old", "payments-rewrite");
    closed_in(&mut workspace, "cdesk-old", true, 1);
    let (store, _fake) = store(workspace);
    let documents = store.state().channel_documents(&payments());
    let from: Vec<&str> = documents
        .iter()
        .map(|(_, row)| row.session.id.as_str())
        .collect();
    assert_eq!(from, ["cdesk-triage", "cdesk-triage", "cdesk-old", "cdesk-old"]);
}
