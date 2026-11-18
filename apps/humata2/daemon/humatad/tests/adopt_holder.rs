//! D784: an orphan holder (a running holder no session names) is adopted
//! into a channel or the Desk and becomes a session there that keeps
//! running the same holder: same pid, still attaches, no longer an orphan.

mod common;

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Attach, Daemon, World, agent_tab, binary, create_channel, folder, get_session, open, start_session, stored_tabs, wait_running};
use humata2_grpc::humata::v1 as pb;
use rustix::process::{Pid, Signal};
use tonic::Code;
use tonic::transport::Channel;

/// How long a holder gets to start or end.
const DEADLINE: Duration = Duration::from_secs(30);

/// A holder this test started by hand, serving `socket`, running `sleep`.
/// Dropped, it is ended if it still runs, whatever the test did.
struct Stray {
    socket: PathBuf,
    program: PathBuf,
    child: std::process::Child,
}

impl Stray {
    /// Starts a holder at `socket` as the daemon would, whose program
    /// writes its pid to `<socket>.program` before it sleeps.
    fn start(socket: &Path) -> Self {
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
        let program = socket.with_extension("program");
        let child = Command::new(binary("HUMATA2_HOLDER"))
            .arg("--socket")
            .arg(socket)
            .arg("--cwd")
            .arg(socket.parent().unwrap())
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(format!("echo $$ > {}; exec sleep 1003", program.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stray = Self {
            socket: socket.to_owned(),
            program,
            child,
        };
        let deadline = Instant::now() + DEADLINE;
        while !stray.answers() || stray.program_pid().is_none() {
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

    fn program_pid(&self) -> Option<u32> {
        std::fs::read_to_string(&self.program).ok()?.trim().parse().ok()
    }

    /// Whether the holder's program still runs as this test's `sleep`.
    fn program_runs(&self) -> bool {
        self.program_pid().is_some_and(|pid| {
            Command::new("/bin/ps")
                .args(["-o", "command=", "-p", &pid.to_string()])
                .output()
                .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("sleep 1003"))
        })
    }
}

impl Drop for Stray {
    fn drop(&mut self) {
        if self.program_runs()
            && let Some(pid) = self.program_pid().and_then(|pid| Pid::from_raw(i32::try_from(pid).ok()?))
        {
            let _ = rustix::process::kill_process(pid, Signal::KILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn machines(daemon: &Daemon) -> pb::machine_service_client::MachineServiceClient<Channel> {
    pb::machine_service_client::MachineServiceClient::new(daemon.channel().await)
}

async fn list(daemon: &Daemon) -> Vec<pb::RunningHolder> {
    machines(daemon)
        .await
        .list_running_holders(pb::ListRunningHoldersRequest { machine: None })
        .await
        .unwrap()
        .into_inner()
        .holders
}

async fn adopt(
    daemon: &Daemon,
    tab: &pb::TabId,
    channel: Option<&pb::ChannelId>,
) -> Result<pb::SessionId, tonic::Status> {
    machines(daemon)
        .await
        .adopt_running_holder(pb::AdoptRunningHolderRequest {
            machine: None,
            tab: Some(tab.clone()),
            channel: channel.cloned(),
        })
        .await
        .map(|response| response.into_inner().session.expect("the adopted session is named"))
}

fn holders_of(daemon: &Daemon) -> PathBuf {
    std::fs::canonicalize(&daemon.state).unwrap().join("holders")
}

async fn started() -> (Daemon, World, pb::Channel) {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, &[]).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    (daemon, world, channel)
}

async fn orphan_tab(daemon: &Daemon, orphan: &Stray) -> pb::TabId {
    list(daemon)
        .await
        .into_iter()
        .find(|holder| holder.pid == orphan.pid())
        .expect("the orphan is listed")
        .tab
        .expect("an orphan has a tab")
}

#[tokio::test]
async fn an_orphan_adopted_into_a_channel_lists_there_with_the_same_pid_and_still_attaches() {
    let (daemon, _world, channel) = started().await;
    let orphan = Stray::start(&holders_of(&daemon).join("tb_00000000000000000000000000000001.sock"));
    let tab = orphan_tab(&daemon, &orphan).await;

    let session = adopt(&daemon, &tab, channel.id.as_ref()).await.expect("the orphan is adopted");

    let row = list(&daemon)
        .await
        .into_iter()
        .find(|holder| holder.pid == orphan.pid())
        .expect("the adopted holder is still listed");
    assert_eq!(row.session.as_ref(), Some(&session));
    assert_eq!(row.channel.as_ref(), channel.id.as_ref());
    assert_eq!(row.tab.as_ref(), Some(&tab));
    assert_eq!(get_session(&daemon, &session).await.id.as_ref(), Some(&session));
    assert!(orphan.answers() && orphan.program_runs(), "adopting ended the holder");
    assert!(
        Attach::try_start(&daemon, open(&tab, 0)).await.is_ok(),
        "an adopted holder can't be attached"
    );
    let kept = stored_tabs(&daemon.state).into_iter().find(|record| record.id == tab.value).unwrap();
    assert_eq!(kept.session.as_deref(), Some(session.value.as_str()));
}

#[tokio::test]
async fn an_orphan_adopted_with_no_channel_goes_to_the_desk() {
    let (daemon, _world, _channel) = started().await;
    let orphan = Stray::start(&holders_of(&daemon).join("tb_00000000000000000000000000000002.sock"));
    let tab = orphan_tab(&daemon, &orphan).await;

    let session = adopt(&daemon, &tab, None).await.expect("the orphan is adopted");

    let row = list(&daemon)
        .await
        .into_iter()
        .find(|holder| holder.pid == orphan.pid())
        .unwrap();
    assert_eq!(row.session.as_ref(), Some(&session));
    assert!(row.desk, "{row:?}");
    assert!(row.channel.is_none());
}

#[tokio::test]
async fn a_tab_a_session_already_holds_or_a_channel_nobody_knows_is_refused_and_nothing_changes() {
    let (daemon, _world, channel) = started().await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    wait_running(&daemon, &id).await;
    let held = agent_tab(&daemon, &id).await;
    let refused = adopt(&daemon, &held, channel.id.as_ref()).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused:?}");

    let orphan = Stray::start(&holders_of(&daemon).join("tb_00000000000000000000000000000003.sock"));
    let tab = orphan_tab(&daemon, &orphan).await;
    let unknown = pb::ChannelId { value: "ch_nobody".to_owned() };
    let refused = adopt(&daemon, &tab, Some(&unknown)).await.unwrap_err();
    assert_eq!(refused.code(), Code::NotFound, "{refused:?}");
    let still = list(&daemon).await.into_iter().find(|holder| holder.pid == orphan.pid()).unwrap();
    assert!(still.session.is_none(), "a refused adopt still named a session");
}
