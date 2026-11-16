//! `humata report record|stop|snapshot` (I9, D31), answered by the app (D133).

mod support;

use humata2_cli::{DaemonError, ReportReply, ReportRequest, exit};
use humata2_report::remote::{Refusal, RefusalKind, RemoteError};
use support::{FakeConsole, FakeDaemon, run};

#[tokio::test]
async fn record_starts_a_recording_with_its_title() {
    let daemon = FakeDaemon::new();
    daemon.report_replies(Ok(ReportReply {
        folder: String::new(),
        recording: true,
    }));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "report", "record", "--title", "Board jumps"]).await;
    assert_eq!(exit, exit::OK);
    assert_eq!(
        daemon.reports(),
        vec![ReportRequest::Record {
            title: Some("Board jumps".into())
        }]
    );
    assert_eq!(console.json()["result"]["recording"], true);
}

#[tokio::test]
async fn stop_prints_where_the_report_was_saved() {
    let daemon = FakeDaemon::new();
    daemon.report_replies(Ok(ReportReply {
        folder: "/reports/2026-11-14-board-jumps".into(),
        recording: false,
    }));
    let console = FakeConsole::pipe();
    assert_eq!(run(&daemon, &console, &["--json", "report", "stop"]).await, exit::OK);
    assert_eq!(daemon.reports(), vec![ReportRequest::Stop]);
    let result = &console.json()["result"];
    assert_eq!(result["folder"], "/reports/2026-11-14-board-jumps");
    assert_eq!(result["recording"], false);
}

#[tokio::test]
async fn snapshot_without_a_title_sends_none() {
    let daemon = FakeDaemon::new();
    daemon.report_replies(Ok(ReportReply {
        folder: "/reports/now".into(),
        recording: false,
    }));
    let console = FakeConsole::pipe();
    run(&daemon, &console, &["--json", "report", "snapshot"]).await;
    assert_eq!(daemon.reports(), vec![ReportRequest::Snapshot { title: None }]);
}

#[tokio::test]
async fn a_daemon_without_reports_says_so() {
    let daemon = FakeDaemon::new();
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "report", "record"]).await;
    assert_eq!(exit, exit::PRECONDITION);
    assert_eq!(console.json()["error"]["kind"], "unimplemented");
}

#[tokio::test]
async fn without_the_app_running_it_says_to_open_it() {
    let daemon = FakeDaemon::new();
    daemon.report_replies(Err(DaemonError::App {
        source: RemoteError::NotRunning {
            path: "/home/me/.humata2/app.sock".into(),
        },
    }));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "report", "snapshot"]).await;
    assert_eq!(exit, exit::PRECONDITION);
    assert_eq!(console.json()["error"]["kind"], "app_not_running");
}

#[tokio::test]
async fn stop_with_nothing_recording_is_refused() {
    let daemon = FakeDaemon::new();
    daemon.report_replies(Err(DaemonError::AppRefused {
        refusal: Refusal {
            kind: RefusalKind::NotRecording,
            message: "nothing is being recorded".into(),
        },
    }));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "report", "stop"]).await;
    assert_eq!(exit, exit::PRECONDITION);
    assert_eq!(console.json()["error"]["kind"], "not_recording");
}

#[tokio::test]
async fn record_while_recording_is_refused() {
    let daemon = FakeDaemon::new();
    daemon.report_replies(Err(DaemonError::AppRefused {
        refusal: Refusal {
            kind: RefusalKind::AlreadyRecording,
            message: "a recording is already running".into(),
        },
    }));
    let console = FakeConsole::pipe();
    let exit = run(&daemon, &console, &["--json", "report", "record"]).await;
    assert_eq!(exit, exit::PRECONDITION);
    assert_eq!(console.json()["error"]["kind"], "already_recording");
}
