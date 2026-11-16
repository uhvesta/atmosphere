//! Start fresh with a summary (PER-15, D143): a session whose resume failed
//! gets a new conversation, seeded with a plain digest of the old one. It
//! keeps its id, folder, branch and tab. The old conversation is recorded
//! and never deleted, and the new conversation id is only kept once the
//! agent started: if it doesn't, the session is left as it was.
//!
//! The digest needs no model: every user message (each capped), the last
//! few assistant text turns, no tool output, about 8 KiB in all.

use std::io::BufRead;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use humata2_holder::{HolderError, KeyedInput};
use humata2_revive::{ReviveError, launch_agent};
use humata2_sessions::{AgentError, Starter};
use humata2_store::{AgentLaunch, AgentRecord, AgentState, FreshSeed, Store, StoreError, TabRecord};
use serde_json::Value;
use tonic::{Code, Status};

use super::{Gated, Loaded, NoAgent, SessionSvc, Tell, now_ms, resume_failed};
use crate::channel_service::{blocking, error_of, failed};
use crate::operations::{Progress, Started};
use crate::restart::Tokens;
use crate::{identity, status};

/// The most the digest holds.
const DIGEST_MAX: usize = 8 * 1024;
/// The most one user message keeps.
const USER_CAP: usize = 1000;
/// The most one assistant turn keeps.
const ASSISTANT_CAP: usize = 1200;
/// How many of the last assistant text turns the digest holds.
const ASSISTANTS_KEPT: usize = 3;
/// The most of the saved terminal output used when there is no transcript.
const SCROLLBACK_MAX: usize = 4 * 1024;
/// How much saved output is read for it: escape sequences take room.
const SCROLLBACK_READ: usize = 64 * 1024;
/// Between typing the seed and pressing return, so an agent that reads a
/// paste whole doesn't take the return as part of it.
const TYPE_PAUSE: Duration = Duration::from_millis(100);

/// What the seed was made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    /// The old conversation file.
    Transcript,
    /// The tail of the saved terminal output.
    Scrollback,
    /// Nothing: only the title.
    Nothing,
}

#[derive(Debug, thiserror::Error)]
enum StartFreshError {
    #[error(
        "session {session} can't start fresh: its agent is {state}, and only a session whose resume failed can"
    )]
    NotFailed {
        session: String,
        state: &'static str,
    },
    #[error("session {session}'s agent {agent} isn't known")]
    Agent {
        session: String,
        agent: String,
        #[source]
        source: AgentError,
    },
    #[error("couldn't {doing} for session {session}")]
    Store {
        session: String,
        doing: &'static str,
        #[source]
        source: StoreError,
    },
    #[error("couldn't {doing} for session {session}: a background task failed ({message})")]
    Background {
        session: String,
        doing: &'static str,
        message: String,
    },
    #[error("couldn't start session {session}'s agent on a new conversation")]
    Launch {
        session: String,
        #[source]
        source: ReviveError,
    },
    #[error("session {session}'s agent started but the summary couldn't be typed into it")]
    Seed {
        session: String,
        #[source]
        source: HolderError,
    },
    #[error(
        "session {session}'s terminal is from an older build and can't take the summary exactly once"
    )]
    SeedUnsupported { session: String },
}

