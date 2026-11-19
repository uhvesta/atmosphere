//! Behaviour the fake and the real daemon client must share. Each case is
//! written once and run against both worlds.

use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_clients::model::{Agent, ChannelId, MachineId, SessionId, SessionOptions};
use humata2_clients::{Daemon, DaemonError, Event, Events, ImageKind, Terminals};
use terminal_wire::{ClientFrame, ServerFrame};

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
    /// Whether the paths the daemon reports are real files on this machine.
    fn paths_exist(&self) -> bool;
}

pub async fn within<T>(what: &str, work: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(DEADLINE, work).await {
        Ok(value) => value,
        Err(_) => panic!("timed out waiting for {what}"),
    }
}

pub async fn this_mac(daemon: &Arc<dyn Daemon>) -> MachineId {
    let workspace = within("load", daemon.load()).await.expect("load");
    workspace
        .machines
        .first()
        .expect("the workspace has a machine")
        .id
        .clone()
}

pub async fn channel_with_repo(
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

pub async fn start_session(
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

/// Waits until `id` has started: its agent's terminal opens. The daemon
/// answers a new session once it is recorded, and clones its repositories
/// and launches its agent after that.
async fn until_started(world: &dyn World, terminals: &Arc<dyn Terminals>, id: &SessionId) {
    let until = Instant::now() + DEADLINE;
    while let Err(error) = terminals.open(id, world.agent()).await {
        assert!(Instant::now() < until, "the session never started: {error:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
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

pub async fn a_started_session_tells_where_it_lives(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    terminals: Arc<dyn Terminals>,
) {
    let (channel, repo, mac) = channel_with_repo(world, &daemon, "payments").await;
    let id = start_session(world, &daemon, &channel, &repo, &mac, "Fix the ledger").await;
    until_started(world, &terminals, &id).await;
    let workspace = within("load", daemon.load()).await.expect("load");
    let session = workspace
        .sessions
        .iter()
        .find(|session| session.id == id)
        .expect("the new session is listed");
    let folder = std::path::Path::new(&session.folder);
    assert!(folder.is_absolute(), "the folder {:?} is not absolute", session.folder);
    assert_eq!(session.clones.len(), 1);
    let clone = &session.clones[0];
    assert_eq!(clone.repo, repo);
    assert!(!clone.branch.is_empty());
    let path = std::path::Path::new(&clone.path);
    assert!(path.is_absolute(), "the clone path {:?} is not absolute", clone.path);
    assert!(path.starts_with(folder), "the clone is outside the session's folder");
    if world.paths_exist() {
        assert!(folder.is_dir(), "{folder:?} does not exist");
        assert!(path.is_dir(), "{path:?} does not exist");
    }
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
    let mut link = within("terminal link", terminals.link(&id, 0))
        .await
        .expect("the tab's link opens");
    let mut text = Vec::new();
    let mut seq = 0;
    let found = loop {
        // A line sent while the link is still attaching is refused; the
        // next round sends it again.
        seq += 1;
        let _ = link.sink.send(ClientFrame::Input {
            id: 0,
            seq,
            bytes: b"ping\r".to_vec(),
        });
        let round = (Instant::now() + Duration::from_secs(2)).min(until);
        let hit = loop {
            let left = round.saturating_duration_since(Instant::now());
            match tokio::time::timeout(left, link.frames.recv()).await {
                Ok(Some(ServerFrame::Output { bytes, .. })) => {
                    text.extend_from_slice(&bytes);
                    if String::from_utf8_lossy(&text).contains(answer) {
                        break true;
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break false,
            }
        };
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

/// A pasted image is saved on the session's machine, under a name of its
/// kind; bytes that are not the kind they claim are refused.
pub async fn a_pasted_image_comes_back_as_its_path(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    terminals: Arc<dyn Terminals>,
) {
    let (channel, repo, mac) = channel_with_repo(world, &daemon, "pastes").await;
    let id = start_session(world, &daemon, &channel, &repo, &mac, "Paste a screenshot").await;
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    let path = within(
        "paste_image",
        terminals.paste_image(&id, ImageKind::Png, png.clone()),
    )
    .await
    .expect("a PNG is saved");
    assert!(path.starts_with('/'), "{path} is absolute");
    assert!(path.ends_with(".png"), "{path} names a PNG");
    let refused = within("paste_image", terminals.paste_image(&id, ImageKind::Jpeg, png)).await;
    assert!(refused.is_err(), "PNG bytes sent as a JPEG are refused");
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

/// Types `ping` to the session's agent until `seen` holds for an event,
/// each round waiting a little: the agent may still be starting, and a
/// view just ended may not have reached the daemon yet.
async fn ping_until(
    world: &dyn World,
    daemon: &Arc<dyn Daemon>,
    terminals: &Arc<dyn Terminals>,
    id: &SessionId,
    mut seen: impl FnMut(&Event) -> bool,
) -> bool {
    let until = Instant::now() + DEADLINE;
    let mut events = within("events", daemon.events()).await.expect("events");
    loop {
        match terminals.open(id, world.agent()).await {
            Ok(_) => break,
            Err(error) => {
                assert!(Instant::now() < until, "could not open the terminal: {error:?}");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
    let mut link = within("terminal link", terminals.link(id, 0))
        .await
        .expect("the tab's link opens");
    let mut seq = 0;
    loop {
        seq += 1;
        let _ = link.sink.send(ClientFrame::Input {
            id: 0,
            seq,
            bytes: b"ping\r".to_vec(),
        });
        while link.frames.try_recv().is_ok() {}
        let round = (Instant::now() + Duration::from_secs(2)).min(until);
        if wait_for(&mut events, round, &mut seen).await {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
    }
}

async fn session_in(daemon: &Arc<dyn Daemon>, id: &SessionId) -> humata2_clients::model::Session {
    let workspace = within("load", daemon.load()).await.expect("load");
    match workspace.sessions.into_iter().find(|session| &session.id == id) {
        Some(session) => session,
        None => panic!("{id} is listed"),
    }
}

/// CH-16, CH-17 (D392): the agent's output on a session nobody views
/// marks it unread, pushed as a session change; marking it read clears it.
pub async fn unviewed_output_marks_a_session_unread(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    terminals: Arc<dyn Terminals>,
) {
    let (channel, repo, mac) = channel_with_repo(world, &daemon, "payments").await;
    let id = start_session(world, &daemon, &channel, &repo, &mac, "Fix the ledger").await;
    // Read, whatever the agent printed as it started; then nobody views it.
    within("mark_read", daemon.mark_read(&id)).await.expect("mark it read");
    within("view", daemon.view(None)).await.expect("view nothing");
    let marked = ping_until(world, &daemon, &terminals, &id, |event| {
        matches!(event, Event::Session(session) if session.id == id && session.unread > 0)
    })
    .await;
    assert!(marked, "the session was never marked unread");
    within("mark_read", daemon.mark_read(&id)).await.expect("mark it read");
    assert_eq!(session_in(&daemon, &id).await.unread, 0, "marking it read cleared it");
}

/// Marking read or viewing a session the daemon doesn't know is no error:
/// there is nothing to clear.
pub async fn reading_an_unknown_session_is_not_an_error(
    _world: &dyn World,
    daemon: Arc<dyn Daemon>,
    _terminals: Arc<dyn Terminals>,
) {
    let missing = SessionId::new("nope");
    within("mark_read", daemon.mark_read(&missing)).await.expect("nothing to clear");
    within("view", daemon.view(Some(&missing))).await.expect("nothing to view");
    within("view", daemon.view(None)).await.expect("view nothing");
}

/// Whether the session is listed and not archived.
async fn listed_live(daemon: &Arc<dyn Daemon>, id: &SessionId) -> bool {
    let workspace = within("load", daemon.load()).await.expect("load");
    workspace
        .sessions
        .iter()
        .any(|session| &session.id == id && !session.archived)
}

pub async fn an_archived_session_is_restored(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    terminals: Arc<dyn Terminals>,
) {
    let (channel, repo, mac) = channel_with_repo(world, &daemon, "payments").await;
    let id = start_session(world, &daemon, &channel, &repo, &mac, "Fix the ledger").await;
    until_started(world, &terminals, &id).await;
    within("archive", daemon.archive(&id, false))
        .await
        .expect("archive");
    assert!(
        !listed_live(&daemon, &id).await,
        "an archived session is not listed as a live one"
    );

    within("restore_session", daemon.restore_session(&id))
        .await
        .expect("restore the archived session");
    assert!(
        listed_live(&daemon, &id).await,
        "the restored session is listed again"
    );
}

pub async fn restoring_a_session_that_is_not_archived_is_refused(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    _terminals: Arc<dyn Terminals>,
) {
    let (channel, repo, mac) = channel_with_repo(world, &daemon, "payments").await;
    let id = start_session(world, &daemon, &channel, &repo, &mac, "Fix the ledger").await;
    let result = within("restore_session", daemon.restore_session(&id)).await;
    assert!(result.is_err(), "expected a refusal, got {result:?}");
    assert!(listed_live(&daemon, &id).await, "the session is untouched");
}
