//! Launching an agent, and resuming one whose holder is gone.

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use humata2_holder::{Event, HolderSpec, Spawned, Welcome};
use humata2_store::fault::point;
use humata2_store::{
    AgentLaunch, AgentRecord, AgentState, Interruption, ResumeFailure, SessionRecord, Store,
    StoreError, TabKind, TabRecord, TabReplies, Told,
};
use tokio::time::Instant;

use crate::reconcile::Revived;
#[path = "rekey.rs"]
pub(crate) mod rekey;
use crate::watch::FileWatch;
use crate::{Env, GateRefusal, ReviveConfig, ReviveError};

/// Told to an agent once it has resumed (PER-14).
const INTERRUPTED: &str = "Humata: this session was interrupted (its machine restarted or its terminal was killed) and you were resumed. Run `git status` and check your work before continuing.";
/// How long a holder asked to stop gets before it's killed.
const STOP_GRACE: Duration = Duration::from_secs(5);
/// Between typing the note and pressing return, so an agent that reads a
/// paste whole doesn't take the return as part of it. D671 (Provisional):
/// 400ms, up from 100ms: Ink-style terminals (Copilot's) took a return 100ms
/// after the text as part of the paste. The return is its own write.
const TYPE_PAUSE: Duration = Duration::from_millis(400);
/// How often a note the gate holds back looks again (D763).
const GATE_EVERY: Duration = Duration::from_millis(500);

/// Why an agent is resumed: after an interruption, because the person
/// asked for it after stopping it, or because the person moved it (its
/// folder changed). The last two are no interruption.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Why<'a> {
    Interrupted,
    /// D671 (Provisional): the person resumed an agent they had stopped, or
    /// one that exited. Recorded as no interruption, told nothing, and
    /// checked only for coming up, staying up and starting no other
    /// conversation: an agent that writes nothing until it is prompted
    /// resumes cleanly.
    Asked,
    /// Resumed with this line shown first in its terminal, not recorded as an
    /// interruption, and not told it was interrupted.
    Restarted(&'a str),
    /// As [`Why::Restarted`], after a move that carried every file the agent
    /// wrote and checked them here: a conversation file missing after it is
    /// one the agent never wrote, so it starts fresh (D757).
    Moved(&'a str),
}

/// The sessions whose agent this daemon is launching, or relaunching from a
/// launching record, right now. A record saved as launching with no entry
/// here was left by a daemon that stopped, so nothing is starting it.
static LAUNCHING_NOW: std::sync::Mutex<std::collections::BTreeSet<String>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Whether this daemon is launching `session`'s agent right now.
pub fn launch_in_flight(session: &str) -> bool {
    LAUNCHING_NOW
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(session)
}

/// Marks a session's launch as in flight until dropped, however it ends.
pub(crate) struct LaunchInFlight(String);

impl LaunchInFlight {
    pub(crate) fn enter(session: &str) -> Self {
        LAUNCHING_NOW
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(session.to_owned());
        Self(session.to_owned())
    }
}

impl Drop for LaunchInFlight {
    fn drop(&mut self) {
        LAUNCHING_NOW
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.0);
    }
}

#[cfg(test)]
mod launch_flight_tests {
    use super::{LaunchInFlight, launch_in_flight};

    #[test]
    fn a_launch_is_in_flight_from_its_guard_until_the_guard_drops() {
        assert!(!launch_in_flight("se_flight"));
        let guard = LaunchInFlight::enter("se_flight");
        assert!(launch_in_flight("se_flight"));
        assert!(!launch_in_flight("se_other"), "only the session the guard names");
        drop(guard);
        assert!(!launch_in_flight("se_flight"));
    }
}

