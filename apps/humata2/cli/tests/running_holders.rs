//! D784 from the CLI over a fake daemon: `humata session stop|resume|end`
//! pick the call by whether the target is a session or a shell tab, `end`
//! of a session archives it by the plan, `session running` lists a
//! machine's holders, and an offline machine's refusal ends the command.

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

fn tab(value: &str) -> Option<pb::TabId> {
    Some(pb::TabId { value: value.into() })
}

#[tokio::test]
async fn stop_with_a_tab_stops_that_shell_on_the_machine_named() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::STOP_RUNNING_HOLDER, &pb::StopRunningHolderResponse {});
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--machine", "ma_1", "session", "stop", "--tab", "tb_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::StopRunningHolderRequest = daemon.only(rpc::machine::STOP_RUNNING_HOLDER);
    assert_eq!((sent.machine, sent.tab), (support::machine("ma_1"), tab("tb_1")));
    assert!(!daemon.called(rpc::session::STOP_SESSION));
}

#[tokio::test]
async fn stop_of_a_session_still_stops_the_session() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "session", "stop", "se_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::StopSessionRequest = daemon.only(rpc::session::STOP_SESSION);
    assert_eq!(sent.id, support::session("se_1"));
    assert!(!daemon.called(rpc::machine::STOP_RUNNING_HOLDER));
}

#[tokio::test]
async fn resume_with_a_tab_resumes_that_shell() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::RESUME_RUNNING_HOLDER, &pb::ResumeRunningHolderResponse {});
    here(&daemon);
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "session", "resume", "--tab", "tb_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::ResumeRunningHolderRequest = daemon.only(rpc::machine::RESUME_RUNNING_HOLDER);
    assert_eq!(sent.tab, tab("tb_1"));
    assert!(!daemon.called(rpc::session::RESUME_SESSION));
}

#[tokio::test]
async fn end_with_a_tab_ends_that_shell_and_archives_nothing() {
    let daemon = FakeDaemon::new();
    daemon.reply(rpc::machine::END_RUNNING_HOLDER, &pb::EndRunningHolderResponse {});
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--yes", "--machine", "ma_1", "session", "end", "--tab", "tb_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::EndRunningHolderRequest = daemon.only(rpc::machine::END_RUNNING_HOLDER);
    assert_eq!((sent.machine, sent.tab), (support::machine("ma_1"), tab("tb_1")));
    assert!(!daemon.called(rpc::session::PLAN_ARCHIVE));
}

#[tokio::test]
async fn end_of_a_session_archives_it_by_the_plan() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::session::PLAN_ARCHIVE,
        &pb::ArchivePlan {
            token: Some(pb::PlanToken { value: "plan-a".into() }),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--yes", "session", "end", "se_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::ArchiveSessionRequest = daemon.only(rpc::session::ARCHIVE_SESSION);
    assert_eq!(sent.id, support::session("se_1"));
    assert_eq!(sent.choice, pb::ArchiveChoice::Archive as i32);
    assert_eq!(sent.plan.map(|plan| plan.value), Some("plan-a".to_owned()));
    assert!(!daemon.called(rpc::machine::END_RUNNING_HOLDER));
}

#[tokio::test]
async fn end_of_a_desk_session_closes_it_on_the_desk() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::session::GET_SESSION,
        &pb::Session {
            id: support::session("se_1"),
            channel: Some(pb::ChannelId { value: "desk".into() }),
            ..Default::default()
        },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--yes", "session", "end", "se_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::CloseDeskSessionRequest = daemon.only(rpc::desk::CLOSE_DESK_SESSION);
    assert_eq!(sent.session, support::session("se_1"));
    assert!(!daemon.called(rpc::session::ARCHIVE_SESSION));
    assert!(!daemon.called(rpc::session::PLAN_ARCHIVE));
}

#[tokio::test]
async fn neither_a_session_nor_a_tab_is_a_usage_error() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    for command in ["stop", "end"] {
        let code = run(&daemon, &console, &["--json", "--yes", "session", command]).await;
        assert_eq!(code, exit::USAGE, "{command}");
    }
    assert!(!daemon.called(rpc::session::STOP_SESSION));
    assert!(!daemon.called(rpc::machine::STOP_RUNNING_HOLDER));
    assert!(!daemon.called(rpc::machine::END_RUNNING_HOLDER));
}

#[tokio::test]
async fn an_offline_machine_refuses_and_the_command_fails_as_offline() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::machine::END_RUNNING_HOLDER,
        pb::error::Kind::MachineOffline(Default::default()),
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--yes", "--machine", "ma_2", "session", "end", "--tab", "tb_1"]).await;
    assert_eq!(code, exit::UNREACHABLE, "{}", console.stderr());
}

#[tokio::test]
async fn running_lists_the_machines_holders() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::LIST_RUNNING_HOLDERS,
        &pb::ListRunningHoldersResponse {
            holders: vec![pb::RunningHolder {
                tab: tab("tb_1"),
                agent: "shell".into(),
                pid: 4000,
                ..Default::default()
            }],
        },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--machine", "ma_1", "session", "running"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::ListRunningHoldersRequest = daemon.only(rpc::machine::LIST_RUNNING_HOLDERS);
    assert_eq!(sent.machine, support::machine("ma_1"));
    let json = console.json();
    assert_eq!(json["result"]["holders"][0]["tab"], "tb_1", "{json}");
}

