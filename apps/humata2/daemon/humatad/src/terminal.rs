//! TerminalService: tabs, each a shell kept by a holder of its own, typing
//! into a session, and pasted images. Every call is checked for its caller
//! first. Typing queued for a turn end isn't built yet.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_access::{AccessRequest, Call, Capability, Target};
use humata2_chunk::Link;
use humata2_grpc::humata::v1 as pb;
use humata2_holder::{HolderError, HolderSpec, KeyState, KeyedInput};
use humata2_store::{
    AgentState, SessionRecord, Store, StoreError, TabKind, TabRecord, TabReplies, TypedKey,
};
use pb::invalid_field::Problem;
use tokio::sync::watch;
use tonic::{Request, Response, Status, Streaming};

use crate::attach::{self, AttachStream};
use crate::channel_service::{Gated, answer_gated, gate};
use crate::compression::Compression;
use crate::desk_service;
use crate::error::DaemonError;
use crate::guard::Guard;
use crate::holder_end;
use crate::holders::{Holders, TypeFailed};
use crate::identity::{self, Caller, Identities};
use crate::inbox::Inbox;
use crate::remote_terminal::RemoteTerminal;
use crate::input_gate;
use crate::serve::SOCKET_PATH_MAX;
use crate::upgrade::items::Migrations;
use crate::{paste, status};

mod lost;

/// How long a holder gets to answer a first connection before a tab is
/// listed without its offsets.
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// How a screen that couldn't start at boot is tried again: the first
/// pause, the longest, and how many tries (about half a minute in all).
const SCREEN_RETRY_FIRST: Duration = Duration::from_millis(100);
const SCREEN_RETRY_LAST: Duration = Duration::from_secs(2);
const SCREEN_RETRIES: usize = 18;

/// Results kept for retries with the same idempotency key.
const REMEMBERED_MAX: usize = 4096;

/// A new tab's size until a client attaches with its own.
const INITIAL_COLS: u16 = 80;
const INITIAL_ROWS: u16 = 24;

/// How long a tab's shell gets to take text typed into it.
const TYPE_TIMEOUT: Duration = Duration::from_secs(10);
/// D671 (Provisional): between typing a Type call's text and pressing
/// return. An Ink-style terminal (Copilot's) reads a burst as a paste and
/// takes a return in it, or 100ms after it, as a newline instead of a
/// submit, so the return is a write of its own after this pause.
const SUBMIT_PAUSE: Duration = Duration::from_millis(400);

/// BUG-T1: a Type call that submits waits, until the program in the tab
/// has run this long, for it to read keys itself before it types, and
/// then up to `SHOWN_WAIT` for it to show the text before it presses
/// return. Either way it goes on after: a program that keeps line editing
/// on, or shows a long paste as a summary, still gets the text and the
/// return.
const KEYS_WAIT: Duration = Duration::from_secs(5);
const SHOWN_WAIT: Duration = Duration::from_secs(5);
/// How long reaching a holder to watch its output may take.
const WATCH_REACH: Duration = Duration::from_secs(2);

/// Waits until the program in the holder at `socket` reads keys itself,
/// then watches its output for the text about to be typed. `None` when
/// the holder can't be watched or doesn't answer (it may be stopped):
/// typing it says why. A holder from an older build is refused, with
/// nothing typed: its session moves onto the current holder through the
/// "Migrate the holders" item (D373, D753), not by typing without the wait.
async fn ready_for_text(socket: &Path) -> Result<Option<humata2_holder::OutputWatch>, Status> {
    // The holder answers by `KEYS_WAIT` after the program started; the
    // rest is for reaching it.
    let asked = tokio::time::timeout(
        KEYS_WAIT + WATCH_REACH,
        humata2_holder::await_keys(socket, KEYS_WAIT),
    )
    .await;
    match asked {
        Ok(Ok(_)) => {}
        Ok(Err(error @ HolderError::Outdated { .. })) => return Err(outdated_holder(&error)),
        Ok(Err(_)) | Err(_) => return Ok(None),
    }
    Ok(tokio::time::timeout(WATCH_REACH, humata2_holder::watch_output(socket))
        .await
        .ok()
        .and_then(Result::ok))
}