impl SessionSvc {
    /// `ResumeSession(MODE_START_FRESH_WITH_SUMMARY)`: answers the
    /// operation, which does the work in the background.
    pub(super) async fn start_fresh(
        &self,
        caller: &identity::Caller,
        key: Option<pb::IdempotencyKey>,
        loaded: Loaded,
    ) -> Result<Gated<pb::Operation>, Status> {
        if let Some(operation) = self.operations.existing(caller, key.as_ref())? {
            return Ok(Gated::Done(operation));
        }
        let id = loaded.record.id.clone();
        let Some(agent) = loaded.agent.as_ref() else {
            return Err(failed(
                Code::FailedPrecondition,
                &NoAgent { session: id },
                "start a new session",
                None,
            ));
        };
        match agent.state {
            AgentState::ResumeFailed { reason, exit_code } => {
                use humata2_store::ResumeFailure::{AgentNotInstalled, FolderMissing};
                // Nothing a fresh conversation fixes: the fix is the
                // resume failure's own.
                if matches!(reason, FolderMissing | AgentNotInstalled) {
                    return Err(status::failure(
                        Code::FailedPrecondition,
                        resume_failed(&id, reason, exit_code),
                    ));
                }
            }
            state => {
                return Err(failed(
                    Code::FailedPrecondition,
                    &StartFreshError::NotFailed {
                        session: id,
                        state: state_word(state),
                    },
                    "resume it, or fork it, if its conversation is intact; start fresh is only for a session whose resume failed",
                    None,
                ));
            }
        }
        let steps = [
            (
                "summarize".to_owned(),
                "Summarizing the old conversation".to_owned(),
            ),
            (
                "launch".to_owned(),
                "Starting the agent on a new conversation".to_owned(),
            ),
            (
                "seed".to_owned(),
                "Typing the summary into it".to_owned(),
            ),
        ];
        let target = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(pb::SessionId {
                value: id.clone(),
            })),
        };
        let progress = match self.operations.start_steps(
            caller,
            pb::OperationKind::StartSession,
            target,
            &steps,
            false,
            key,
        )? {
            Started::New(progress) => progress,
            Started::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.operation();
        tokio::spawn(run(
            self.starter.clone(),
            self.tell.clone(),
            identity::actor(caller),
            self.tokens.clone(),
            loaded,
            progress,
        ));
        Ok(Gated::Done(operation))
    }
}

fn state_word(state: AgentState) -> &'static str {
    match state {
        AgentState::Launching => "still starting",
        AgentState::Running { .. } => "running",
        AgentState::Resuming => "being resumed",
        AgentState::Exited { .. } => "exited with its conversation intact",
        AgentState::Stopped => "stopped with its conversation intact",
        AgentState::ResumeFailed { .. } => "failed to resume",
    }
}

