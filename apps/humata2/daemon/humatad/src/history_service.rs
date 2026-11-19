//! HistoryService (history.proto; LD-8, LD-21 to LD-25): searching and
//! reading the conversations the host keeps, from `humata2_mirror`'s
//! copies under the state folder.
//!
//! The service keeps the mirror up to date from events, never by scanning
//! or polling: once at start it catches up every session of the store
//! that has an agent, and after that each Session event on the feed
//! catches that session up (its title, channel and machine are described
//! to the mirror first, so hits and filters name what is current). A
//! session that is gone from the store is forgotten (LD-22). While a
//! session's agent runs here, a change to its conversation file catches it
//! up too (`follow.rs`, N1), since an agent may write the end of a turn
//! after the session's last event.
//!
//! Sessions on other machines are mirrored by the host reading them there
//! (D189): `remote.rs` follows the machines' links and reports and, for each
//! machine in a task of its own, asks the machine's `ReadMirror` (`source.rs`)
//! for the next part of the conversation from the offset the mirror saved.
//!
//! The mirror is opened the first time it is needed, so one that can't be
//! used fails the calls that need it and never stops the daemon.
//!
//! Callers: the user, a script and the peer link search everything. An
//! agent searches and reads its own session's channel only; asking for
//! another channel is refused (LD-24).
//!
//! How failures are told (errors.proto):
//! - an empty query is `invalid_field` on `query`;
//! - a page token the mirror didn't give out is `invalid_field` on the
//!   page's `page_token`;
//! - anything else of the mirror is `internal`, with the chain of causes.

mod follow;
mod remote;
pub(crate) use humata2_rpc_kit::rfc3339;
mod source;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use humata2_grpc::humata::v1 as pb;
use humata2_mirror::{Mirror, MirrorError, PageAsk, Query, SessionMeta};
use humata2_store::{AgentState, SessionRecord, Store, StoreError};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc;
use tonic::{Code, Request, Response, Status};

use crate::channel_service::blocking;
use crate::compression::Compression;
use crate::events::{Feed, Scope};
use crate::guard::Guard;
use crate::host_relay::Links;
use crate::identity::{self, Caller};
use crate::peer_reports::PeerReports;
use crate::status;

/// The mirror and what a call needs to check its caller and to keep the
/// mirror current.
pub(crate) struct HistorySvc {
    shared: Arc<Shared>,
}

/// What the service and its feed follower share.
struct Shared {
    /// The `<state>` folder; the mirror lives under it.
    state: PathBuf,
    /// The mirror, once something has opened it. Held across the open, so
    /// two callers never open it at once: a second open would trim an
    /// entries log the first is appending to.
    mirror: Mutex<Option<Mirror>>,
    store: Arc<Store>,
    /// Where an index catch-up is told (D438).
    feed: Arc<Feed>,
    /// Reads of other machines' sessions (D189); set by `use_links` on the
    /// host, never on a machine's own daemon.
    remote: OnceLock<Arc<remote::Remote>>,
    /// The codec ReadMirror's answers use (TR-11).
    compression: Compression,
    /// The agents and the home their CLIs run in, so a conversation an
    /// agent wrote before its session named it (Codex) is learned here and
    /// mirrored with no resume first (B9).
    agents: humata2_sessions::Agents,
    agent_home: PathBuf,
    /// The conversation files of this machine's running agents (N1).
    follow: follow::Follow,
}

/// Why a session couldn't be caught up. Logged, never shown to a caller:
/// the next event for the session tries again.
#[derive(Debug, thiserror::Error)]
enum SyncError {
    #[error("couldn't read the session's record from the store; check that the state folder is readable and has free space")]
    Store(#[source] StoreError),
    #[error("couldn't update the session's mirror; the next change to the session tries again")]
    Mirror(#[source] MirrorError),
    #[error("the session's conversation wasn't copied, because this machine's host variable values couldn't be read to scrub it; the next change to the session tries again")]
    Scrub(#[source] source::ScrubberError),
}

impl HistorySvc {
    /// `state` is the `<state>` folder. Never fails and touches no disk. On
    /// a tokio runtime it catches up the store's sessions once and then
    /// follows the feed's Session events.
    pub(crate) fn new(
        state: PathBuf,
        _guard: Arc<Guard>,
        feed: Arc<Feed>,
        store: Arc<Store>,
        agents: humata2_sessions::Agents,
        agent_home: PathBuf,
    ) -> Self {
        let compression = Compression::new(&state);
        let (touch, touched) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            state,
            mirror: Mutex::new(None),
            store,
            feed: feed.clone(),
            remote: OnceLock::new(),
            compression,
            agents,
            agent_home,
            follow: follow::Follow::new(touch),
        });
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(follow_sessions(Arc::downgrade(&shared), feed, touched));
        }
        Self { shared }
    }

