//! D784: an agent may stop, end or adopt a machine's running holders only
//! under the machine-destroy switch (Never by default); with it on the same
//! calls go through. The user is never gated.

mod common;

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Daemon, World, binary, create_channel, folder, shell_token, start_session, wait_running};
use humata2_grpc::humata::v1 as pb;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Code, Request};

const DEADLINE: Duration = Duration::from_secs(30);
const TOKEN_HEADER: &str = "humata-session-token";

/// A holder started by hand at `socket`; ended when dropped.
struct Stray {
    socket: PathBuf,
    child: std::process::Child,
}

impl Stray {
    fn start(socket: &Path) -> Self {
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let child = Command::new(binary("HUMATA2_HOLDER"))
            .arg("--socket")
            .arg(socket)
            .arg("--cwd")
            .arg(socket.parent().unwrap())
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg("exec sleep 1004")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stray = Self { socket: socket.to_owned(), child };
        let deadline = Instant::now() + DEADLINE;
        while !stray.answers() {
            assert!(Instant::now() < deadline, "the stray holder never started");
            std::thread::sleep(Duration::from_millis(20));
        }
        stray
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn answers(&self) -> bool {
        UnixStream::connect(&self.socket).is_ok()
    }
}

impl Drop for Stray {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn machines(daemon: &Daemon) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(daemon.channel().await)
}

async fn tab_of(daemon: &Daemon, orphan: &Stray) -> pb::TabId {
    machines(daemon)
        .await
        .list_running_holders(pb::ListRunningHoldersRequest { machine: None })
        .await
        .unwrap()
        .into_inner()
        .holders
        .into_iter()
        .find(|holder| holder.pid == orphan.pid())
        .expect("the orphan is listed")
        .tab
        .expect("an orphan has a tab")
}

fn as_agent<T>(message: T, token: &str) -> Request<T> {
    let mut request = Request::new(message);
    request
        .metadata_mut()
        .insert(TOKEN_HEADER, MetadataValue::try_from(token).unwrap());
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_stops_ends_or_adopts_a_holder_only_when_the_destroy_switch_is_on() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, &[]).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    wait_running(&daemon, &id).await;
    let (token, _shell) = shell_token(&daemon, &id).await;
    let holders = std::fs::canonicalize(&daemon.state).unwrap().join("holders");
    let first = Stray::start(&holders.join("tb_00000000000000000000000000000011.sock"));
    let second = Stray::start(&holders.join("tb_00000000000000000000000000000012.sock"));
    let (first_tab, second_tab) = (tab_of(&daemon, &first).await, tab_of(&daemon, &second).await);

    // Off by default: every one of the three is refused and nothing changes.
    let mut service = machines(&daemon).await;
    let refused = [
        service
            .stop_running_holder(as_agent(
                pb::StopRunningHolderRequest { machine: None, tab: Some(first_tab.clone()) },
                &token,
            ))
            .await
            .map(drop),
        service
            .end_running_holder(as_agent(
                pb::EndRunningHolderRequest { machine: None, tab: Some(first_tab.clone()) },
                &token,
            ))
            .await
            .map(drop),
        service
            .adopt_running_holder(as_agent(
                pb::AdoptRunningHolderRequest { machine: None, tab: Some(first_tab.clone()), channel: None },
                &token,
            ))
            .await
            .map(drop),
    ];
    for refusal in refused {
        assert_eq!(refusal.unwrap_err().code(), Code::PermissionDenied);
    }
    assert!(first.answers(), "a refused call ended the holder");

    pb::access_service_client::AccessServiceClient::new(daemon.channel().await)
        .set_channel_rule(pb::SetChannelRuleRequest {
            channel: channel.id.clone(),
            capability: pb::Capability::MachineDestroy as i32,
            decision: pb::Decision::Allow as i32,
            ..Default::default()
        })
        .await
        .expect("the user turns the switch on");

    service
        .adopt_running_holder(as_agent(
            pb::AdoptRunningHolderRequest { machine: None, tab: Some(second_tab), channel: channel.id.clone() },
            &token,
        ))
        .await
        .expect("an agent adopts with the switch on");
    service
        .end_running_holder(as_agent(
            pb::EndRunningHolderRequest { machine: None, tab: Some(first_tab) },
            &token,
        ))
        .await
        .expect("an agent ends with the switch on");
    let deadline = Instant::now() + DEADLINE;
    while first.answers() {
        assert!(Instant::now() < deadline, "the ended holder still answers");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(second.answers(), "adopting ended the holder");
}