/// The refusal for text typed into a holder from an older build.
fn outdated_holder(error: &HolderError) -> Status {
    status::failure(
        tonic::Code::FailedPrecondition,
        pb::Error {
            message: error.to_string(),
            fix: "click Migrate on the \"Migrate the holders\" item in the Inbox, which restarts the session's agent onto the current holder at its next idle moment and keeps its conversation, then type again. A shell tab is never migrated: close it and open a new one".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// Waits until `watch` shows `text`, once `body` typed it.
async fn shown(
    watch: &mut Option<humata2_holder::OutputWatch>,
    body: &Result<(), TypeFailure>,
    text: &[u8],
) {
    if let (Some(watch), Ok(())) = (watch, body) {
        let _ = watch.shown(text, SHOWN_WAIT).await;
    }
}

/// How long a Type call's key is kept, as every idempotency key is (GRPC.md
/// 5.2): 30 days.
const TYPED_KEYS_KEPT_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Where a tab's holder listens: `holders/` and 16 hex digits of the tab's
/// id, so the path stays under the socket limit for an ordinary state
/// folder. The digits are random, so two tabs don't share one.
pub(crate) fn holder_socket(holders: &Path, tab: uuid::Uuid) -> PathBuf {
    holders.join(format!("{:016x}.sock", tab.as_u128() >> 64))
}

/// Refuses a state folder whose holder sockets would be longer than Unix
/// sockets allow: every tab's would be as long, so no terminal could start.
pub(crate) fn check_holder_sockets(holders: &Path) -> Result<(), DaemonError> {
    // Every name has the same length, so the nil id's stands for them all.
    let socket = holder_socket(holders, uuid::Uuid::nil());
    let len = socket.as_os_str().len();
    if len > SOCKET_PATH_MAX {
        return Err(DaemonError::HolderSocketPathTooLong {
            socket,
            len,
            max: SOCKET_PATH_MAX,
        });
    }
    Ok(())
}

pub(crate) struct TerminalsConfig {
    pub(crate) store: Arc<Store>,
    pub(crate) machine: String,
    /// The `humata2_holder` binary.
    pub(crate) holder: PathBuf,
    pub(crate) shell: PathBuf,
    pub(crate) shell_args: Vec<OsString>,
    /// Where a machine shell starts.
    pub(crate) root: PathBuf,
    /// Where holders put their sockets.
    pub(crate) holders: PathBuf,
    /// Where holders write what goes wrong.
    pub(crate) logs: PathBuf,
    pub(crate) scrollback_bytes: usize,
    pub(crate) guard: Arc<Guard>,
    /// Gives each session tab's shell its session token.
    pub(crate) identities: Arc<Identities>,
    /// Picks the codec an attach's output goes with.
    pub(crate) compression: Arc<Compression>,
    /// Where a call a rule sends to Ask me waits for the user (D38.6).
    pub(crate) inbox: Arc<Inbox>,
    /// The host's way to the terminals of other machines (D139); none on a machine's
    /// own daemon, where every session is here.
    pub(crate) remote: Option<RemoteTerminal>,
    /// The socket this daemon serves on, given to session tabs.
    pub(crate) socket: PathBuf,
    /// UP-1, D281 e: what would run across releases is refused, by the Inbox's Migrate items.
    pub(crate) migrations: Migrations,
    /// D373: keeps which holder file each tab's holder was spawned from.
    pub(crate) builds: Arc<crate::upgrade::holders::Builds>,
    /// Tells the feed a session whose tabs changed, so every client lists
    /// them without asking (a list read just before is not the last word).
    pub(crate) tell: crate::session_service::Tell,
    /// This start, which brings back the shells whose holders died with the
    /// last run: an attach to one waits for it rather than call it gone.
    pub(crate) started: Arc<crate::restart::Started>,
}

/// The service. Its state sits behind an `Arc` so a call waiting for the
/// user's approval can own what it needs to run later.
/// The folders a shell may be started in for `session` (PL-10): its folder
/// and clones, and, when its folder is in a channel's, that channel's folder
/// and the `received` folder beside the channels.
fn session_places(store: &Store, session: &SessionRecord) -> Result<Vec<PathBuf>, StoreError> {
    let mut places = vec![session.folder.clone()];
    if let Some(detail) = store.session_detail(&session.id)? {
        places.extend(detail.repos.into_iter().map(|clone| clone.path));
    }
    if let Some(channel) = store.channel(&session.channel)? {
        places.extend(channel_places(&session.folder, &channel.folder));
    }
    Ok(places)
}

/// A session's folder is `<channels>/<channel folder>/sessions/<name>`; this
/// is the channel's folder and `<channels>/received` when it is.
fn channel_places(session_folder: &Path, channel_folder: &str) -> Vec<PathBuf> {
    let Some(sessions) = session_folder.parent() else {
        return Vec::new();
    };
    if sessions.file_name() != Some(std::ffi::OsStr::new("sessions")) {
        return Vec::new();
    }
    let Some(channel) = sessions.parent().filter(|channel| channel.ends_with(channel_folder)) else {
        return Vec::new();
    };
    let depth = Path::new(channel_folder).components().count();
    let mut places = vec![channel.to_owned()];
    if let Some(channels) = channel.ancestors().nth(depth) {
        places.push(channels.join("received"));
    }
    places
}

/// `asked` as a canonical folder inside one of `places`, or why it is not.
fn resolve_folder(asked: &str, places: &[PathBuf]) -> Result<PathBuf, Status> {
    let path = Path::new(asked);
    if !path.is_absolute() {
        return Err(status::invalid_field(
            "folder",
            Problem::BadFormat,
            format!("folder {asked} isn't an absolute path"),
            "give the full path of the folder on the session's machine",
        ));
    }
    if path.components().any(|part| part == std::path::Component::ParentDir) {
        return Err(status::invalid_field(
            "folder",
            Problem::NotAllowedHere,
            format!("folder {asked} goes up with \"..\", which could leave the session's folders"),
            "give the folder's path without \"..\"",
        ));
    }
    let real = match std::fs::canonicalize(path) {
        Ok(real) => real,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(status::not_found(
                pb::EntityKind::File,
                asked,
                format!("folder {asked} doesn't exist on this machine"),
                "open the terminal from a folder that exists, or refresh the file list and try again",
            ));
        }
        Err(error) => {
            return Err(status::invalid_field(
                "folder",
                Problem::BadFormat,
                format!("couldn't look at folder {asked}: {error}"),
                "check that the path names a folder Humata can open",
            ));
        }
    };
    let inside = places
        .iter()
        .filter_map(|place| std::fs::canonicalize(place).ok())
        .any(|place| real.starts_with(place));
    if !inside {
        return Err(status::invalid_field(
            "folder",
            Problem::NotAllowedHere,
            format!("folder {asked} isn't inside the session's folder, its clones, its channel's folder or the received files"),
            "open the terminal from a folder in one of those places",
        ));
    }
    if !real.is_dir() {
        return Err(status::invalid_field(
            "folder",
            Problem::BadFormat,
            format!("{asked} is a file, not a folder"),
            "open the terminal in the folder that holds the file",
        ));
    }
    Ok(real)
}

pub(crate) struct Terminals {
    core: Arc<Core>,
}

pub(crate) struct Core {
    config: TerminalsConfig,
    remembered: Mutex<Remembered>,
    writers: Arc<Writers>,
    /// Shared with every attach, which asks it for the tab's link each time
    /// it needs the screen, so it never keeps a link that was replaced.
    holders: Arc<Holders>,
}

impl Terminals {
    /// The shared state, for the review's comment sink.
    pub(crate) fn core(&self) -> Arc<Core> {
        self.core.clone()
    }

    /// Starts the screen of every tab the store lists, in the background,
    /// so a program's queries are answered whether or not anyone opens
    /// the tab (D271). A tab whose holder isn't up yet, as one being
    /// brought back after a restart, is tried again for a while.
    pub(crate) fn start_screens(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let core = self.core.clone();
        runtime.spawn(async move {
            let store = core.config.store.clone();
            let tabs = match blocking(move || store.tabs()).await {
                Ok(Ok(tabs)) => tabs,
                _ => return,
            };
            for tab in tabs {
                tokio::spawn(core.clone().start_screen_when_up(tab));
            }
        });
    }
}

impl std::ops::Deref for Terminals {
    type Target = Core;

    fn deref(&self) -> &Core {
        &self.core
    }
}

impl Terminals {
    pub(crate) fn new(config: TerminalsConfig) -> Self {
        let terminals = Self {
            core: Arc::new(Core {
                remembered: Mutex::new(Remembered::default()),
                writers: Arc::new(Writers::default()),
                holders: Arc::new(Holders::new(config.scrollback_bytes, config.store.clone())),
                config,
            }),
        };
        terminals.start_screens();
        terminals.drop_old_typed_keys();
        terminals
    }

    /// The links to the tabs' holders, which type an Inbox answer to a
    /// question on an agent's screen (D767).
    pub(crate) fn holder_links(&self) -> Arc<Holders> {
        self.core.holders.clone()
    }

    /// Drops the Type keys older than [`TYPED_KEYS_KEPT_MS`], in the
    /// background, so the table holds only the window's calls. A failure
    /// leaves them for the next start.
    fn drop_old_typed_keys(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let store = self.core.config.store.clone();
        runtime.spawn_blocking(move || {
            if let Err(error) = store.prune_typed_keys(unix_ms() - TYPED_KEYS_KEPT_MS) {
                crate::log::say!("humatad2: couldn't drop old Type keys: {error}");
            }
        });
    }
}

/// Why typing into a tab failed: before any text reached its shell, or at
/// a point where some of it may have.
enum TypeFailure {
    Untyped(Status),
    MaybeTyped(Status),
}

/// Splits `text` into what to type and whether it ended with a newline
/// (`\n`, `\r` or `\r\n`), the Type call's way of asking to submit: the
/// request has no separate submit field, so text ending in a newline
/// submits, as before. D671 (Provisional).
fn split_submit(text: &[u8]) -> (&[u8], bool) {
    let body = text
        .strip_suffix(b"\r\n")
        .or_else(|| text.strip_suffix(b"\n"))
        .or_else(|| text.strip_suffix(b"\r"));
    match body {
        Some(body) => (body, true),
        None => (text, false),
    }
}

/// What became of typing a batch of review comments (D138).
pub(crate) enum CommentsTyped {
    /// All of it reached the tab's shell.
    Yes,
    /// None of it was typed.
    No(Status),
    /// Some or all of it may have been typed.
    Unknown(Status),
}

/// Where a session's comment batches are typed: the tab, and the holder
/// process that held its terminal when the batch was begun.
pub(crate) struct CommentsTarget {
    pub(crate) tab: String,
    /// The holder's instance (`humata2_holder::KeyState`), or 0 when it
    /// can't confirm what it typed: an older holder, or one that didn't
    /// answer.
    pub(crate) holder: u64,
}

/// Whether a batch reached its tab, after a restart.
pub(crate) enum CommentsArrived {
    Yes,
    No,
    Unknown,
}

impl Core {
    /// The tab a session's typing goes to (its agent's, else its first
    /// shell) and its holder's instance, for the review's comment batches
    /// (SM-9). A holder that can't answer in time gives `holder: 0`.
    pub(crate) async fn comments_target(&self, session: &str) -> Result<CommentsTarget, Status> {
        // MV-4: a session that moved to another machine has its tabs there;
        // its batches go to its agent's tab through the relay.
        if let Some(record) = self.find_session(session).await?
            && self.elsewhere(&record).is_some()
        {
            return Ok(CommentsTarget {
                tab: record.id,
                holder: 0,
            });
        }
        let tab = self.typing_tab(session).await?;
        let asked = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::key_arrived(&tab.holder_socket, "instance"),
        )
        .await;
        let holder = match asked {
            Ok(Ok(KeyState::Arrived { holder } | KeyState::NotArrived { holder })) => holder,
            _ => 0,
        };
        Ok(CommentsTarget {
            tab: tab.id,
            holder,
        })
    }

    /// Types `text` into the target's tab, once for `key` when the holder
    /// can remember keys. Without that (`target.holder == 0`, or a holder
    /// that turns out not to know keyed input) it is plain typing, which
    /// cannot be told apart from a second try after a restart.
    /// BUG-T1: once the agent in the comments' tab on this machine reads
    /// keys itself, watches its output for the comments about to be typed.
    /// `None` for a tab elsewhere or one that can't be watched.
    pub(crate) async fn watch_for_comments(
        &self,
        target: &CommentsTarget,
    ) -> Result<Option<humata2_holder::OutputWatch>, Status> {
        match self.find_tab(&target.tab).await {
            Ok(Some(tab)) => ready_for_text(&tab.holder_socket).await,
            Ok(None) | Err(_) => Ok(None),
        }
    }

    pub(crate) async fn type_comments(
        &self,
        target: &CommentsTarget,
        key: &str,
        text: &[u8],
    ) -> CommentsTyped {
        let tab = match self.find_tab(&target.tab).await {
            Ok(Some(tab)) => tab,
            Ok(None) => return self.type_comments_elsewhere(target, key, text).await,
            Err(status) => return CommentsTyped::No(status),
        };
        // MV-4: as for Type, nothing is typed into a session that moves. None of
        // the batch is typed, so the refusal reads as No.
        if let Some(session) = &tab.session
            && let Err(status) = crate::session_service::refuse_input_while_moving(session)
        {
            return CommentsTyped::No(status);
        }
        // D762: not into a question the agent asks on its screen, and not
        // blind. The return after the paste is only a return.
        if tab.kind == TabKind::Agent
            && text != b"\r"
            && let Err(refused) = input_gate::guard(&tab.holder_socket).await
        {
            let session = tab.session.as_deref().unwrap_or(&tab.id);
            return CommentsTyped::No(input_gate::refusal(session, &refused));
        }
        if target.holder == 0 {
            return self.type_plain(&tab, text).await;
        }
        let no_log = Path::new("");
        let typed = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::keyed_input(&tab.holder_socket, key, text),
        )
        .await;
        match typed {
            Ok(Ok(KeyedInput::Written | KeyedInput::Repeat)) => {
                CommentsTyped::Yes
            }
            // D753: a holder from before keyed input types nothing.
            Ok(Err(error @ HolderError::Outdated { .. })) => {
                CommentsTyped::No(outdated_holder(&error))
            }
            // Failed before anything could be typed.
            Ok(Err(
                error @ (HolderError::Connect { .. }
                | HolderError::KeyTooLong { .. }
                | HolderError::KeyedInputTooLarge { .. }),
            )) => CommentsTyped::No(holder_failed("reach the tab's shell", &error, no_log)),
            // A refused keyed write may have been partly typed.
            Ok(Err(error)) => {
                CommentsTyped::Unknown(holder_failed("type the comments into the tab", &error, no_log))
            }
            Err(_) => CommentsTyped::Unknown(status::internal(
                format!(
                    "tab {}'s shell didn't take the comments within {} seconds; some of them may have been typed",
                    tab.id,
                    TYPE_TIMEOUT.as_secs()
                ),
                "",
            )),
        }
    }

    /// After a restart: whether the batch `key` reached the target's tab.
    /// Only the holder process that held the terminal when the batch was
    /// begun can say it never arrived: a holder started again for the same
    /// tab has an empty memory, so its "no" proves nothing and reads as
    /// `Unknown`.
    pub(crate) async fn comments_arrived(&self, target: &CommentsTarget, key: &str) -> CommentsArrived {
        if target.holder == 0 {
            return CommentsArrived::Unknown;
        }
        let tab = match self.find_tab(&target.tab).await {
            Ok(Some(tab)) => tab,
            _ => return CommentsArrived::Unknown,
        };
        let asked = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::key_arrived(&tab.holder_socket, key),
        )
        .await;
        match asked {
            Ok(Ok(KeyState::Arrived { holder })) if holder == target.holder => CommentsArrived::Yes,
            Ok(Ok(KeyState::NotArrived { holder })) if holder == target.holder => CommentsArrived::No,
            _ => CommentsArrived::Unknown,
        }
    }

    /// Types a batch into the agent's tab of a session on another machine,
    /// where `target.tab` names the session (see [`Core::comments_target`]).
    /// The relayed Type carries `key`, so a second try types nothing twice.
    async fn type_comments_elsewhere(&self, target: &CommentsTarget, key: &str, text: &[u8]) -> CommentsTyped {
        let record = match self.find_session(&target.tab).await {
            Ok(Some(record)) => record,
            Ok(None) => return CommentsTyped::No(no_tab(&target.tab)),
            Err(status) => return CommentsTyped::No(status),
        };
        let Some((machine, remote)) = self.elsewhere(&record) else {
            return CommentsTyped::No(no_tab(&target.tab));
        };
        if let Err(status) = crate::session_service::refuse_input_while_moving(&record.id) {
            return CommentsTyped::No(status);
        }
        let Ok(text) = String::from_utf8(text.to_vec()) else {
            return CommentsTyped::No(status::internal(
                format!("the comments for session {} aren't UTF-8 text, so they weren't typed", record.id),
                "edit the comments so they are plain text, then send them again",
            ));
        };
        let request = pb::TypeRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: key.to_owned() }),
            session: Some(pb::SessionId { value: record.id.clone() }),
            tab: None,
            text,
            when: pb::type_request::When::Now as i32,
        };
        match remote.type_text(&machine, request).await {
            Ok(typed) if typed.get_ref().typed => CommentsTyped::Yes,
            Ok(_) => CommentsTyped::Unknown(status::internal(
                format!("{machine} took the comments for session {} but didn't say it typed them", record.id),
                "look at the agent's terminal; send the comments again if they aren't there",
            )),
            // Refused before anything was typed.
            Err(refused)
                if matches!(
                    refused.code(),
                    tonic::Code::NotFound
                        | tonic::Code::FailedPrecondition
                        | tonic::Code::PermissionDenied
                        | tonic::Code::InvalidArgument
                ) =>
            {
                CommentsTyped::No(refused)
            }
            Err(failed) => CommentsTyped::Unknown(failed),
        }
    }

    /// Plain typing through the tab's holder link.
    async fn type_plain(&self, tab: &TabRecord, text: &[u8]) -> CommentsTyped {
        let no_log = Path::new("");
        let link = match self.holders.link(&tab.id, &tab.holder_socket).await {
            Ok(link) => link,
            Err(error) => {
                return CommentsTyped::No(holder_failed("reach the tab's shell", &error, no_log));
            }
        };
        crate::read_marks::tab_typed(tab);
        match tokio::time::timeout(TYPE_TIMEOUT, link.type_text(text)).await {
            Ok(Ok(())) => CommentsTyped::Yes,
            Ok(Err(TypeFailed::Exited)) => CommentsTyped::No(status::not_found(
                pb::EntityKind::Tab,
                &tab.id,
                format!("tab {}'s shell exited before the comments reached it", tab.id),
                "start the session's agent again, then send the comments",
            )),
            Ok(Err(failed @ TypeFailed::Refused { .. })) => {
                CommentsTyped::No(status::internal(status::chain(&failed), ""))
            }
            Ok(Err(TypeFailed::Lost(error))) => {
                CommentsTyped::Unknown(holder_failed("type the comments into the tab", &error, no_log))
            }
            Err(_) => {
                // A late answer must not reach the next call.
                self.holders.forget(&tab.id);
                CommentsTyped::Unknown(status::internal(
                    format!(
                        "tab {}'s shell didn't take the comments within {} seconds; some of them may have been typed",
                        tab.id,
                        TYPE_TIMEOUT.as_secs()
                    ),
                    "",
                ))
            }
        }
    }
}