/// Saves `session`'s agent as launching, and fsyncs it (PER-11), then
/// starts it in a holder of its own and saves it as running with its tab.
/// `launch.argv` pins the conversation id (PER-12). A crash at any point
/// leaves a record [`crate::reconcile`] brings back. `line`, if given, is
/// shown in the terminal before the agent's first output.
pub async fn launch_agent(
    store: &Arc<Store>,
    config: &ReviveConfig,
    session: &SessionRecord,
    tab: &str,
    launch: AgentLaunch,
    line: Option<&str>,
    env: Env<'_>,
) -> Result<AgentRecord, ReviveError> {
    let _in_flight = LaunchInFlight::enter(&session.id);
    let mut record = AgentRecord {
        session: session.id.clone(),
        tab: tab.to_owned(),
        launch,
        state: AgentState::Launching,
        generation: 1,
        interruptions: Vec::new(),
        updated_unix_ms: unix_ms(),
    };
    save(store, &record, "save the agent before it starts").await?;
    point("revive.launch.recorded");

    let created = unix_ms();
    let variables = env(&agent_tab(&record, 0, created));
    let argv = record.launch.argv.clone();
    let spawned = match start_holder(config, &record, &argv, variables, line).await {
        Ok(spawned) => spawned,
        Err(error) => {
            record.state = AgentState::Stopped;
            let _ = save(store, &record, "save that the agent didn't start").await;
            return Err(error);
        }
    };
    point("revive.launch.spawned");
    running(store, record, spawned, created).await
}

/// Saves `record` as running in `spawned` with its tab. If that can't be
/// saved the holder is stopped, keeping its saved output: nothing could
/// find it again, and the record still says it was starting or resuming,
/// so the next start redoes that with the output so far.
pub(crate) async fn running(
    store: &Arc<Store>,
    mut record: AgentRecord,
    spawned: Spawned,
    created: i64,
) -> Result<AgentRecord, ReviveError> {
    record.state = AgentState::Running {
        holder_pid: spawned.pid,
    };
    record.updated_unix_ms = unix_ms();
    let mut tab = agent_tab(&record, spawned.pid, created);
    let (generation, new_holder) = match holder_generation(store, &record, spawned.pid).await {
        Ok(found) => found,
        Err(error) => {
            // The same as a save that fails: nothing could find the holder.
            stop_holder(&tab.holder_socket, Some(spawned.pid)).await;
            return Err(error);
        }
    };
    tab.holder_generation = generation;
    // A holder new to the tab has answered nothing: its record starts empty,
    // in the same commit as the tab, so no link finds the tab without it.
    let fresh = new_holder.then(|| fresh_replies(generation));
    let (saving, saved_tab) = (record.clone(), tab.clone());
    let saved = blocking({
        let store = store.clone();
        move || store.put_agent_and_tab_with_replies(&saving, &saved_tab, fresh.as_ref())
    })
    .await;
    let saved = match saved {
        Ok(saved) => saved,
        Err(error) => {
            stop_holder(&tab.holder_socket, Some(spawned.pid)).await;
            return Err(error);
        }
    };
    if let Err(source) = saved {
        stop_holder(&tab.holder_socket, Some(spawned.pid)).await;
        return Err(store_error(&record.session, "save the running agent and its tab", source));
    }
    Ok(record)
}

/// Points `record`'s tab at the holder `spawned`, with a new generation and
/// an empty reply record when it named another, in one commit; the agent
/// record is left as it is. [`running`] then finds the tab already naming
/// it and keeps its generation.
async fn hand_tab_to(
    store: &Arc<Store>,
    record: &AgentRecord,
    spawned: &Spawned,
    created: i64,
) -> Result<(), ReviveError> {
    let (generation, new_holder) = holder_generation(store, record, spawned.pid).await?;
    if !new_holder {
        return Ok(());
    }
    let mut tab = agent_tab(record, spawned.pid, created);
    tab.holder_generation = generation;
    let fresh = fresh_replies(generation);
    blocking({
        let store = store.clone();
        move || store.put_tab_with_replies(&tab, &fresh)
    })
    .await?
    .map_err(|source| store_error(&record.session, "point the agent's tab at its new holder", source))
}

/// The generation the agent's tab has for the holder `pid`, and whether
/// that holder is new to the tab: the saved tab's own when it already names
/// `pid`, one more when it names another process (a new holder, so clients
/// see a new terminal), and 0 for a tab never saved.
async fn holder_generation(
    store: &Arc<Store>,
    record: &AgentRecord,
    pid: u32,
) -> Result<(u64, bool), ReviveError> {
    let (reader, id) = (store.clone(), record.tab.clone());
    let saved = blocking(move || reader.tab(&id))
        .await?
        .map_err(|source| store_error(&record.session, "read the agent's tab", source))?;
    Ok(match saved {
        Some(tab) if tab.holder_pid == pid => (tab.holder_generation, false),
        Some(tab) => (tab.holder_generation.saturating_add(1), true),
        None => (0, true),
    })
}