/// Does the start fresh for `progress`. A failure before the agent started
/// puts the record back as it was, a failed resume.
async fn run(
    starter: Starter,
    tell: Tell,
    by: pb::Actor,
    tokens: Arc<Tokens>,
    loaded: Loaded,
    progress: Progress,
) {
    let Loaded { record, agent, .. } = loaded;
    let session = record.id.clone();
    let Some(old) = agent else {
        progress.fail(error_of(&NoAgent { session }, "start a new session", None));
        return;
    };
    progress.set_step("summarize", pb::StepState::Running, None);
    // A start fresh whose first message never reached this conversation:
    // that message still says what the session was about, and this
    // conversation has nothing to add.
    let pending = {
        let (store, key) = (starter.store.clone(), session.clone());
        match blocking(move || store.fresh_seed(&key)).await {
            Ok(Ok(found)) => found.filter(|seed| seed.conversation_id == old.launch.conversation_id),
            Ok(Err(error)) => {
                crate::log::say!(
                    "humatad2: couldn't read session {session}'s untyped first message, so start fresh summarizes again: {}",
                    status::chain(&error)
                );
                None
            }
            Err(_) => None,
        }
    };
    let seed = match pending {
        Some(pending) => pending.seed,
        None => {
            let (source, digest) = {
                let (file, socket) = (
                    old.launch.conversation_file.clone(),
                    old.launch.holder_socket.clone(),
                );
                match blocking(move || summarize(&file, &socket)).await {
                    Ok(found) => found,
                    Err(status) => {
                        progress.fail(pb::Error {
                            message: status.message().to_owned(),
                            fix: "check humatad2's log, then start fresh again".to_owned(),
                            retry: None,
                            kind: None,
                        });
                        return;
                    }
                }
            };
            seed_text(
                &record.title,
                source,
                digest.as_deref(),
                &old.launch.conversation_file,
                &old.launch.conversation_id,
            )
        }
    };
    progress.set_step("summarize", pb::StepState::Done, None);

    progress.set_step("launch", pb::StepState::Running, None);
    // Kept before anything changes: the old conversation is never lost.
    let kept = {
        let (store, session, conversation, path) = (
            starter.store.clone(),
            session.clone(),
            old.launch.conversation_id.clone(),
            old.launch.conversation_file.clone(),
        );
        blocking(move || store.push_previous_conversation(&session, &conversation, &path, now_ms()))
            .await
    };
    if let Some(error) = store_failure(kept, &session, "record the old conversation") {
        progress.set_step("launch", pb::StepState::Failed, Some(status::chain(&error)));
        progress.fail(error_of(&error, "check humatad2's log, then start fresh again", None));
        return;
    }
    // Marks it busy, so a second resume or start fresh is refused.
    let mut busy = old.clone();
    busy.state = AgentState::Resuming;
    busy.updated_unix_ms = now_ms();
    let saved = {
        let (store, saving) = (starter.store.clone(), busy);
        blocking(move || store.put_agent(&saving)).await
    };
    if let Some(error) = store_failure(saved, &session, "save that the agent is starting fresh") {
        progress.set_step("launch", pb::StepState::Failed, Some(status::chain(&error)));
        progress.fail(error_of(&error, "check humatad2's log, then start fresh again", None));
        return;
    }
    tell.session(&session, pb::Change::Updated, Some(by.clone()))
        .await;

    let env = |tab: &TabRecord| tokens.env(tab);
    let launched = match fresh_launch(&starter, &record.agent, &old, &session) {
        Ok((launch, marker)) => match keep_seed(&starter.store, &session, &launch, &seed).await {
            Some(error) => Err(error),
            None => launch_agent(
                &starter.store,
                &starter.revive,
                &record,
                &old.tab,
                launch,
                Some(&marker),
                &env,
            )
            .await
            .map_err(|source| StartFreshError::Launch {
                session: session.clone(),
                source,
            }),
        },
        Err(error) => Err(error),
    };
    let agent = match launched {
        Ok(agent) => agent,
        Err(error) => {
            // The new conversation id is dropped: the old one stays pinned,
            // and a message kept for it goes too (one kept for the old
            // conversation stays, for the next try).
            let (store, restoring) = (starter.store.clone(), old.clone());
            let _ = blocking(move || {
                if let Ok(Some(kept)) = store.fresh_seed(&restoring.session)
                    && kept.conversation_id != restoring.launch.conversation_id
                {
                    let _ = store.typed_fresh_seed(&kept.session_id, &kept.conversation_id);
                }
                store.put_agent(&restoring)
            })
            .await;
            progress.set_step("launch", pb::StepState::Failed, Some(status::chain(&error)));
            progress.fail(error_of(
                &error,
                "fix the cause the message names, then start fresh again; the session still holds its old conversation",
                None,
            ));
            tell.session(&session, pb::Change::Updated, Some(by)).await;
            return;
        }
    };
    progress.set_step("launch", pb::StepState::Done, None);
    // A kill here leaves the agent running on the new conversation with its
    // first message kept but not typed: the restart types it (D153).
    stop_point("start_fresh.launched");

    progress.set_step("seed", pb::StepState::Running, None);
    let typed = type_seed(&agent.launch.holder_socket, &session, &agent.launch.conversation_id, &seed).await;
    // Typed, or a terminal that can't take it: either way nothing types it
    // later. A failure to reach the terminal leaves it for the restart.
    if !matches!(typed, Err(StartFreshError::Seed { .. })) {
        forget_seed(&starter.store, &session, &agent.launch.conversation_id).await;
    }
    match typed {
        Ok(()) => {
            progress.set_step("seed", pb::StepState::Done, None);
            progress.succeed(Some(pb::operation::Result::Session(pb::SessionId {
                value: session.clone(),
            })));
            super::watch::agent(tell.clone(), session.clone());
        }
        Err(error) => {
            progress.set_step("seed", pb::StepState::Failed, Some(status::chain(&error)));
            progress.fail(error_of(
                &error,
                "the agent is running on its new conversation; open the session and tell it what the old one was about",
                None,
            ));
            super::watch::agent(tell.clone(), session.clone());
        }
    }
    tell.session(&session, pb::Change::Updated, Some(by)).await;
}

