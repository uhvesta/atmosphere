//! A machine's host variables (HO-7, HO-13, SEC-1) through a real `humatad2`:
//! the user sets, lists by name and removes them, the values stay in one
//! file only the user can read and never come back in an answer, agents and
//! scripts are refused, and agents started after a change get the values in
//! their environment while running ones keep theirs.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use common::{Build, Daemon, WAIT, World, error_of, folder, invalid_field, machine_id, shell_token, wait_status};
use humata2_grpc::humata::v1 as pb;
use pb::machine_service_client::MachineServiceClient;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

async fn machines(daemon: &Daemon) -> MachineServiceClient<Channel> {
    MachineServiceClient::new(daemon.channel().await)
}

fn set(name: &str, value: &str, id: Option<pb::MachineId>) -> pb::SetHostVariableRequest {
    pb::SetHostVariableRequest {
        idempotency_key: None,
        id,
        name: name.to_owned(),
        value: value.to_owned(),
        presence: None,
    }
}

async fn names(daemon: &Daemon) -> Vec<String> {
    machines(daemon)
        .await
        .list_host_variables(pb::ListHostVariablesRequest { id: None })
        .await
        .expect("the user lists the names")
        .into_inner()
        .names
}

/// Every file under `root` that holds `needle`.
fn files_holding(root: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut folders = vec![root.to_owned()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => folders.push(path),
                Ok(kind) if kind.is_file() => {
                    if std::fs::read(&path)
                        .is_ok_and(|bytes| bytes.windows(needle.len()).any(|window| window == needle.as_bytes()))
                    {
                        found.push(path);
                    }
                }
                _ => {}
            }
        }
    }
    found
}