/// The reply record of a holder that has answered nothing yet, so its first
/// queries are answered from the start of its output.
pub(crate) fn fresh_replies(generation: u64) -> TabReplies {
    TabReplies {
        generation,
        answered_through: 0,
        pending: Vec::new(),
        uncertain: false,
    }
}

/// Resumes `record`'s agent with its pinned conversation in a new holder
/// on the same socket, which starts with the old one's saved output, and
/// checks it took the conversation up (PER-13, PER-14). Anything short of
/// that is a failed resume and nothing is left running (PER-15).
///
/// A daemon stopped part way through is done again when it starts (PER-27):
/// the record says it was resuming, so the next reconcile stops whatever
/// holder this one left and resumes once more. The agent is told it was
/// interrupted once: the record says when it was about to be told, and a
/// redone resume whose conversation already holds the note since then
/// doesn't tell it again.
pub(crate) async fn resume(
    store: &Arc<Store>,
    config: &ReviveConfig,
    mut record: AgentRecord,
    created: i64,
    env: Env<'_>,
    why: Why<'_>,
) -> Result<Revived, ReviveError> {
    let socket = record.launch.holder_socket.clone();
    // D801 D: a channel renamed under this agent while it ran. Its
    // conversation is carried to the new folder's key and its launch points
    // at the new folder before anything below looks for either.
    let rekeyed = match rekey::apply(config, &mut record).await {
        Ok(done) => done,
        Err(error) => {
            let mut chain = error.to_string();
            let mut cause = std::error::Error::source(&error);
            while let Some(next) = cause {
                chain.push_str(&format!(": {next}"));
                cause = next.source();
            }
            eprintln!("humatad2: session {}'s agent can't be resumed yet: {chain}", record.session);
            return failed(store, record, ResumeFailure::ConversationFileMissing, None).await;
        }
    };
    let saved = read_saved(&socket, &record.session).await?;
    let already_told = match why {
        Why::Interrupted => already_told(&record).await?,
        Why::Asked | Why::Restarted(_) | Why::Moved(_) => None,
    };
    // CV-3: the CLI may have been upgraded since the launch; what it says
    // now is kept, so the adapter's check is given the current version.
    if let Some(versions) = &config.versions {
        record.launch.cli_version = versions.current(&record.launch.agent).await;
    }
    record.generation += 1;
    let mut interruption = Interruption {
        at_unix_ms: unix_ms(),
        output_saved_through: saved.map_or(0, |saved| saved.saved_through),
        output_resumes_at: saved.map_or(0, |saved| saved.resumes_at),
        saved_tail_damaged: saved.is_some_and(|saved| saved.tail_damaged),
        told: already_told,
    };
    if matches!(why, Why::Interrupted) {
        record.interruptions.push(interruption.clone());
    }
    record.state = AgentState::Resuming;
    record.updated_unix_ms = unix_ms();
    save(store, &record, "save that the agent is resuming").await?;
    if rekeyed {
        // The record names the new folder now: the note has done its work.
        rekey::remove_note(&socket);
    }
    point("revive.resume.recorded");

    match cannot_resume(&record.launch) {
        // A move carries every file the agent wrote and checks them on the
        // new machine before it starts here, so a conversation file missing
        // after a move is one the agent never wrote: it had no turns yet,
        // and starts here fresh in its new folder. Any other restart had a
        // conversation to resume, and its loss is a failed resume (D757).
        Some(ResumeFailure::ConversationFileMissing) if matches!(why, Why::Moved(_)) => {
            return start_moved(store, config, record, created, env).await;
        }
        Some(reason) => return failed(store, record, reason, None).await,
        None => {}
    }
    let before = Conversation::look(&record.launch.conversation_file);
    let mut line = match why {
        Why::Interrupted => format!(
            "[Humata] This terminal stopped: its machine restarted or its holder was killed. Output after this point wasn't saved. Resuming the {} conversation {}.",
            record.launch.agent, record.launch.conversation_id
        ),
        Why::Asked => format!(
            "[Humata] Resuming the {} conversation {}.",
            record.launch.agent, record.launch.conversation_id
        ),
        Why::Restarted(text) | Why::Moved(text) => format!("[Humata] {text}"),
    };
    if interruption.saved_tail_damaged {
        line.push_str(" The end of the saved output was damaged and dropped.");
    }
    let variables = env(&agent_tab(&record, 0, created));
    let argv = record.launch.resume_argv.clone();
    let spawned = start_holder(config, &record, &argv, variables, Some(&line)).await?;
    point("revive.resume.spawned");
    // SEC-15 trusts the process tree of the holder the tab names, and the
    // resumed agent's own tools call in while it is still being checked:
    // the tab names the new holder from now on, not once it's verified.
    if let Err(error) = hand_tab_to(store, &record, &spawned, created).await {
        // The record still says resuming: the next reconcile resumes again.
        stop_holder(&socket, Some(spawned.pid)).await;
        return Err(error);
    }

    // A start fresh's first message that was kept but not typed is this
    // conversation's first message, and says what the session is about: the
    // agent isn't told it was interrupted ahead of it (D153). A move tells
    // it nothing.
    let seeded = match seed_pending(store, &record).await {
        Ok(seeded) => seeded,
        Err(error) => {
            // The holder is running and the record still says resuming: the
            // next reconcile resumes again, so this one doesn't leak.
            stop_holder(&socket, Some(spawned.pid)).await;
            return Err(error);
        }
    };
    let tell = matches!(why, Why::Interrupted) && already_told.is_none() && !seeded;
    match verify(store, config, &mut record, interruption.output_resumes_at, before, tell).await {
        Ok(Ok(())) => {
            point("revive.resume.verified");
            // With what verify added: when it was told. A move records no
            // interruption, so there is nothing to take from the record.
            if let (Why::Interrupted, Some(last)) = (why, record.interruptions.last()) {
                interruption = last.clone();
            }
            running(store, record, spawned, created).await?;
            Ok(Revived::Resumed { interruption })
        }
        Ok(Err((reason, exit_code))) => {
            // Stopped by a signal, so its output stays saved for the next
            // try.
            stop_holder(&socket, Some(spawned.pid)).await;
            // Stopped here, the record still says resuming: the next
            // reconcile tries again and fails the same way.
            point("revive.resume.failing");
            failed(store, record, reason, exit_code).await
        }
        Err(error) => {
            stop_holder(&socket, Some(spawned.pid)).await;
            Err(error)
        }
    }
}

