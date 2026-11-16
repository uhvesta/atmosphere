//! Behaviour the fake and the real daemon client must share. Each case is
//! written once and run against both worlds.

use std::sync::Arc;
use std::time::{Duration, Instant};

use humata2_clients::comments::{
    Anchor, Comment, CommentEvent, CommentEvents, CommentsClient, Side, Status, Which,
};
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

// ---------------------------------------------------------------------------
// Review comments: the same cases against FakeComments and LiveComments.
// ---------------------------------------------------------------------------

async fn session_to_comment_on(
    world: &dyn World,
    daemon: &Arc<dyn Daemon>,
) -> (SessionId, String) {
    let (channel, repo, mac) = channel_with_repo(world, daemon, "payments").await;
    let id = start_session(world, daemon, &channel, &repo, &mac, "Fix the ledger").await;
    (id, repo)
}

fn anchor_in(repo: &str) -> Anchor {
    Anchor {
        repo: repo.to_owned(),
        path: "README.md".to_owned(),
        side: Side::New,
        first: 1,
        last: 1,
        quoted: vec![repo.to_owned()],
        base: None,
        detached: false,
    }
}

async fn draft_in(
    comments: &Arc<dyn CommentsClient>,
    session: &SessionId,
    repo: &str,
    key: &str,
) -> Comment {
    within(
        "create",
        comments.create(session, anchor_in(repo), "Rename this", key),
    )
    .await
    .expect("create a comment")
}

pub async fn comment_is_listed_as_a_draft(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    assert_eq!(made.status, Status::Draft);
    let listing = within("list", comments.list(&session, Which::Open))
        .await
        .expect("list");
    assert_eq!(listing.comments.len(), 1);
    assert_eq!(listing.comments[0].id, made.id);
    assert_eq!(listing.comments[0].text, "Rename this");
    assert_eq!(listing.comments[0].anchor.path, "README.md");
    assert_eq!(listing.draft_count, 1);
}

pub async fn edit_with_a_stale_revision_is_refused(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    let edited = within("edit", comments.edit(&made.id, "Rename that", made.revision))
        .await
        .expect("edit at the current revision");
    assert_eq!(edited.text, "Rename that");
    assert!(edited.revision > made.revision);
    let stale = within("edit", comments.edit(&made.id, "Too late", made.revision)).await;
    assert!(
        matches!(stale, Err(DaemonError::Refused { .. })),
        "expected a refusal, got {stale:?}"
    );
}

pub async fn archived_comment_moves_to_the_archive(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    let archived = within("archive", comments.archive(&made.id))
        .await
        .expect("archive");
    assert!(archived.archived);
    let open = within("list", comments.list(&session, Which::Open))
        .await
        .expect("list open");
    assert!(open.comments.is_empty());
    assert_eq!(open.draft_count, 0);
    let kept = within("list", comments.list(&session, Which::Archived))
        .await
        .expect("list archived");
    assert_eq!(kept.comments.len(), 1);
    assert_eq!(kept.comments[0].id, made.id);
}

/// The next `Changed` for `id`; a `Resync` on the way is ignored (the
/// daemon opens its feed with one, the fake's feed has none).
async fn changed_comment(events: &mut CommentEvents, id: &humata2_clients::comments::CommentId) -> Comment {
    within("a comment event", async {
        loop {
            match events.recv().await {
                Some(CommentEvent::Changed(comment)) if &comment.id == id => return comment,
                Some(_) => {}
                None => panic!("the comment feed ended"),
            }
        }
    })
    .await
}

pub async fn watch_follows_a_new_comment(
    world: &dyn World,
    daemon: Arc<dyn Daemon>,
    comments: Arc<dyn CommentsClient>,
) {
    let (session, repo) = session_to_comment_on(world, &daemon).await;
    let mut events = within("watch", comments.watch(&session))
        .await
        .expect("watch");
    let made = draft_in(&comments, &session, &repo, "key-1").await;
    let heard = changed_comment(&mut events, &made.id).await;
    assert_eq!(heard.text, "Rename this");
    assert_eq!(heard.status, Status::Draft);
}
