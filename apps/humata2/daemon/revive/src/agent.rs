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
    StoreError, TabKind, TabRecord, Told,
};
use rustix::process::{Pid, Signal};
use tokio::time::Instant;

use crate::reconcile::Revived;
use crate::watch::FileWatch;
use crate::{Env, ReviveConfig, ReviveError};

/// Told to an agent once it has resumed (PER-14).
const INTERRUPTED: &str = "Humata: this session was interrupted (its machine restarted or its terminal was killed) and you were resumed. Run `git status` and check your work before continuing.";
/// How long a holder asked to stop gets before it's killed.
const STOP_GRACE: Duration = Duration::from_secs(5);
/// Between typing the note and pressing return, so an agent that reads a
/// paste whole doesn't take the return as part of it.
const TYPE_PAUSE: Duration = Duration::from_millis(100);

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
    let tab = agent_tab(&record, spawned.pid, created);
    let (saving, saved_tab) = (record.clone(), tab.clone());
    let saved = blocking({
        let store = store.clone();
        move || store.put_agent_and_tab(&saving, &saved_tab)
    })
    .await?;
    if let Err(source) = saved {
        stop_holder(&tab.holder_socket, Some(spawned.pid)).await;
        return Err(store_error(&record.session, "save the running agent and its tab", source));
    }
    Ok(record)
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
) -> Result<Revived, ReviveError> {
    let socket = record.launch.holder_socket.clone();
    let saved = read_saved(&socket, &record.session).await?;
    let already_told = already_told(&record).await?;
    record.generation += 1;
    let mut interruption = Interruption {
        at_unix_ms: unix_ms(),
        output_saved_through: saved.map_or(0, |saved| saved.saved_through),
        output_resumes_at: saved.map_or(0, |saved| saved.resumes_at),
        saved_tail_damaged: saved.is_some_and(|saved| saved.tail_damaged),
        told: already_told,
    };
    record.interruptions.push(interruption.clone());
    record.state = AgentState::Resuming;
    record.updated_unix_ms = unix_ms();
    save(store, &record, "save that the agent is resuming").await?;
    point("revive.resume.recorded");

    if let Some(reason) = cannot_resume(&record.launch) {
        return failed(store, record, reason, None).await;
    }
    let before = Conversation::look(&record.launch.conversation_file);
    let mut line = format!(
        "[Humata] This terminal stopped: its machine restarted or its holder was killed. Output after this point wasn't saved. Resuming the {} conversation {}.",
        record.launch.agent, record.launch.conversation_id
    );
    if interruption.saved_tail_damaged {
        line.push_str(" The end of the saved output was damaged and dropped.");
    }
    let variables = env(&agent_tab(&record, 0, created));
    let argv = record.launch.resume_argv.clone();
    let spawned = start_holder(config, &record, &argv, variables, Some(&line)).await?;
    point("revive.resume.spawned");

    // A start fresh's first message that was kept but not typed is this
    // conversation's first message, and says what the session is about: the
    // agent isn't told it was interrupted ahead of it (D153).
    let seeded = seed_pending(store, &record).await?;
    let tell = already_told.is_none() && !seeded;
    match verify(store, config, &mut record, interruption.output_resumes_at, before, tell).await {
        Ok(Ok(())) => {
            point("revive.resume.verified");
            // With what verify added: when it was told.
            if let Some(last) = record.interruptions.last() {
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
    let spawned = start_holder(config, &record, &argv, variables, Some(&line)).await?;
    running(store, record, spawned, created).await?;
    Ok(Revived::Relaunched)
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

    /// Whether a conversation other than this one was started or written.
    fn another_grew(&self) -> bool {
        siblings(&self.file)
            .iter()
            .any(|(name, now)| self.siblings.get(name) != Some(now))
    }
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
    let deadline = Instant::now() + config.resume_timeout;
    let Ok(attached) = humata2_holder::attach(&record.launch.holder_socket, from).await else {
        return Ok(Err((ResumeFailure::AgentExited, None)));
    };
    if let Welcome {
        exited: Some(code), ..
    } = attached.welcome
    {
        return Ok(Err((ResumeFailure::AgentExited, Some(code))));
    }
    let (mut control, mut events) = (attached.control, attached.events);
    let before = Arc::new(before);
    let mut watch = FileWatch::new(&before.file);
    let mut last_output = None;
    let mut told = false;
    loop {
        // Told once its output has been quiet for `settle`.
        let settled = last_output.filter(|_| !told).map(|at: Instant| at + config.settle);
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
                    let typed = control.input(1, INTERRUPTED.as_bytes()).await;
                    tokio::time::sleep(TYPE_PAUSE).await;
                    if typed.is_err() || control.input(2, b"\r").await.is_err() {
                        return Ok(Err((ResumeFailure::AgentExited, None)));
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
        let (grew, another_grew) = blocking(move || (looked.grew(), looked.another_grew()))
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

/// Stops the holder answering on `socket` with SIGTERM, which keeps its
/// saved output, and kills it if it's still there after a grace period.
pub(crate) async fn stop_holder(socket: &Path, pid: Option<u32>) {
    let Some(pid) = pid
        .or_else(|| humata2_holder::holder_pid(socket))
        .and_then(|pid| i32::try_from(pid).ok())
        .and_then(Pid::from_raw)
    else {
        return;
    };
    // Attached first: its feed ends when the holder is gone.
    let attached = humata2_holder::attach(socket, u64::MAX).await.ok();
    let _ = rustix::process::kill_process(pid, Signal::TERM);
    let Some(attached) = attached else { return };
    // Kept, so the holder doesn't end the feed for a client that left.
    let (_control, mut events) = (attached.control, attached.events);
    let gone = async { while let Ok(Some(_)) = events.next().await {} };
    if tokio::time::timeout(STOP_GRACE, gone).await.is_err() {
        let _ = rustix::process::kill_process(pid, Signal::KILL);
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