/// When a resume a crash cut short is being done again and its agent was
/// already told it was interrupted, as its conversation shows since it was
/// about to be told: the note isn't told again. None otherwise, including
/// when the note never reached the conversation (the agent stopped before
/// it wrote it down), so it's told.
async fn already_told(record: &AgentRecord) -> Result<Option<Told>, ReviveError> {
    if record.state != AgentState::Resuming {
        return Ok(None);
    }
    let Some(told) = record.interruptions.last().and_then(|last| last.told) else {
        return Ok(None);
    };
    let file = record.launch.conversation_file.clone();
    let holds = blocking(move || holds_note(&file, told.conversation_len)).await?;
    Ok(holds.then_some(told))
}

/// Whether a start fresh kept a first message for `record`'s conversation
/// and hasn't typed it.
async fn seed_pending(store: &Arc<Store>, record: &AgentRecord) -> Result<bool, ReviveError> {
    let (store, session) = (store.clone(), record.session.clone());
    let found = blocking(move || store.fresh_seed(&session)).await?;
    match found {
        Ok(seed) => Ok(seed.is_some_and(|seed| seed.conversation_id == record.launch.conversation_id)),
        Err(source) => Err(store_error(
            &record.session,
            "look up the first message kept for the new conversation",
            source,
        )),
    }
}

/// Whether `file` holds the note after `from`, or anywhere if it's now
/// shorter than that (the agent rewrote it).
fn holds_note(file: &Path, from: u64) -> bool {
    let Ok(bytes) = std::fs::read(file) else {
        return false;
    };
    let start = usize::try_from(from).ok().filter(|start| *start <= bytes.len()).unwrap_or(0);
    let note = INTERRUPTED.as_bytes();
    bytes[start..].windows(note.len()).any(|window| window == note)
}

