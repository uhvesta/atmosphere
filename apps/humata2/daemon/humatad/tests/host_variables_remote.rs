//! A box's host variables (HO-7, HO-13): the host sends a value to the box
//! it belongs to over the link; the box keeps it in its own state folder and
//! no other; listing shows names only; a box that is offline answers at once.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::{Path, PathBuf};

use common::error_of;
use humata2_grpc::humata::v1 as pb;
use pb::machine_service_client::MachineServiceClient;
use remote_rig::{Rig, is_live};
use tonic::Code;
use tonic::transport::Channel;

const BOX: &str = "box-1";

async fn machines(rig: &Rig) -> MachineServiceClient<Channel> {
    MachineServiceClient::new(rig.host.channel().await)
}

fn set(value: &str) -> pb::SetHostVariableRequest {
    pb::SetHostVariableRequest {
        idempotency_key: None,
        id: Some(Rig::box_id(BOX)),
        name: "API_TOKEN".to_owned(),
        value: value.to_owned(),
        presence: None,
    }
}

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
async fn a_value_set_for_a_box_is_kept_on_the_box_and_only_its_name_comes_back() {
    let rig = Rig::start(&[BOX], &[]).await;
    let secret = "s3cr3t-value-for-the-box";
    let mut client = machines(&rig).await;

    let reply = client.set_host_variable(set(secret)).await.expect("the host sets it on the box").into_inner();
    assert_eq!(reply.running_sessions_keep_old_value, 0, "no agent runs on the box yet");
    let listed = client
        .list_host_variables(pb::ListHostVariablesRequest { id: Some(Rig::box_id(BOX)) })
        .await
        .expect("the host lists the box's names")
        .into_inner();

    assert_eq!(listed.names, ["API_TOKEN"]);
    assert!(!format!("{reply:?} {listed:?}").contains(secret));
    assert!(
        files_holding(&rig.host.state, secret).is_empty(),
        "the host keeps no copy of a box's value"
    );

    client
        .remove_host_variable(pb::RemoveHostVariableRequest {
            idempotency_key: None,
            id: Some(Rig::box_id(BOX)),
            name: "API_TOKEN".to_owned(),
            presence: None,
        })
        .await
        .expect("the host removes it on the box");
    let after = client
        .list_host_variables(pb::ListHostVariablesRequest { id: Some(Rig::box_id(BOX)) })
        .await
        .unwrap()
        .into_inner();
    assert!(after.names.is_empty());
    assert!(files_holding(&rig.the(BOX).daemon().state, secret).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_that_is_offline_answers_machine_offline_at_once() {
    let rig = Rig::start(&[BOX], &[]).await;
    rig.link_lost(BOX).await;
    let mut client = machines(&rig).await;

    let set = client.set_host_variable(set("vvvv")).await.expect_err("offline");
    let list = client
        .list_host_variables(pb::ListHostVariablesRequest { id: Some(Rig::box_id(BOX)) })
        .await
        .expect_err("offline");

    for refused in [set, list] {
        assert_eq!(refused.code(), Code::Unavailable);
        assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::MachineOffline(_))));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_started_on_a_box_after_the_value_was_set_there_sees_it() {
    let rig = Rig::start(&[BOX], &[("acme", "api")]).await;
    // The box's test agent writes its environment where the test can read it.
    let home = rig.the(BOX).home().to_owned();
    let out = home.join("env-out");
    std::fs::create_dir_all(&out).unwrap();
    let script = format!(
        "CONV_DIR='{conv}'\nOUT='{out}'\nmode=$1; id=$2\ncase \"$mode\" in\n  --session-id) f=\"$CONV_DIR/$id.jsonl\"; : > \"$f\" ;;\n  --resume) f=\"$CONV_DIR/$id.jsonl\" ;;\nesac\nprintf '%s\\n' \"$@\" > \"$OUT/argv-$id.args\"\nenv > \"$OUT/env-$id.part\" && mv \"$OUT/env-$id.part\" \"$OUT/env-$id.txt\"\nstty -icanon 2>/dev/null\necho \"agent up $mode\"\nwhile IFS= read -r line; do echo \"got: $line\"; done\n",
        conv = home.join("conv").display(),
        out = out.display()
    );
    std::fs::write(home.join("agent.sh"), script).unwrap();
    let secret = "box-agent-secret-5e6f";
    machines(&rig)
        .await
        .set_host_variable(pb::SetHostVariableRequest {
            idempotency_key: None,
            id: Some(Rig::box_id(BOX)),
            name: "HV_PROBE".to_owned(),
            value: secret.to_owned(),
            presence: None,
        })
        .await
        .expect("the host sets it on the box");

    let on = [rig.host_id().await, Rig::box_id(BOX)];
    let channel = rig.create_channel("Payments", &[("acme", "api")], &on).await;
    let id = rig.start_on(&channel, &Rig::box_id(BOX), "Env").await;
    rig.wait_session(&id, "live", is_live).await;

    let deadline = std::time::Instant::now() + common::WAIT;
    let env = loop {
        let found = std::fs::read_dir(&out)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .find(|path| path.extension().is_some_and(|extension| extension == "txt"));
        if let Some(text) = found.and_then(|path| std::fs::read_to_string(path).ok()) {
            break text;
        }
        assert!(std::time::Instant::now() < deadline, "the agent never wrote its environment");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    };
    assert!(env.lines().any(|line| line == format!("HV_PROBE={secret}")), "{env}");
    for argv in std::fs::read_dir(&out).unwrap().flatten().filter(|entry| entry.path().extension().is_some_and(|e| e == "args")) {
        assert!(!std::fs::read_to_string(argv.path()).unwrap().contains(secret));
    }
}
