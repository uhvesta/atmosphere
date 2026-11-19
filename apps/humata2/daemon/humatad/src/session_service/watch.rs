//! Hears an agent's terminal exit while the daemon runs, records it and
//! tells the feed, so the app sees "Exited" without asking (T5b). Without
//! this the store keeps a gone agent as running until the next start's
//! reconcile.
//!
//! One connection per running agent, opened at the end of its output. The
//! output it prints is not kept, but it times the output fallback for the
//! agent's turn (SE-21, D169): a timer is armed at attach and re-armed at
//! each output, and when it fires the agent's status is told to the feed
//! (see `turns.rs`). A watch ends when the holder says the agent exited or
//! hangs up; a holder that hangs up without an exit is left to the next
//! start's reconcile.

use std::path::{Path, PathBuf};

use humata2_holder::{Attachment, Event, Events};
use humata2_store::{AgentState, StoreError};
use tokio::sync::mpsc;
use tokio::task::{JoinHandle, spawn_blocking};
use tokio::time::Instant;

use super::turns::{self, Hook, Quieting};
use super::{Tell, now_ms};
use crate::status;

/// Events waiting for the watch to read them.
const EVENTS_QUEUE: usize = 64;

/// Where a running agent is, for its watch.
struct Running {
    holder_pid: u32,
    socket: PathBuf,
    /// What its hooks last said and when, kept across a restart.
    hook: Option<(Hook, i64)>,
    /// Whether its kind has turns to time (SE-21).
    timed: bool,
}

/// Watches `session`'s agent if its record says it runs. Returns at once.
pub(crate) fn agent(tell: Tell, session: String) {
    drop(agent_attached(tell, session));
}

/// As [`agent`], and the returned receiver is told once the watch is
/// attached to the agent's holder (or won't be). A start waits for it
/// before it says the agent runs: output after that is then always heard,
/// so the agent's first answer marks its session (CH-16), however slow the
/// watch's own attach is.
pub(crate) fn agent_attached(tell: Tell, session: String) -> tokio::sync::oneshot::Receiver<()> {
    let (ready, attached) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        // `ready` dropped on a return before the attach lets the waiter go on.
        let Some(Running {
            holder_pid,
            socket,
            hook,
            timed,
        }) = running(&tell, &session).await
        else {
            return;
        };
        let entered = turns::enter(&session, holder_pid, hook, timed);
        turns::save_pending(tell.store.clone(), &session, holder_pid, entered.pending).await;
        if entered.inserted {
            tell.session(&session, humata2_grpc::humata::v1::Change::Updated, None)
                .await;
        }
        let code = follow(&tell, &session, holder_pid, &socket, timed, ready).await;
        // D767: a question its screen asked goes with the agent.
        crate::screen_ask::gone(&session);
        // A watch that lost its holder keeps what it knew: the next one
        // finds it, and a report in between still has an entry to change.
        let Some(code) = code else {
            // D784: a holder that is gone for good (killed, not replaced by
            // an upgrade) leaves its agent showing as ended, so it reads as
            // one to resume rather than a running row that can't attach.
            // The record stays running, so a start brings it back as
            // before; a Resume finds no holder and starts it again.
            if holder_gone(holder_pid, &socket).await {
                mark_lost(&session, holder_pid);
                tell.session(&session, humata2_grpc::humata::v1::Change::Updated, None)
                    .await;
            }
            return;
        };
        turns::forget(&session, holder_pid);
        if record_exit(&tell, &session, holder_pid, code).await {
            tell.session(&session, humata2_grpc::humata::v1::Change::Updated, None)
                .await;
        }
    });
    attached
}