/// Starts the agent of a launch that never got as far as running, with
/// its first command: it never had a conversation to resume.
pub(crate) async fn relaunch(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: AgentRecord,
    created: i64,
    env: Env<'_>,
) -> Result<Revived, ReviveError> {
    let line = format!(
        "[Humata] Starting {} again: humatad2 stopped before it had finished starting it.",
        record.launch.agent
    );
    let variables = env(&agent_tab(&record, 0, created));
    let argv = record.launch.argv.clone();
    let spawned = match start_holder(config, &record, &argv, variables, Some(&line)).await {
        Ok(spawned) => spawned,
        Err(error) => {
            stopped_unstarted(store, record).await;
            return Err(error);
        }
    };
    if let Err(error) = running(store, record.clone(), spawned, created).await {
        stopped_unstarted(store, record).await;
        return Err(error);
    }
    Ok(Revived::Relaunched)
}

/// Starts a moved agent that had written no conversation yet with its
/// first command, in its new folder: there is nothing to resume, and its
/// session id is still the one it was launched with.
async fn start_moved(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: AgentRecord,
    created: i64,
    env: Env<'_>,
) -> Result<Revived, ReviveError> {
    let line = format!(
        "[Humata] Starting {} here with a new conversation: it hadn't written one yet.",
        record.launch.agent
    );
    let variables = env(&agent_tab(&record, 0, created));
    let argv = record.launch.argv.clone();
    let spawned = start_holder(config, &record, &argv, variables, Some(&line)).await?;
    running(store, record, spawned, created).await?;
    Ok(Revived::Relaunched)
}

/// Saves an agent a relaunch couldn't start as stopped, as `launch_agent`
/// does for a first launch that fails. Left as launching, every handler
/// answered "still starting" for good and the person could neither resume
/// nor start it fresh (Provisional: stopped, not a launching take-over;
/// recommend keeping it). The save is best effort: if it fails too, the
/// next restart tries the relaunch again.
async fn stopped_unstarted(store: &Arc<Store>, mut record: AgentRecord) {
    record.state = AgentState::Stopped;
    record.updated_unix_ms = unix_ms();
    let _ = save(store, &record, "save that the agent didn't start again").await;
}

async fn failed(
    store: &Arc<Store>,
    mut record: AgentRecord,
    reason: ResumeFailure,
    exit_code: Option<i32>,
) -> Result<Revived, ReviveError> {
    record.state = AgentState::ResumeFailed { reason, exit_code };
    record.updated_unix_ms = unix_ms();
    save(store, &record, "save why the resume failed").await?;
    Ok(Revived::ResumeFailed { reason, exit_code })
}

/// Why `launch` can't be resumed before trying, if it can't.
fn cannot_resume(launch: &AgentLaunch) -> Option<ResumeFailure> {
    if !launch.folder.is_dir() {
        return Some(ResumeFailure::FolderMissing);
    }
    // D784 (Fable B2): an adopted agent keeps its kind but Humata never
    // learned its conversation; with nothing pinned there is nothing to
    // resume, and an empty command must never be spawned.
    if launch.conversation_id.is_empty() {
        return Some(ResumeFailure::ConversationUnknown);
    }
    let program = launch.resume_argv.first()?;
    if !installed(Path::new(program), &launch.folder) {
        return Some(ResumeFailure::AgentNotInstalled);
    }
    if !launch.conversation_file.is_file() {
        return Some(ResumeFailure::ConversationFileMissing);
    }
    None
}

/// Whether `program` names an executable file, looked up on `PATH` when
/// it has no slash, as the holder will.
fn installed(program: &Path, folder: &Path) -> bool {
    let executable = |path: &Path| {
        path.metadata()
            .is_ok_and(|found| found.is_file() && found.permissions().mode() & 0o111 != 0)
    };
    if program.components().count() > 1 || program.is_absolute() {
        return executable(&folder.join(program));
    }
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| executable(&dir.join(program)))
    })
}

/// A conversation file and its siblings, as they stood before a resume.
struct Conversation {
    file: PathBuf,
    seen: Option<(u64, SystemTime)>,
    siblings: HashMap<OsString, (u64, SystemTime)>,
}

impl Conversation {
    fn look(file: &Path) -> Self {
        Self {
            file: file.to_owned(),
            seen: stamp(file),
            siblings: siblings(file),
        }
    }

    fn grew(&self) -> bool {
        stamp(&self.file) != self.seen
    }

