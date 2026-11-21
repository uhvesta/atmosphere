//! SPEC flow 26.10 and flow 08 step 11 (macro case 14 step 11), the app
//! side, on a real admin `humatad2` that has a linked second machine,
//! devbox-1 (`World::start_with_machine`): an agent's request raised on
//! devbox-1 while the host's link to it is down is answered on devbox-1 by a
//! passphrase login and `humata inbox approve`; when the link comes back
//! the host's Inbox holds the request answered by the user at devbox-1's
//! CLI, and the app, which saw devbox-1 go away and come back, has no
//! request waiting for the person.
//!
//! What the app can't show yet: that the request was answered at devbox-1.
//! The live Inbox client lists waiting requests only, and the app's
//! `Request` has no field for where it was answered (see D397).

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::Duration;

use humata2_clients::Clients;
use humata2_clients::model::{Agent, SessionOptions};
use humata2_grpc::humata::v1 as pb;
use rustix::fs::{Mode, OFlags};
use rustix::pty::OpenptFlags;
use rustix::termios::QueueSelector;
use tonic::Streaming;

use super::world::{DEVBOX, LONG, World};
use super::{App, runtime};

const PASSPHRASE: &str = "copper lantern drifts over harbor";
/// `humata`'s exit code for success.
const OK: i32 = 0;
/// How long one command in the terminal may take.
const COMMAND: Duration = Duration::from_secs(30);

/// A terminal of the test's own, on devbox-1: `/bin/sh` reading commands
/// from the test, as the leader of a new terminal session whose controlling
/// terminal is a pty, with `HUMATA_SOCKET` naming devbox-1's daemon. Like a
/// real terminal it reads everything written to its screen as it comes, so
/// the shell never blocks on a full pty.
struct Terminal {
    shell: Child,
    commands: ChildStdin,
    /// The keyboard: what the test writes here is typed at the terminal.
    keyboard: File,
    /// The terminal's own end, to drop what was typed but never read.
    screen: File,
    /// What the screen showed and no wait has taken yet.
    shown: Vec<u8>,
    screen_output: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    home: tempfile::TempDir,
    runs: usize,
}

impl Terminal {
    fn open(socket: &Path) -> Self {
        let pty: OwnedFd = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        rustix::io::fcntl_setfd(&pty, rustix::io::FdFlags::CLOEXEC).unwrap();
        rustix::pty::grantpt(&pty).unwrap();
        rustix::pty::unlockpt(&pty).unwrap();
        let name = rustix::pty::ptsname(&pty, Vec::new()).unwrap();
        let screen = rustix::fs::open(
            name.as_c_str(),
            OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        let screen = File::from(screen);
        let mut output = File::from(rustix::io::fcntl_dupfd_cloexec(&pty, 0).unwrap());
        let (sender, screen_output) = tokio::sync::mpsc::unbounded_channel();
        // Passes on what the screen shows, as it comes. Keeps reading after
        // the test stops waiting. Ends when the last screen end closes.
        std::thread::spawn(move || {
            let mut bytes = [0_u8; 4096];
            loop {
                match output.read(&mut bytes) {
                    Ok(read) if read > 0 => {
                        let _ = sender.send(bytes[..read].to_vec());
                    }
                    _ => return,
                }
            }
        });
        let home = tempfile::tempdir().unwrap();
        let humata = std::fs::canonicalize(
            std::env::var_os("HUMATA").expect("set HUMATA to the binary's path (the bazel target passes it)"),
        )
        .expect("the humata binary exists");
        let mut command = Command::new("/bin/sh");
        command
            .current_dir(home.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.path())
            .env("HUMATA_SOCKET", socket)
            .env("HUMATA", humata)
            .stdin(Stdio::piped())
            .stdout(Stdio::from(screen.try_clone().unwrap()))
            .stderr(Stdio::from(screen.try_clone().unwrap()));
        // SAFETY: only async-signal-safe calls between fork and exec.
        unsafe {
            command.pre_exec(|| {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(2))?;
                Ok(())
            });
        }
        let mut shell = command.spawn().unwrap();
        let commands = shell.stdin.take().unwrap();
        Self {
            shell,
            commands,
            keyboard: File::from(pty),
            screen,
            shown: Vec::new(),
            screen_output,
            home,
            runs: 0,
        }
    }