#[tokio::test]
async fn adopt_puts_a_session_less_holder_into_the_channel() {
    let daemon = FakeDaemon::new();
    daemon.reply(
        rpc::machine::ADOPT_RUNNING_HOLDER,
        &pb::AdoptRunningHolderResponse {
            session: support::session("se_9"),
        },
    );
    daemon.reply(
        rpc::machine::LIST_RUNNING_HOLDERS,
        &pb::ListRunningHoldersResponse {
            holders: vec![pb::RunningHolder {
                tab: tab("tb_1"),
                session: support::session("se_9"),
                agent: "shell".into(),
                ..Default::default()
            }],
        },
    );
    let console = FakeConsole::pipe();
    let adopt = ["--json", "--machine", "ma_1", "--channel", "ch_1", "session", "adopt", "tb_1"];
    let code = run(&daemon, &console, &adopt).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::AdoptRunningHolderRequest = daemon.only(rpc::machine::ADOPT_RUNNING_HOLDER);
    assert_eq!(
        (sent.machine, sent.tab, sent.channel),
        (support::machine("ma_1"), tab("tb_1"), support::channel("ch_1"))
    );
    assert_eq!(console.json()["result"]["session"], "se_9");

    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--machine", "ma_1", "session", "running"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    assert_eq!(console.json()["result"]["holders"][0]["session"], "se_9");
}

#[tokio::test]
async fn adopt_without_a_channel_sends_none_so_the_desk_gets_it() {
    let daemon = FakeDaemon::new();
    here(&daemon);
    daemon.reply(
        rpc::machine::ADOPT_RUNNING_HOLDER,
        &pb::AdoptRunningHolderResponse {
            session: support::session("se_9"),
        },
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "session", "adopt", "tb_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::AdoptRunningHolderRequest = daemon.only(rpc::machine::ADOPT_RUNNING_HOLDER);
    assert_eq!(sent.channel, None);
}

#[tokio::test]
async fn adopt_refused_by_the_daemon_fails_the_command() {
    let daemon = FakeDaemon::new();
    daemon.refuse(
        rpc::machine::ADOPT_RUNNING_HOLDER,
        pb::error::Kind::MachineOffline(Default::default()),
    );
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "--machine", "ma_2", "session", "adopt", "tb_1"]).await;
    assert_eq!(code, exit::UNREACHABLE, "{}", console.stderr());
}

#[tokio::test]
async fn attach_by_tab_alone_reaches_the_tab_without_looking_up_a_session() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--machine", "ma_1", "attach", "--tab", "tb_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    assert!(daemon.called(rpc::terminal::ATTACH));
    assert!(!daemon.called(rpc::terminal::LIST_TABS));
}

#[tokio::test]
async fn attach_with_neither_a_session_nor_a_tab_is_a_usage_error() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["attach"]).await;
    assert_eq!(code, exit::USAGE);
    assert!(!daemon.called(rpc::terminal::ATTACH));
}

#[tokio::test]
async fn restore_with_stopped_asks_for_the_session_to_come_back_stopped() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "session", "restore", "--stopped", "se_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::RestoreSessionRequest = daemon.only(rpc::session::RESTORE_SESSION);
    assert_eq!((sent.id, sent.stopped), (support::session("se_1"), true));
}

#[tokio::test]
async fn a_plain_restore_brings_the_session_back_running() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["--json", "session", "restore", "se_1"]).await;
    assert_eq!(code, exit::OK, "{}", console.stderr());
    let sent: pb::RestoreSessionRequest = daemon.only(rpc::session::RESTORE_SESSION);
    assert!(!sent.stopped);
}

fn here(daemon: &FakeDaemon) {
    daemon.reply(
        rpc::daemon::GET_SERVER_INFO,
        &pb::ServerInfo {
            machine: support::machine("ma_here"),
            ..Default::default()
        },
    );
}

#[tokio::test]
async fn without_a_machine_holder_commands_name_this_machine() {
    let daemon = FakeDaemon::new();
    for _ in 0..4 {
        here(&daemon);
    }
    let console = FakeConsole::pipe();
    for words in [
        &["--json", "session", "running"][..],
        &["--json", "session", "stop", "--tab", "tb_1"][..],
        &["--json", "--yes", "session", "end", "--tab", "tb_1"][..],
        &["--json", "session", "adopt", "tb_1"][..],
    ] {
        assert_eq!(run(&daemon, &console, words).await, exit::OK, "{words:?}: {}", console.stderr());
    }
    let machines = [
        daemon.only::<pb::ListRunningHoldersRequest>(rpc::machine::LIST_RUNNING_HOLDERS).machine,
        daemon.only::<pb::StopRunningHolderRequest>(rpc::machine::STOP_RUNNING_HOLDER).machine,
        daemon.only::<pb::EndRunningHolderRequest>(rpc::machine::END_RUNNING_HOLDER).machine,
        daemon.only::<pb::AdoptRunningHolderRequest>(rpc::machine::ADOPT_RUNNING_HOLDER).machine,
    ];
    assert!(machines.iter().all(|machine| *machine == support::machine("ma_here")), "{machines:?}");
}