    /// The conversations other than this one that were started or written.
    fn others_grown(&self) -> Vec<OsString> {
        siblings(&self.file)
            .into_iter()
            .filter(|(name, now)| self.siblings.get(name) != Some(now))
            .map(|(name, _)| name)
            .collect()
    }
}

/// The file names of the conversations `store`'s other sessions' agents
/// keep beside `file`. A machine that restarts resumes all its agents at
/// once, and agents of one CLI can share a conversation folder: another
/// session's agent writing its own conversation is no sign that this one
/// started a new one.
fn known_others(store: &Store, session: &str, file: &Path) -> Result<Vec<OsString>, StoreError> {
    let folder = file.parent();
    Ok(store
        .agents()?
        .into_iter()
        .filter(|agent| agent.session != session)
        .map(|agent| agent.launch.conversation_file)
        .filter(|other| other.parent() == folder)
        .filter_map(|other| other.file_name().map(OsString::from))
        .collect())
}

fn stamp(file: &Path) -> Option<(u64, SystemTime)> {
    let found = file.metadata().ok()?;
    Some((found.len(), found.modified().ok()?))
}

/// The other files beside `file` with its extension: the agent's other
/// conversations.
fn siblings(file: &Path) -> HashMap<OsString, (u64, SystemTime)> {
    let (Some(folder), Some(name)) = (file.parent(), file.file_name()) else {
        return HashMap::new();
    };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_name() != name)
        .filter(|entry| entry.path().extension() == file.extension())
        .filter_map(|entry| Some((entry.file_name(), stamp(&entry.path())?)))
        .collect()
}

/// Waits for the resumed agent to show it's up, tells it it was
/// interrupted, and checks its own conversation file is written while no
/// other is. Looks when its conversation file changes, the only file
/// watched (TR-12), and once more when `resume_timeout` is up.
///
/// Just before telling it, saves in `record` that it's about to be told.
/// Without `tell` (its conversation already holds the note), it isn't told,
/// and so writes nothing: the check is then only that it came up, stayed
/// up and started no other conversation.
async fn verify(
    store: &Arc<Store>,
    config: &ReviveConfig,
    record: &mut AgentRecord,
    from: u64,
    before: Conversation,
    tell: bool,
) -> Result<Result<(), (ResumeFailure, Option<i32>)>, ReviveError> {
    let mut deadline = Instant::now() + config.resume_timeout;
    let Ok(attached) = humata2_holder::attach(&record.launch.holder_socket, from).await else {
        return Ok(Err((ResumeFailure::AgentExited, None)));
    };
    if let Welcome {
        exited: Some(code), ..
    } = attached.welcome
    {
        return Ok(Err((ResumeFailure::AgentExited, Some(code))));
    }
    // Kept open: the holder stops sending output once a client's side
    // closes.
    let (_control, mut events) = (attached.control, attached.events);
    let before = Arc::new(before);
    let mut watch = FileWatch::new(&before.file);
    let mut last_output = None;
    let mut told = false;
    // When a note the gate held back looks again.
    let mut recheck: Option<Instant> = None;
    loop {
        // Told once its output has been quiet for `settle`, and the gate
        // lets it.
        let settled = last_output.filter(|_| !told).map(|at: Instant| {
            let at = at + config.settle;
            recheck.map_or(at, |again| again.max(at))
        });
        let timed_out = tokio::select! {
            event = events.next() => {
                match event {
                    Ok(Some(Event::Output { .. })) => last_output = Some(Instant::now()),
                    Ok(Some(Event::Exited { code })) => {
                        return Ok(Err((ResumeFailure::AgentExited, Some(code))));
                    }
                    Ok(Some(_)) => {}
                    Ok(None) | Err(_) => return Ok(Err((ResumeFailure::AgentExited, None))),
                }
                continue;
            }
            () = until(settled) => {
                if tell {
                    // D763: never typed into a question the agent asks of
                    // its own; the resume waits while the person answers.
                    if let Err(refused) = gate_says(config, &record.launch.holder_socket).await {
                        if refused == GateRefusal::Question {
                            deadline = Instant::now() + config.resume_timeout;
                        }
                        recheck = Some(Instant::now() + GATE_EVERY);
                        continue;
                    }
                    let conversation_len = stamp(&before.file).map_or(0, |(len, _)| len);
                    if let Some(last) = record.interruptions.last_mut() {
                        last.told = Some(Told {
                            at_unix_ms: unix_ms(),
                            conversation_len,
                        });
                    }
                    record.updated_unix_ms = unix_ms();
                    save(store, record, "save that the agent is being told it was interrupted").await?;
                    point("revive.resume.telling");
                    // Its conversation is only written once it's told
                    // something, so telling it is also the check.
                    // Its return is pressed once it shows the note, and a
                    // note it dropped is typed again once, never twice.
                    let typed = humata2_holder::deliver(
                        &record.launch.holder_socket,
                        INTERRUPTED.as_bytes(),
                        TYPE_PAUSE,
                        None,
                        ready(config, &record.launch.holder_socket),
                        config.resume_timeout,
                    )
                    .await;
                    if typed.is_err() {
                        return Ok(Err((ResumeFailure::AgentExited, None)));
                    }
                    if recheck.is_some() {
                        // It waited on the person: its time to write the
                        // conversation starts once it's told.
                        deadline = Instant::now() + config.resume_timeout;
                    }
                    point("revive.resume.told");
                }
                told = true;
                false
            }
            () = watch.changed() => false,
            () = tokio::time::sleep_until(deadline) => true,
        };
        let looked = before.clone();
        let (reader, session) = (store.clone(), record.session.clone());
        let (grew, another_grew) = blocking(move || {
            let grew = looked.grew();
            let mut grown = looked.others_grown();
            if !grew && !grown.is_empty() {
                // A store that can't be read leaves every grown file
                // counted, as before the other sessions were known.
                let known = known_others(&reader, &session, &looked.file).unwrap_or_default();
                grown.retain(|name| !known.contains(name));
            }
            (grew, !grown.is_empty())
        })
        .await
        .unwrap_or((false, false));
        if another_grew && !grew {
            return Ok(Err((ResumeFailure::AgentStartedAnotherConversation, None)));
        }
        if told && (grew || !tell) {
            return Ok(Ok(()));
        }
        if timed_out {
            if !tell {
                // Up all along, but it never printed anything to settle.
                return Ok(Ok(()));
            }
            // Up, but it never wrote the conversation it was resumed
            // with: it isn't that conversation.
            return Ok(Err((ResumeFailure::AgentStartedAnotherConversation, None)));
        }
    }
}