/// After the start's reconcile: tells the feed of every session as it now
/// stands, since the app may have read it before it was brought back, and
/// watches each agent that runs.
pub(crate) async fn after_restart(tell: Tell) {
    // The restart report was set before this runs. A start with no agent
    // to tell of publishes nothing else, so the app would never read the
    // report; Resync is the existing cue that its lists may be stale. It
    // goes to those subscribed now, unnumbered and not kept, so a client
    // resuming later doesn't replay a stale one (D341).
    tell.feed.resync_all();
    // A start fresh the stop cut short types its first message now.
    tokio::spawn(super::start_fresh::retype_pending(tell.store.clone()));
    let store = tell.store.clone();
    let agents = match spawn_blocking(move || store.agents()).await {
        Ok(Ok(agents)) => agents,
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: couldn't read the agents after bringing them back, so the app hears of their state only when it reloads: {}",
                status::chain(&error)
            );
            return;
        }
        Err(_) => return,
    };
    for agent in agents {
        tell.session(
            &agent.session,
            humata2_grpc::humata::v1::Change::Updated,
            None,
        )
        .await;
        if matches!(agent.state, AgentState::Running { .. }) {
            self::agent(tell.clone(), agent.session);
        }
    }
}

/// The running agent's holder pid, its tab's holder socket and its saved
/// hook state.
async fn running(tell: &Tell, session: &str) -> Option<Running> {
    let (store, key) = (tell.store.clone(), session.to_owned());
    let read = spawn_blocking(move || -> Result<Option<Running>, StoreError> {
        let Some(agent) = store.agent(&key)? else {
            return Ok(None);
        };
        let AgentState::Running { holder_pid } = agent.state else {
            return Ok(None);
        };
        let hook = store
            .hook_turn(&key)?
            .and_then(|(code, at)| Some((Hook::from_code(code)?, at)));
        let timed = store
            .session(&key)?
            .is_some_and(|record| {
                turns::source_of(&record.agent) != humata2_grpc::humata::v1::TurnEndSource::None
            });
        Ok(store.tab(&agent.tab)?.map(|tab| Running {
            holder_pid,
            socket: tab.holder_socket,
            hook,
            timed,
        }))
    })
    .await;
    match read {
        Ok(Ok(found)) => found,
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: couldn't read session {session}'s agent to watch for its exit, so the app hears of it only when it reloads: {}",
                status::chain(&error)
            );
            None
        }
        Err(_) => None,
    }
}

/// Reads the holder's events into `sender`, in a task of their own: a read
/// can't be cancelled midway, so the watch can't select on it directly.
fn read_events(
    mut events: Events,
    sender: mpsc::Sender<Result<Event, humata2_holder::HolderError>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let next = events.next().await.transpose();
            let failed = matches!(next, Some(Err(_)));
            let Some(next) = next else { return };
            if sender.send(next).await.is_err() || failed {
                return;
            }
        }
    })
}

/// Ends the reader task with the watch.
struct Reader(JoinHandle<()>);

