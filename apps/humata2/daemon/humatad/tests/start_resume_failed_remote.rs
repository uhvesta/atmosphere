//! A box's restart whose resume of an agent fails (SAFE; D150, PER-24): the
//! box's agent holder is gone when it starts again, and the start of the
//! holder fails once (`HUMATA2_FAIL_AT=revive.start_holder@1`, a fault
//! build). The record is left resuming, which the host shows as starting,
//! and the restart names the session in what it couldn't bring back. The
//! host's resume then ends instead of waiting for a report that never
//! comes: the box takes the record over (it is the restart's, and nothing of
//! this process is resuming it), the resume succeeds, and only one agent
//! ever runs for the session. Waits are bounded by `LONG`.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{AgentState, TabKind};
use remote_rig::{LONG, Rig, is_live};

const BOX: &str = "box-1";
const REPO: (&str, &str) = ("acme", "api");
/// The first start of a holder in the box's restarted process fails: its
/// resume of the agent.
const FAIL_AT: &str = "revive.start_holder@1";

/// How many holders answer for `session`'s agent tabs on the box.
fn running_agents(rig: &Rig, session: &pb::SessionId) -> usize {
    rig.the(BOX)
        .tabs()
        .iter()
        .filter(|tab| tab.session.as_deref() == Some(session.value.as_str()))
        .filter(|tab| matches!(tab.kind, TabKind::Agent))
        .filter(|tab| common::holder_answers(tab))
        .count()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_the_boxs_restart_failed_is_taken_over_and_runs_one_agent() {
    let mut rig = Rig::start_seen(&[BOX], &[REPO]).await;
    let on = Rig::box_id(BOX);
    let channel = rig.create_channel("resume-fails", &[REPO], std::slice::from_ref(&on)).await;
    let session = rig.start_on(&channel, &on, "Resume fails").await;
    assert_eq!(running_agents(&rig, &session), 1, "the agent runs before the box goes down");

    // The box loses power with its holders: the agent has to be resumed, and
    // the first holder its restart starts fails.
    let tabs = rig.the(BOX).tabs();
    rig.the_mut(BOX).kill();
    rig.link_ended(BOX).await;
    common::kill_holders(&tabs).await;
    rig.the_mut(BOX).relaunch(&[("HUMATA2_FAIL_AT", FAIL_AT)]).await;
    rig.link_back(BOX).await;

    let last = common::last_start(rig.the(BOX).daemon()).await;
    assert!(
        last.not_brought_back.iter().any(|line| line.session == session.value && line.tab.is_empty()),
        "the restart names the agent it couldn't bring back: {:?}",
        last.not_brought_back
    );
    let left = rig.the(BOX).store().agent(&session.value).unwrap().expect("the box kept the agent");
    assert_eq!(left.state, AgentState::Resuming, "the failed resume left the record resuming");
    assert_eq!(running_agents(&rig, &session), 0, "nothing runs for it");

    // The host's resume ends, through the take-over, with one agent running.
    let resumed = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .resume_session(pb::ResumeSessionRequest { id: Some(session.clone()), ..Default::default() })
        .await
        .expect("the host takes the resume")
        .into_inner();
    let ended = rig.ended(&resumed).await;
    assert_eq!(ended.state, pb::OperationState::Succeeded as i32, "the resume ended: {:?}", ended.error);
    let shown = rig.wait_session(&session, "live again", |one| is_live(one)).await;
    assert_eq!(shown.id.map(|id| id.value), Some(session.value.clone()));
    assert_eq!(running_agents(&rig, &session), 1, "only one agent ever runs for the session");
    let agents: Vec<_> = rig
        .the(BOX)
        .store()
        .agents()
        .unwrap()
        .into_iter()
        .filter(|agent| agent.session == session.value)
        .collect();
    assert_eq!(agents.len(), 1);
    assert!(matches!(agents[0].state, AgentState::Running { .. }), "{:?}", agents[0].state);
    let _ = LONG;
}
