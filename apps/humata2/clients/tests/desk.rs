//! The fake daemon's Desk: starting, closing, reopening, keeping, cleaning
//! up and copying documents, against the Desk of frames 36 and 55.

use std::sync::Arc;

use humata2_clients::desk::{
    self, Access, CleanUp, Document, FOLDER, Folder, FolderChoice, NewDeskSession,
};
use humata2_clients::model::{Agent, Channel, ChannelId, MachineId, RequestId, SessionId, Status};
use humata2_clients::{
    Daemon, DaemonError, DeskClient, FakeDaemon, Operation, Terminals, Workspace, fixture,
};
use tca::TestClock;
use tokio::runtime::Builder;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(future),
        Err(error) => panic!("a runtime starts: {error}"),
    }
}

fn with_desk() -> Workspace {
    let mut workspace = fixture::design();
    fixture::desk(&mut workspace);
    workspace
}

fn fake(workspace: Workspace) -> FakeDaemon {
    FakeDaemon::new(workspace, Arc::new(TestClock::new()))
}

fn id(text: &str) -> SessionId {
    SessionId::new(text)
}

fn scratch(name: &str, machine: &str) -> NewDeskSession {
    NewDeskSession {
        title: "Plan the quarter".to_owned(),
        agent: Agent::Claude,
        machine: MachineId::new(machine),
        folder: FolderChoice::Scratch {
            name: name.to_owned(),
        },
        repos: Vec::new(),
        access: Access::default(),
    }
}

fn closed_days(daemon: &FakeDaemon, session: &str) -> Option<u32> {
    let workspace = daemon.workspace();
    let Some(record) = workspace.desk.sessions.get(&id(session)) else {
        panic!("{session} is on the Desk");
    };
    let Some(closed) = &record.closed else {
        panic!("{session} is closed");
    };
    closed.days_left
}

fn order(daemon: &FakeDaemon, session: &str) -> u64 {
    let workspace = daemon.workspace();
    match workspace.desk.sessions.get(&id(session)) {
        Some(record) => record.last_used.order,
        None => panic!("{session} is on the Desk"),
    }
}

#[test]
fn starting_in_a_scratch_folder_opens_a_desk_session() {
    let daemon = fake(with_desk());
    let access = Access {
        boards: false,
        channels_and_machines: true,
    };
    let mut new = scratch("quarter", "this-mac");
    new.access = access;
    let started = block_on(daemon.start(new)).expect("the session starts");
    let workspace = daemon.workspace();
    let Some(session) = workspace.session(&started) else {
        panic!("the session is in the workspace");
    };
    assert_eq!(session.channel, desk::channel());
    assert_eq!(session.channel.as_str(), FOLDER);
    assert!(!session.archived);
    let Some(record) = workspace.desk.sessions.get(&started) else {
        panic!("the Desk knows the session");
    };
    assert_eq!(
        record.folder,
        Folder::Scratch {
            path: "~/Humata/desk/quarter".to_owned(),
            kept: false
        }
    );
    assert_eq!(record.access, access);
    assert!(record.closed.is_none());
    assert!(order(&daemon, started.as_str()) > order(&daemon, "desk-plan-q4"));
}

#[test]
fn a_taken_scratch_name_gets_a_suffix() {
    let daemon = fake(with_desk());
    let first = block_on(daemon.start(scratch("plan-q4", "this-mac"))).expect("it starts");
    let second = block_on(daemon.start(scratch("plan-q4", "this-mac"))).expect("it starts");
    let workspace = daemon.workspace();
    let path = |session: &SessionId| {
        workspace
            .desk
            .sessions
            .get(session)
            .map(|record| record.folder.path().to_owned())
    };
    assert_eq!(path(&first).as_deref(), Some("~/Humata/desk/plan-q4-2"));
    assert_eq!(path(&second).as_deref(), Some("~/Humata/desk/plan-q4-3"));
}

#[test]
fn starting_on_an_offline_host_is_offline() {
    let daemon = fake(with_desk());
    let before = daemon.workspace().desk.sessions.len();
    let result = block_on(daemon.start(scratch("quarter", "gpu-box")));
    assert!(matches!(result, Err(DaemonError::Offline { .. })));
    assert_eq!(daemon.workspace().desk.sessions.len(), before);
}