impl Drop for Reader {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Follows the agent's output until its holder says it exited, timing the
/// quiet after each output for its turn status (told to the feed when it
/// changes). Returns the exit code; `None` when the holder can't be reached
/// or hangs up first.
async fn follow(
    tell: &Tell,
    session: &str,
    holder_pid: u32,
    socket: &Path,
    timed: bool,
    ready: tokio::sync::oneshot::Sender<()>,
) -> Option<i32> {
    let attached = humata2_holder::attach(socket, u64::MAX).await;
    let _ = ready.send(());
    let Attachment {
        welcome,
        events,
        control: _control,
    } = attached.ok()?;
    if let Some(code) = welcome.exited {
        return Some(code);
    }
    let (sender, mut received) = mpsc::channel(EVENTS_QUEUE);
    let _reader = Reader(read_events(events, sender));
    let mut quieting = Quieting::new(Instant::now(), timed);
    // A working report restarts the quiet clock (D733).
    let mut changes = turns::changes();
    let mut armed = turns::armed(session, holder_pid);
    // The words the agent printed lately, to tell a repaint from new text.
    let mut printed = crate::read_marks::Printed::default();
    loop {
        let level = tokio::select! {
            event = received.recv() => match event {
                Some(Ok(Event::Exited { code })) => return Some(code),
                Some(Ok(Event::Output { data, .. })) => {
                    // CH-16 (D392): output no client views marks the session
                    // unread; the feed hears of it from `read_marks`. A
                    // repaint after our own resize is not output (D393).
                    // D767 amendment: a screen question's card follows
                    // the screen while the agent goes on writing.
                    crate::screen_ask::output(session, holder_pid);
                    let shows_new_text = printed.hear(&data);
                    if crate::read_marks::would_mark(session, shows_new_text) {
                        let id = session.to_owned();
                        let _ = spawn_blocking(move || crate::read_marks::heard_output(&id)).await;
                    }
                    match quieting.output(Instant::now()) {
                        Some(level) => level,
                        None => continue,
                    }
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => return None,
            },
            level = quieting.next_level() => level,
            Ok(()) = changes.changed() => {
                let now = turns::armed(session, holder_pid);
                if now != armed {
                    armed = now;
                    quieting.rearm(Instant::now());
                }
                continue;
            }
        };
        // D767: a quiet agent may be asking on its screen, where no hook
        // fires, such as Copilot's folder trust.
        if level == turns::Quiet::Quiet {
            crate::screen_ask::quiet(session, holder_pid);
        }
        if turns::set_quiet(session, holder_pid, level) {
            tell.session(session, humata2_grpc::humata::v1::Change::Updated, None)
                .await;
        }
    }
}

/// Agents whose holder went away without an exit while this daemon runs,
/// by session and the holder pid their record runs under (D784).
static LOST: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<(String, u32)>>> =
    std::sync::LazyLock::new(Default::default);

fn mark_lost(session: &str, holder_pid: u32) {
    LOST.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert((session.to_owned(), holder_pid));
}

/// Whether `session`'s agent, running under `holder_pid`, lost its holder
/// while this daemon runs: it shows as ended (D784).
pub(crate) fn lost(session: &str, holder_pid: u32) -> bool {
    LOST.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&(session.to_owned(), holder_pid))
}

/// How long a holder that hung up gets for its socket to answer again, as
/// one that replaced it in an upgrade does.
const GONE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Whether the holder that hung up is gone: its process has ended and
/// nothing answers on its socket within [`GONE_GRACE`].
async fn holder_gone(holder_pid: u32, socket: &Path) -> bool {
    let deadline = Instant::now() + GONE_GRACE;
    loop {
        if tokio::net::UnixStream::connect(socket).await.is_ok() {
            return false;
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    match humata2_holder::process::Snapshot::take().await {
        Ok(snapshot) => !snapshot.is_alive(holder_pid),
        // Without the process list it may still run: leave it as it was.
        Err(_) => false,
    }
}

/// Saves the agent as exited, unless it was stopped or started again since
/// (its record no longer runs under `holder_pid`). Says whether it saved.
async fn record_exit(tell: &Tell, session: &str, holder_pid: u32, code: i32) -> bool {
    let (store, key) = (tell.store.clone(), session.to_owned());
    let saved = spawn_blocking(move || -> Result<bool, StoreError> {
        let Some(mut agent) = store.agent(&key)? else {
            return Ok(false);
        };
        if agent.state != (AgentState::Running { holder_pid }) {
            return Ok(false);
        }
        agent.state = AgentState::Exited { code };
        agent.updated_unix_ms = now_ms();
        store.put_agent(&agent)?;
        Ok(true)
    })
    .await;
    match saved {
        Ok(Ok(saved)) => saved,
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: session {session}'s agent exited with {code}, but saving that failed, so it shows as running until humatad2 restarts: {}",
                status::chain(&error)
            );
            false
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::events::Feed;

    /// A start with nothing to resume still cues a subscriber, so the app
    /// reads the restart report the start set.
    #[tokio::test]
    async fn a_restart_with_no_agents_still_tells_the_feed() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("a temp folder: {e}"));
        let store = humata2_store::Store::open(dir.path()).unwrap_or_else(|e| panic!("a store: {e}"));
        let feed = Arc::new(Feed::default());
        let (_, mut live) = feed.subscribe(None);
        after_restart(Tell::new(feed, Arc::new(store), "this-mac".to_owned())).await;
        assert!(live.try_recv().is_ok(), "a restart with no agents told nobody");
    }
}
