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