/// `Some(error)` when the blocking store work failed or didn't run.
fn store_failure(
    done: Result<Result<(), StoreError>, Status>,
    session: &str,
    doing: &'static str,
) -> Option<StartFreshError> {
    match done {
        Ok(Ok(())) => None,
        Ok(Err(source)) => Some(StartFreshError::Store {
            session: session.to_owned(),
            doing,
            source,
        }),
        Err(status) => Some(StartFreshError::Background {
            session: session.to_owned(),
            doing,
            message: status.message().to_owned(),
        }),
    }
}

/// Keeps `seed` for `launch`'s conversation until it is typed: `Some(error)`
/// if it couldn't be.
async fn keep_seed(
    store: &Arc<Store>,
    session: &str,
    launch: &AgentLaunch,
    seed: &str,
) -> Option<StartFreshError> {
    let row = FreshSeed {
        session_id: session.to_owned(),
        conversation_id: launch.conversation_id.clone(),
        seed: seed.to_owned(),
        created_unix_ms: now_ms(),
    };
    let store = store.clone();
    let kept = blocking(move || store.put_fresh_seed(&row)).await;
    store_failure(kept, session, "save the first message for the new conversation")
}

/// Forgets the message kept for `conversation`; a failure is logged, and
/// the next restart checks the conversation before typing it again.
async fn forget_seed(store: &Arc<Store>, session: &str, conversation: &str) {
    let (store, key, conversation) = (store.clone(), session.to_owned(), conversation.to_owned());
    if let Ok(Err(error)) = blocking(move || store.typed_fresh_seed(&key, &conversation)).await {
        crate::log::say!(
            "humatad2: couldn't forget session {session}'s typed first message: {}",
            status::chain(&error)
        );
    }
}

/// After a restart, types each first message a start fresh kept but didn't
/// get to type, into the conversation it was for, if that conversation's
/// agent runs and hasn't had it. One kept for a conversation the session
/// left stays only while that conversation is the one a start fresh would
/// replace.
pub(super) async fn retype_pending(store: Arc<Store>) {
    let read = {
        let store = store.clone();
        blocking(move || store.fresh_seeds()).await
    };
    let pending = match read {
        Ok(Ok(pending)) => pending,
        Ok(Err(error)) => {
            crate::log::say!(
                "humatad2: couldn't read the untyped start fresh messages, so none is typed after this restart: {}",
                status::chain(&error)
            );
            return;
        }
        Err(_) => return,
    };
    for pending in pending {
        tokio::spawn(retype(store.clone(), pending));
    }
}

async fn retype(store: Arc<Store>, pending: FreshSeed) {
    let session = pending.session_id.clone();
    let agent = {
        let (store, key) = (store.clone(), session.clone());
        match blocking(move || store.agent(&key)).await {
            Ok(Ok(agent)) => agent,
            _ => return,
        }
    };
    let Some(agent) = agent else {
        forget_seed(&store, &session, &pending.conversation_id).await;
        return;
    };
    if agent.launch.conversation_id != pending.conversation_id {
        forget_seed(&store, &session, &pending.conversation_id).await;
        return;
    }
    if !matches!(agent.state, AgentState::Running { .. }) {
        // Kept for a start fresh of this conversation, or for when it runs.
        return;
    }
    let had = {
        let (file, conversation) = (agent.launch.conversation_file.clone(), pending.conversation_id.clone());
        blocking(move || holds_seed(&file, &conversation)).await.unwrap_or(false)
    };
    if !had {
        match type_seed(&agent.launch.holder_socket, &session, &pending.conversation_id, &pending.seed).await {
            Err(StartFreshError::Seed { .. }) => {
                crate::log::say!(
                    "humatad2: couldn't type session {session}'s start fresh message after the restart; the next restart tries again"
                );
                return;
            }
            Err(StartFreshError::SeedUnsupported { .. }) => {
                crate::log::say!(
                    "humatad2: session {session}'s terminal is from an older build, so its start fresh message isn't typed after the restart; tell the agent what the old conversation was about"
                );
            }
            _ => {}
        }
    }
    forget_seed(&store, &session, &pending.conversation_id).await;
}

