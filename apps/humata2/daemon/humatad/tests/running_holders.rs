//! D784: the machine lists every running holder of ours, a session's and
//! one no session names, and never one of another state folder. Each listed
//! holder attaches by its tab, and only an explicit End ends it, with its
//! program. A shell stops and resumes where it was; an agent's tab is its
//! session's to stop, resume or end.

mod common;

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{
    Attach, Daemon, World, agent_tab, binary, create_channel, finished, folder, get_session,
    is_running, open, start_session, stored_tabs,
};
use humata2_grpc::humata::v1 as pb;
use rustix::process::{Pid, Signal};
use tonic::Status;
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

async fn end(daemon: &Daemon, tab: &pb::TabId) -> Result<(), Status> {
    machines(daemon)
        .await
        .end_running_holder(pb::EndRunningHolderRequest {
            machine: None,
            tab: Some(tab.clone()),
        })
        .await
        .map(drop)
}

async fn stop(daemon: &Daemon, tab: &pb::TabId) -> Result<(), Status> {
    machines(daemon)
        .await
        .stop_running_holder(pb::StopRunningHolderRequest {
            machine: None,
            tab: Some(tab.clone()),
        })
        .await
        .map(drop)
}

async fn resume(daemon: &Daemon, tab: &pb::TabId) -> Result<(), Status> {
    machines(daemon)
        .await
        .resume_running_holder(pb::ResumeRunningHolderRequest {
            machine: None,
            tab: Some(tab.clone()),
        })
        .await
        .map(drop)
}

/// The stored tab `tab`, as the daemon keeps it.
fn stored(daemon: &Daemon, tab: &pb::TabId) -> humata2_store::TabRecord {
    stored_tabs(&daemon.state)
        .into_iter()
        .find(|record| record.id == tab.value)
        .expect("the tab is kept")
}

async fn wait_gone(tab: &humata2_store::TabRecord, what: &str) {
    let deadline = Instant::now() + DEADLINE;
    while common::holder_answers(tab) {
        assert!(Instant::now() < deadline, "{what} still runs");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The daemon's holders folder as it names it: made canonical, as `serve`
/// does.
fn holders_of(daemon: &Daemon) -> PathBuf {
    std::fs::canonicalize(&daemon.state).unwrap().join("holders")
}

#[tokio::test]
async fn every_holder_of_ours_is_listed_attaches_and_ends_only_when_asked() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, &[]).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    common::wait_running(&daemon, &id).await;

    let orphan = Stray::start(&holders_of(&daemon).join("tb_00000000000000000000000000000001.sock"));
    let elsewhere = std::fs::canonicalize(daemon.dir.path())
        .unwrap()
        .join("other/state/holders/tb_00000000000000000000000000000002.sock");
    let foreign = Stray::start(&elsewhere);

    let listed = list(&daemon).await;
    let session_rows: Vec<_> = listed
        .iter()
        .filter(|holder| holder.session.as_ref() == Some(&id))
        .collect();
    assert!(!session_rows.is_empty(), "the running session's holder isn't listed");
    assert!(session_rows.iter().all(|holder| holder.title == "Fix login"));
    let orphan_row = listed
        .iter()
        .find(|holder| holder.pid == orphan.pid())
        .expect("a holder no session names isn't listed");
    assert!(orphan_row.session.is_none(), "an orphan is listed under a session");
    assert!(
        listed.iter().all(|holder| holder.pid != foreign.pid()),
        "a holder of another state folder is listed as ours"
    );
    // Listing again names the same tab for it.
    let again = list(&daemon).await;
    let orphan_tab = orphan_row.tab.clone().unwrap();
    assert!(
        again
            .iter()
            .any(|holder| holder.pid == orphan.pid() && holder.tab.as_ref() == Some(&orphan_tab))
    );
    // Nothing was ended by being listed.
    assert!(orphan.answers() && orphan.program_runs());

    assert!(
        Attach::try_start(&daemon, open(&orphan_tab, 0)).await.is_ok(),
        "a listed orphan can't be attached"
    );

    // An agent's tab is its session's to stop, resume or end.
    let agent = agent_tab(&daemon, &id).await;
    for refused in [
        end(&daemon, &agent).await,
        stop(&daemon, &agent).await,
        resume(&daemon, &agent).await,
    ] {
        assert_eq!(refused.unwrap_err().code(), tonic::Code::FailedPrecondition);
    }
    assert!(is_running(&get_session(&daemon, &id).await), "a refused call touched the agent");

    end(&daemon, &orphan_tab).await.expect("the orphan ends");
    let deadline = Instant::now() + DEADLINE;
    while orphan.answers() || orphan.program_runs() {
        assert!(Instant::now() < deadline, "an ended orphan, or its program, still runs");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        list(&daemon).await.iter().all(|holder| holder.pid != orphan.pid()),
        "an ended orphan is still listed"
    );

    // A stopped session is listed as stopped, so it can be found and resumed.
    let agent_holder = stored(&daemon, &agent);
    daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()) })
        .await
        .expect("the session stops");
    wait_gone(&agent_holder, "a stopped session's agent").await;
    let listed = list(&daemon).await;
    assert!(
        listed
            .iter()
            .any(|holder| holder.session.as_ref() == Some(&id) && holder.stopped),
        "a stopped session isn't listed as stopped: {listed:?}"
    );
    assert!(
        foreign.answers() && foreign.program_runs(),
        "a holder of another state folder was ended"
    );
}

