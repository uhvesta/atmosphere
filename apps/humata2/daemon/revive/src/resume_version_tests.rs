//! A resume asks the agent's CLI for its version again and keeps what it
//! says (CV-3): the CLI may have been upgraded since the launch.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use humata2_store::{AgentLaunch, AgentRecord, AgentState, SessionRecord, Store, TabRecord};

use crate::agent::{Why, resume};
use crate::{CliVersions, RestartReports, Revived, ReviveConfig, VersionAsked};

/// A CLI that says `.0` for its version, whichever agent is asked.
#[derive(Debug)]
struct Says(Option<&'static str>);

impl CliVersions for Says {
    fn current<'a>(&'a self, _agent: &'a str) -> VersionAsked<'a> {
        Box::pin(async move { self.0.map(str::to_owned) })
    }
}

fn config(root: &Path, versions: Option<Arc<dyn CliVersions>>) -> ReviveConfig {
    ReviveConfig {
        holder: root.join("no-such-holder"),
        logs: root.join("logs"),
        scrollback_bytes: 1 << 16,
        shell: "/bin/sh".into(),
        shell_args: Vec::new(),
        home: root.to_owned(),
        resume_timeout: Duration::from_secs(1),
        settle: Duration::from_millis(10),
        reports: RestartReports {
            folder: root.join("restarts"),
            version: "0.0.0-test".to_owned(),
            machine: "ma_1".to_owned(),
            os: "test".to_owned(),
            daemon_log: None,
        },
        contain: None,
        builds: None,
        versions,
        gate: None,
    }
}

fn running(root: &Path, version: Option<&str>) -> AgentRecord {
    AgentRecord {
        session: "se_a".to_owned(),
        tab: "tb_a".to_owned(),
        launch: AgentLaunch {
            agent: "fake".to_owned(),
            folder: root.to_owned(),
            argv: vec![OsString::from("/bin/sh")],
            resume_argv: vec![OsString::from("/bin/sh")],
            variable_names: Vec::new(),
            conversation_id: "c1".to_owned(),
            // Never written, so the resume stops before it starts a holder.
            conversation_file: root.join("c1.jsonl"),
            holder_socket: root.join("h.sock"),
            cols: 80,
            rows: 24,
            cli_version: version.map(str::to_owned),
        },
        state: AgentState::Running { holder_pid: 1 },
        generation: 1,
        interruptions: Vec::new(),
        updated_unix_ms: 1,
    }
}

/// The version saved for the agent after a resume of a record that held
/// `before`, run with `versions`.
async fn kept_after_resume(before: Option<&str>, versions: Option<Arc<dyn CliVersions>>) -> Option<String> {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("state")).unwrap();
    let store = Arc::new(Store::open(root.join("state")).unwrap());
    store
        .put_session(&SessionRecord {
            id: "se_a".to_owned(),
            channel: "ch_1".to_owned(),
            machine: "ma_1".to_owned(),
            title: "Fix the build".to_owned(),
            agent: "fake".to_owned(),
            folder: root.to_owned(),
            created_unix_ms: 1,
        })
        .unwrap();
    let record = running(root, before);
    store.put_agent(&record).unwrap();
    let env = |_: &TabRecord| Vec::new();
    let outcome = resume(&store, &config(root, versions), record, 0, &env, Why::Interrupted).await;
    assert!(matches!(outcome, Ok(Revived::ResumeFailed { .. })), "{outcome:?}");
    store.agent("se_a").unwrap().expect("the agent is still saved").launch.cli_version
}

#[tokio::test]
async fn a_resume_keeps_the_version_the_cli_says_now() {
    let versions: Arc<dyn CliVersions> = Arc::new(Says(Some("2.0.0")));
    assert_eq!(kept_after_resume(Some("1.0.0"), Some(versions.clone())).await.as_deref(), Some("2.0.0"));
    assert_eq!(kept_after_resume(None, Some(versions)).await.as_deref(), Some("2.0.0"));
}

#[tokio::test]
async fn a_cli_that_no_longer_answers_leaves_no_stale_version() {
    let versions: Arc<dyn CliVersions> = Arc::new(Says(None));
    assert_eq!(kept_after_resume(Some("1.0.0"), Some(versions)).await, None);
}

#[tokio::test]
async fn a_resume_with_nobody_to_ask_keeps_the_recorded_version() {
    assert_eq!(kept_after_resume(Some("1.0.0"), None).await.as_deref(), Some("1.0.0"));
}
