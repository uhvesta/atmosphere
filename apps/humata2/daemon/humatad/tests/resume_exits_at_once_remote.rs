//! A resume, asked of the host, of a box-1 session whose agent exits as
//! soon as it starts: the op fails and box-1 ends the holder the resume
//! started there (D784). Nothing runs for the session on the box, and the
//! host lists no holder of box-1's for its tab.

use humatad_test_common as common;
use humatad_test_common::remote_rig;

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use humata2_grpc::humata::v1 as pb;
use remote_rig::{Rig, is_live};

const REPO: (&str, &str) = ("acme", "api");
const BOX: &str = "box-1";
/// How long the stopped agent's holder gets to go.
const DEADLINE: Duration = Duration::from_secs(30);

/// Box-1's test agent from now on: a resume prints a line and exits at once.
const EXITS_ON_RESUME: &str = r#"case "$1" in
  --resume) echo "agent exits at once"; exit 0 ;;
esac
exec sleep 1000
"#;

fn answers(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

async fn running_holders(rig: &Rig) -> Vec<pb::RunningHolder> {
    pb::machine_service_client::MachineServiceClient::new(rig.channel().await)
        .list_running_holders(pb::ListRunningHoldersRequest { machine: Some(Rig::box_id(BOX)) })
        .await
        .expect("the host lists box-1's holders")
        .into_inner()
        .holders
}

async fn sessions(rig: &Rig) -> pb::session_service_client::SessionServiceClient<tonic::transport::Channel> {
    pb::session_service_client::SessionServiceClient::new(rig.channel().await)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_resume_whose_agent_exits_at_once_fails_and_leaves_no_holder_there() {
    let rig = Rig::start(&[BOX], &[REPO]).await;
    let (host, on) = (rig.host_id().await, Rig::box_id(BOX));
    let channel = rig.create_channel("Payments", &[REPO], &[host, on.clone()]).await;
    let id = rig.start_on(&channel, &on, "Fix login").await;
    rig.wait_session(&id, "live", is_live).await;
    let agent = rig
        .the(BOX)
        .store()
        .agent(&id.value)
        .unwrap()
        .expect("box-1 keeps the session's agent");
    let (tab, socket) = (pb::TabId { value: agent.tab.clone() }, agent.launch.holder_socket.clone());
    assert!(answers(&socket), "the agent runs on box-1 before it is stopped");

    sessions(&rig)
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()), for_move: false })
        .await
        .expect("the session stops");
    let deadline = Instant::now() + DEADLINE;
    while answers(&socket) {
        assert!(Instant::now() < deadline, "the stopped agent's holder still runs on box-1");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    std::fs::write(rig.the(BOX).home().join("agent.sh"), EXITS_ON_RESUME).unwrap();
    let asked = sessions(&rig)
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .expect("the host takes the resume")
        .into_inner();
    let ended = rig.ended(&asked).await;

    assert_eq!(ended.state, pb::OperationState::Failed as i32, "{ended:?}");
    assert!(!answers(&socket), "the failed resume left its holder running on box-1");
    let left: Vec<_> = rig
        .the(BOX)
        .tabs()
        .into_iter()
        .filter(|record| record.session.as_deref() == Some(id.value.as_str()) || record.id == agent.tab)
        .filter(common::holder_answers)
        .collect();
    assert!(left.is_empty(), "box-1 still runs a holder for the session: {left:?}");
    let listed: Vec<_> = running_holders(&rig)
        .await
        .into_iter()
        .filter(|row| !row.stopped)
        .filter(|row| row.tab.as_ref() == Some(&tab) || row.session.as_ref() == Some(&id))
        .collect();
    assert!(listed.is_empty(), "the host lists a holder of box-1's for the session's tab: {listed:?}");
}