impl Core {
    /// Links `tab`'s holder so its screen runs, trying again with a growing
    /// pause until it connects, the tab is gone, or about half a minute
    /// passed. A later call that needs the link starts it then.
    async fn start_screen_when_up(self: Arc<Self>, tab: TabRecord) {
        let mut pause = SCREEN_RETRY_FIRST;
        for _ in 0..SCREEN_RETRIES {
            let linked = tokio::time::timeout(
                PROBE_TIMEOUT,
                self.holders.link(&tab.id, &tab.holder_socket),
            )
            .await;
            if matches!(linked, Ok(Ok(_))) {
                return;
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 2).min(SCREEN_RETRY_LAST);
            match self.find_tab(&tab.id).await {
                Ok(Some(_)) => {}
                _ => return,
            }
        }
        crate::log::say!(
            "humatad2: tab {}'s screen didn't start at boot; it starts when the tab is next used",
            tab.id
        );
    }

    /// Drops the in-memory link and writer of `tab` once its holder has been
    /// shut down (D182.4), so nothing reuses a connection to a gone holder.
    pub(crate) fn forget_tab(&self, tab: &str) {
        self.holders.forget(tab);
        self.writers.forget(tab);
    }

    async fn find_session(&self, id: &str) -> Result<Option<SessionRecord>, Status> {
        let store = self.config.store.clone();
        let key = id.to_owned();
        match blocking(move || store.session(&key)).await? {
            Ok(found) => Ok(found),
            Err(StoreError::InvalidId { .. }) => Ok(None),
            Err(error) => Err(status::store(
                error,
                &self.config.machine,
                "look up the session",
            )),
        }
    }

