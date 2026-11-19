//! MV-17, UP-1, D281 e: a Desk session, plain or a channel's own, starts on
//! another machine only while that machine runs this host's release. Until
//! it does, the start is refused with `migration_needed` naming the
//! machine's Migrate item, and nothing is made on any machine.

use humatad_test_common as common;
use humatad_test_common::remote_rig;
use humatad_test_common::upgrade;
use humatad_test_common::upgrade_rig;

use common::error_of;
use humata2_grpc::humata::v1 as pb;
use remote_rig::Rig;
use tonic::Status;
use upgrade_rig::{BOX, box_id};

/// The refusal a start ended in, whether the call failed or its operation
/// did.
async fn refusal_of(rig: &Rig, started: Result<pb::StartDeskSessionResponse, Status>) -> pb::Error {
    match started {
        Err(status) => error_of(&status),
        Ok(response) => {
            let operation = response.operation.expect("a start that wasn't refused has an operation");
            let ended = rig.ended(&operation).await;
            assert_eq!(ended.state(), pb::OperationState::Failed, "the start went through: {ended:?}");
            ended.error.expect("a failed operation says why")
        }
    }
}

fn names(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|found| {
            found
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_on_a_machine_at_another_release_is_refused_with_its_item_and_makes_nothing() {
    let (mut rig, _) = upgrade_rig::at_one_release().await;
    let host = rig.host_id().await;
    let channel = rig.create_channel("Payments", &[("acme", "api")], &[host.clone()]).await;
    let target = rig.start_on(&channel, &host, "Ledger fix").await;
    upgrade_rig::box_at_release(&mut rig, "b").await;
    let item = upgrade_rig::daemon_item(&rig).await;
    let tabs_before = rig.the(BOX).tabs().len();

    let requests = [
        pb::StartDeskSessionRequest {
            title: "Scratch notes".to_owned(),
            machine: Some(box_id()),
            access: Some(pb::DeskAccess {
                boards: true,
                channels_and_machines: false,
            }),
            ..Default::default()
        },
        pb::StartDeskSessionRequest {
            title: "Watch the ledger".to_owned(),
            machine: Some(box_id()),
            channel: channel.id.clone(),
            target: Some(target.clone()),
            ..Default::default()
        },
    ];
    for request in requests {
        let title = request.title.clone();
        let started = pb::desk_service_client::DeskServiceClient::new(rig.channel().await)
            .start_desk_session(request)
            .await
            .map(tonic::Response::into_inner);
        let needed = upgrade::migration_needed(&refusal_of(&rig, started).await);
        assert_eq!(needed.thing(), pb::MigrateThing::Daemon, "{title}: {needed:?}");
        assert_eq!(needed.machine, Some(box_id()), "{title}: {needed:?}");
        assert_eq!(needed.request, item.id, "{title}: {needed:?}");
    }

    assert_eq!(rig.the(BOX).tabs().len(), tabs_before, "nothing started on {BOX}");
    for root in [
        rig.the(BOX).channels().join("desk"),
        rig.the(BOX).path(&channel, "desk"),
        rig.world.channels.join("desk"),
        rig.host_path(&channel, "desk"),
    ] {
        assert!(names(&root).is_empty(), "a refused start made {root:?}: {:?}", names(&root));
    }
}
