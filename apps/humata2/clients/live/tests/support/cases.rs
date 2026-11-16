//! Behaviour the fake and the real daemon client must share. Each case is
//! written once and run against both worlds.

use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_clients::model::{Agent, ChannelId, MachineId, SessionId, SessionOptions};
use humata2_clients::{Daemon, DaemonError, Event, Events, Terminals};

pub const DEADLINE: Duration = Duration::from_secs(20);

/// What a case needs from the implementation it runs on.
pub trait World: Send + Sync {
    /// Makes `owner/name` available to channels and returns that name.
    fn repo(&self, owner: &str, name: &str) -> String;
    /// The agent sessions start with.
    fn agent(&self) -> Agent;
    /// Text that appears only in the agent's answer to the line `ping`,
    /// never in the echo of the typed line itself.
    fn answer_to_ping(&self) -> &'static str;
}

async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

async fn this_mac(daemon: &Arc<dyn Daemon>) -> MachineId {
    let workspace = within("load", daemon.load()).await.expect("load");
    workspace
        .machines
        .first()
        .expect("the workspace has a machine")
        .id
        .clone()
}

async fn channel_with_repo(
    world: &dyn World,
    daemon: &Arc<dyn Daemon>,
    name: &str,
) -> (ChannelId, String, MachineId) {
    let mac = this_mac(daemon).await;
    let repo = world.repo("owner", "ledger");
    let id = within(
        "create_channel",
        daemon.create_channel(name, vec![repo.clone()], vec![mac.clone()]),
    )
    .await
    .expect("create channel");
    (id, repo, mac)
}

async fn start_session(
    world: &dyn World,
    daemon: &Arc<dyn Daemon>,
    channel: &ChannelId,
    repo: &str,
    mac: &MachineId,
    title: &str,
) -> SessionId {
    within(
        "create_session",
        daemon.create_session(
            channel,
            world.agent(),
            mac,
            title,
            SessionOptions {
                repos: vec![repo.to_owned()],
                first_message: String::new(),
                ..SessionOptions::default()
            },
        ),
    )
    .await
    .expect("create session")
}

pub async fn new_channel_is_listed(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    _terminals: Arc<dyn Terminals>,
) {
    let (id, repo, mac) = channel_with_repo(world, &daemon, "payments").await;
    let workspace = within("load", daemon.load()).await.expect("load");
    let channel = workspace
        .channels
        .iter()
        .find(|channel| channel.id == id)
        .expect("the new channel is listed");
    assert_eq!(channel.name, "payments");
    assert!(channel.repos.contains(&repo));
    assert!(channel.machines.contains(&mac));
}

pub async fn channel_needs_a_name(
    _world: &dyn World,
    daemon: Arc<dyn Daemon>,
    _terminals: Arc<dyn Terminals>,
) {
    let mac = this_mac(&daemon).await;
    let result = within("create_channel", daemon.create_channel("", vec![], vec![mac])).await;
    assert!(
        matches!(result, Err(DaemonError::Refused { .. })),
        "expected a refusal, got {result:?}"
    );
}

pub async fn new_session_is_listed(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    _terminals: Arc<dyn Terminals>,
) {
    let (channel, repo, mac) = channel_with_repo(world, &daemon, "payments").await;
    let id = start_session(world, &daemon, &channel, &repo, &mac, "Fix the ledger").await;
    let workspace = within("load", daemon.load()).await.expect("load");
    let session = workspace
        .sessions
        .iter()
        .find(|session| session.id == id)
        .expect("the new session is listed");
    assert_eq!(session.title, "Fix the ledger");
    assert_eq!(session.channel, channel);
    assert_eq!(session.machine, mac);
    assert!(!session.branch.is_empty());
}

pub async fn session_in_missing_channel(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    _terminals: Arc<dyn Terminals>,
) {
    let mac = this_mac(&daemon).await;
    let repo = world.repo("owner", "ledger");
    let result = within(
        "create_session",
        daemon.create_session(
            &ChannelId::new("nope"),
            world.agent(),
            &mac,
            "Lost",
            SessionOptions {
                repos: vec![repo],
                ..SessionOptions::default()
            },
        ),
    )
    .await;
    assert!(
        matches!(result, Err(DaemonError::NotFound { .. })),
        "expected NotFound, got {result:?}"
    );
}

/// Whether `events` delivers `done` before the deadline; `seen` looks at
/// every event and says whether it is what the case waits for.
async fn wait_for(
    events: &mut Events,
    until: Instant,
    mut seen: impl FnMut(&Event) -> bool,
) -> bool {
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, events.recv()).await {
            Ok(Some(Ok(event))) => {
                if seen(&event) {
                    return true;
                }
            }
            Ok(Some(Err(_))) => {}
            Ok(None) | Err(_) => return false,
        }
    }
}

pub async fn typed_text_reaches_the_agent(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    terminals: Arc<dyn Terminals>,
) {
    let mut events = within("events", daemon.events()).await.expect("events");
    let (channel, repo, mac) = channel_with_repo(world, &daemon, "payments").await;
    let id = start_session(world, &daemon, &channel, &repo, &mac, "Fix the ledger").await;
    let until = Instant::now() + DEADLINE;
    // The agent may still be starting; try to open until it answers.
    let tab = loop {
        match terminals.open(&id, world.agent()).await {
            Ok((tab, _)) => break tab,
            Err(error) => {
                assert!(Instant::now() < until, "could not open the terminal: {error:?}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    };
    assert_eq!(tab, 0, "the session's own agent is tab 0");
    let answer = world.answer_to_ping();
    let mut text = Vec::new();
    let found = loop {
        within(
            "terminal input",
            terminals.input(&id, 0, b"ping\r".to_vec()),
        )
        .await
        .expect("type into the agent");
        let round = (Instant::now() + Duration::from_secs(2)).min(until);
        let hit = wait_for(&mut events, round, |event| match event {
            Event::TerminalData {
                session,
                tab: 0,
                bytes,
                ..
            } if session == &id => {
                text.extend_from_slice(bytes);
                String::from_utf8_lossy(&text).contains(answer)
            }
            Event::Output {
                session,
                tab: 0,
                line,
            } if session == &id => {
                text.extend_from_slice(line.text.as_bytes());
                String::from_utf8_lossy(&text).contains(answer)
            }
            _ => false,
        })
        .await;
        if hit {
            break true;
        }
        if Instant::now() >= until {
            break false;
        }
    };
    assert!(found, "the agent never answered the typed line");
}

pub async fn resume_missing_session(
    _world: &dyn World,
    daemon: Arc<dyn Daemon>,
    _terminals: Arc<dyn Terminals>,
) {
    let result = within("resume", daemon.resume(&SessionId::new("nope"))).await;
    assert!(
        matches!(result, Err(DaemonError::NotFound { .. })),
        "expected NotFound, got {result:?}"
    );
}

pub async fn session_change_is_pushed(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    _terminals: Arc<dyn Terminals>,
) {
    let mut events = within("events", daemon.events()).await.expect("events");
    let (channel, repo, mac) = channel_with_repo(world, &daemon, "payments").await;
    let id = start_session(world, &daemon, &channel, &repo, &mac, "Fix the ledger").await;
    let found = wait_for(&mut events, Instant::now() + DEADLINE, |event| {
        matches!(event, Event::Session(session) if session.id == id)
    })
    .await;
    assert!(found, "no session event arrived for the new session");
}
