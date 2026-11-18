//! A relaunch that can't start the agent leaves it somewhere the person can
//! act on, instead of launching for good.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use humata2_store::{AgentLaunch, AgentRecord, AgentState, SessionRecord, Store, TabRecord};

use crate::agent::relaunch;
use crate::{ReviveConfig, ReviveError, RestartReports};

fn config(root: &Path) -> ReviveConfig {
    ReviveConfig {
        // A holder that isn't there, so starting the agent fails.
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
        versions: None,
        gate: None,
    }
}

fn launching(root: &Path) -> AgentRecord {
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
            // Never written: the agent never got as far as a conversation.
            conversation_file: root.join("c1.jsonl"),
            holder_socket: root.join("h.sock"),
            cols: 80,
            rows: 24,
            cli_version: None,
        },
        state: AgentState::Launching,
        generation: 1,
        interruptions: Vec::new(),
        updated_unix_ms: 1,
    }
}

#[tokio::test]
async fn an_agent_that_cannot_be_started_again_is_left_stopped_not_launching() {
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
    let record = launching(root);
    store.put_agent(&record).unwrap();

    let env = |_: &TabRecord| Vec::new();
    let outcome = relaunch(&store, &config(root), record, 0, &env).await;

    assert!(matches!(outcome, Err(ReviveError::Holder { .. })), "{outcome:?}");
    let kept = store.agent("se_a").unwrap().expect("the agent is still saved");
    assert_eq!(kept.state, AgentState::Stopped);
}