#[tokio::test]
async fn a_stopped_shell_stays_stopped_through_a_restart_and_resumes_where_it_was() {
    let mut daemon = Daemon::start().await;
    let tab = daemon
        .terminals()
        .await
        .open_machine_shell(pb::OpenMachineShellRequest::default())
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    attach.type_text("echo stop-$((40+2))\n").await;
    attach.wait_for("stop-42").await;
    drop(attach);
    let holder = stored(&daemon, &tab);

    stop(&daemon, &tab).await.expect("the shell stops");
    wait_gone(&holder, "a stopped shell").await;
    let row = list(&daemon)
        .await
        .into_iter()
        .find(|row| row.tab.as_ref() == Some(&tab))
        .expect("a stopped shell isn't listed");
    assert!(row.stopped, "a stopped shell is listed as running");

    // A start revives shells whose holders are gone, but not a stopped one.
    daemon.restart().await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!common::holder_answers(&stored(&daemon, &tab)), "a start revived a stopped shell");

    resume(&daemon, &tab).await.expect("the shell resumes");
    let deadline = Instant::now() + DEADLINE;
    while !common::holder_answers(&stored(&daemon, &tab)) {
        assert!(Instant::now() < deadline, "a resumed shell never started");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    // What it showed before the stop is shown above the new shell.
    attach.wait_for("stop-42").await;
    drop(attach);
    assert!(
        list(&daemon).await.iter().any(|row| row.tab.as_ref() == Some(&tab) && !row.stopped),
        "a resumed shell isn't listed as running"
    );

    let running = stored(&daemon, &tab);
    end(&daemon, &tab).await.expect("the shell ends");
    wait_gone(&running, "an ended shell").await;
    assert!(
        stored_tabs(&daemon.state).iter().all(|record| record.id != tab.value),
        "an ended shell's tab is kept"
    );
}

#[tokio::test]
async fn a_stopped_session_resumes_in_the_same_conversation_and_folder() {
    let dir = folder();
    let world = World::new(dir.path());
    let args = world.args();
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let daemon = Daemon::start_with_env(common::Build::Admin, dir, &refs, &[]).await;
    world.remote("acme", "api");
    let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
    let (id, _) = start_session(&daemon, &channel, "Fix login").await;
    let before = common::wait_running(&daemon, &id).await;
    let holder = stored(&daemon, &agent_tab(&daemon, &id).await);

    daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()) })
        .await
        .expect("the session stops");
    wait_gone(&holder, "a stopped session's agent").await;
    assert!(!is_running(&get_session(&daemon, &id).await));

    let resumed = daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    let done = finished(&daemon, &resumed).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
    let after = common::wait_running(&daemon, &id).await;
    assert_eq!(after.conversation_id, before.conversation_id);
    assert_eq!(after.folder, before.folder);
    let agent = agent_tab(&daemon, &id).await;
    assert!(Attach::try_start(&daemon, open(&agent, 0)).await.is_ok());
}