#[test]
fn a_picked_folder_keeps_the_path_given() {
    let daemon = fake(with_desk());
    let mut new = scratch("unused", "this-mac");
    new.folder = FolderChoice::Picked {
        path: "~/Documents/taxes".to_owned(),
    };
    let started = block_on(daemon.start(new)).expect("the session starts");
    let workspace = daemon.workspace();
    let folder = workspace
        .desk
        .sessions
        .get(&started)
        .map(|record| record.folder.clone());
    assert_eq!(
        folder,
        Some(Folder::Picked {
            path: "~/Documents/taxes".to_owned()
        })
    );
}

#[test]
fn an_empty_title_or_folder_name_is_refused() {
    let daemon = fake(with_desk());
    let mut untitled = scratch("quarter", "this-mac");
    untitled.title = "   ".to_owned();
    assert!(matches!(
        block_on(daemon.start(untitled)),
        Err(DaemonError::Refused { .. })
    ));
    assert!(matches!(
        block_on(daemon.start(scratch("a/b", "this-mac"))),
        Err(DaemonError::Refused { .. })
    ));
}

#[test]
fn closing_gives_seven_days_only_to_a_scratch_folder_nobody_kept() {
    let daemon = fake(with_desk());
    for session in ["desk-plan-q4", "desk-brin-rfc", "desk-tidy-infra"] {
        block_on(daemon.close(&id(session))).expect("it closes");
    }
    assert_eq!(closed_days(&daemon, "desk-plan-q4"), Some(7));
    assert_eq!(closed_days(&daemon, "desk-brin-rfc"), None);
    assert_eq!(closed_days(&daemon, "desk-tidy-infra"), None);
    let workspace = daemon.workspace();
    let Some(session) = workspace.session(&id("desk-plan-q4")) else {
        panic!("the session is still there");
    };
    assert!(session.archived);
    assert_eq!(session.status, Status::Exited { code: 0 });
}

#[test]
fn closing_twice_is_refused_and_a_scripted_failure_is_scripted() {
    let daemon = fake(with_desk());
    block_on(daemon.close(&id("desk-plan-q4"))).expect("it closes");
    assert!(matches!(
        block_on(daemon.close(&id("desk-plan-q4"))),
        Err(DaemonError::Refused { .. })
    ));
    daemon.fail_next(Operation::Desk(desk::Operation::Close), "disk is full");
    assert!(matches!(
        block_on(daemon.close(&id("desk-brin-rfc"))),
        Err(DaemonError::Scripted { .. })
    ));
}

#[test]
fn reopening_opens_it_again_as_the_most_recently_used() {
    let daemon = fake(with_desk());
    block_on(daemon.reopen(&id("desk-release-checklist"))).expect("it reopens");
    let workspace = daemon.workspace();
    let Some(session) = workspace.session(&id("desk-release-checklist")) else {
        panic!("the session is there");
    };
    assert!(!session.archived);
    assert_eq!(session.status, Status::Idle);
    let Some(record) = workspace.desk.sessions.get(&id("desk-release-checklist")) else {
        panic!("the Desk knows it");
    };
    assert!(record.closed.is_none());
    assert_eq!(record.last_used.label, "now");
    assert!(record.last_used.order > order(&daemon, "desk-plan-q4"));
    assert!(matches!(
        block_on(daemon.reopen(&id("desk-plan-q4"))),
        Err(DaemonError::Refused { .. })
    ));
}

#[test]
fn keeping_a_closed_scratch_folder_clears_its_days() {
    let daemon = fake(with_desk());
    assert_eq!(closed_days(&daemon, "desk-release-checklist"), Some(6));
    block_on(daemon.keep(&id("desk-release-checklist"))).expect("it is kept");
    assert_eq!(closed_days(&daemon, "desk-release-checklist"), None);
    let workspace = daemon.workspace();
    let folder = workspace
        .desk
        .sessions
        .get(&id("desk-release-checklist"))
        .map(|record| record.folder.clone());
    assert!(matches!(folder, Some(Folder::Scratch { kept: true, .. })));
    block_on(daemon.keep(&id("desk-tidy-infra"))).expect("a picked folder stays as it is");
}

#[test]
fn the_clean_up_list_is_least_recently_used_first_with_sizes() {
    let daemon = fake(with_desk());
    let rows = block_on(daemon.clean_up_list()).expect("the list loads");
    let listed: Vec<_> = rows
        .iter()
        .map(|row| (row.session.as_str().to_owned(), row.size_bytes))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("desk-onboarding-doc".to_owned(), Some(48_000_000)),
            ("desk-port-cron".to_owned(), Some(2_300_000_000)),
            ("desk-notes-cleanup".to_owned(), None),
            ("desk-release-checklist".to_owned(), Some(120_000_000)),
        ]
    );
    assert!(rows[1].unpushed.is_some());
    assert!(rows[0].unpushed.is_none());
}