    /// The scrubbed lines the mirror kept of a session on another machine,
    /// as they came (D195), for rebuilding it; none when it keeps none.
    pub(crate) fn raw_lines(&self, session: &str) -> Result<Option<Vec<u8>>, MirrorError> {
        Ok(self
            .shared
            .mirror()?
            .raw_conversation(session)?
            .map(|raw| raw.data))
    }

    /// The session's conversation now lives in a new file on `machine`
    /// (`""`: this host) that starts with what the old one held; see
    /// [`humata2_mirror::Mirror::rebase_source`] (MV-13, D209).
    pub(crate) fn rebase_source(&self, session: &str, machine: &str, skip: Option<u64>) -> Result<(), MirrorError> {
        self.shared.mirror()?.rebase_source(session, machine, skip)
    }

    /// The machines' links and reports (the host only; a machine's own
    /// daemon has none and mirrors no other machine). From now on a link
    /// coming up, or a machine reporting on a session, has the host read that
    /// machine's conversations (D189). Both are subscribed to before this
    /// returns, so nothing falls between.
    pub(crate) fn use_links(&self, links: Links, reports: PeerReports) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let remote = Arc::new(remote::Remote::new(
            handle.clone(),
            Arc::downgrade(&self.shared),
            links.clone(),
        ));
        if self.shared.remote.set(remote.clone()).is_err() {
            return;
        }
        let follow = links.watch_all();
        let updates = reports.subscribe();
        handle.spawn(remote::dispatch(remote, follow, updates));
    }
}