    /// Whether `session`'s agent is mid-turn: WORKING or NEEDS_INPUT (D775).
    /// Unlike the desk's move and rename guards, PROBABLY_IDLE is not busy.
    async fn agent_busy(&self, session: &str) -> Result<bool, Status> {
        let store = self.config.store.clone();
        let key = session.to_owned();
        let agent = blocking(move || store.agent(&key))
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "read the session's agent"))?;
        Ok(match agent.map(|agent| agent.state) {
            Some(AgentState::Running { holder_pid }) => {
                // PROBABLY_IDLE (output quiet, no hook said the turn ended)
                // is an agent at its prompt, as Copilot sits: not mid-turn.
                matches!(
                    crate::session_service::running_turn(session, holder_pid),
                    pb::SessionStatus::Working | pb::SessionStatus::NeedsInput
                )
            }
            _ => false,
        })
    }

    async fn session(&self, id: &str) -> Result<SessionRecord, Status> {
        self.find_session(id).await?.ok_or_else(|| no_session(id))
    }

    /// The folder of `session`'s clone of `repo`, for a shell started there.
    async fn clone_path(&self, session: &SessionRecord, repo: &pb::RepoRef) -> Result<PathBuf, Status> {
        let store = self.config.store.clone();
        let id = session.id.clone();
        let detail = blocking(move || store.session_detail(&id))
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "look up the session's clones"))?;
        detail
            .into_iter()
            .flat_map(|detail| detail.repos)
            .find(|clone| clone.owner == repo.owner && clone.name == repo.name)
            .map(|clone| clone.path)
            .ok_or_else(|| {
                status::invalid_field(
                    "repo",
                    Problem::UnknownValue,
                    format!("session {} has no clone of {}/{}", session.id, repo.owner, repo.name),
                    "pick one of the session's repositories, or leave repo unset to start in the session folder",
                )
            })
    }

    /// `asked` made canonical, when it is a folder inside one of `session`'s
    /// places (PL-10): its own folder, its clones, its channel's folder, or the
    /// place files from other machines land.
    async fn checked_folder(&self, session: &SessionRecord, asked: &str) -> Result<PathBuf, Status> {
        let store = self.config.store.clone();
        let record = session.clone();
        let places = blocking(move || session_places(&store, &record))
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "look up the session's folders"))?;
        let asked = asked.to_owned();
        blocking(move || resolve_folder(&asked, &places)).await?
    }

    async fn find_tab(&self, id: &str) -> Result<Option<TabRecord>, Status> {
        let store = self.config.store.clone();
        let key = id.to_owned();
        match blocking(move || store.tab(&key)).await? {
            Ok(found) => Ok(found),
            Err(StoreError::InvalidId { .. }) => Ok(None),
            Err(error) => Err(status::store(
                error,
                &self.config.machine,
                "look up the tab",
            )),
        }
    }

    /// `tab` as it stands once this start has brought it back. A holder
    /// that died with the last run is started again by the restart, in the
    /// background: until then nothing answers on its socket, yet the tab
    /// isn't gone. So when the start isn't done and the holder doesn't
    /// answer, this waits for the start and reads the tab again.
    async fn tab_once_brought_back(
        &self,
        tab: TabRecord,
        session: Option<&str>,
    ) -> Result<TabRecord, Status> {
        if self.config.started.get().is_some()
            || self.holders.link(&tab.id, &tab.holder_socket).await.is_ok()
        {
            return Ok(tab);
        }
        self.config
            .started
            .wait_for(crate::restart::Awaited::Attach)
            .await?;
        self.tab(&tab.id, session).await
    }

    async fn tab(&self, id: &str, session: Option<&str>) -> Result<TabRecord, Status> {
        match self.find_tab(id).await? {
            Some(tab) if session.is_none_or(|session| tab.session.as_deref() == Some(session)) => {
                Ok(tab)
            }
            _ => Err(no_tab(id)),
        }
    }

    async fn tabs(&self) -> Result<Vec<TabRecord>, Status> {
        let store = self.config.store.clone();
        blocking(move || store.tabs())
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "list the tabs"))
    }

    /// What a call on `record` is checked against.
    fn session_target(&self, record: &SessionRecord) -> Target {
        Target::Session(identity::session_ref(record, &self.config.machine))
    }

    /// The machine `record`'s session lives on and the way to it, when it isn't this
    /// one and this daemon works with other machines (D139). Callers branch to it only
    /// after their own access check: the machine checks nothing more (D144).
    fn elsewhere(&self, record: &SessionRecord) -> Option<(String, &RemoteTerminal)> {
        let remote = self.config.remote.as_ref()?;
        (!record.machine.is_empty() && record.machine != self.config.machine)
            .then(|| (record.machine.clone(), remote))
    }

    /// For the calls the machine answers whole: the machine and relay of `session` when it
    /// lives elsewhere, once the caller may type into it. None for a session here.
    async fn remote_input(
        &self,
        caller: &Caller,
        session: Option<pb::SessionId>,
    ) -> Result<Option<(String, RemoteTerminal)>, Status> {
        let Some(id) = id_of(session.map(|session| session.value)) else {
            return Ok(None);
        };
        let Some(record) = self.find_session(&id).await? else {
            return Ok(None);
        };
        let Some((machine, remote)) = self.elsewhere(&record) else {
            return Ok(None);
        };
        self.check_input(caller, self.session_target(&record))?;
        self.refuse_if_machine_gone(&record).await?;
        Ok(Some((machine, remote.clone())))
    }

    /// What typing into `tab` is checked against: its session, or for a
    /// machine shell (or a tab whose session is gone) the whole machine.
    async fn tab_target(&self, tab: &TabRecord) -> Result<Target, Status> {
        let Some(session) = &tab.session else {
            return Ok(Target::Global);
        };
        Ok(match self.find_session(session).await? {
            Some(record) => self.session_target(&record),
            None => Target::Global,
        })
    }

    /// Checks that `caller` may type into `target`: the user only.
    fn check_input(&self, caller: &Caller, target: Target) -> Result<(), Status> {
        self.config.guard.require(
            caller,
            &Call::Use {
                capability: Capability::TerminalInput,
                target,
            },
        )
    }

    /// `Some` when the rule for typing into `target` says Ask; an `approved`
    /// call goes on.
    fn gate_input(
        &self,
        caller: &Caller,
        target: Target,
        approved: bool,
    ) -> Result<Option<Box<AccessRequest>>, Status> {
        let call = Call::Use {
            capability: Capability::TerminalInput,
            target,
        };
        gate(&self.config.guard, caller, &call, approved)
    }

    /// The tab text typed into a session goes to: its agent's, else its
    /// first shell.
    async fn typing_tab(&self, session: &str) -> Result<TabRecord, Status> {
        let mut tabs: Vec<TabRecord> = self
            .tabs()
            .await?
            .into_iter()
            .filter(|tab| tab.session.as_deref() == Some(session))
            .collect();
        tabs.sort_by_key(|tab| (tab.kind != TabKind::Agent, tab.created_unix_ms));
        tabs.into_iter().next().ok_or_else(|| {
            status::not_found(
                pb::EntityKind::Tab,
                "",
                format!("session {session} has no open tab to type into"),
                "open a shell tab in the session, or start its agent, then type again",
            )
        })
    }

    /// A tab as the API shows it, with the offsets its holder link last
    /// heard.
    async fn describe(&self, tab: &TabRecord) -> pb::Tab {
        let link = tokio::time::timeout(
            PROBE_TIMEOUT,
            self.holders.link(&tab.id, &tab.holder_socket),
        )
        .await;
        let (earliest_offset, end_offset) = match link {
            Ok(Ok(link)) => {
                let offsets = link.offsets();
                (offsets.earliest, offsets.end)
            }
            _ => (0, 0),
        };
        let program = {
            let (store, id) = (self.config.store.clone(), tab.id.clone());
            blocking(move || store.tab_program(&id))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default()
        };
        pb::Tab {
            id: Some(pb::TabId {
                value: tab.id.clone(),
            }),
            session: tab.session.clone().map(|value| pb::SessionId { value }),
            program_title: Some(program.title).filter(|title| !title.is_empty()),
            pwd: Some(program.pwd).filter(|pwd| !pwd.is_empty()),
            kind: match tab.kind {
                TabKind::Agent => pb::tab::Kind::Agent,
                TabKind::Shell => pb::tab::Kind::Shell,
            } as i32,
            title: tab.title.clone(),
            end_offset,
            earliest_offset,
            machine: tab.session.is_none().then(|| pb::MachineId {
                value: self.config.machine.clone(),
            }),
        }
    }

    /// Starts a shell in `cwd` under a holder of its own and saves its tab.
    /// A session's shell gets a session token of its own, so what runs in
    /// it is that session's agent to the daemon.
    async fn open_shell(
        &self,
        session: Option<&SessionRecord>,
        cwd: PathBuf,
    ) -> Result<pb::Tab, Status> {
        let config = &self.config;
        let tab = uuid::Uuid::new_v4();
        let id = format!("tb_{}", tab.simple());
        let socket = holder_socket(&config.holders, tab);
        let log = config.logs.join(format!("{id}.log"));
        let mut env = Vec::new();
        if let Some(record) = session {
            let identities = config.identities.clone();
            let (tab, record) = (id.clone(), record.clone());
            let token = blocking(move || identities.mint(&tab, &record))
                .await?
                .map_err(|error| {
                    status::store(error, &config.machine, "save the tab's session token")
                })?;
            env = identity::tab_env(token, &config.socket);
        }
        let spec = HolderSpec {
            holder: config.holder.clone(),
            socket: socket.clone(),
            program: config.shell.clone(),
            args: config.shell_args.clone(),
            cwd,
            env,
            cols: INITIAL_COLS,
            rows: INITIAL_ROWS,
            scrollback_bytes: config.scrollback_bytes,
            log: log.clone(),
        };
        let (builds, shell) = (config.builds.clone(), id.clone());
        blocking(move || builds.record_shell(&shell)).await?;
        let spawned = match blocking(move || humata2_holder::spawn(&spec)).await? {
            Ok(spawned) => spawned,
            Err(error) => {
                self.revoke(&id).await;
                return Err(holder_failed("start the tab's shell", &error, &log));
            }
        };
        let tab = TabRecord {
            id,
            session: session.map(|record| record.id.clone()),
            kind: TabKind::Shell,
            title: config
                .shell
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "shell".to_owned()),
            holder_socket: socket,
            holder_pid: spawned.pid,
            created_unix_ms: unix_ms(),
            holder_generation: 0,
        };
        // A new shell has answered nothing, so its screen answers every
        // query it prints, from the first byte, whenever its link starts.
        // Saved with the tab, so no link finds the tab without it.
        let store = config.store.clone();
        let saved = tab.clone();
        let first = TabReplies {
            generation: tab.holder_generation,
            ..TabReplies::default()
        };
        if let Err(error) = blocking(move || store.put_tab_with_replies(&saved, &first)).await? {
            // Nothing could find the shell again, so it doesn't stay.
            if let Err(error) = holder_end::shut_or_end(
                &tab.holder_socket,
                tab.holder_pid,
                &format!("new tab {}", tab.id),
            )
            .await
            {
                crate::log::say!("humatad2: {error}");
            }
            self.revoke(&tab.id).await;
            return Err(status::store(error, &config.machine, "save the new tab"));
        }
        // The screen runs from now on, watched or not. Describing the tab
        // below links it; a failure is tried again by whatever uses it.
        let described = self.describe(&tab).await;
        if let Some(session) = &tab.session {
            self.config.tell.session(session, pb::Change::Updated, None).await;
        }
        Ok(described)
    }

    /// Ends `tab`'s session token. A failure to save that is left for the
    /// next start, which drops tokens whose tab is gone.
    async fn revoke(&self, tab: &str) {
        let identities = self.config.identities.clone();
        let tab = tab.to_owned();
        let _ = blocking(move || identities.revoke(&tab)).await;
    }

    /// Types `text` into `tab`'s terminal through its holder link and
    /// waits until all of it is there. A failure says whether some of the
    /// text may have been typed.
    async fn type_into(&self, tab: &TabRecord, text: &[u8]) -> Result<(), TypeFailure> {
        let no_log = Path::new("");
        let link = self
            .holders
            .link(&tab.id, &tab.holder_socket)
            .await
            .map_err(|error| {
                TypeFailure::Untyped(holder_failed("reach the tab's shell", &error, no_log))
            })?;
        crate::read_marks::tab_typed(tab);
        let typed = match tokio::time::timeout(TYPE_TIMEOUT, link.type_text(text)).await {
            Ok(typed) => typed,
            Err(_) => {
                // A late answer must not reach the next call: it gets a
                // connection of its own.
                self.holders.forget(&tab.id);
                return Err(TypeFailure::MaybeTyped(status::internal(
                    format!(
                        "tab {}'s shell didn't take the text within {} seconds; some of it may have been typed",
                        tab.id,
                        TYPE_TIMEOUT.as_secs()
                    ),
                    "",
                )));
            }
        };
        typed.map_err(|failed| match failed {
            // The link failed part way: some of the text may be there.
            TypeFailed::Lost(error) => {
                TypeFailure::MaybeTyped(holder_failed("type into the tab", &error, no_log))
            }
            TypeFailed::Refused { .. } => {
                TypeFailure::Untyped(status::internal(status::chain(&failed), ""))
            }
            TypeFailed::Exited => TypeFailure::Untyped(status::not_found(
                pb::EntityKind::Tab,
                &tab.id,
                format!("tab {}'s shell exited before the text reached it", tab.id),
                "open a new tab in the session and type again",
            )),
        })
    }

    /// After `body` was typed, presses return in `tab` as a write of its own
    /// when `submit`, once `SUBMIT_PAUSE` has passed. The text is already
    /// there if the return fails, so that failure is a maybe-typed one.
    ///
    /// With `keyed` (the session, the call's key and the holder an earlier
    /// call with it went to), the return is a step of its own under the key
    /// `<key>/submit`, beside the body's step, so a retry whose body arrived
    /// but whose return didn't sends only the return, and one whose both
    /// arrived sends nothing.
    async fn then_submit(
        &self,
        tab: &TabRecord,
        body: Result<(), TypeFailure>,
        submit: bool,
        keyed: Option<(&str, &str, Option<u64>)>,
    ) -> Result<(), TypeFailure> {
        body?;
        if !submit {
            return Ok(());
        }
        let Some((session, key, retry_of)) = keyed else {
            tokio::time::sleep(SUBMIT_PAUSE).await;
            return self.submit_return(tab).await;
        };
        let held = holder_key(&submit_step_key(key));
        let asked = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::key_arrived(&tab.holder_socket, &held),
        )
        .await;
        let maybe = |status: Status| TypeFailure::MaybeTyped(status);
        let state = match asked {
            Ok(Ok(state)) => state,
            Ok(Err(error @ HolderError::Outdated { .. })) => {
                return Err(maybe(outdated_holder(&error)));
            }
            // Nothing is known about the return: some of the call's text is
            // already in the tab, so it is maybe-typed either way.
            Ok(Err(error)) => {
                return Err(maybe(holder_failed("reach the tab's shell", &error, Path::new(""))));
            }
            Err(_) => return Err(maybe(maybe_typed_before(session))),
        };
        match submit_step(state, retry_of) {
            SubmitStep::Skip => return Ok(()),
            SubmitStep::Unknown => return Err(maybe(maybe_typed_before(session))),
            SubmitStep::Send => {}
        }
        if let (KeyState::NotArrived { holder }, None) = (state, retry_of) {
            // A call with no text of its own typed through no holder yet.
            self.note_typed_key_holder(key, holder).await;
        }
        tokio::time::sleep(SUBMIT_PAUSE).await;
        let sent = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::keyed_input(&tab.holder_socket, &held, b"\r"),
        )
        .await;
        match sent {
            Ok(Ok(KeyedInput::Written | KeyedInput::Repeat)) => Ok(()),
            Ok(Err(error @ HolderError::Outdated { .. })) => {
                Err(maybe(outdated_holder(&error)))
            }
            Ok(Err(error)) => Err(maybe(holder_failed(
                "press return in the tab",
                &error,
                Path::new(""),
            ))),
            Err(_) => Err(maybe(maybe_typed_before(session))),
        }
    }

    /// Presses return in `tab` as a plain write.
    async fn submit_return(&self, tab: &TabRecord) -> Result<(), TypeFailure> {
        self.type_into(tab, b"\r").await.map_err(|failure| match failure {
            TypeFailure::Untyped(status) | TypeFailure::MaybeTyped(status) => {
                TypeFailure::MaybeTyped(status)
            }
        })
    }

    /// D426: types `text` into `tab` once for `key` through its holder,
    /// which remembers the keys it typed, and notes that holder with the
    /// key so a retry can ask it. `retry_of` is the holder an earlier call
    /// with the key went to: only its answer counts, since a holder started
    /// again has a new terminal and an empty memory. A holder from a build
    /// before keyed input types nothing and is refused as outdated (D753);
    /// it keeps running until its session is migrated.
    async fn type_keyed(
        &self,
        tab: &TabRecord,
        session: &str,
        key: &str,
        retry_of: Option<u64>,
        text: &[u8],
    ) -> Result<(), TypeFailure> {
        let no_log = Path::new("");
        let held = holder_key(key);
        let asked = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::key_arrived(&tab.holder_socket, &held),
        )
        .await;
        let holder = match (asked, retry_of) {
            // The earlier call's text is there, all of it.
            (Ok(Ok(KeyState::Arrived { holder })), Some(first)) if holder == first => return Ok(()),
            // None of it is: it is typed now.
            (Ok(Ok(KeyState::NotArrived { holder })), Some(first)) if holder == first => holder,
            // Another holder, or none that answers, proves nothing.
            (_, Some(_)) => return Err(TypeFailure::MaybeTyped(maybe_typed_before(session))),
            // This holder typed it for a key the store no longer had.
            (Ok(Ok(KeyState::Arrived { .. })), None) => return Ok(()),
            (Ok(Ok(KeyState::NotArrived { holder })), None) => holder,
            (Ok(Err(error @ HolderError::Outdated { .. })), None) => {
                return Err(TypeFailure::Untyped(outdated_holder(&error)));
            }
            (Ok(Err(error)), None) => {
                return Err(TypeFailure::Untyped(holder_failed(
                    "reach the tab's shell",
                    &error,
                    no_log,
                )));
            }
            (Err(_), None) => {
                return Err(TypeFailure::Untyped(status::internal(
                    format!(
                        "tab {}'s shell didn't answer within {} seconds, so nothing was typed. Try again once the session responds",
                        tab.id,
                        TYPE_TIMEOUT.as_secs()
                    ),
                    "",
                )));
            }
        };
        if retry_of.is_none() {
            self.note_typed_key_holder(key, holder).await;
        }
        crate::fault::point("terminal.type.held");
        let typed = tokio::time::timeout(
            TYPE_TIMEOUT,
            humata2_holder::keyed_input(&tab.holder_socket, &held, text),
        )
        .await;
        match typed {
            Ok(Ok(KeyedInput::Written | KeyedInput::Repeat)) => Ok(()),
            // Started again from an older build since it was asked: nothing
            // was typed.
            Ok(Err(error @ HolderError::Outdated { .. })) => {
                Err(TypeFailure::Untyped(outdated_holder(&error)))
            }
            // Failed before anything could be typed.
            Ok(Err(
                error @ (HolderError::Connect { .. }
                | HolderError::KeyTooLong { .. }
                | HolderError::KeyedInputTooLarge { .. }),
            )) => Err(TypeFailure::Untyped(holder_failed("type into the tab", &error, no_log))),
            Ok(Err(error)) => Err(TypeFailure::MaybeTyped(holder_failed(
                "type into the tab",
                &error,
                no_log,
            ))),
            Err(_) => Err(TypeFailure::MaybeTyped(status::internal(
                format!(
                    "tab {}'s shell didn't take the text within {} seconds; it may have been typed. Retry with the same key to find out, which never types it twice",
                    tab.id,
                    TYPE_TIMEOUT.as_secs()
                ),
                "",
            ))),
        }
    }

    /// Notes the holder the call holding `key` types through (D426). If
    /// that can't be saved, a retry says the text may have been typed.
    async fn note_typed_key_holder(&self, key: &str, holder: u64) {
        let store = self.config.store.clone();
        let key = key.to_owned();
        if let Ok(Err(error)) = blocking(move || store.note_typed_key_holder(&key, holder)).await {
            crate::log::say!("humatad2: couldn't note which shell a Type call types into: {error}");
        }
    }

    /// Notes Type key `key` before its text is typed, and says where an
    /// earlier call with it stands.
    async fn claim_typed_key(&self, key: &str) -> Result<TypedKey, Status> {
        let store = self.config.store.clone();
        let owned = key.to_owned();
        blocking(move || store.claim_typed_key(&owned, unix_ms()))
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "note the Type call's key"))
    }

    /// Notes that the call holding `key` typed all of its text. If that
    /// can't be saved, the key stays pending, and a retry says the text
    /// may have been typed rather than typing it again.
    async fn finish_typed_key(&self, key: String) {
        let store = self.config.store.clone();
        if let Ok(Err(error)) = blocking(move || store.finish_typed_key(&key)).await {
            crate::log::say!("humatad2: couldn't note a Type call as typed: {error}");
        }
    }

    /// Forgets `key`, for a call that typed none of its text. If that can't
    /// be saved, a retry says the text may have been typed: it never types
    /// twice.
    async fn release_typed_key(&self, key: String) {
        let store = self.config.store.clone();
        if let Ok(Err(error)) = blocking(move || store.release_typed_key(&key)).await {
            crate::log::say!("humatad2: couldn't forget an untyped Type call's key: {error}");
        }
    }

    fn recall(&self, key: Option<&str>) -> Option<Outcome> {
        let key = key?;
        lock(&self.remembered).results.get(key).cloned()
    }

    fn remember(&self, key: Option<String>, outcome: Outcome) {
        let Some(key) = key else { return };
        let mut remembered = lock(&self.remembered);
        if remembered.results.insert(key.clone(), outcome).is_none() {
            remembered.order.push_back(key);
        }
        while remembered.order.len() > REMEMBERED_MAX {
            if let Some(oldest) = remembered.order.pop_front() {
                remembered.results.remove(&oldest);
            }
        }
    }

    async fn tabs_listing(
        &self,
        caller: &Caller,
        request: pb::ListTabsRequest,
        approved: bool,
    ) -> Result<Gated<pb::ListTabsResponse>, Status> {
        let session = id_of(request.session.clone().map(|session| session.value));
        match &session {
            Some(session) => {
                let record = self.session(session).await?;
                let read = Call::ReadSession(identity::session_ref(&record, &self.config.machine));
                if let Some(asked) = gate(&self.config.guard, caller, &read, approved)? {
                    return Ok(Gated::Ask(asked));
                }
                if let Some((machine, remote)) = self.elsewhere(&record) {
                    return Ok(Gated::Done(remote.list_tabs(&machine, request).await?.into_inner()));
                }
            }
            None => {
                if let Some(asked) = self.gate_input(caller, Target::Global, approved)? {
                    return Ok(Gated::Ask(asked));
                }
            }
        }
        let mut tabs = Vec::new();
        for tab in self.tabs().await? {
            if tab.session == session {
                tabs.push(self.describe(&tab).await);
            }
        }
        Ok(Gated::Done(pb::ListTabsResponse { tabs }))
    }

    async fn shell_in_session(
        &self,
        caller: &Caller,
        request: pb::OpenShellTabRequest,
        approved: bool,
    ) -> Result<Gated<pb::Tab>, Status> {
        let key = idempotency("OpenShellTab", request.idempotency_key.clone());
        let session = id_of(request.session.clone().map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        let session = self.session(&session).await?;
        // A closed Desk session gets no new tab until Reopen (DK-11).
        desk_service::refuse_if_closed(&self.config.store, &self.config.machine, &session.id).await?;
        if let Some(asked) = self.gate_input(caller, self.session_target(&session), approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some((machine, remote)) = self.elsewhere(&session) {
            return Ok(Gated::Done(remote.open_shell_tab(&machine, request).await?.into_inner()));
        }
        if let Some(Outcome::Tab(tab)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(tab));
        }
        // PL-10: a folder asked for wins over the repo; it is checked here, on
        // the session's own machine, so a session elsewhere is judged by its
        // machine's daemon once the request is forwarded.
        let folder = if !request.folder.is_empty() {
            self.checked_folder(&session, &request.folder).await?
        } else {
            match &request.repo {
                None => session.folder.clone(),
                Some(repo) => self.clone_path(&session, repo).await?,
            }
        };
        let tab = self.open_shell(Some(&session), folder).await?;
        self.remember(key, Outcome::Tab(tab.clone()));
        Ok(Gated::Done(tab))
    }

    async fn shell_on_machine(
        &self,
        caller: &Caller,
        request: pb::OpenMachineShellRequest,
        approved: bool,
    ) -> Result<Gated<pb::Tab>, Status> {
        if let Some(asked) = self.gate_input(caller, Target::Global, approved)? {
            return Ok(Gated::Ask(asked));
        }
        let key = idempotency("OpenMachineShell", request.idempotency_key.clone());
        if let Some(remote) = &self.config.remote
            && let Some(machine) = id_of(request.machine.clone().map(|machine| machine.value))
            && machine != self.config.machine
        {
            return Ok(Gated::Done(remote.open_machine_shell(&machine, request).await?.into_inner()));
        }
        if let Some(Outcome::Tab(tab)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(tab));
        }
        if let Some(machine) = id_of(request.machine.map(|machine| machine.value))
            && machine != self.config.machine
        {
            return Err(status::not_found(
                pb::EntityKind::Machine,
                &machine,
                format!(
                    "machine {machine} isn't this one, and this daemon can't open shells elsewhere yet"
                ),
                "open the shell through that machine's own daemon",
            ));
        }
        let tab = self.open_shell(None, self.config.root.clone()).await?;
        self.remember(key, Outcome::Tab(tab.clone()));
        Ok(Gated::Done(tab))
    }

    async fn close_one(
        &self,
        caller: &Caller,
        request: pb::CloseTabRequest,
        approved: bool,
    ) -> Result<Gated<pb::CloseTabResponse>, Status> {
        let key = idempotency("CloseTab", request.idempotency_key.clone());
        let id = id_of(request.tab.clone().map(|tab| tab.value)).ok_or_else(|| status::missing("tab"))?;
        // A tab of a session on another machine is that machine's; the session is what
        // the access check is for.
        let remote_session = match id_of(request.session.clone().map(|session| session.value)) {
            Some(session) => self
                .find_session(&session)
                .await?
                .filter(|record| self.elsewhere(record).is_some()),
            None => None,
        };
        let target = match &remote_session {
            Some(record) => self.session_target(record),
            None => match self.find_tab(&id).await? {
                Some(tab) => self.tab_target(&tab).await?,
                None => Target::Global,
            },
        };
        if let Some(asked) = self.gate_input(caller, target, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(record) = &remote_session
            && let Some((machine, remote)) = self.elsewhere(record)
        {
            return Ok(Gated::Done(remote.close_tab(&machine, request).await?.into_inner()));
        }
        if let Some(Outcome::Closed) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(pb::CloseTabResponse {}));
        }
        let session = id_of(request.session.map(|session| session.value));
        let tab = self.tab(&id, session.as_deref()).await?;
        if tab.kind == TabKind::Agent {
            return Err(status::invalid_field(
                "tab",
                Problem::NotAllowedHere,
                format!("tab {id} is the session's agent, which can't be closed on its own"),
                "stop the session instead; that ends the agent",
            ));
        }
        // A holder that's gone already, or doesn't answer, is ended by its pid.
        if let Err(error) = holder_end::shut_or_end(
            &tab.holder_socket,
            tab.holder_pid,
            &format!("tab {}", tab.id),
        )
        .await
        {
            return Err(status::internal(status::chain(&error), ""));
        }
        let store = self.config.store.clone();
        blocking(move || store.remove_tab(&id))
            .await?
            .map_err(|error| status::store(error, &self.config.machine, "forget the closed tab"))?;
        self.writers.forget(&tab.id);
        self.holders.forget(&tab.id);
        self.revoke(&tab.id).await;
        if let Some(session) = &tab.session {
            self.config.tell.session(session, pb::Change::Updated, None).await;
        }
        self.remember(key, Outcome::Closed);
        Ok(Gated::Done(pb::CloseTabResponse {}))
    }

    /// MACHINESCRIPTS (D682): a `send` step types `text` into `session` as the host's own call, once for
    /// `key`, then presses return when `enter`. Never asks.
    pub(crate) async fn type_for_script(&self, session: &str, text: &str, enter: bool, key: &str) -> Result<(), Status> {
        let caller = Caller::User(identity::UserVia::Host);
        crate::stop::gate::require_running(&caller)?;
        // A trailing newline is the submit (D671): the Type call presses
        // return once the agent has shown the text (BUG-T1).
        let text = if enter { format!("{text}\n") } else { text.to_owned() };
        let request = pb::TypeRequest {
            idempotency_key: Some(pb::IdempotencyKey { value: format!("{key}/text") }),
            session: Some(pb::SessionId { value: session.to_owned() }),
            text,
            ..pb::TypeRequest::default()
        };
        match self.type_in(&caller, request, true).await? {
            Gated::Done(_) => Ok(()),
            Gated::Ask(_) => Err(status::internal(
                "typing for a machine script was sent to Ask me, which an approved call never is",
                "",
            )),
        }
    }

    async fn type_in(
        &self,
        caller: &Caller,
        request: pb::TypeRequest,
        approved: bool,
    ) -> Result<Gated<pb::TypeResponse>, Status> {
        let key = idempotency("Type", request.idempotency_key.clone());
        let session = id_of(request.session.clone().map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        let session = self.session(&session).await?;
        // D783: nothing is relayed for a session whose machine is gone; a
        // machine added later under its name doesn't have it.
        if self.elsewhere(&session).is_some() {
            self.refuse_if_machine_gone(&session).await?;
        }
        // UP-1, D281 e: nothing is typed into a session on a machine at another release.
        crate::upgrade::gate::machine_ready(
            &self.config.migrations,
            &pb::MachineId {
                value: session.machine.clone(),
            },
        )?;
        if let Some(asked) = gate(
            &self.config.guard,
            caller,
            &Call::Use {
                capability: Capability::SessionType,
                target: self.session_target(&session),
            },
            approved,
        )? {
            return Ok(Gated::Ask(asked));
        }
        // MV-4: a session that moves takes no input until its agent resumes.
        // Refused here, before the relay to a box, which never learns of the move.
        crate::session_service::refuse_input_while_moving(&session.id)?;
        if let Some((machine, remote)) = self.elsewhere(&session) {
            return Ok(Gated::Done(remote.type_text(&machine, request).await?.into_inner()));
        }
        if let Some(Outcome::Typed(typed)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(typed));
        }
        if request.when == pb::type_request::When::TurnEnds as i32 {
            return Err(status::invalid_field(
                "when",
                pb::invalid_field::Problem::NotAllowedHere,
                "typing at the end of the turn isn't something the daemon does; only the app holds text until a turn ends",
                "type now with when=now (or leave when out), or ask the user with inbox_ask to send it once your turn is over; updating Humata doesn't add this",
            ));
        }
        // Read before anything is written: the agent's turn as it is when
        // the text arrives.
        let agent_busy = self.agent_busy(&session.id).await?;
        // Only reached once the text and, for a trailing newline, the
        // return are written: a call that fails returns before this is
        // given back.
        let typed = pb::TypeResponse {
            queued: None,
            typed: true,
            turn_end_uncertain: false,
            submitted: split_submit(request.text.as_bytes()).1,
            agent_busy,
        };
        // D337: the key is noted in the store before anything is typed, so
        // a retry this daemon's memory can't answer, as one after a
        // restart, still types the text at most once.
        // D426: a call with the key that never finished typed through the
        // holder noted with it, which remembers its keys: the retry asks it.
        let mut retry_of = None;
        if let Some(key) = key.as_deref() {
            match self.claim_typed_key(key).await? {
                TypedKey::Claimed => {}
                TypedKey::Typed => {
                    self.remember(Some(key.to_owned()), Outcome::Typed(typed.clone()));
                    return Ok(Gated::Done(typed));
                }
                TypedKey::Pending { holder: 0 } => return Err(maybe_typed_before(&session.id)),
                TypedKey::Pending { holder } => retry_of = Some(holder),
            }
        }
        let tab = match id_of(request.tab.map(|tab| tab.value)) {
            Some(tab) => self.tab(&tab, Some(&session.id)).await,
            None => self.typing_tab(&session.id).await,
        };
        // D671 (Provisional): a trailing newline is the submit, sent as its
        // own write after the text has settled in the agent's prompt.
        let (text, submit) = split_submit(request.text.as_bytes());
        // BUG-T1: a return pressed before the program has taken the text
        // reaches it with the text, as part of a paste. A retry's text may
        // be there already, shown before this could watch for it.
        let mut watch = match &tab {
            Ok(tab) if submit && !text.is_empty() && retry_of.is_none() => {
                match ready_for_text(&tab.holder_socket).await {
                    Ok(watch) => watch,
                    Err(refused) => {
                        if let Some(key) = key {
                            self.release_typed_key(key).await;
                        }
                        return Err(refused);
                    }
                }
            }
            _ => None,
        };
        // D762: text typed while the agent asks a question of its own on
        // its screen would go into the question, and its Enter answer it.
        // Enter alone (no text) is the person answering it on purpose.
        // An unreadable screen refuses too: nothing is typed blind.
        let refused = match &tab {
            Ok(tab) if !text.is_empty() && tab.kind == TabKind::Agent => {
                input_gate::guard(&tab.holder_socket).await.err()
            }
            _ => None,
        };
        if let Some(refused) = refused {
            // A retry's text may be there already: its key stays.
            if let (Some(key), None) = (key.clone(), retry_of) {
                self.release_typed_key(key).await;
            }
            return Err(input_gate::refusal(&session.id, &refused));
        }
        let typing = match (tab, key.as_deref()) {
            (Ok(tab), Some(key)) => {
                let body = if text.is_empty() && submit {
                    Ok(())
                } else {
                    self.type_keyed(&tab, &session.id, key, retry_of, text).await
                };
                shown(&mut watch, &body, text).await;
                self.then_submit(&tab, body, submit, Some((&session.id, key, retry_of))).await
            }
            (Ok(tab), None) => {
                let body = if text.is_empty() && submit {
                    Ok(())
                } else {
                    self.type_into(&tab, text).await
                };
                shown(&mut watch, &body, text).await;
                self.then_submit(&tab, body, submit, None).await
            }
            // The earlier call's text may be in a tab this can't find now.
            (Err(status), _) if retry_of.is_some() => Err(TypeFailure::MaybeTyped(status)),
            (Err(status), _) => Err(TypeFailure::Untyped(status)),
        };
        match typing {
            Ok(()) => {
                crate::fault::point("terminal.type.typed");
                if let Some(key) = key.clone() {
                    self.finish_typed_key(key).await;
                }
            }
            Err(TypeFailure::Untyped(status)) => {
                if let Some(key) = key {
                    self.release_typed_key(key).await;
                }
                return Err(status);
            }
            // The key stays pending: a retry asks the holder what arrived.
            Err(TypeFailure::MaybeTyped(status)) => return Err(status),
        }
        self.remember(key, Outcome::Typed(typed.clone()));
        Ok(Gated::Done(typed))
    }

    async fn paste_in(
        &self,
        caller: &Caller,
        request: pb::PasteImageRequest,
        approved: bool,
    ) -> Result<Gated<pb::PasteImageResponse>, Status> {
        let key = idempotency("PasteImage", request.idempotency_key.clone());
        let session = id_of(request.session.clone().map(|session| session.value))
            .ok_or_else(|| status::missing("session"))?;
        let record = self.session(&session).await?;
        if let Some(asked) = self.gate_input(caller, self.session_target(&record), approved)? {
            return Ok(Gated::Ask(asked));
        }
        // MV-4: as for Type, nothing is saved into a session that moves.
        crate::session_service::refuse_input_while_moving(&record.id)?;
        if let Some((machine, remote)) = self.elsewhere(&record) {
            return Ok(Gated::Done(remote.paste_image(&machine, request).await?.into_inner()));
        }
        if let Some(Outcome::Paste(pasted)) = self.recall(key.as_deref()) {
            return Ok(Gated::Done(pasted));
        }
        let extension = paste::check(request.format, &request.data)?;
        let size_bytes = request.data.len() as u64;
        let store = self.config.store.clone();
        let folder = session.clone();
        // The key names the file, so a retry after a restart, which this
        // daemon's memory no longer answers, still saves once.
        let named = key.clone();
        let saved = blocking(move || match named {
            Some(key) => store.save_keyed_paste(&folder, &key, extension, &request.data),
            None => store.save_paste(&folder, extension, &request.data),
        })
        .await?;
        let path = match saved {
            Ok(path) => path,
            Err(StoreError::NoSession { .. } | StoreError::InvalidId { .. }) => {
                return Err(no_session(&session));
            }
            Err(error) => {
                return Err(status::store(
                    error,
                    &self.config.machine,
                    "save the pasted image",
                ));
            }
        };
        let Some(path) = path.to_str().map(str::to_owned) else {
            return Err(status::internal(
                format!(
                    "the pasted image was saved at {}, a path that isn't UTF-8",
                    path.display()
                ),
                "",
            ));
        };
        let pasted = pb::PasteImageResponse { path, size_bytes };
        self.remember(key, Outcome::Paste(pasted.clone()));
        Ok(Gated::Done(pasted))
    }
}

