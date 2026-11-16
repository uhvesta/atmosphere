//! A real public SDK consumer: no private host commands, metadata files or protocols.
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::PathBuf, time::Duration};
use terminal_host::{
    Config, Host, InjectionOptions, NameSource, Profile, SpawnCwd, SpawnFromOptions,
};
use terminal_session::{Endpoint, SessionStore};
use terminal_wire::SessionState;

#[tokio::test]
async fn public_handle_discovers_names_injects_forks_and_stops_owned_terminals() {
    let root = tempfile::Builder::new()
        .prefix("terminal-sdk-")
        .tempdir_in("/tmp")
        .unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
    let host = Host::start(
        Config {
            state: root.path().into(),
            label: "public native SDK".into(),
            start_profiles: vec!["shell".into()],
            default_history: Default::default(),
        },
        SessionStore::open_memory_only(root.path(), holder).unwrap(),
        vec![Profile {
            name: "shell".into(),
            title: "SDK shell".into(),
            program: PathBuf::from("/bin/sh"),
            args: vec!["-i".into()],
            cwd: root.path().into(),
            endpoint: Endpoint::Local,
            recovery: None,
            history: Default::default(),
        }],
    )
    .await
    .unwrap();
    let records = host.terminals().await.unwrap();
    assert_eq!(records.len(), 1);
    let terminal = host.terminal(&records[0].uuid).await.unwrap();
    let manual = terminal
        .set_name_with_source("Manual shell".into(), NameSource::Manual)
        .await
        .unwrap();
    assert_eq!(manual.title, "Manual shell");
    let named = terminal.set_name("Task shell".into()).await.unwrap();
    assert_eq!(
        named.name_override.as_ref().unwrap().source,
        NameSource::Programmatic
    );
    let attrs = BTreeMap::from([("task".into(), "SDK-1".into())]);
    terminal.set_attrs(attrs.clone()).await.unwrap();
    assert_eq!(terminal.info().await.unwrap().attributes, attrs);
    let proof = root.path().join("executed");
    let command = format!("printf 'PUBLIC_SDK_EXECUTED' > '{}'", proof.display());
    let queued = terminal
        .inject_text(command, InjectionOptions::raw())
        .await
        .unwrap();
    let first_ticket = queued.ticket().clone();
    assert_eq!(queued.written().await.unwrap(), first_ticket);
    // PTY delivery alone did not submit this line to the shell.
    assert!(!proof.exists());
    let enter = terminal
        .inject_text(
            String::new(),
            InjectionOptions {
                submit: true,
                ..InjectionOptions::raw()
            },
        )
        .await
        .unwrap();
    let second_ticket = enter.written().await.unwrap();
    assert!(second_ticket.seq > first_ticket.seq);
    let output = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if let Ok(output) = std::fs::read_to_string(&proof) {
                if output == "PUBLIC_SDK_EXECUTED" {
                    break output;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let sibling = terminal
        .spawn_from(SpawnFromOptions {
            cwd: SpawnCwd::AllowLastKnown,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_ne!(sibling.uuid, terminal.uuid());
    let sibling_handle = host.terminal(&sibling.uuid).await.unwrap();
    assert_eq!(sibling_handle.route(), sibling.id);
    let stopped_sibling = sibling_handle.stop().await.unwrap();
    assert_eq!(stopped_sibling.terminal_uuid, sibling.uuid);
    terminal.stop().await.unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let first = terminal.info().await.unwrap();
            let second = sibling_handle.info().await.unwrap();
            if matches!(first.state, SessionState::Exited { .. })
                && matches!(second.state, SessionState::Exited { .. })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    host.shutdown().await.unwrap();
    assert_eq!(output, "PUBLIC_SDK_EXECUTED");
    assert_eq!(first_ticket.terminal_uuid, terminal.uuid());
}
