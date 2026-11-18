//! A session that moves to another machine carries the names of its host
//! variables and nothing else (HO-7, HO-13, SEC-1): the value stays on the
//! machine it was set on, and the machine the session moves to holds no
//! copy of it, in its state, its folders or the host's.

mod common;
#[path = "common/remote_rig.rs"]
mod remote_rig;

use std::path::{Path, PathBuf};

use humata2_grpc::humata::v1 as pb;
use pb::machine_service_client::MachineServiceClient;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

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
async fn a_moved_session_carries_no_host_variable_value_to_the_machine_it_moves_to() {
    let rig = Rig::start(&["box-1", "box-2"], &[REPO]).await;
    let secret = "moved-session-secret-7a8b";
    let mut machines = MachineServiceClient::new(rig.host.channel().await);
    machines
        .set_host_variable(pb::SetHostVariableRequest {
            idempotency_key: None,
            id: Some(Rig::box_id("box-1")),
            name: "HV_PROBE".to_owned(),
            value: secret.to_owned(),
            presence: None,
        })
        .await
        .expect("the host sets it on box-1");

    let on = [rig.host_id().await, Rig::box_id("box-1"), Rig::box_id("box-2")];
    let channel = rig.create_channel("Payments", &[REPO], &on).await;
    let id = rig.start_on(&channel, &Rig::box_id("box-1"), "Carry").await;
    rig.wait_session(&id, "live", is_live).await;

    let to = Rig::box_id("box-2");
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .move_session(pb::MoveSessionRequest {
            idempotency_key: Some(pb::IdempotencyKey {
                value: "move-host-variables".to_owned(),
            }),
            id: Some(id.clone()),
            to: Some(to.clone()),
        })
        .await
        .expect("the move starts")
        .into_inner();
    let ended = rig.ended(&started).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the move failed: {:?}", ended.error);
    rig.wait_session(&id, "live on box-2", |session| session.machine.as_ref() == Some(&to) && is_live(session)).await;

    // box-2 was never told the value, and nothing of the session's copy holds it.
    let names = machines
        .list_host_variables(pb::ListHostVariablesRequest { id: Some(to.clone()) })
        .await
        .unwrap()
        .into_inner()
        .names;
    assert!(names.is_empty(), "{names:?}");
    for (what, root) in [
        ("box-2's state", rig.the("box-2").daemon().state.clone()),
        ("box-2's home", rig.the("box-2").home().to_owned()),
        ("the host's state", rig.host.state.clone()),
    ] {
        let found = files_holding(&root, secret);
        assert!(found.is_empty(), "{what} holds the value: {found:?}");
    }
}