impl Shared {
    /// The mirror, opened now if nothing has yet.
    fn mirror(&self) -> Result<Mirror, MirrorError> {
        let mut kept = self.mirror.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mirror) = kept.as_ref() {
            return Ok(mirror.clone());
        }
        let opened = Mirror::open(&self.state)?;
        *kept = Some(opened.clone());
        Ok(opened)
    }

    /// This host's machine id, which a session recorded with no machine
    /// runs on; empty when the host hasn't saved one.
    fn here(&self) -> String {
        std::fs::read_to_string(self.state.join("machine-id"))
            .map(|id| id.trim().to_owned())
            .unwrap_or_default()
    }

    /// Whether `record` is a session on another machine that this host reads
    /// there. Without links (a machine's own daemon) none is.
    fn is_remote(&self, record: &SessionRecord) -> bool {
        self.remote.get().is_some() && !record.machine.is_empty() && record.machine != self.here()
    }

    /// Names the session's channel, machine, agent and title to the mirror,
    /// so its hits and filters say what is current.
    fn describe(&self, record: &SessionRecord) -> Result<Mirror, SyncError> {
        let channel_name = self
            .store
            .channel(&record.channel)
            .map_err(SyncError::Store)?
            .map(|channel| channel.name)
            .unwrap_or_default();
        let archived = self.store.archive(&record.id).map_err(SyncError::Store)?.is_some();
        let machine = if record.machine.is_empty() {
            self.here()
        } else {
            record.machine.clone()
        };
        // SEAM: machine names live with MachineService, which is not built
        // yet; until then a hit's machine name is its id.
        let meta = SessionMeta {
            session: record.id.clone(),
            channel: record.channel.clone(),
            channel_name,
            title: record.title.clone(),
            machine_name: machine.clone(),
            machine,
            agent: record.agent.clone(),
            archived,
        };
        let mirror = self.mirror().map_err(SyncError::Mirror)?;
        mirror.describe(&meta).map_err(SyncError::Mirror)?;
        Ok(mirror)
    }

    /// Describes the session to the mirror and reads what its agent wrote
    /// since; forgets it when the store no longer has it.
    fn catch_up(&self, id: &str) -> Result<(), SyncError> {
        let Some(record) = self.store.session(id).map_err(SyncError::Store)? else {
            self.follow.unfollow(id);
            return self.mirror().and_then(|m| m.forget(id)).map_err(SyncError::Mirror);
        };
        if self.is_remote(&record) {
            self.follow.unfollow(id);
            // Another machine's conversation is read there (D189): name the
            // session to the mirror and hand it to that machine's task, so
            // this follower never waits on a machine.
            self.describe(&record)?;
            if let Some(remote) = self.remote.get() {
                remote.trigger(&record.machine, id);
            }
            return Ok(());
        }
        self.learn(id).map_err(SyncError::Store)?;
        let Some(agent) = self.store.agent(id).map_err(SyncError::Store)? else {
            // No agent has run: there is no conversation to copy yet.
            self.follow.unfollow(id);
            return Ok(());
        };
        let mirror = self.describe(&record)?;
        let file = agent.launch.conversation_file;
        // Followed before it is read, so a line written in between is seen.
        let runs = matches!(
            agent.state,
            AgentState::Launching | AgentState::Running { .. } | AgentState::Resuming
        );
        if runs && !file.as_os_str().is_empty() {
            if let Err(error) = self.follow.follow(id, &file) {
                crate::log::say!("humatad2: {}", status::chain(&error));
            }
        } else {
            self.follow.unfollow(id);
        }
        if file.as_os_str().is_empty() || !file.exists() {
            return Ok(());
        }
        // LD-21: the host's own variable values, read now; when they can't be
        // read nothing is copied (D189.3).
        let scrubber = source::scrubber().map_err(SyncError::Scrub)?;
        let added = mirror
            .catch_up(id, &file, &scrubber)
            .map_err(SyncError::Mirror)?;
        self.indexed(id, &record.channel, added);
        Ok(())
    }

    /// Saves the conversation session `id`'s agent wrote, when the session
    /// doesn't name it yet and it is there now (Codex). Blocking.
    fn learn(&self, id: &str) -> Result<(), StoreError> {
        self.agents
            .learn_conversation(&self.store, &self.agent_home, id)
            .map(|_| ())
    }

    /// Tells whoever may read session `id` of channel `channel` that the
    /// index has `added` more of its entries, so a search that missed them
    /// can run again (D438). Adding none tells no one.
    fn indexed(&self, id: &str, channel: &str, added: usize) {
        if added == 0 {
            return;
        }
        self.feed.publish_in(
            pb::Change::Updated,
            pb::event::Payload::HistoryIndexed(pb::HistoryIndexed {
                session: Some(pb::SessionId {
                    value: id.to_owned(),
                }),
                added: u64::try_from(added).unwrap_or(u64::MAX),
            }),
            Scope {
                channel: channel.to_owned(),
                session: id.to_owned(),
            },
            None,
        );
    }
}

/// Catches every session of the store up once, then follows the feed's
/// Session events and the running agents' conversation files while the
/// service is there.
async fn follow_sessions(
    shared: Weak<Shared>,
    feed: Arc<Feed>,
    mut touched: mpsc::UnboundedReceiver<follow::Touch>,
) {
    // Subscribe before the first pass so no event falls between the two.
    let mut live = feed.subscribe(None).1;
    if let Some(start) = shared.upgrade() {
        let ids = blocking(move || -> Result<Vec<String>, StoreError> {
            Ok(start
                .store
                .sessions()?
                .into_iter()
                .map(|session| session.id)
                .collect())
        })
        .await;
        match ids {
            Ok(Ok(ids)) => {
                for id in ids {
                    catch_up_logged(&shared, id).await;
                }
            }
            Ok(Err(error)) => {
                crate::log::say!(
                    "humatad2: couldn't list sessions to mirror their history: {}",
                    status::chain(&error)
                );
            }
            Err(error) => crate::log::say!("humatad2: couldn't mirror session history: {error}"),
        }
    }
    loop {
        let received = tokio::select! {
            received = live.recv() => received,
            Some(touch) = touched.recv() => {
                caught_up_touched(&shared, touch, &mut touched).await;
                continue;
            }
        };
        let published = match received {
            Ok(published) => published,
            // Events missed while behind: the next ones still count.
            Err(RecvError::Lagged(_)) => continue,
            Err(RecvError::Closed) => return,
        };
        let Some(pb::event::Payload::Session(session)) = &published.payload else {
            continue;
        };
        let Some(id) = session.id.as_ref().map(|id| id.value.clone()) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        if shared.strong_count() == 0 {
            return;
        }
        catch_up_logged(&shared, id).await;
    }
}