#[test]
fn cleaning_up_trashes_the_sessions_and_reports_what_was_freed() {
    let daemon = fake(with_desk());
    let before = daemon.workspace().trash.len();
    let rows = block_on(daemon.clean_up_list()).expect("the list loads");
    let delete = vec![id("desk-onboarding-doc"), id("desk-notes-cleanup")];
    let result = block_on(daemon.clean_up(rows, delete)).expect("the clean-up runs");
    assert_eq!(
        result,
        CleanUp::Deleted {
            sessions: 2,
            freed_bytes: 48_000_000
        }
    );
    let workspace = daemon.workspace();
    assert!(workspace.session(&id("desk-onboarding-doc")).is_none());
    assert!(workspace.session(&id("desk-notes-cleanup")).is_none());
    assert!(workspace.session(&id("desk-port-cron")).is_some());
    assert!(
        !workspace
            .desk
            .sessions
            .contains_key(&id("desk-onboarding-doc"))
    );
    assert_eq!(workspace.trash.len(), before + 2);
}

#[test]
fn cleaning_up_an_unknown_session_deletes_nothing() {
    let daemon = fake(with_desk());
    let rows = block_on(daemon.clean_up_list()).expect("the list loads");
    let delete = vec![id("desk-onboarding-doc"), id("desk-plan-q4")];
    let result = block_on(daemon.clean_up(rows, delete));
    assert!(matches!(result, Err(DaemonError::NotFound { .. })));
    assert!(daemon.workspace().session(&id("desk-onboarding-doc")).is_some());
}

#[test]
fn cleaning_up_from_a_stale_list_changes_nothing_and_returns_the_list() {
    let daemon = fake(with_desk());
    let stale = block_on(daemon.clean_up_list()).expect("the list loads");
    block_on(daemon.close(&id("desk-plan-q4"))).expect("another window closes a session");
    let result = block_on(daemon.clean_up(stale.clone(), vec![id("desk-onboarding-doc")]))
        .expect("the call answers");
    let CleanUp::Changed(now) = result else {
        panic!("a stale list must come back as Changed");
    };
    assert_eq!(now.len(), stale.len() + 1);
    assert!(daemon.workspace().session(&id("desk-onboarding-doc")).is_some());
}

fn with_one_machine_channel() -> Workspace {
    let mut workspace = with_desk();
    workspace.channels.push(Channel {
        id: ChannelId::new("solo"),
        name: "solo".to_owned(),
        repos: Vec::new(),
        machines: vec![MachineId::new("this-mac")],
        purpose: String::new(),
    });
    workspace
}

fn rfc() -> Document {
    Document {
        session: id("desk-brin-rfc"),
        path: "rfc-brin.md".to_owned(),
    }
}

#[test]
fn copying_to_a_one_machine_channel_adds_the_file_and_keeps_the_document() {
    let daemon = fake(with_one_machine_channel());
    block_on(daemon.copy_document(&rfc(), &ChannelId::new("solo"), None)).expect("it copies");
    let workspace = daemon.workspace();
    let copied = workspace.files.iter().find(|file| {
        file.machine == MachineId::new("this-mac")
            && file.path == "~/Humata/solo/shared/rfc-brin.md"
    });
    assert!(copied.is_some_and(|file| !file.folder));
    assert!(workspace.desk.documents.contains(&rfc()));
}

#[test]
fn copying_to_a_channel_on_several_machines_needs_the_machine() {
    let daemon = fake(with_desk());
    let channel = ChannelId::new("web");
    assert!(matches!(
        block_on(daemon.copy_document(&rfc(), &channel, None)),
        Err(DaemonError::Refused { .. })
    ));
    block_on(daemon.copy_document(&rfc(), &channel, Some(&MachineId::new("devbox-1"))))
        .expect("it copies once the machine is named");
    assert!(
        daemon
            .workspace()
            .files
            .iter()
            .any(|file| file.path == "~/Humata/web/shared/rfc-brin.md")
    );
    assert!(matches!(
        block_on(daemon.copy_document(&rfc(), &channel, Some(&MachineId::new("gpu-box")))),
        Err(DaemonError::Refused { .. })
    ));
}

