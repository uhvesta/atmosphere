use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use humata2_store::{
    AgentLaunch, AgentRecord, AgentState, Interruption, ResumeFailure, SessionRecord, Store,
    StoreError, TabKind, TabRecord, Told,
};

fn session(id: &str, created_unix_ms: i64) -> SessionRecord {
    SessionRecord {
        id: id.to_owned(),
        channel: "ch_1".to_owned(),
        machine: "ma_1".to_owned(),
        title: "Fix the build".to_owned(),
        agent: "claude".to_owned(),
        folder: PathBuf::from("/work/web"),
        created_unix_ms,
    }
}

fn tab(id: &str, session: &str) -> TabRecord {
    TabRecord {
        id: id.to_owned(),
        session: Some(session.to_owned()),
        kind: TabKind::Agent,
        title: "claude".to_owned(),
        holder_socket: PathBuf::from(format!("/tmp/{id}.sock")),
        holder_pid: 77,
        created_unix_ms: 3,
        holder_generation: 0,
    }
}

fn agent(session: &str, state: AgentState) -> AgentRecord {
    AgentRecord {
        session: session.to_owned(),
        tab: format!("tb_{session}"),
        launch: AgentLaunch {
            agent: "claude".to_owned(),
            folder: PathBuf::from("/work/web"),
            argv: vec!["claude".into(), "--session-id".into(), "c0ffee".into()],
            resume_argv: vec!["claude".into(), "--resume".into(), "c0ffee".into()],
            variable_names: vec!["HUMATA_TOKEN".to_owned()],
            conversation_id: "c0ffee".to_owned(),
            conversation_file: PathBuf::from("/home/me/.claude/projects/web/c0ffee.jsonl"),
            holder_socket: PathBuf::from(format!("/tmp/tb_{session}.sock")),
            cols: 120,
            rows: 40,
            cli_version: Some("2.1.0 (Claude Code)".to_owned()),
        },
        state,
        generation: 1,
        interruptions: Vec::new(),
        updated_unix_ms: 10,
    }
}

/// Arguments varied enough that zstd can't fold them under the 64 KiB a
/// row holds, so the launch goes in a file of its own.
fn noisy_args() -> Vec<OsString> {
    let mut state = 0x2545_F491_4F6C_DD1D_u64;
    (0..20_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            OsString::from(format!("{state:016x}"))
        })
        .collect()
}

#[test]
fn the_hook_state_is_kept_for_the_running_agent_and_cleared_by_a_launch_or_resume() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_h", 1)).unwrap();
    store.put_agent(&agent("se_h", AgentState::Launching)).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), None);
    // Not running: nothing is kept.
    store.set_hook_turn("se_h", 4242, 2, 10).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), None);

    store
        .put_agent(&agent("se_h", AgentState::Running { holder_pid: 4242 }))
        .unwrap();
    // Another holder's report is not this agent's.
    store.set_hook_turn("se_h", 1, 2, 10).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), None);
    store.set_hook_turn("se_h", 4242, 2, 10).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), Some((2, 10)));
    // An older report arriving late is dropped; a newer one replaces it.
    store.set_hook_turn("se_h", 4242, 1, 5).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), Some((2, 10)));
    store.set_hook_turn("se_h", 4242, 3, 20).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), Some((3, 20)));

    // The agent exiting keeps it; launching or resuming clears it.
    store.put_agent(&agent("se_h", AgentState::Exited { code: 0 })).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), Some((3, 20)));
    store.put_agent(&agent("se_h", AgentState::Resuming)).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), None);
    store
        .put_agent(&agent("se_h", AgentState::Running { holder_pid: 5 }))
        .unwrap();
    store.set_hook_turn("se_h", 5, 1, 30).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), Some((1, 30)));
    store.put_agent(&agent("se_h", AgentState::Launching)).unwrap();
    assert_eq!(store.hook_turn("se_h").unwrap(), None);
}