/// What the gate says of the agent tab on `socket`; clear without one.
async fn gate_says(config: &ReviveConfig, socket: &Path) -> Result<(), GateRefusal> {
    match &config.gate {
        Some(gate) => gate.guard(socket).await,
        None => Ok(()),
    }
}

/// The gate as the holder asks it while it sees a typed note through.
fn ready(config: &ReviveConfig, socket: &Path) -> Option<humata2_holder::Ready> {
    let gate = config.gate.clone()?;
    let socket = socket.to_owned();
    Some(Arc::new(move || {
        let asked = gate.guard(&socket);
        Box::pin(async move { asked.await.is_ok() })
    }))
}

/// Returns at `at`, or never without one.
async fn until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

/// Starts `argv` for `record` in a holder on its socket. With `line`, the
/// terminal prints it first, so whoever attaches sees what happened.
pub(crate) async fn start_holder(
    config: &ReviveConfig,
    record: &AgentRecord,
    argv: &[OsString],
    env: Vec<(OsString, OsString)>,
    line: Option<&str>,
) -> Result<Spawned, ReviveError> {
    // A fault build can be told to fail here (HUMATA2_FAIL_AT); in any other
    // build this is false and compiled away.
    if humata2_store::fault::fails("revive.start_holder") {
        return Err(ReviveError::Injected { point: "revive.start_holder" });
    }
    let argv = match &config.contain {
        Some(contain) => contain
            .wrap(record, argv)
            .map_err(|source| ReviveError::Contain {
                session: record.session.clone(),
                source,
            })?,
        None => argv.to_vec(),
    };
    let Some((program, args)) = argv.split_first() else {
        return Err(ReviveError::NoCommand {
            session: record.session.clone(),
        });
    };
    let spec = HolderSpec {
        holder: config.holder.clone(),
        socket: record.launch.holder_socket.clone(),
        program: PathBuf::from(program),
        args: args.to_vec(),
        cwd: record.launch.folder.clone(),
        env,
        cols: record.launch.cols.max(1),
        rows: record.launch.rows.max(1),
        scrollback_bytes: config.scrollback_bytes,
        log: config.logs.join(format!("{}.log", record.tab)),
    };
    if let Some(builds) = &config.builds {
        let (builds, session) = (builds.clone(), record.session.clone());
        // Kept before the holder starts, so a record is never older than
        // the holder it describes. A task that couldn't run is no reason
        // not to start the agent.
        let _ = blocking(move || builds.agent_spawned(&session)).await;
    }
    spawn(spec, line, format!("session {}'s agent", record.session)).await
}