#[test]
fn using_a_desk_session_makes_it_the_most_recently_used() {
    let daemon = fake(with_desk());
    daemon.use_desk_session(&id("desk-tidy-infra"));
    for other in ["desk-plan-q4", "desk-brin-rfc", "desk-release-checklist"] {
        assert!(order(&daemon, "desk-tidy-infra") > order(&daemon, other));
    }
    let before = daemon.workspace();
    daemon.use_desk_session(&id("not-a-session"));
    assert_eq!(daemon.workspace(), before);
}

#[test]
fn repositories_are_added_once_and_removed_when_present() {
    let daemon = fake(with_desk());
    let session = id("desk-plan-q4");
    block_on(daemon.add_repo(&session, "acme/web")).expect("it is added");
    assert!(matches!(
        block_on(daemon.add_repo(&session, "acme/web")),
        Err(DaemonError::Refused { .. })
    ));
    assert!(matches!(
        block_on(daemon.add_repo(&session, "not a repo")),
        Err(DaemonError::Refused { .. })
    ));
    block_on(daemon.remove_repo(&session, "acme/web")).expect("it is removed");
    assert!(matches!(
        block_on(daemon.remove_repo(&session, "acme/web")),
        Err(DaemonError::NotFound { .. })
    ));
}

#[test]
fn moving_needs_two_online_hosts_and_an_open_session() {
    let daemon = fake(with_desk());
    let session = id("desk-plan-q4");
    assert!(matches!(
        block_on(daemon.move_to(&session, &MachineId::new("this-mac"))),
        Err(DaemonError::Refused { .. })
    ));
    assert!(matches!(
        block_on(daemon.move_to(&session, &MachineId::new("gpu-box"))),
        Err(DaemonError::Offline { .. })
    ));
    block_on(daemon.move_to(&session, &MachineId::new("devbox-1"))).expect("it moves");
    let workspace = daemon.workspace();
    assert_eq!(
        workspace.session(&session).map(|found| found.machine.clone()),
        Some(MachineId::new("devbox-1"))
    );
    assert!(matches!(
        block_on(daemon.move_to(&id("desk-release-checklist"), &MachineId::new("devbox-1"))),
        Err(DaemonError::Refused { .. })
    ));
}

#[test]
fn a_folder_name_is_the_titles_slug_cut_at_a_word() {
    assert_eq!(desk::folder_name("Plan Q4: board!"), "plan-q4-board");
    assert_eq!(desk::folder_name("  ---  "), "");
    assert_eq!(
        desk::folder_name("Write the quarterly infrastructure migration retrospective notes"),
        "write-the-quarterly-infrastructure"
    );
}

/// The Desk session that was used most recently.
fn latest(daemon: &FakeDaemon) -> SessionId {
    let workspace = daemon.workspace();
    let Some((session, _)) = workspace
        .desk
        .sessions
        .iter()
        .max_by_key(|(_, record)| record.last_used.order)
    else {
        panic!("the Desk has sessions");
    };
    session.clone()
}

/// A channel session that takes input, not on the Desk.
fn channel_session(workspace: &Workspace) -> SessionId {
    let Some(session) = workspace
        .sessions
        .iter()
        .find(|session| !desk::is_desk(session) && !session.archived && session.status.accepts_input())
    else {
        panic!("the design has a channel session that takes input");
    };
    session.id.clone()
}

/// The design workspace's Desk with a waiting request from `session`.
fn with_request_from(session: &str) -> Workspace {
    let mut workspace = with_desk();
    let Some(mut request) = workspace.waiting().next().cloned() else {
        panic!("the design has a waiting request");
    };
    request.id = RequestId::new("desk-request");
    request.session = id(session);
    request.channel = desk::channel();
    workspace.requests.push(request);
    workspace
}

#[test]
fn typing_into_a_desk_agent_makes_it_the_most_recent() {
    let daemon = fake(with_desk());
    assert_eq!(latest(&daemon), id("desk-plan-q4"));
    block_on(daemon.send(&id("desk-tidy-infra"), "ls")).expect("the agent takes input");
    assert_eq!(latest(&daemon), id("desk-tidy-infra"));
}

#[test]
fn typing_bytes_into_a_desk_terminal_makes_it_the_most_recent() {
    let daemon = fake(with_desk());
    block_on(daemon.input(&id("desk-brin-rfc"), 0, b"ls".to_vec())).expect("tab 0 takes input");
    assert_eq!(latest(&daemon), id("desk-brin-rfc"));
}

#[test]
fn writing_a_line_into_a_desk_terminal_makes_it_the_most_recent() {
    let daemon = fake(with_desk());
    block_on(daemon.write(&id("desk-tidy-infra"), 0, "ls")).expect("tab 0 takes input");
    assert_eq!(latest(&daemon), id("desk-tidy-infra"));
}