#[tokio::test(flavor = "multi_thread")]
async fn a_value_is_set_listed_by_name_only_replaced_and_removed() {
    let daemon = Daemon::start().await;
    let secret = "s3cr3t-value-of-the-variable";
    let mut client = machines(&daemon).await;

    let set_reply = client.set_host_variable(set("API_TOKEN", secret, None)).await.unwrap().into_inner();
    client.set_host_variable(set("DB_URL", "postgres://db", None)).await.unwrap();
    let listed = client
        .list_host_variables(pb::ListHostVariablesRequest { id: None })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(listed.names, ["API_TOKEN", "DB_URL"]);
    assert!(!format!("{set_reply:?} {listed:?}").contains(secret));
    // Wherever the store keeps the value, the daemon's logs and state hold no
    // second copy of it: replacing the value leaves no trace of the old one.
    client.set_host_variable(set("API_TOKEN", "replacement", None)).await.unwrap();
    assert!(files_holding(&daemon.state, secret).is_empty());

    client
        .remove_host_variable(pb::RemoveHostVariableRequest {
            idempotency_key: None,
            id: None,
            name: "API_TOKEN".to_owned(),
            presence: None,
        })
        .await
        .unwrap();
    assert_eq!(names(&daemon).await, ["DB_URL"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_values_survive_a_restart_of_the_daemon() {
    let mut daemon = Daemon::start().await;
    machines(&daemon).await.set_host_variable(set("API_TOKEN", "kept", None)).await.unwrap();

    daemon.restart().await;

    assert_eq!(names(&daemon).await, ["API_TOKEN"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_or_value_that_cant_be_kept_is_refused_and_nothing_is_stored() {
    let daemon = Daemon::start().await;
    let mut client = machines(&daemon).await;

    for (name, value, field) in [
        ("1BAD", "v", "name"),
        ("HUMATA_SESSION_TOKEN", "v", "name"),
        ("API_TOKEN", "", "value"),
        ("API_TOKEN", "abc", "value"),
    ] {
        let refused = client.set_host_variable(set(name, value, None)).await.expect_err(name);
        assert_eq!(refused.code(), Code::InvalidArgument, "{name}");
        assert_eq!(invalid_field(&refused).0, field);
    }
    assert!(names(&daemon).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_machine_the_host_doesnt_know_is_not_found() {
    let daemon = Daemon::start().await;
    let unknown = Some(pb::MachineId { value: "nowhere".to_owned() });

    let refused = machines(&daemon)
        .await
        .set_host_variable(set("API_TOKEN", "vvvv", unknown))
        .await
        .expect_err("no such machine");

    assert_eq!(refused.code(), Code::NotFound);
}

/// A world whose test agent writes its environment to `out` when it starts.
async fn env_daemon() -> (Daemon, PathBuf) {
    let dir = folder();
    let world = World::new(dir.path());
    let out = dir.path().join("env-out");
    std::fs::create_dir_all(&out).unwrap();
    let script = format!(
        r#"CONV_DIR='{}'
OUT='{}'
mode=$1; id=$2
case "$mode" in
  --session-id) f="$CONV_DIR/$id.jsonl"; : > "$f" ;;
  --resume) f="$CONV_DIR/$id.jsonl" ;;
esac
printf '%s\n' "$@" > "$OUT/argv-$id.args"
env > "$OUT/env-$id.part" && mv "$OUT/env-$id.part" "$OUT/env-$id.txt"
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; done
"#,
        world.conv.display(),
        out.display()
    );
    std::fs::write(dir.path().join("agent.sh"), script).unwrap();
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    (Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await, out)
}

async fn desk_session(daemon: &Daemon, title: &str) -> pb::SessionId {
    let started = pb::desk_service_client::DeskServiceClient::new(daemon.channel().await)
        .start_desk_session(pb::StartDeskSessionRequest {
            title: title.to_owned(),
            access: Some(pb::DeskAccess {
                boards: true,
                channels_and_machines: false,
            }),
            ..Default::default()
        })
        .await
        .expect("start a desk session")
        .into_inner();
    let id = started.session.expect("the session id");
    wait_status(daemon, &id, pb::SessionStatus::Working).await;
    id
}

fn env_files(out: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(out)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
        .collect();
    files.sort();
    files
}

/// The environment file the agent started after `before` wrote.
async fn new_env(out: &Path, before: &[PathBuf]) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(new) = env_files(out).into_iter().find(|file| !before.contains(file)) {
            return std::fs::read_to_string(new).unwrap();
        }
        assert!(Instant::now() < deadline, "the agent never wrote its environment");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_started_after_a_change_gets_the_variable_and_a_running_one_keeps_its_own() {
    let (daemon, out) = env_daemon().await;
    machines(&daemon).await.set_host_variable(set("HV_PROBE", "first", None)).await.unwrap();
    desk_session(&daemon, "One").await;
    let first = new_env(&out, &[]).await;
    assert!(first.lines().any(|line| line == "HV_PROBE=first"), "{first}");

    let changed = machines(&daemon)
        .await
        .set_host_variable(set("HV_PROBE", "second", None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(changed.running_sessions_keep_old_value, 1, "the running agent keeps the old value");
    let before = env_files(&out);
    desk_session(&daemon, "Two").await;
    let second = new_env(&out, &before).await;

    assert!(second.lines().any(|line| line == "HV_PROBE=second"), "{second}");
    let kept = std::fs::read_to_string(&before[0]).unwrap();
    assert!(kept.lines().any(|line| line == "HV_PROBE=first"), "the running agent keeps its value: {kept}");
    // The value reaches an agent through its environment, never its command line.
    for argv in std::fs::read_dir(&out).unwrap().flatten().filter(|entry| entry.path().extension().is_some_and(|e| e == "args")) {
        let args = std::fs::read_to_string(argv.path()).unwrap();
        assert!(!args.contains("first") && !args.contains("second"), "{args}");
    }
}

/// Every file under the daemon's folders that holds `needle`, except the
/// secrets folder and what the test's own agent wrote to `out`.
fn leaks(daemon: &Daemon, out: &Path, needle: &str) -> Vec<PathBuf> {
    files_holding(daemon.dir.path(), needle)
        .into_iter()
        .filter(|path| !path.starts_with(daemon.state.join("secrets")) && !path.starts_with(out))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_value_is_in_no_reply_log_report_or_file_outside_the_secrets_folder() {
    let (mut daemon, out) = env_daemon().await;
    let secret = "leak-probe-4f9c1d7a";
    let mut client = machines(&daemon).await;
    let mut replies = vec![format!(
        "{:?}",
        client.set_host_variable(set("HV_PROBE", secret, None)).await.unwrap()
    )];
    // An error that carries the request: a bad name sent with the value.
    replies.push(format!("{:?}", client.set_host_variable(set("1BAD", secret, None)).await.unwrap_err()));
    replies.push(format!("{:?}", client.set_host_variable(set("HV_PROBE", secret, Some(pb::MachineId { value: "nowhere".to_owned() }))).await.unwrap_err()));
    replies.push(format!(
        "{:?}",
        client.list_host_variables(pb::ListHostVariablesRequest { id: None }).await.unwrap()
    ));
    desk_session(&daemon, "Leak").await;
    new_env(&out, &[]).await;

    // A restart writes its report; the agent's holder survives it.
    daemon.restart().await;
    replies.push(format!("{:?}", names(&daemon).await));

    for reply in &replies {
        assert!(!reply.contains(secret), "{reply}");
    }
    let found = leaks(&daemon, &out, secret);
    assert!(found.is_empty(), "the value is outside the secrets folder: {found:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_brought_back_after_its_holder_died_gets_the_variables_the_store_has_now() {
    let (mut daemon, out) = env_daemon().await;
    machines(&daemon).await.set_host_variable(set("HV_PROBE", "before-reboot-1a2b", None)).await.unwrap();
    desk_session(&daemon, "Reboot").await;
    new_env(&out, &[]).await;

    // The machine goes down with its agent, and the value changes while no
    // agent is running; the restart brings the agent back with what is kept now.
    common::kill_holders(&common::stored_tabs(&daemon.state)).await;
    machines(&daemon).await.set_host_variable(set("HV_PROBE", "after-reboot-3c4d", None)).await.unwrap();
    daemon.restart().await;

    let deadline = Instant::now() + std::time::Duration::from_secs(90);
    loop {
        let relaunched = env_files(&out)
            .into_iter()
            .any(|file| std::fs::read_to_string(file).is_ok_and(|text| text.lines().any(|line| line == "HV_PROBE=after-reboot-3c4d")));
        if relaunched {
            break;
        }
        assert!(Instant::now() < deadline, "the agent was never brought back with the current value");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // What the restart report says of it holds neither value.
    for secret in ["before-reboot-1a2b", "after-reboot-3c4d"] {
        let found = leaks(&daemon, &out, secret);
        assert!(found.is_empty(), "{secret} is outside the secrets folder: {found:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_may_not_set_list_or_remove_a_variable() {
    let (daemon, _out) = env_daemon().await;
    let id = desk_session(&daemon, "Agent").await;
    let (token, _attach) = shell_token(&daemon, &id).await;
    fn with_token<T>(token: &str, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request
            .metadata_mut()
            .insert("humata-session-token", MetadataValue::try_from(token).unwrap());
        request
    }
    let as_agent = |message| with_token(&token, message);
    let as_agent_list = |message| with_token(&token, message);
    let as_agent_remove = |message| with_token(&token, message);
    let mut client = machines(&daemon).await;
    let own = machine_id(&daemon).await;

    let refused = client
        .set_host_variable(as_agent(set("API_TOKEN", "v", Some(own.clone()))))
        .await
        .expect_err("an agent can't set");
    assert_eq!(refused.code(), Code::PermissionDenied);
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::UserOnly(_))));
    let listed = client
        .list_host_variables(as_agent_list(pb::ListHostVariablesRequest { id: Some(own.clone()) }))
        .await
        .expect_err("an agent can't list");
    assert_eq!(listed.code(), Code::PermissionDenied);
    let removed = client
        .remove_host_variable(as_agent_remove(pb::RemoveHostVariableRequest {
            idempotency_key: None,
            id: Some(own),
            name: "API_TOKEN".to_owned(),
            presence: None,
        }))
        .await
        .expect_err("an agent can't remove");
    assert_eq!(removed.code(), Code::PermissionDenied);
    assert!(names(&daemon).await.is_empty());
}