/// How long a burst of writes to conversation files is let to finish
/// before the sessions are caught up, once.
const TOUCH_SETTLE: std::time::Duration = std::time::Duration::from_millis(50);

/// Catches up the sessions whose files `first` and the touches that follow
/// it within [`TOUCH_SETTLE`] changed, each once.
async fn caught_up_touched(
    shared: &Weak<Shared>,
    first: follow::Touch,
    touched: &mut mpsc::UnboundedReceiver<follow::Touch>,
) {
    tokio::time::sleep(TOUCH_SETTLE).await;
    let Some(held) = shared.upgrade() else {
        return;
    };
    let mut sessions: Vec<String> = held.follow.touched(&first);
    while let Ok(touch) = touched.try_recv() {
        sessions.extend(held.follow.touched(&touch));
    }
    drop(held);
    sessions.sort();
    sessions.dedup();
    for id in sessions {
        catch_up_logged(shared, id).await;
    }
}

async fn catch_up_logged(shared: &Weak<Shared>, id: String) {
    let Some(shared) = shared.upgrade() else {
        return;
    };
    let named = id.clone();
    match blocking(move || shared.catch_up(&id)).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => crate::log::say!(
            "humatad2: couldn't keep session {named}'s history: {}",
            status::chain(&error)
        ),
        Err(error) => crate::log::say!("humatad2: couldn't keep session {named}'s history: {error}"),
    }
}

/// The channel an agent is limited to, or `None` for a caller that may
/// search everything.
fn limited_to(caller: &Caller) -> Option<String> {
    match caller {
        Caller::Agent(agent) => Some(agent.session.channel.to_string()),
        Caller::User(_) | Caller::Script(_) | Caller::Peer => None,
    }
}

/// The channels a search covers: what was asked, or the agent's own.
fn channels_of(asked: Option<&pb::ChannelId>, limit: Option<&str>) -> Result<Vec<String>, Status> {
    let asked = asked.map(|id| id.value.as_str()).filter(|id| !id.is_empty());
    match (asked, limit) {
        (Some(asked), Some(own)) if asked != own => Err(other_channel(asked)),
        (Some(asked), _) => Ok(vec![asked.to_owned()]),
        (None, Some(own)) => Ok(vec![own.to_owned()]),
        (None, None) => Ok(Vec::new()),
    }
}