    /// Runs `line` (a shell command running `"$HUMATA"`), typing `typed` at
    /// the terminal, and returns its exit code. Whatever was typed but not
    /// read is dropped afterwards, so it can't answer a later prompt.
    async fn exit_code(&mut self, line: &str, typed: Option<&str>) -> i32 {
        let run = self.runs;
        self.runs += 1;
        writeln!(
            self.commands,
            "{line}; echo $? > code{run}.partial && mv code{run}.partial code{run}; echo '<<done{run}>>'"
        )
        .unwrap();
        self.commands.flush().unwrap();
        if let Some(typed) = typed {
            self.keyboard.write_all(format!("{typed}\n").as_bytes()).unwrap();
            self.keyboard.flush().unwrap();
        }
        self.wait_for_screen(&format!("<<done{run}>>")).await;
        let code = std::fs::read_to_string(self.home.path().join(format!("code{run}")))
            .unwrap_or_else(|error| panic!("the exit code of command {run} was not kept: {error}"));
        rustix::termios::tcflush(&self.screen, QueueSelector::IFlush).unwrap();
        code.trim().parse().unwrap()
    }

    /// Waits until `text` has come to the screen, within [`COMMAND`].
    async fn wait_for_screen(&mut self, text: &str) {
        let needle = text.as_bytes();
        let deadline = tokio::time::Instant::now() + COMMAND;
        loop {
            let found = self
                .shown
                .windows(needle.len())
                .position(|window| window == needle);
            if let Some(at) = found {
                self.shown.drain(..at + needle.len());
                return;
            }
            match tokio::time::timeout_at(deadline, self.screen_output.recv()).await {
                Ok(Some(bytes)) => self.shown.extend_from_slice(&bytes),
                Ok(None) => panic!("the terminal's screen closed before {text:?} came"),
                Err(_) => panic!("{text:?} never came to the terminal's screen within {COMMAND:?}"),
            }
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.shell.kill();
        let _ = self.shell.wait();
    }
}

/// A feed of `topic` on devbox-1's own daemon. Opened before the event it
/// is to tell of, so none falls between.
async fn box_feed(world: &World, topic: pb::Topic) -> Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(world.devbox().channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![topic as i32],
            ..Default::default()
        })
        .await
        .expect("devbox-1's feed opens")
        .into_inner()
}

/// Waits until devbox-1 sees its host as `want`. The feed tells of each
/// change in its link (to the user only), so the view is read once and then
/// again each time the feed tells of a change to `want`. Nothing sleeps.
async fn host_reachable_becomes(world: &World, want: bool, mut feed: Streaming<pb::Event>) {
    let mut daemon = pb::daemon_service_client::DaemonServiceClient::new(world.devbox().channel().await);
    let deadline = tokio::time::Instant::now() + LONG;
    let now = daemon
        .get_server_info(pb::GetServerInfoRequest {})
        .await
        .expect("devbox-1 tells its server info")
        .into_inner();
    if now.host_reachable == want {
        return;
    }
    loop {
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(event))) => {
                let changed = matches!(
                    &event.payload,
                    Some(pb::event::Payload::HostLink(link)) if link.reachable == want
                );
                if changed {
                    let now = daemon
                        .get_server_info(pb::GetServerInfoRequest {})
                        .await
                        .expect("devbox-1 tells its server info")
                        .into_inner();
                    if now.host_reachable == want {
                        return;
                    }
                }
            }
            Ok(ended) => panic!(
                "devbox-1's feed ended ({ended:?}) before it saw its host as reachable={want}"
            ),
            Err(_) => panic!("devbox-1 never saw its host as reachable={want} within {LONG:?}"),
        }
    }
}

/// The id of the request that waits in devbox-1's own Inbox, once the feed
/// tells of it.
async fn pending_request(mut feed: Streaming<pb::Event>) -> String {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(event))) => {
                if let Some(pb::event::Payload::Request(request)) = event.payload
                    && request.state() == pb::RequestState::Pending
                    && let Some(id) = request.id
                {
                    return id.value;
                }
            }
            Ok(ended) => panic!("devbox-1's Inbox feed ended ({ended:?}) before a request waited"),
            Err(_) => panic!("no request waited in devbox-1's Inbox within {LONG:?}"),
        }
    }
}

/// The host's Inbox feed. It is opened before the host's link to devbox-1
/// comes back, which is when devbox-1's journal, and the answer in it,
/// reach the host.
async fn inbox_feed(world: &World) -> Streaming<pb::Event> {
    pb::event_service_client::EventServiceClient::new(world.host_channel().await)
        .subscribe(pb::SubscribeRequest {
            topics: vec![pb::Topic::Inbox as i32],
            ..Default::default()
        })
        .await
        .expect("the host's Inbox feed opens")
        .into_inner()
}