/// Whether the conversation file already holds the first message typed for
/// `conversation`, which names it. Blocking.
fn holds_seed(file: &Path, conversation: &str) -> bool {
    let Ok(bytes) = std::fs::read(file) else {
        return false;
    };
    let tag = seed_tag(conversation);
    bytes.windows(tag.len()).any(|window| window == tag.as_bytes())
}

/// What the first message for `conversation` starts with: it tells a
/// conversation that had it from one that didn't.
fn seed_tag(conversation: &str) -> String {
    format!("[start fresh {conversation}]")
}

/// The launch for a new conversation in `old`'s folder, tab and terminal,
/// and the "Started fresh" line its terminal shows first, below the old
/// output it keeps.
fn fresh_launch(
    starter: &Starter,
    agent: &str,
    old: &AgentRecord,
    session: &str,
) -> Result<(AgentLaunch, String), StartFreshError> {
    let kind = starter
        .agents
        .get(agent)
        .map_err(|source| StartFreshError::Agent {
            session: session.to_owned(),
            agent: agent.to_owned(),
            source,
        })?;
    let conversation = uuid::Uuid::new_v4().hyphenated().to_string();
    let plan = kind.launch(&starter.home, &old.launch.folder, &conversation);
    let marker = format!(
        "[Humata] Started fresh: the earlier conversation {} couldn't be resumed. Its file is kept at {}.",
        old.launch.conversation_id,
        old.launch.conversation_file.display()
    );
    let launch = AgentLaunch {
        argv: plan.argv,
        resume_argv: plan.resume_argv,
        conversation_id: conversation,
        conversation_file: plan.conversation_file,
        ..old.launch.clone()
    };
    Ok((launch, marker))
}

/// Types `seed` and presses return, each once however often this runs: the
/// terminal remembers the keys.
async fn type_seed(
    socket: &Path,
    session: &str,
    conversation: &str,
    seed: &str,
) -> Result<(), StartFreshError> {
    let seed_error = |source| StartFreshError::Seed {
        session: session.to_owned(),
        source,
    };
    let key = format!("{session}/fresh-{conversation}");
    let text = format!("{} {seed}", seed_tag(conversation));
    for (key, data) in [
        (format!("{key}/text"), text.as_bytes()),
        (format!("{key}/return"), b"\r".as_slice()),
    ] {
        if data == b"\r" {
            tokio::time::sleep(TYPE_PAUSE).await;
        }
        match humata2_holder::keyed_input(socket, &key, data)
            .await
            .map_err(seed_error)?
        {
            KeyedInput::Written | KeyedInput::Repeat => {}
            KeyedInput::Unsupported => {
                return Err(StartFreshError::SeedUnsupported {
                    session: session.to_owned(),
                });
            }
        }
    }
    Ok(())
}

/// What a start fresh of `agent` could summarize, for the session shown to
/// the app: unspecified unless its resume failed. It asks [`summarize`], the
/// summarizer's own question, so the two can't disagree. Blocking.
pub(super) fn summary_source(agent: Option<&AgentRecord>) -> pb::session::SummarySource {
    use pb::session::SummarySource;
    let Some(agent) = agent.filter(|agent| matches!(agent.state, AgentState::ResumeFailed { .. }))
    else {
        return SummarySource::Unspecified;
    };
    match summarize(&agent.launch.conversation_file, &agent.launch.holder_socket).0 {
        Source::Transcript => SummarySource::Transcript,
        Source::Scrollback => SummarySource::Scrollback,
        Source::Nothing => SummarySource::None,
    }
}