/// Starts a holder for `spec`; with `banner`, the terminal shows it before
/// the program's first output (D152).
pub(crate) async fn spawn(
    spec: HolderSpec,
    banner: Option<&str>,
    what: String,
) -> Result<Spawned, ReviveError> {
    let banner = banner.map(str::to_owned);
    blocking(move || match &banner {
        Some(banner) => humata2_holder::spawn_with_banner(&spec, banner),
        None => humata2_holder::spawn(&spec),
    })
    .await?
    .map_err(|source| ReviveError::Holder { what, source })
}

/// What a holder on `socket` says when attached to, if one answers.
pub(crate) async fn probe(socket: &Path) -> Option<Welcome> {
    let attached = humata2_holder::attach(socket, u64::MAX).await.ok()?;
    Some(attached.welcome)
}

/// Stops the holder answering on `socket` (process `pid`, or the one its
/// pid file names) by its pid: SIGTERM, which keeps its saved output, then
/// SIGKILL if it's still there after a grace period. Waits until it is gone.
/// Only a live process named as a holder is signalled, so a pid reused by
/// another program is left alone.
pub(crate) async fn stop_holder(socket: &Path, pid: Option<u32>) {
    let pid = match pid {
        Some(pid) => pid,
        // Found by its pid file, or by the socket it serves if that is gone.
        None => match humata2_holder::find_holder(socket).await {
            Ok(Some(pid)) => pid,
            Ok(None) => return,
            Err(error) => {
                eprintln!("humata2 revive: {error}");
                return;
            }
        },
    };
    if let Err(error) = humata2_holder::end_by_pid(socket, pid, STOP_GRACE).await {
        eprintln!(
            "humata2 revive: stopping the holder at {}: {error}",
            socket.display()
        );
    }
}

async fn read_saved(
    socket: &Path,
    session: &str,
) -> Result<Option<humata2_holder::SavedOutput>, ReviveError> {
    let path = humata2_holder::saved_scrollback_path(socket);
    blocking(move || humata2_holder::read_saved(&path))
        .await?
        .map_err(|source| ReviveError::Holder {
            what: format!("session {session}'s agent"),
            source,
        })
}

pub(crate) fn agent_tab(record: &AgentRecord, holder_pid: u32, created_unix_ms: i64) -> TabRecord {
    TabRecord {
        id: record.tab.clone(),
        session: Some(record.session.clone()),
        kind: TabKind::Agent,
        title: record.launch.agent.clone(),
        holder_socket: record.launch.holder_socket.clone(),
        holder_pid,
        created_unix_ms,
        holder_generation: 0,
    }
}

pub(crate) async fn save(
    store: &Arc<Store>,
    record: &AgentRecord,
    doing: &'static str,
) -> Result<(), ReviveError> {
    let (store, saving) = (store.clone(), record.clone());
    blocking(move || store.put_agent(&saving))
        .await?
        .map_err(|source| store_error(&record.session, doing, source))
}

pub(crate) fn store_error(session: &str, doing: &'static str, source: StoreError) -> ReviveError {
    ReviveError::Store {
        what: format!("session {session}"),
        doing,
        source,
    }
}

pub(crate) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ReviveError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|source| ReviveError::Background { source })
}

pub(crate) fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod conversation_unknown_tests {
    use super::*;

    /// D784 (Fable B2): an adopted agent with no known conversation is
    /// never resumed, so no empty command is spawned.
    #[test]
    fn an_agent_whose_conversation_is_unknown_cannot_be_resumed() {
        let launch = AgentLaunch {
            agent: "copilot".to_owned(),
            folder: std::env::temp_dir(),
            ..AgentLaunch::default()
        };
        assert_eq!(cannot_resume(&launch), Some(ResumeFailure::ConversationUnknown));
    }
}