#[tonic::async_trait]
impl pb::terminal_service_server::TerminalService for Terminals {
    /// The tabs of `session`, or with no session, this machine's shells.
    async fn list_tabs(
        &self,
        request: Request<pb::ListTabsRequest>,
    ) -> Result<Response<pb::ListTabsResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = None;
        let first = self.tabs_listing(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.ListTabsResponse",
            first,
            move || async move { core.tabs_listing(&who, request, true).await },
        )
        .await
    }

    async fn open_shell_tab(
        &self,
        request: Request<pb::OpenShellTabRequest>,
    ) -> Result<Response<pb::Tab>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.shell_in_session(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.Tab",
            first,
            move || async move { core.shell_in_session(&who, request, true).await },
        )
        .await
    }

    async fn open_machine_shell(
        &self,
        request: Request<pb::OpenMachineShellRequest>,
    ) -> Result<Response<pb::Tab>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.shell_on_machine(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.Tab",
            first,
            move || async move { core.shell_on_machine(&who, request, true).await },
        )
        .await
    }

    async fn close_tab(
        &self,
        request: Request<pb::CloseTabRequest>,
    ) -> Result<Response<pb::CloseTabResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.close_one(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.CloseTabResponse",
            first,
            move || async move { core.close_one(&who, request, true).await },
        )
        .await
    }

    type AttachStream = AttachStream;

    async fn attach(
        &self,
        request: Request<Streaming<pb::AttachClientMessage>>,
    ) -> Result<Response<Self::AttachStream>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let mut incoming = request.into_inner();
        let open = match incoming.message().await? {
            Some(pb::AttachClientMessage {
                message: Some(pb::attach_client_message::Message::Open(open)),
            }) => open,
            _ => {
                return Err(status::invalid_field(
                    "open",
                    Problem::Missing,
                    "an Attach must start with Open",
                    "send AttachOpen as the first message, naming the tab",
                ));
            }
        };
        let id = id_of(open.tab.clone().map(|tab| tab.value))
            .ok_or_else(|| status::missing("open.tab"))?;
        let session = id_of(open.session.clone().map(|session| session.value));
        // A session on another machine: its tab record is there, not here. The host
        // checks the same access for the session, then relays the stream untouched
        // (TR-11); the codec is the machine's pick from what the client accepts.
        if let Some(session) = &session
            && let Some(record) = self.find_session(session).await?
            && let Some((machine, remote)) = self.elsewhere(&record)
        {
            // UP-1, D281 e: a writable attach into a session on a machine at another release is refused.
            if !open.read_only {
                crate::upgrade::gate::machine_ready(
                    &self.config.migrations,
                    &pb::MachineId {
                        value: machine.clone(),
                    },
                )?;
            }
            if open.read_only {
                self.config.guard.require(
                    &caller,
                    &Call::ReadSession(identity::session_ref(&record, &self.config.machine)),
                )?;
            } else {
                self.check_input(&caller, self.session_target(&record))?;
            }
            // D783: a machine added later under its name doesn't have it.
            self.refuse_if_machine_gone(&record).await?;
            return Ok(Response::new(remote.attach(&machine, open, incoming).await?));
        }
        let tab = self.tab(&id, session.as_deref()).await?;
        let tab = self.tab_once_brought_back(tab, session.as_deref()).await?;
        let target = self.tab_target(&tab).await?;
        // UP-1, D281 e: a writable attach into a session on a machine at another release is refused.
        if !open.read_only
            && let Some(session) = &tab.session
            && let Some(record) = self.find_session(session).await?
        {
            crate::upgrade::gate::machine_ready(
                &self.config.migrations,
                &pb::MachineId {
                    value: record.machine,
                },
            )?;
        }
        match (&target, open.read_only) {
            (Target::Session(session), true) => {
                // A stream can't be held and replayed once approved, so an Ask stays refused here.
                self.config
                    .guard
                    .require(&caller, &Call::ReadSession(session.clone()))?;
            }
            // Same: a stream can't wait in the Inbox, so Ask stays refused.
            _ => self.check_input(&caller, target)?,
        }
        let codec = self
            .config
            .compression
            .pick(Link::Local, &open.accept_codecs);
        let stream = attach::start(&tab, &open, incoming, &self.writers, &self.holders, codec).await?;
        Ok(Response::new(stream))
    }

    /// Types text into a session's tab, as if typed at its keyboard: raw,
    /// with no bracketed paste around it.
    async fn r#type(
        &self,
        request: Request<pb::TypeRequest>,
    ) -> Result<Response<pb::TypeResponse>, Status> {
        let caller = identity::caller(&request)?;
        crate::stop::gate::require_running(&caller)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.type_in(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.TypeResponse",
            first,
            move || async move { core.type_in(&who, request, true).await },
        )
        .await
    }

    async fn cancel_queued_typing(
        &self,
        request: Request<pb::CancelQueuedTypingRequest>,
    ) -> Result<Response<pb::CancelQueuedTypingResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        if let Some((machine, remote)) = self.remote_input(&caller, request.session.clone()).await? {
            return remote.cancel_queued_typing(&machine, request).await;
        }
        Err(status::not_built(
            "humata.v1.TerminalService.CancelQueuedTyping",
        ))
    }

    async fn list_queued(
        &self,
        request: Request<pb::ListQueuedRequest>,
    ) -> Result<Response<pb::ListQueuedResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        if let Some((machine, remote)) = self.remote_input(&caller, request.session.clone()).await? {
            return remote.list_queued(&machine, request).await;
        }
        Err(status::not_built("humata.v1.TerminalService.ListQueued"))
    }

    async fn send_queued_now(
        &self,
        request: Request<pb::SendQueuedNowRequest>,
    ) -> Result<Response<pb::SendQueuedNowResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        if let Some((machine, remote)) = self.remote_input(&caller, request.session.clone()).await? {
            if let Some(session) = &request.session {
                // MV-4: as for Type, nothing is sent into a session that moves.
                crate::session_service::refuse_input_while_moving(&session.value)?;
            }
            return remote.send_queued_now(&machine, request).await;
        }
        // Not built: nothing to run once approved, so Ask stays refused.
        self.check_input(&caller, Target::Global)?;
        Err(status::not_built("humata.v1.TerminalService.SendQueuedNow"))
    }

    async fn paste_image(
        &self,
        request: Request<pb::PasteImageRequest>,
    ) -> Result<Response<pb::PasteImageResponse>, Status> {
        let caller = identity::caller(&request)?;
        let request = request.into_inner();
        let key = request
            .idempotency_key
            .as_ref()
            .map(|key| key.value.clone());
        let first = self.paste_in(&caller, request.clone(), false).await;
        let (core, who) = (self.core.clone(), caller.clone());
        answer_gated(
            &self.config.inbox,
            &caller,
            key,
            "humata.v1.PasteImageResponse",
            first,
            move || async move { core.paste_in(&who, request, true).await },
        )
        .await
    }
}