/// The digest of the old conversation and where it came from. Blocking.
fn summarize(file: &Path, socket: &Path) -> (Source, Option<String>) {
    if let Ok(opened) = std::fs::File::open(file)
        && let Some(digest) = digest_of(std::io::BufReader::new(opened))
    {
        return (Source::Transcript, Some(digest));
    }
    if let Some(tail) = saved_tail(socket) {
        return (Source::Scrollback, Some(tail));
    }
    (Source::Nothing, None)
}

/// The tail of the terminal output the old holder saved, as plain text.
/// Blocking. Unreadable output is as good as none: the seed then says
/// nothing of it was kept.
fn saved_tail(socket: &Path) -> Option<String> {
    let path = humata2_holder::saved_scrollback_path(socket);
    match humata2_holder::read_saved_tail(&path, SCROLLBACK_READ) {
        Ok(output) => plain_tail(&output?),
        Err(error) => {
            crate::log::say!(
                "humatad2: couldn't read the saved terminal output at {} for a start fresh, so the summary leaves it out: {}",
                path.display(),
                status::chain(&error)
            );
            None
        }
    }
}

/// `output` as plain text for a seed: escape sequences and control
/// characters dropped, the last `SCROLLBACK_MAX` bytes kept.
fn plain_tail(output: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(output);
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: up to a final byte in @..~.
                Some('[') => {
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' || (next == '\u{1b}' && chars.next() == Some('\\')) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\t' => plain.push(c),
            c if c.is_control() => {}
            c => plain.push(c),
        }
    }
    let plain = plain.trim();
    if plain.is_empty() {
        return None;
    }
    Some(tail_chars(plain, SCROLLBACK_MAX))
}

/// What a start-fresh agent is told first.
fn seed_text(
    title: &str,
    source: Source,
    digest: Option<&str>,
    old_file: &Path,
    old_conversation: &str,
) -> String {
    let mut seed = String::from(
        "Humata: this session's earlier conversation couldn't be resumed, so you are starting fresh in the same folder and branch.",
    );
    match (source, digest) {
        (Source::Transcript, Some(digest)) => {
            seed.push_str(" Here is a plain summary of the earlier conversation:\n");
            seed.push_str(digest);
            seed.push_str(&format!(
                "\nThe full transcript is at {} (read it if you need more).",
                old_file.display()
            ));
        }
        (Source::Scrollback, Some(digest)) => {
            seed.push_str(&format!(
                " The earlier conversation file ({old_conversation}) was lost; this is the end of the terminal's output, as plain text, not a transcript:\n"
            ));
            seed.push_str(digest);
        }
        _ => {
            seed.push_str(&format!(
                " The earlier conversation ({old_conversation}) was lost and nothing of it could be summarized."
            ));
        }
    }
    seed.push_str(&format!("\nThe session's title is: {title}"));
    seed.push_str("\nRun `git status` and check your work before continuing.");
    seed
}

