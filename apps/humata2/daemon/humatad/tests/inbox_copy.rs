//! IN-10, D467: an agent that asks to copy files from one machine to another
//! while its rule says Ask me waits in the Inbox with a card that tells what
//! would be copied: the two machines, and how many files and bytes the source
//! holds, measured on the source's disk. Nothing is copied before the answer.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use common::{error_of, shell_token};
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};
use tonic::metadata::MetadataValue;

#[tokio::test(flavor = "multi_thread")]
async fn an_agents_copy_from_another_machine_waits_with_its_machines_files_and_bytes() {
    let rig = Rig::start(&["box-1"], &[("acme", "api")]).await;
    let host = rig.host_id().await;
    let on = [host.clone(), Rig::box_id("box-1")];
    let channel = rig.create_channel("Payments", &[("acme", "api")], &on).await;
    let session = rig.start_on(&channel, &host, "Fix it").await;
    rig.wait_session(&session, "live", is_live).await;

    // Three files, 10 + 20 + 0 bytes, in the channel's shared folder on box-1.
    let source = rig.the("box-1").path(&channel, "shared/datasets");
    std::fs::create_dir_all(source.join("nested")).unwrap();
    std::fs::write(source.join("a.bin"), [1u8; 10]).unwrap();
    std::fs::write(source.join("nested/b.bin"), [2u8; 20]).unwrap();
    std::fs::write(source.join("empty"), b"").unwrap();

    let (token, _shell) = shell_token(&rig.host, &session).await;
    let mut request = tonic::Request::new(pb::StartTransferRequest {
        idempotency_key: None,
        sources: vec![pb::FileLocation {
            machine: Some(Rig::box_id("box-1")),
            root: Some(pb::file_location::Root::ChannelShared(pb::ChannelShared {
                channel: channel.id.clone(),
            })),
            path: "datasets".to_owned(),
        }],
        destination: Some(pb::FileLocation {
            machine: Some(host.clone()),
            root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
                session: Some(session.clone()),
                repo: Some(common::repo_ref("acme", "api")),
            })),
            path: String::new(),
        }),
        mode: pb::TransferMode::Copy as i32,
        on_existing: pb::OnExisting::Fail as i32,
        reason: "the training data".to_owned(),
        name: String::new(),
    });
    request.metadata_mut().insert(
        "humata-session-token",
        MetadataValue::try_from(token.as_str()).expect("a token is ASCII"),
    );
    let refused = pb::transfer_service_client::TransferServiceClient::new(rig.channel().await)
        .start_transfer(request)
        .await
        .expect_err("a copy the rules ask about doesn't start");
    assert!(
        matches!(error_of(&refused).kind, Some(pb::error::Kind::ApprovalPending(_))),
        "{refused:?}"
    );

    let waiting: Vec<pb::InboxRequest> = rig
        .host
        .inbox_service()
        .await
        .list_requests(pb::ListRequestsRequest::default())
        .await
        .unwrap()
        .into_inner()
        .requests
        .into_iter()
        .filter(|request| request.state() == pb::RequestState::Pending)
        .collect();
    assert_eq!(waiting.len(), 1, "{waiting:?}");
    let Some(pb::request_preview::Preview::CrossMachineCopy(copy)) = waiting[0]
        .preview
        .as_ref()
        .and_then(|preview| preview.preview.clone())
    else {
        panic!("the card isn't a cross-machine copy: {:?}", waiting[0].preview);
    };
    assert_eq!(copy.source_machine, Some(Rig::box_id("box-1")));
    assert_eq!(copy.destination_machine, Some(host));
    assert_eq!((copy.files, copy.bytes), (3, 30), "{copy:?}");
    assert!(!copy.destination_place.is_empty(), "{copy:?}");
}
