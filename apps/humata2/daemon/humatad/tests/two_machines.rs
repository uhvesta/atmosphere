//! The multi-machine rig itself: a host and two machines behind one fake
//! `ssh`, each with conversations of its own (`common/remote_rig.rs`).

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use common::holder_answers;
use remote_rig::{Rig, is_live, is_offline};

#[tokio::test(flavor = "multi_thread")]
async fn a_session_started_on_box_2_runs_there_with_its_conversation_on_box_2() {
    let rig = Rig::start(&["box-1", "box-2"], &[("acme", "api")]).await;
    let on = [rig.host_id().await, Rig::box_id("box-1"), Rig::box_id("box-2")];
    let channel = rig.create_channel("Payments", &[("acme", "api")], &on).await;

    let id = rig.start_on(&channel, &Rig::box_id("box-2"), "Fix it").await;
    let session = rig.wait_session(&id, "live", is_live).await;

    assert_eq!(session.machine, Some(Rig::box_id("box-2")));
    let (one, two) = (rig.the("box-1"), rig.the("box-2"));
    assert!(two.store().sessions().unwrap().iter().any(|record| record.id == id.value));
    assert!(one.store().sessions().unwrap().is_empty());
    assert!(one.tabs().is_empty(), "box-1 ran nothing");
    assert!(common::stored_tabs(&rig.host.state).is_empty(), "the host ran nothing");
    assert_eq!(two.tabs().len(), 1);
    assert!(holder_answers(&two.tabs()[0]));
    assert!(two.path(&channel, "main/api/.git").exists());

    // The agent makes its conversation file before it says it is up.
    let tab = common::agent_tab(&rig.host, &id).await;
    // A remote tab is named with its session, so the host knows where to relay.
    let open = humata2_grpc::humata::v1::AttachOpen {
        session: Some(id.clone()),
        ..common::open(&tab, 0)
    };
    common::Attach::start(&rig.host, open).await.wait_for("agent up").await;
    let conversation = two.conversation_id(&id);
    assert!(!conversation.is_empty());
    assert!(two.conversation(&conversation).exists(), "box-2 holds the conversation");
    assert!(!one.conversation(&conversation).exists());
    assert!(!rig.host_conversation(&conversation).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_restarted_with_an_env_relinks_both_machines() {
    let mut rig = Rig::start(&["box-1", "box-2"], &[("acme", "api")]).await;
    let on = [rig.host_id().await, Rig::box_id("box-1"), Rig::box_id("box-2")];
    let channel = rig.create_channel("Payments", &[("acme", "api")], &on).await;
    let id = rig.start_on(&channel, &Rig::box_id("box-2"), "Fix it").await;
    rig.wait_session(&id, "live", is_live).await;

    rig.restart_host(&[("HUMATA2_RIG_MARK", "1")]).await;

    for name in ["box-1", "box-2"] {
        rig.linked(name).await;
    }
    rig.wait_session(&id, "live after the host restarted", is_live).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_link_comes_back_and_the_session_is_the_same() {
    let rig = Rig::start(&["box-1"], &[("acme", "api")]).await;
    let on = [rig.host_id().await, Rig::box_id("box-1")];
    let channel = rig.create_channel("Payments", &[("acme", "api")], &on).await;
    let id = rig.start_on(&channel, &Rig::box_id("box-1"), "Fix it").await;
    rig.wait_session(&id, "live", is_live).await;
    let holders = rig.the("box-1").tabs();

    rig.link_lost("box-1").await;
    rig.wait_session(&id, "offline", is_offline).await;

    rig.link_back("box-1").await;
    let back = rig.wait_session(&id, "live again", is_live).await;
    assert_eq!(back.id, Some(id));
    let tabs = rig.the("box-1").tabs();
    assert_eq!(tabs.len(), holders.len(), "no second tab was made");
    assert_eq!(tabs[0].holder_pid, holders[0].holder_pid, "the same holder, reattached");
}