/// What a mutation returned, for a retry with the same key.
#[derive(Clone)]
enum Outcome {
    Tab(pb::Tab),
    Closed,
    Paste(pb::PasteImageResponse),
    Typed(pb::TypeResponse),
}

#[derive(Default)]
struct Remembered {
    results: HashMap<String, Outcome>,
    order: VecDeque<String>,
}

/// How many attaches that can type each tab has (SE-24). Attaches watch
/// their tab's count to say how many others there are.
#[derive(Default)]
pub(crate) struct Writers {
    tabs: Mutex<HashMap<String, watch::Sender<u32>>>,
}

impl Writers {
    fn sender(&self, tab: &str) -> watch::Sender<u32> {
        lock(&self.tabs)
            .entry(tab.to_owned())
            .or_insert_with(|| watch::channel(0).0)
            .clone()
    }

    pub(crate) fn watch(&self, tab: &str) -> watch::Receiver<u32> {
        self.sender(tab).subscribe()
    }

    /// Counts an attach that can type until the guard is dropped.
    pub(crate) fn join(&self, tab: &str) -> WriterGuard {
        let sender = self.sender(tab);
        sender.send_modify(|count| *count += 1);
        WriterGuard { sender }
    }

    fn forget(&self, tab: &str) {
        lock(&self.tabs).remove(tab);
    }
}