/// The digest of a Claude JSONL conversation read from `reader`: every user
/// message, each capped, and the last few assistant text turns, tool output
/// left out, in about [`DIGEST_MAX`] bytes. None if it holds neither.
fn digest_of(mut reader: impl BufRead) -> Option<String> {
    let mut users: Vec<String> = Vec::new();
    let mut assistants: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        let Ok(entry) = serde_json::from_str::<Value>(text.trim()) else {
            continue;
        };
        if entry.get("isMeta").and_then(Value::as_bool) == Some(true)
            || entry.get("isSidechain").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let role = entry.get("type").and_then(Value::as_str);
        let Some(content) = entry.get("message").and_then(|m| m.get("content")) else {
            continue;
        };
        let said = collapse(&text_of(content));
        if said.is_empty() {
            continue;
        }
        match role {
            Some("user") if !is_noise(&said) => users.push(cap_chars(&said, USER_CAP)),
            Some("assistant") => {
                assistants.push_back(cap_chars(&said, ASSISTANT_CAP));
                if assistants.len() > ASSISTANTS_KEPT {
                    assistants.pop_front();
                }
            }
            _ => {}
        }
    }
    if users.is_empty() && assistants.is_empty() {
        return None;
    }

    let mut tail = String::new();
    if !assistants.is_empty() {
        tail.push_str("Its last replies:\n");
        for said in &assistants {
            tail.push_str("- ");
            tail.push_str(said);
            tail.push('\n');
        }
    }
    // The newest user messages that fit, kept in the order they were said.
    let head = "What was asked, oldest first:\n";
    let note_room = 80;
    let room = DIGEST_MAX.saturating_sub(tail.len() + head.len() + note_room);
    let mut used = 0;
    let mut kept: Vec<&String> = Vec::new();
    for said in users.iter().rev() {
        let cost = said.len() + 3;
        if used + cost > room {
            break;
        }
        used += cost;
        kept.push(said);
    }
    kept.reverse();
    let left_out = users.len() - kept.len();
    let mut digest = String::new();
    if !kept.is_empty() {
        digest.push_str(head);
        if left_out > 0 {
            digest.push_str(&format!("({left_out} earlier messages left out)\n"));
        }
        for said in kept {
            digest.push_str("- ");
            digest.push_str(said);
            digest.push('\n');
        }
    }
    digest.push_str(&tail);
    Some(cap_bytes(digest.trim_end(), DIGEST_MAX))
}

