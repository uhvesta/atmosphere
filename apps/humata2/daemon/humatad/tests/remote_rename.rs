//! Flow 26.19 on a box (FC-16, FC-17; D196): renaming a session that runs
//! on another machine with "Rename the branch too" renames the branch in
//! that machine's clone, and the host shows the new name.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::path::Path;

use common::git;
use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");

fn head_of(clone: &Path) -> String {
    git(&["-C", &clone.display().to_string(), "rev-parse", "--abbrev-ref", "HEAD"])
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_box_session_with_the_branch_too_renames_the_branch_on_the_box() {
    let rig = Rig::start(&["box-1"], &[REPO]).await;
    let one = Rig::box_id("box-1");
    let channel = rig.create_channel("Payments", &[REPO], &[rig.host_id().await, one.clone()]).await;
    let id = rig.start_on(&channel, &one, "Fix refund rounding").await;
    let before = rig.wait_session(&id, "live", is_live).await;
    let old = before.repos[0].branch.clone();
    let clone = rig.the("box-1").path(&channel, "sessions/fix-refund-rounding/api");
    assert_eq!(head_of(&clone), old);

    let renamed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .rename_session(pb::RenameSessionRequest {
            id: Some(id.clone()),
            title: "Round refunds half up".to_owned(),
            rename_branch: true,
            ..Default::default()
        })
        .await
        .expect("an unpushed branch on a box is renamed")
        .into_inner();

    assert_eq!(renamed.title, "Round refunds half up");
    let new = renamed.repos[0].branch.clone();
    assert!(new.ends_with("round-refunds-half-up") && new != old, "{new}");
    assert_eq!(head_of(&clone), new, "the box's clone is on the new branch");
    assert!(git(&["-C", &clone.display().to_string(), "branch", "--list", &old]).is_empty(), "the old branch is gone");
    let shown = rig.session(&id).await;
    assert_eq!(shown.repos[0].branch, new);
    assert_eq!(shown.title, "Round refunds half up");
}