pub(crate) struct WriterGuard {
    sender: watch::Sender<u32>,
}

impl Drop for WriterGuard {
    fn drop(&mut self) {
        self.sender
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Runs file work off the async threads.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, Status> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| status::internal(format!("a background task failed: {error}"), ""))
}

/// An id field's value, or `None` when it's unset or empty.
fn id_of(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.is_empty())
}

fn idempotency(method: &str, key: Option<pb::IdempotencyKey>) -> Option<String> {
    id_of(key.map(|key| key.value)).map(|key| format!("{method}/{key}"))
}

/// The key a Type call's return goes to its holder under, a step of the
/// call's key beside the body's own step (which keeps the call's key).
fn submit_step_key(key: &str) -> String {
    format!("{key}/submit")
}

/// What to do about one write of a keyed Type call, given what its holder
/// says about the step's key.
#[derive(Debug, PartialEq, Eq)]
enum SubmitStep {
    /// It arrived before: send nothing.
    Skip,
    /// It didn't: send it.
    Send,
    /// Nothing proves either way: send nothing and say it may have arrived.
    Unknown,
}

/// Whether a step's write goes out. `retry_of` is the holder an earlier call
/// with the key went to: only its answer counts, since a holder started
/// again has a new terminal and an empty memory.
fn submit_step(state: KeyState, retry_of: Option<u64>) -> SubmitStep {
    match (state, retry_of) {
        (KeyState::Arrived { holder }, Some(first)) if holder == first => SubmitStep::Skip,
        (KeyState::NotArrived { holder }, Some(first)) if holder == first => SubmitStep::Send,
        (_, Some(_)) => SubmitStep::Unknown,
        (KeyState::Arrived { .. }, None) => SubmitStep::Skip,
        (KeyState::NotArrived { .. }, None) => SubmitStep::Send,
    }
}