/// The text of a message's content: a string, or its text blocks. Tool
/// calls and results are not text.
fn text_of(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Messages the agent's own tooling wrote as the user: not what was asked.
fn is_noise(said: &str) -> bool {
    ["<command-", "<local-command", "Caveat:", "Humata:", "[Humata]"]
        .iter()
        .any(|prefix| said.starts_with(prefix))
}

/// `text` with every run of whitespace as one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// At most `max` characters of `text`, with an ellipsis if it was cut.
fn cap_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

/// The last `max` bytes of `text`, from a character boundary.
fn tail_chars(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

/// At most `max` bytes of `text`, cut on a character boundary.
fn cap_bytes(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_owned();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Kills the process with SIGKILL when `HUMATA2_STOP_AT` names this point, so
/// a test can show what a crash here leaves behind. Test daemons only: a
/// release build never reads the variable.
#[cfg(feature = "admin")]
fn stop_point(name: &str) {
    if std::env::var("HUMATA2_STOP_AT").is_ok_and(|wanted| wanted == name) {
        let _ = rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::KILL);
        std::process::abort();
    }
}

#[cfg(not(feature = "admin"))]
#[inline(always)]
fn stop_point(_name: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> String {
        serde_json::json!({"type":"user","message":{"role":"user","content":text}}).to_string()
    }

    fn assistant(text: &str) -> String {
        serde_json::json!({"type":"assistant","message":{"role":"assistant","content":[
            {"type":"text","text":text},
            {"type":"tool_use","id":"t","name":"Bash","input":{"command":"ls"}}
        ]}})
        .to_string()
    }

    fn tool_result(text: &str) -> String {
        serde_json::json!({"type":"user","message":{"role":"user","content":[
            {"type":"tool_result","tool_use_id":"t","content":text}
        ]}})
        .to_string()
    }

    fn digest(lines: &[String]) -> Option<String> {
        digest_of(std::io::Cursor::new(lines.join("\n")))
    }

    #[test]
    fn keeps_user_messages_and_drops_tool_output() {
        let made = digest(&[
            user("fix the login redirect"),
            assistant("looking"),
            tool_result("SECRET-TOOL-OUTPUT"),
            user("and add a test"),
        ])
        .unwrap();
        assert!(made.contains("fix the login redirect"));
        assert!(made.contains("and add a test"));
        assert!(!made.contains("SECRET-TOOL-OUTPUT"));
        assert!(!made.contains("Bash"));
    }

    #[test]
    fn keeps_only_the_last_assistant_turns() {
        let mut lines = vec![user("go")];
        lines.extend((0..6).map(|n| assistant(&format!("reply-number-{n}"))));
        let made = digest(&lines).unwrap();
        assert!(!made.contains("reply-number-2"));
        assert!(made.contains("reply-number-3"));
        assert!(made.contains("reply-number-5"));
    }

    #[test]
    fn stays_under_the_cap_keeping_the_newest_messages() {
        let long = "x".repeat(5_000);
        let mut lines: Vec<String> = (0..60).map(|n| user(&format!("{n:03} {long}"))).collect();
        lines.push(user("the very last ask"));
        lines.extend((0..4).map(|_| assistant(&long)));
        let made = digest(&lines).unwrap();
        assert!(made.len() <= DIGEST_MAX);
        assert!(made.contains("the very last ask"));
        assert!(!made.contains("000 x"));
    }

    #[test]
    fn caps_each_user_message() {
        let made = digest(&[user(&"y".repeat(USER_CAP * 3))]).unwrap();
        assert!(made.matches('y').count() <= USER_CAP);
    }

    #[test]
    fn skips_lines_that_are_not_json_and_agent_notes() {
        let made = digest(&[
            "not json".to_owned(),
            user("<command-name>/clear</command-name>"),
            user("real ask"),
        ])
        .unwrap();
        assert!(made.contains("real ask"));
        assert!(!made.contains("command-name"));
    }

    #[test]
    fn an_empty_conversation_has_no_digest() {
        assert_eq!(digest(&[tool_result("only output")]), None);
        assert_eq!(digest(&[]), None);
    }

    #[test]
    fn plain_tail_drops_escape_sequences() {
        let plain = plain_tail(b"\x1b[31mred\x1b[0m text\x1b]0;title\x07 done\r\n").unwrap();
        assert_eq!(plain, "red text done");
        assert_eq!(plain_tail(b"\x1b[2J"), None);
    }

    #[test]
    fn cutting_respects_character_boundaries() {
        assert_eq!(cap_bytes("aé", 2), "a");
        assert_eq!(cap_chars("ééé", 2), "éé…");
        assert_eq!(tail_chars("éa", 2), "a");
    }

    #[test]
    fn a_conversation_that_had_its_first_message_is_told_from_one_that_did_not() {
        let dir = std::env::temp_dir().join(format!("h2-seed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("c.jsonl");
        std::fs::write(&file, user(&format!("{} pick up the build", seed_tag("c-new")))).unwrap();
        assert!(holds_seed(&file, "c-new"));
        assert!(!holds_seed(&file, "c-other"));
        assert!(!holds_seed(&dir.join("missing.jsonl"), "c-new"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn failed_agent(file: &Path, socket: &Path) -> AgentRecord {
        AgentRecord {
            session: "se_1".to_owned(),
            tab: "t".to_owned(),
            launch: AgentLaunch {
                agent: "claude".to_owned(),
                argv: Vec::new(),
                resume_argv: Vec::new(),
                conversation_id: "c1".to_owned(),
                conversation_file: file.to_owned(),
                holder_socket: socket.to_owned(),
                folder: std::env::temp_dir(),
                variable_names: Vec::new(),
                cols: 80,
                rows: 24,
            },
            state: AgentState::ResumeFailed {
                reason: humata2_store::ResumeFailure::AgentExited,
                exit_code: None,
            },
            generation: 1,
            interruptions: Vec::new(),
            updated_unix_ms: 0,
        }
    }

    #[test]
    fn the_summary_source_follows_what_is_left_of_the_conversation() {
        use pb::session::SummarySource;
        let dir = std::env::temp_dir().join(format!("h2-source-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("c.jsonl");
        let socket = dir.join("h.sock");
        let mut agent = failed_agent(&file, &socket);

        // Neither a conversation nor saved output.
        assert_eq!(summary_source(Some(&agent)), SummarySource::None);
        std::fs::write(&file, user("fix the login redirect")).unwrap();
        assert_eq!(summary_source(Some(&agent)), SummarySource::Transcript);
        // Only a session whose resume failed has one.
        agent.state = AgentState::Stopped;
        assert_eq!(summary_source(Some(&agent)), SummarySource::Unspecified);
        assert_eq!(summary_source(None), SummarySource::Unspecified);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