/// The host's own Inbox entry for `id`, once it is answered. The Inbox is
/// listed, then listed again each time `feed` tells of a change.
async fn answered_on_host(
    world: &World,
    id: &str,
    mut feed: Streaming<pb::Event>,
) -> pb::InboxRequest {
    let deadline = tokio::time::Instant::now() + LONG;
    loop {
        let listed = pb::inbox_service_client::InboxServiceClient::new(world.host_channel().await)
            .list_requests(pb::ListRequestsRequest {
                include_answered: true,
                ..Default::default()
            })
            .await
            .expect("the host lists its Inbox")
            .into_inner()
            .requests;
        let answered = listed.into_iter().find(|request| {
            request.id.as_ref().is_some_and(|found| found.value == id)
                && request.state() == pb::RequestState::Approved
        });
        if let Some(answered) = answered {
            return answered;
        }
        match tokio::time::timeout_at(deadline, feed.message()).await {
            Ok(Ok(Some(_))) => {}
            Ok(ended) => panic!(
                "the host's Inbox feed ended ({ended:?}) before request {id} was answered on the host"
            ),
            Err(_) => panic!("the host's Inbox never showed request {id} answered within {LONG:?}"),
        }
    }
}

impl App<'_> {
    /// Whether the app lists devbox-1, and whether it shows it online.
    fn devbox_online(&self) -> Option<bool> {
        self.ui.read(|state| {
            state
                .workspace
                .machines
                .iter()
                .find(|machine| machine.name == DEVBOX)
                .map(|machine| machine.online)
        })
    }

    /// As `wait_until`, for the [`LONG`] a link takes to come back. It
    /// parks between looks until gpui or the host wakes it (D399).
    fn wait_within_link(&mut self, what: &str, done: impl FnMut(&mut Self) -> bool) {
        self.wait_within(what, LONG, done);
    }
}

/// What the world makes before the app opens: a channel on This Mac and
/// devbox-1, two sessions on devbox-1 (the first is the asker), the channel's
/// While I'm away rule Wait and the approval passphrase, which devbox-1
/// holds when this returns. Returns the id of the second session.
async fn prepare(world: &World, clients: &Clients) -> String {
    let daemon = clients.daemon.clone();
    let workspace = daemon.load().await.expect("load");
    let devbox = workspace
        .machines
        .iter()
        .find(|machine| machine.name == DEVBOX)
        .map(|machine| machine.id.clone())
        .expect("the host lists devbox-1");
    let mac = workspace
        .machines
        .iter()
        .find(|machine| machine.id != devbox)
        .map(|machine| machine.id.clone())
        .expect("the host lists This Mac");
    let repo = world.repo("owner", "ledger").expect("make a repo");
    let channel = super::create_channel_ready(
        &daemon,
        "ledger",
        vec![repo.clone()],
        vec![mac, devbox.clone()],
    )
    .await;
    let start = |title: &'static str, first_message: &'static str| {
        let daemon = daemon.clone();
        let channel = channel.clone();
        let devbox = devbox.clone();
        let repo = repo.clone();
        async move {
            daemon
                .create_session(
                    &channel,
                    Agent::Claude,
                    &devbox,
                    title,
                    SessionOptions {
                        repos: vec![repo],
                        first_message: first_message.to_owned(),
                        ..SessionOptions::default()
                    },
                )
                .await
                .expect("start a session on devbox-1")
        }
    };
    // The agent whose first message is `asker` is the one that asks. It
    // waits on devbox-1's fifo for the session to ask about, so neither
    // agent needs to be seen running before the test goes on.
    start("asker", "asker").await;
    let other = start("other", "hello").await;

    let mut access =
        pb::access_service_client::AccessServiceClient::new(world.host_channel().await);
    access
        .set_away_rule(pb::SetAwayRuleRequest {
            channel: Some(pb::ChannelId {
                value: channel.as_str().to_owned(),
            }),
            away: Some(pb::AwayRule {
                mode: pb::AwayMode::Wait as i32,
                low_risk: Vec::new(),
            }),
            ..Default::default()
        })
        .await
        .expect("the channel's While I'm away rule is set");
    // The rule is set first, so the policy copy that carries the passphrase's
    // hash carries the rule: devbox-1 listed as updated holds both.
    let set = access
        .set_approval_passphrase(pb::SetApprovalPassphraseRequest {
            passphrase: PASSPHRASE.to_owned(),
            ..Default::default()
        })
        .await
        .expect("the approval passphrase is set")
        .into_inner();
    assert!(
        set.updated.iter().any(|machine| machine.value == devbox.as_str()),
        "devbox-1 took the policy copy while linked; updated {:?}, pending {:?}",
        set.updated,
        set.pending
    );
    other.as_str().to_owned()
}