/// An agent asked about a channel that isn't its own.
fn other_channel(channel: &str) -> Status {
    // SEAM: SESSIONS_SEARCH_OTHER, "Search other channels" (Ask): when the
    // capability exists, this is where an agent asks for it (needs_approval)
    // and the call goes on once the user allows it.
    status::failure(
        Code::PermissionDenied,
        status::error(
            format!(
                "an agent can search and read only its own channel's sessions, and {channel} is another channel"
            ),
            "search without naming a channel to cover your own, or ask the user to search other channels from the app or with `humata sessions search`",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

pub(crate) use humata2_rpc_kit::rfc3339::timestamp;

/// The scrubber for this machine's host variable values, for a read that
/// leaves the daemon; it fails rather than read unscrubbed (D189.3).
pub(crate) fn scrubber() -> Result<humata2_mirror::Scrubber, Status> {
    source::scrubber().map_err(|error| status::internal(status::chain(&error), ""))
}

fn page_ask(page: Option<&pb::PageRequest>) -> PageAsk {
    PageAsk {
        size: page.map_or(0, |page| page.page_size),
        token: page.map(|page| page.page_token.clone()).unwrap_or_default(),
    }
}

fn page_response(next_token: String) -> Option<pb::PageResponse> {
    Some(pb::PageResponse {
        next_page_token: next_token,
    })
}

fn hit_message(hit: humata2_mirror::Hit) -> pb::SearchHit {
    pb::SearchHit {
        session: Some(pb::SessionId { value: hit.session }),
        session_title: hit.title,
        channel: Some(pb::ChannelId { value: hit.channel }),
        channel_name: hit.channel_name,
        machine: Some(pb::MachineId { value: hit.machine }),
        machine_name: hit.machine_name,
        agent: hit.agent,
        turn: hit.turn,
        seq: hit.seq,
        snippet: hit.snippet,
        at: hit.at.as_deref().and_then(timestamp),
        as_of: timestamp(&hit.as_of),
    }
}

fn entry_message(entry: humata2_mirror::MirrorEntry) -> pb::MirrorEntry {
    use humata2_mirror::Role;
    use pb::mirror_entry::Role as Wire;
    let role = match entry.role {
        Role::User => Wire::User,
        Role::Agent => Wire::Agent,
        Role::Tool => Wire::Tool,
    };
    pb::MirrorEntry {
        seq: entry.seq,
        turn: entry.turn,
        role: role as i32,
        text: entry.text,
        at: entry.at.as_deref().and_then(timestamp),
    }
}

/// A failure of the mirror, as a status.
fn failed(error: &MirrorError) -> Status {
    match error {
        MirrorError::EmptyQuery => status::invalid_field(
            "query",
            pb::invalid_field::Problem::Missing,
            error.to_string(),
            "give at least one word to look for",
        ),
        MirrorError::BadPageToken => status::invalid_field(
            "page.page_token",
            pb::invalid_field::Problem::BadFormat,
            error.to_string(),
            "ask again without a page token to start from the first page",
        ),
        MirrorError::Io { path, .. }
        | MirrorError::Database { path, .. }
        | MirrorError::Corrupt { path, .. } => {
            status::internal(status::chain(error), &path.display().to_string())
        }
        MirrorError::Request { .. } => status::internal(status::chain(error), ""),
        MirrorError::BackfillPast { .. } => status::internal(status::chain(error), ""),
    }
}

#[tonic::async_trait]
impl pb::history_service_server::HistoryService for HistorySvc {
    /// LD-23, LD-24: every word must match; an agent searches its own
    /// channel only.
    async fn search_sessions(
        &self,
        request: Request<pb::SearchSessionsRequest>,
    ) -> Result<Response<pb::SearchSessionsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let channels = channels_of(request.channel.as_ref(), limited_to(&caller).as_deref())?;
        let query = Query {
            text: request.query,
            channels,
            machine: request
                .machine
                .map(|machine| machine.value)
                .filter(|machine| !machine.is_empty()),
            agent: Some(request.agent).filter(|agent| !agent.is_empty()),
            since: request.since.as_ref().map(rfc3339::format),
            until: request.until.as_ref().map(rfc3339::format),
            page: page_ask(request.page.as_ref()),
        };
        let shared = self.shared.clone();
        let page = blocking(move || shared.mirror()?.search(&query))
            .await?
            .map_err(|error| failed(&error))?;
        Ok(Response::new(pb::SearchSessionsResponse {
            hits: page.items.into_iter().map(hit_message).collect(),
            page: page_response(page.next_token),
        }))
    }

    /// LD-25: a session's turns, paged. An agent reads its own channel's
    /// sessions only; a session the mirror doesn't know has no entries.
    async fn read_turns(
        &self,
        request: Request<pb::ReadTurnsRequest>,
    ) -> Result<Response<pb::ReadTurnsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let session = request
            .session
            .map(|session| session.value)
            .filter(|session| !session.is_empty())
            .ok_or_else(|| status::missing("session"))?;
        let limit = limited_to(&caller);
        let page = page_ask(request.page.as_ref());
        let shared = self.shared.clone();
        let read = blocking(move || -> Result<Result<_, Status>, MirrorError> {
            if let Some(own) = &limit {
                match shared.store.session(&session) {
                    Ok(Some(record)) if &record.channel == own => {}
                    Ok(Some(record)) => return Ok(Err(other_channel(&record.channel))),
                    // Unknown to the store: nothing of it is shown to an agent.
                    Ok(None) | Err(_) => return Ok(Ok(humata2_mirror::Page {
                        items: Vec::new(),
                        next_token: String::new(),
                    })),
                }
            }
            shared
                .mirror()?
                .read(&session, request.first_turn, request.last_turn, &page)
                .map(Ok)
        })
        .await?
        .map_err(|error| failed(&error))??;
        Ok(Response::new(pb::ReadTurnsResponse {
            entries: read.items.into_iter().map(entry_message).collect(),
            page: page_response(read.next_token),
        }))
    }

    /// LD-8, D189: the next part of a session's conversation for the host's
    /// mirror; the host only.
    async fn read_mirror(
        &self,
        request: Request<pb::ReadMirrorRequest>,
    ) -> Result<Response<pb::ReadMirrorResponse>, Status> {
        source::read_mirror(&self.shared, request).await.map(Response::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Codex session saved before its CLI wrote its rollout names the
    /// rollout once the history catches the session up, with no resume.
    #[test]
    fn catching_up_a_codex_session_learns_the_conversation_its_cli_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let state = dir.path().join("state");
        let folder = home.join("Humata/desk/sessions/se_codex");
        std::fs::create_dir_all(&folder).unwrap();
        let store = Arc::new(Store::open(state.join("store")).unwrap());
        let agents = humata2_sessions::Agents::builtin();
        let plan = agents.get("codex").unwrap().launch(&home, &folder, "placeholder-id");
        store
            .put_session(&SessionRecord {
                id: "se_codex".to_owned(),
                channel: "desk".to_owned(),
                machine: String::new(),
                title: "codex".to_owned(),
                agent: "codex".to_owned(),
                folder: folder.clone(),
                created_unix_ms: 1,
            })
            .unwrap();
        store
            .put_agent(&humata2_store::AgentRecord {
                session: "se_codex".to_owned(),
                tab: "se_codex_tab".to_owned(),
                launch: humata2_store::AgentLaunch {
                    agent: "codex".into(),
                    folder: folder.clone(),
                    argv: plan.argv,
                    resume_argv: plan.resume_argv,
                    variable_names: Vec::new(),
                    conversation_id: "placeholder-id".into(),
                    conversation_file: plan.conversation_file,
                    holder_socket: PathBuf::from("/nowhere.sock"),
                    cols: 80,
                    rows: 24,
                    cli_version: None,
                },
                state: humata2_store::AgentState::Running { holder_pid: 7 },
                generation: 1,
                interruptions: Vec::new(),
                updated_unix_ms: 1,
            })
            .unwrap();
        let day = home.join(".codex/sessions/2026/11/16");
        std::fs::create_dir_all(&day).unwrap();
        let rollout = day.join("rollout-2026-11-16T10-00-00-019a-real.jsonl");
        let meta = serde_json::json!({
            "timestamp": "2026-11-16T10:00:00.000Z",
            "type": "session_meta",
            "payload": {"id": "019a-real", "cwd": folder, "cli_version": "0.169.0"},
        });
        std::fs::write(&rollout, format!("{meta}\n")).unwrap();
        let history = HistorySvc::new(
            state,
            Arc::new(Guard::new("m1", pb::DaemonRole::Host)),
            Arc::new(Feed::default()),
            store.clone(),
            agents,
            home,
        );

        // The mirror's copy may still fail on this machine's host variables;
        // the conversation is learned before that.
        let _ = history.shared.catch_up("se_codex");

        let saved = store.agent("se_codex").unwrap().unwrap();
        assert_eq!(saved.launch.conversation_id, "019a-real");
        assert_eq!(saved.launch.conversation_file, rollout);
    }

    #[test]
    fn an_agent_with_no_channel_asked_gets_its_own() {
        assert_eq!(channels_of(None, Some("ch_a")).ok(), Some(vec!["ch_a".to_owned()]));
    }

    #[test]
    fn an_agent_asking_for_another_channel_is_refused() {
        let other = pb::ChannelId { value: "ch_b".to_owned() };
        let refused = channels_of(Some(&other), Some("ch_a")).expect_err("another channel");
        assert_eq!(refused.code(), Code::PermissionDenied);
    }

    #[test]
    fn the_user_may_name_any_channel_or_none() {
        let any = pb::ChannelId { value: "ch_b".to_owned() };
        assert_eq!(channels_of(Some(&any), None).ok(), Some(vec!["ch_b".to_owned()]));
        assert_eq!(channels_of(None, None).ok(), Some(Vec::new()));
    }
}