#[test]
fn opening_a_desk_session_makes_it_the_most_recent() {
    let daemon = fake(with_desk());
    block_on(daemon.mark_read(&id("desk-brin-rfc"))).expect("it opens");
    assert_eq!(latest(&daemon), id("desk-brin-rfc"));
}

#[test]
fn answering_a_request_from_a_desk_session_makes_it_the_most_recent() {
    let daemon = fake(with_request_from("desk-tidy-infra"));
    block_on(daemon.answer(&RequestId::new("desk-request"), humata2_clients::model::Answer::Approved))
        .expect("the request is answered");
    assert_eq!(latest(&daemon), id("desk-tidy-infra"));
}

#[test]
fn using_a_channel_session_leaves_the_desk_alone() {
    let workspace = with_desk();
    let session = channel_session(&workspace);
    let before = workspace.desk.clone();
    let daemon = fake(workspace);
    block_on(daemon.send(&session, "ls")).expect("the agent takes input");
    block_on(daemon.input(&session, 0, b"ls".to_vec())).expect("tab 0 takes input");
    block_on(daemon.write(&session, 0, "ls")).expect("tab 0 takes input");
    block_on(daemon.mark_read(&session)).expect("it opens");
    assert_eq!(daemon.workspace().desk, before);
}

#[test]
fn answering_a_channel_request_leaves_the_desk_alone() {
    let workspace = with_desk();
    let before = workspace.desk.clone();
    let Some(request) = workspace.waiting().next().map(|request| request.id.clone()) else {
        panic!("the design has a waiting request");
    };
    let daemon = fake(workspace);
    block_on(daemon.answer(&request, humata2_clients::model::Answer::Approved))
        .expect("the request is answered");
    assert_eq!(daemon.workspace().desk, before);
}

#[test]
fn no_channel_is_named_after_the_desk_folder() {
    for name in ["desk", "Desk"] {
        let daemon = fake(with_desk());
        let channels = daemon.workspace().channels.len();
        assert!(matches!(
            block_on(daemon.create_channel(name, Vec::new(), Vec::new())),
            Err(DaemonError::Refused { .. })
        ));
        assert_eq!(daemon.workspace().channels.len(), channels);
    }
}

#[test]
fn a_place_reads_desk_for_the_desk_and_the_channel_name_otherwise() {
    let workspace = with_desk();
    assert_eq!(desk::place(&workspace, &desk::channel()), "Desk");
    let Some(channel) = workspace.channels.first() else {
        panic!("the design has a channel");
    };
    assert_eq!(
        desk::place(&workspace, &channel.id),
        format!("# {}", channel.name)
    );
    assert_eq!(desk::place(&workspace, &ChannelId::new("nowhere")), "nowhere");
}

#[test]
fn a_clone_sits_on_its_repositorys_default_branch_until_it_is_removed() {
    let daemon = fake(with_desk());
    let session = id("desk-plan-q4");
    let key = (session.clone(), "acme/web".to_owned());
    block_on(daemon.add_repo(&session, "acme/web")).expect("it is added");
    assert_eq!(
        daemon.workspace().desk.clone_branches.get(&key).map(String::as_str),
        Some("main")
    );
    block_on(daemon.remove_repo(&session, "acme/web")).expect("it is removed");
    assert!(!daemon.workspace().desk.clone_branches.contains_key(&key));
}

#[test]
fn one_waiting_request_stands_behind_plan_q4s_inbox_badge() {
    let waiting = |workspace: &Workspace| -> Vec<(String, String, String)> {
        workspace
            .requests
            .iter()
            .filter(|request| request.answer.is_none())
            .map(|request| {
                (
                    request.id.to_string(),
                    request.session.to_string(),
                    request.channel.to_string(),
                )
            })
            .collect()
    };
    let before = waiting(&fixture::design());
    let workspace = with_desk();
    let after = waiting(&workspace);
    let plan: Vec<_> = after
        .iter()
        .filter(|(_, session, _)| session == "desk-plan-q4")
        .collect();
    assert_eq!(plan.len(), 1, "{after:?}");
    assert_eq!(plan[0].2, desk::channel().to_string());
    assert_eq!(&after[..before.len()], &before[..]);
    assert_eq!(after.len(), before.len() + 1);
    let status = workspace
        .sessions
        .iter()
        .find(|session| session.id == id("desk-plan-q4"))
        .map(|session| session.status.clone());
    assert_eq!(status, Some(Status::Working));
}