#[test]
fn every_state_and_its_interruptions_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let states = [
        AgentState::Launching,
        AgentState::Running { holder_pid: 4242 },
        AgentState::Resuming,
        AgentState::Exited { code: 3 },
        AgentState::Stopped,
        AgentState::ResumeFailed {
            reason: ResumeFailure::AgentStartedAnotherConversation,
            exit_code: None,
        },
        AgentState::ResumeFailed {
            reason: ResumeFailure::AgentExited,
            exit_code: Some(1),
        },
    ];
    let mut saved = Vec::new();
    {
        let store = Store::open(dir.path()).unwrap();
        for (n, state) in states.into_iter().enumerate() {
            let id = format!("se_{n}");
            store.put_session(&session(&id, n as i64)).unwrap();
            let mut record = agent(&id, state);
            record.generation = n as u32 + 1;
            record.interruptions = (0..n)
                .map(|at| Interruption {
                    at_unix_ms: at as i64,
                    output_saved_through: 100 * at as u64,
                    output_resumes_at: 100 * at as u64 + 7,
                    saved_tail_damaged: at % 2 == 1,
                    told: (at % 2 == 0).then_some(Told {
                        at_unix_ms: at as i64 + 1,
                        conversation_len: 40 * at as u64,
                    }),
                })
                .collect();
            store.put_agent(&record).unwrap();
            saved.push(record);
        }
    }

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.agents().unwrap(), saved);
    assert_eq!(store.agent("se_3").unwrap().as_ref(), Some(&saved[3]));
    assert_eq!(store.agent("se_none").unwrap(), None);
}

#[test]
fn a_rewrite_replaces_state_and_interruptions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();
    let mut record = agent("se_a", AgentState::Running { holder_pid: 9 });
    record.interruptions.push(Interruption {
        at_unix_ms: 1,
        output_saved_through: 5,
        output_resumes_at: 5,
        saved_tail_damaged: false,
        told: None,
    });
    store.put_agent(&record).unwrap();

    record.state = AgentState::Exited { code: 0 };
    record.interruptions.clear();
    store.put_agent(&record).unwrap();

    assert_eq!(store.agent("se_a").unwrap(), Some(record));
}

#[test]
fn arguments_that_are_not_utf8_and_a_big_launch_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();
    let mut record = agent("se_a", AgentState::Launching);
    record.launch.argv.push(OsString::from_vec(vec![0xff, 0xfe, b'x']));
    record.launch.resume_argv.extend(noisy_args());

    store.put_agent(&record).unwrap();
    drop(store);

    let store = Store::open(dir.path()).unwrap();
    assert_eq!(store.agent("se_a").unwrap(), Some(record));
}

#[test]
fn an_agent_and_its_tab_are_saved_together() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.put_session(&session("se_a", 1)).unwrap();
    let record = agent("se_a", AgentState::Running { holder_pid: 77 });

    store.put_agent_and_tab(&record, &tab("tb_se_a", "se_a")).unwrap();

    assert_eq!(store.agent("se_a").unwrap(), Some(record));
    assert_eq!(store.tab("tb_se_a").unwrap(), Some(tab("tb_se_a", "se_a")));
}

#[test]
fn an_agent_needs_its_session_and_goes_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();

    let refused = store.put_agent_and_tab(
        &agent("se_gone", AgentState::Launching),
        &tab("tb_se_gone", "se_gone"),
    );
    assert!(matches!(refused, Err(StoreError::NoSession { id }) if id == "se_gone"));
    assert!(store.tabs().unwrap().is_empty(), "the tab was saved without its agent");

    store.put_session(&session("se_a", 1)).unwrap();
    let mut big = agent("se_a", AgentState::Launching);
    big.launch.resume_argv.extend(noisy_args());
    store.put_agent(&big).unwrap();
    store.remove_session("se_a").unwrap();

    assert_eq!(store.agent("se_a").unwrap(), None);
    assert_eq!(
        fs::read_dir(dir.path().join("blobs")).unwrap().count(),
        0,
        "the launch's file outlived its agent"
    );
}