/// The key a Type call's text goes to its holder under: a digest of the
/// call's key, which fits the holder's limit however long the key is.
fn holder_key(key: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(key.as_bytes());
    let hex: String = digest[..16].iter().map(|byte| format!("{byte:02x}")).collect();
    format!("type-{hex}")
}

/// A Type call came with the key of an earlier one that started typing
/// and never finished, as when the daemon stopped part way. Some or all of
/// that text may have reached the shell, so nothing is typed again.
fn maybe_typed_before(session: &str) -> Status {
    status::internal(
        format!(
            "an earlier Type call with this key into session {session} started and didn't finish, so some or all of its text may have been typed; nothing was typed again. Look at the session's terminal, and type what's missing with a new key"
        ),
        "",
    )
}

fn no_session(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Session,
        id,
        format!("no session {id} is on this machine"),
        "check the session id; it may have been removed or be on another machine",
    )
}

pub(crate) fn no_tab(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Tab,
        id,
        format!("no tab {id} is open on this machine"),
        "list the session's tabs for the ones still open",
    )
}

fn holder_failed(doing: &str, error: &HolderError, log: &Path) -> Status {
    status::internal(
        format!("couldn't {doing}: {}", status::chain(error)),
        &log.display().to_string(),
    )
}

fn unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod submit_tests {
    use super::{holder_key, split_submit, submit_step, submit_step_key, SubmitStep};
    use humata2_holder::KeyState;

    #[test]
    fn a_trailing_newline_is_split_off_as_the_submit() {
        for text in [&b"hello\n"[..], b"hello\r", b"hello\r\n"] {
            assert_eq!(split_submit(text), (&b"hello"[..], true));
        }
    }

    #[test]
    fn text_without_a_trailing_newline_is_not_submitted() {
        assert_eq!(split_submit(b"hello"), (&b"hello"[..], false));
        // Newlines inside the text stay in it.
        assert_eq!(split_submit(b"a\nb"), (&b"a\nb"[..], false));
    }

    #[test]
    fn a_lone_newline_is_only_a_submit() {
        assert_eq!(split_submit(b"\n"), (&b""[..], true));
    }

    #[test]
    fn a_retry_whose_return_arrived_sends_nothing() {
        let arrived = KeyState::Arrived { holder: 7 };
        assert_eq!(submit_step(arrived, Some(7)), SubmitStep::Skip);
        assert_eq!(submit_step(arrived, None), SubmitStep::Skip);
    }

    #[test]
    fn a_retry_whose_return_did_not_arrive_sends_it() {
        let missing = KeyState::NotArrived { holder: 7 };
        assert_eq!(submit_step(missing, Some(7)), SubmitStep::Send);
    }

    #[test]
    fn a_first_try_sends_the_return() {
        assert_eq!(submit_step(KeyState::NotArrived { holder: 7 }, None), SubmitStep::Send);
    }

    #[test]
    fn another_holder_proves_nothing_about_a_retry() {
        for state in [
            KeyState::Arrived { holder: 8 },
            KeyState::NotArrived { holder: 8 },
        ] {
            assert_eq!(submit_step(state, Some(7)), SubmitStep::Unknown);
        }
    }

    #[test]
    fn the_return_has_a_step_key_of_its_own() {
        assert_ne!(submit_step_key("k"), "k");
        assert_ne!(holder_key(&submit_step_key("k")), holder_key("k"));
    }
}