/// Flow 08 step 11 / macro 14 step 11, the app side: devbox-1's request,
/// raised and answered there while the link is down, is settled on the host
/// when the link returns, and the app never holds it as waiting.
#[test]
fn a_request_answered_on_devbox_1_while_the_link_was_down_is_settled_when_the_link_returns() {
    let runtime = runtime();
    let world = runtime
        .block_on(World::start_with_machine())
        .expect("start the host with devbox-1");
    let clients = runtime.block_on(world.clients()).expect("connect");
    let other = runtime.block_on(prepare(&world, &clients));
    let mut app = App::open(&runtime, clients);
    app.wait_within_link("devbox-1 online in the app", |app| {
        app.devbox_online() == Some(true)
    });

    // 1. The link goes down, as a closed Mac's does. The app shows devbox-1
    //    offline, and devbox-1 sees its host gone.
    let mut terminal = Terminal::open(world.devbox().socket());
    let inbox = runtime.block_on(inbox_feed(&world));
    let box_inbox = runtime.block_on(box_feed(&world, pb::Topic::Inbox));
    let box_link = runtime.block_on(box_feed(&world, pb::Topic::Machines));
    world.devbox().cut_link();
    app.wait_within_link("devbox-1 offline in the app", |app| {
        app.devbox_online() == Some(false)
    });
    runtime.block_on(host_reachable_becomes(&world, false, box_link));

    // 2. The agent on devbox-1 asks to type into the other session; the
    //    rule is Wait, so the request waits in devbox-1's own Inbox. The
    //    write blocks until the asker reads the fifo.
    let go = world.devbox().folder().join("go");
    let target = other.clone();
    // Both the timer and the blocking task need the runtime, so they are
    // made inside it.
    let released = runtime.block_on(async move {
        tokio::time::timeout(
            LONG,
            tokio::task::spawn_blocking(move || std::fs::write(go, target)),
        )
        .await
    });
    released
        .unwrap_or_else(|_| panic!("the asker never read devbox-1's fifo within {LONG:?}"))
        .expect("the write finished")
        .expect("the agent was released");

    // 3. The person logs in at a terminal on devbox-1 with the passphrase
    //    (devbox-1 no longer sees its host, so it takes it), finds the
    //    request that devbox-1's Inbox told of and approves it there.
    let id = runtime.block_on(async {
        assert_eq!(
            terminal.exit_code(r#""$HUMATA" login < /dev/tty"#, Some(PASSPHRASE)).await,
            OK,
            "devbox-1 takes the passphrase login"
        );
        let id = pending_request(box_inbox).await;
        let approve = format!(r#""$HUMATA" --json inbox approve {id} > /dev/null 2>&1"#);
        assert_eq!(terminal.exit_code(&approve, None).await, OK, "the approval is accepted");
        id
    });

    // 4. Still away: the host knows nothing of the request, so the app holds
    //    none waiting.
    app.settle();
    assert_eq!(
        app.ui.read(|state| state.workspace.waiting().count()),
        0,
        "the app holds no waiting request while devbox-1 is away"
    );

    // 5. The link comes back by the host's own dial. devbox-1 replays its
    //    Inbox journal: the host's Inbox shows the request answered, by the
    //    user at devbox-1's CLI.
    world.devbox().restore_link();
    app.wait_within_link("devbox-1 online in the app again", |app| {
        app.devbox_online() == Some(true)
    });
    let answered = runtime.block_on(answered_on_host(&world, &id, inbox));
    let by = answered.answered.and_then(|answered| answered.by).and_then(|actor| actor.actor);
    match by {
        Some(pb::actor::Actor::User(user)) => {
            assert_eq!(user.via(), pb::user_actor::Via::CliOnMachine);
            assert_eq!(user.machine.map(|machine| machine.value).as_deref(), Some(devbox_id(&app).as_str()));
        }
        unexpected => panic!("the answer is credited to the user on devbox-1, not {unexpected:?}"),
    }

    // 6. The app has no request of it waiting: the first answer won.
    app.wait_within_link("the app to hold no request waiting", |app| {
        app.ui.read(|state| state.workspace.waiting().count()) == 0
    });
    drop(app);
    drop(terminal);
    drop(world);
}

/// devbox-1's machine id, as the app lists it.
fn devbox_id(app: &App) -> String {
    app.ui
        .read(|state| {
            state
                .workspace
                .machines
                .iter()
                .find(|machine| machine.name == DEVBOX)
                .map(|machine| machine.id.as_str().to_owned())
        })
        .expect("the app lists devbox-1")
}
