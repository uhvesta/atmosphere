//! TP-5 (D774): notes and questions between sessions. SendNote types one
//! message naming the sender into the target's agent once its turn is
//! over; AskSession types the question the same way with its id and waits
//! for the asked session's AnswerQuestion, its no, or the question to
//! expire. Both go through the input gate (D762) and wait for the agent to
//! read keys itself (BUG-T1): nothing is typed into a question the agent
//! asks on its screen, and nothing is typed blind. A question waits in this
//! daemon's memory only, as long as the asking call does: a restart ends
//! both.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use humata2_access::{Capability, Target};
use humata2_grpc::humata::v1 as pb;
use humata2_holder::{HolderError, KeyedInput};
use humata2_store::AgentState;
use pb::ask_session_response::Outcome;
use pb::invalid_field::Problem;
use sha2::{Digest, Sha256};
use tokio::sync::oneshot;
use tokio::time::Instant;
use tonic::{Code, Status};

use super::SessionSvc;
use crate::channel_service::Gated;
use crate::identity::Caller;
use crate::input_gate::{self, InputRefused};
use crate::status;

/// How long a note waits for the target's turn to end (TP-11's default wait).
const TURN_WAIT: Duration = Duration::from_secs(45);
/// How long a question waits when the call names no time, and at most.
const ASK_WAIT: Duration = Duration::from_secs(30 * 60);
/// The longest note or question taken, in bytes.
const TEXT_MAX: usize = 16 * 1024;
/// BUG-T1: as for Type, the agent gets this long to read keys itself.
const KEYS_WAIT: Duration = Duration::from_secs(5);
/// How long reaching the agent's holder may take.
const REACH: Duration = Duration::from_secs(2);
/// How long the agent gets to show the text before the return.
const SHOWN_WAIT: Duration = Duration::from_secs(5);
/// D671 (Provisional): the return is its own write after the text settles.
const RETURN_PAUSE: Duration = Duration::from_millis(400);
/// How many answered questions are remembered, so a retried answer is
/// taken again rather than refused.
const ANSWERED_KEPT: usize = 256;

/// Why a note or question wasn't typed into the target's agent.
#[derive(Debug, thiserror::Error)]
enum NotTyped {
    #[error("session {session}'s agent isn't running, so nothing was typed")]
    NotRunning { session: String },
    #[error("session {session}'s agent was still in its turn after {} seconds, so nothing was typed", .waited.as_secs())]
    StillInTurn { session: String, waited: Duration },
    #[error("session {session}'s terminal didn't answer within {} seconds, so nothing was typed", REACH.as_secs() + KEYS_WAIT.as_secs())]
    Unreachable { session: String },
    #[error("the input gate kept the text out of session {session}'s agent")]
    Gate { session: String, #[source] source: InputRefused },
    #[error("session {session}'s agent runs in a holder from an older build, which takes no typing")]
    Outdated { session: String, #[source] source: HolderError },
    #[error("humatad2 couldn't type into session {session}'s terminal")]
    Holder { session: String, #[source] source: HolderError },
}

impl NotTyped {
    fn status(&self) -> Status {
        let (code, fix) = match self {
            Self::NotRunning { session } => (
                Code::FailedPrecondition,
                format!("resume session {session} first (humata session resume {session}), then send it again"),
            ),
            Self::StillInTurn { .. } => (
                Code::Unavailable,
                "send it again once the session's turn is over; Humata doesn't hold it for you, so it isn't sent until you do".to_owned(),
            ),
            Self::Unreachable { .. } => (
                Code::Unavailable,
                "send it again in a moment. If it keeps failing, restart humatad2; the agent keeps running in its holder".to_owned(),
            ),
            Self::Gate { session, source } => return input_gate::refusal(session, source),
            Self::Outdated { .. } => (
                Code::FailedPrecondition,
                "click Migrate on the \"Migrate the holders\" item in the Inbox, which restarts the session's agent onto the current holder at its next idle moment and keeps its conversation, then send it again".to_owned(),
            ),
            Self::Holder { .. } => (
                Code::Unavailable,
                "look at the session's terminal: part of the text may be in the agent's input, unsent. Clear it there before you send it again, or it is sent twice".to_owned(),
            ),
        };
        status::failure(
            code,
            pb::Error {
                message: status::chain(self),
                fix,
                retry: None,
                kind: None,
            },
        )
    }
}

/// The agent a note or question is typed into.
struct Reached {
    session: String,
    socket: PathBuf,
    holder_pid: u32,
}

/// What the asked session said.
enum Said {
    Answer(String),
    Declined,
}

/// A question that waits for its answer.
struct Waiting {
    /// The session asked, the only one that may answer.
    to: String,
    said: oneshot::Sender<Said>,
}

#[derive(Default)]
struct Questions {
    waiting: HashMap<String, Waiting>,
    /// Answered questions and the idempotency key each was answered with.
    answered: VecDeque<(String, Option<String>)>,
}

static QUESTIONS: LazyLock<Mutex<Questions>> = LazyLock::new(Mutex::default);

fn questions() -> MutexGuard<'static, Questions> {
    QUESTIONS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Takes the question out when the asking call ends, however it ends.
struct Asked {
    id: String,
}

impl Drop for Asked {
    fn drop(&mut self) {
        questions().waiting.remove(&self.id);
    }
}

impl SessionSvc {
    pub(super) async fn send_note_of(
        &self,
        caller: &Caller,
        request: pb::SendNoteRequest,
        approved: bool,
    ) -> Result<Gated<pb::SendNoteResponse>, Status> {
        let to = session_of(request.to.as_ref())?;
        let text = checked("text", &request.text)?;
        let reached = match self.reach(caller, &to, approved).await? {
            Ok(reached) => reached,
            Err(asked) => return Ok(Gated::Ask(asked)),
        };
        let key = typing_key(caller, request.idempotency_key.as_ref());
        let message = format!("Note from {}: {text}", sender(caller));
        let deadline = Instant::now() + TURN_WAIT;
        if !turn_over(&reached, deadline).await {
            return Err(NotTyped::StillInTurn {
                session: reached.session,
                waited: TURN_WAIT,
            }
            .status());
        }
        super::refuse_input_while_moving(&reached.session)?;
        type_message(&reached, &key, &message)
            .await
            .map_err(|error| error.status())?;
        Ok(Gated::Done(pb::SendNoteResponse {
            delivery: pb::Delivery::Applied as i32,
            journal: None,
            typed: true,
        }))
    }

    pub(super) async fn ask_session_of(
        &self,
        caller: &Caller,
        request: pb::AskSessionRequest,
        approved: bool,
    ) -> Result<Gated<pb::AskSessionResponse>, Status> {
        let to = session_of(request.to.as_ref())?;
        let question = checked("question", &request.question)?;
        let reached = match self.reach(caller, &to, approved).await? {
            Ok(reached) => reached,
            Err(asked) => return Ok(Gated::Ask(asked)),
        };
        let wait = match request.timeout_seconds {
            0 => ASK_WAIT,
            seconds => Duration::from_secs(u64::from(seconds)).min(ASK_WAIT),
        };
        let deadline = Instant::now() + wait;
        let key = typing_key(caller, request.idempotency_key.as_ref());
        // A retried call has the id the first one typed, and its keyed
        // typing types nothing again.
        let id = match request.idempotency_key.as_ref() {
            Some(_) => format!("qu_{}", &key["tp5/".len()..]),
            None => format!("qu_{}", uuid::Uuid::new_v4().simple()),
        };
        let (said, heard) = oneshot::channel();
        {
            let mut questions = questions();
            if questions.waiting.contains_key(&id) {
                return Err(status::failure(
                    Code::Aborted,
                    pb::Error {
                        message: format!("another call with this idempotency key already waits for session {to}'s answer"),
                        fix: "wait for that call to return, or ask with a new idempotency key".to_owned(),
                        retry: None,
                        kind: None,
                    },
                ));
            }
            questions.waiting.insert(id.clone(), Waiting { to: to.clone(), said });
        }
        let _asked = Asked { id: id.clone() };
        if !turn_over(&reached, deadline).await {
            return Ok(Gated::Done(outcome(Outcome::Expired, String::new())));
        }
        super::refuse_input_while_moving(&reached.session)?;
        let message = format!(
            "Question {id} from {}: {question} (answer it with the session_answer tool, or `humata session answer {id} \"<answer>\"`; add --decline to say no)",
            sender(caller)
        );
        type_message(&reached, &format!("{key}/ask"), &message)
            .await
            .map_err(|error| error.status())?;
        Ok(Gated::Done(match tokio::time::timeout_at(deadline, heard).await {
            Ok(Ok(Said::Answer(answer))) => outcome(Outcome::Answered, answer),
            Ok(Ok(Said::Declined)) => outcome(Outcome::Declined, String::new()),
            Ok(Err(_)) | Err(_) => outcome(Outcome::Expired, String::new()),
        }))
    }

    /// The asked session's agent answers, or declines, a question typed
    /// into its terminal. Never asks: the question was its to answer.
    pub(super) fn answer_question_of(
        caller: &Caller,
        request: &pb::AnswerQuestionRequest,
    ) -> Result<pb::AnswerQuestionResponse, Status> {
        let id = request
            .question
            .as_ref()
            .map(|question| question.value.trim())
            .filter(|id| !id.is_empty())
            .ok_or_else(|| status::missing("question"))?;
        let Caller::Agent(agent) = caller else {
            return Err(not_yours(id));
        };
        let key = request.idempotency_key.as_ref().map(|key| key.value.clone());
        let mut questions = questions();
        let Some(waiting) = questions.waiting.get(id) else {
            let repeat = questions
                .answered
                .iter()
                .any(|(answered, with)| answered == id && with.is_some() && *with == key);
            return if repeat {
                Ok(pb::AnswerQuestionResponse {})
            } else {
                Err(nothing_waits(id))
            };
        };
        if waiting.to != agent.session.id.as_str() {
            return Err(not_yours(id));
        }
        let Some(waiting) = questions.waiting.remove(id) else {
            return Err(nothing_waits(id));
        };
        if questions.answered.len() >= ANSWERED_KEPT {
            questions.answered.pop_front();
        }
        questions.answered.push_back((id.to_owned(), key));
        let said = if request.decline {
            Said::Declined
        } else {
            Said::Answer(request.answer.clone())
        };
        // An asker gone since leaves the answer unheard.
        waiting.said.send(said).map_err(|_| nothing_waits(id))?;
        Ok(pb::AnswerQuestionResponse {})
    }

    /// The running agent of session `to`, once `caller` may reach it:
    /// `Err` holds the Inbox request when the rule says Ask.
    async fn reach(
        &self,
        caller: &Caller,
        to: &str,
        approved: bool,
    ) -> Result<Result<Reached, Box<humata2_access::AccessRequest>>, Status> {
        if matches!(caller, Caller::Agent(agent) if agent.session.id.as_str() == to) {
            return Err(status::invalid_field(
                "to",
                Problem::NotAllowedHere,
                "a session can't send a note or question to itself",
                "name another session in `to`",
            ));
        }
        let loaded = self.load(to).await?;
        if let Some(asked) = self.gate(
            caller,
            Capability::SessionNote,
            Target::Session(self.session_ref(&loaded.record)),
            approved,
        )? {
            return Ok(Err(asked));
        }
        super::refuse_input_while_moving(to)?;
        if let Some((machine, _)) = self.lives_elsewhere(&loaded.record) {
            return Err(status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: format!("session {to} runs on machine {machine}, and notes and questions to a session on another machine aren't relayed yet, so nothing was typed"),
                    fix: format!("type it into the session yourself with `humata session type {to} \"<text>\"`, which asks first as a note does"),
                    retry: None,
                    kind: None,
                },
            ));
        }
        match loaded.agent {
            Some(agent) => match agent.state {
                AgentState::Running { holder_pid } => Ok(Ok(Reached {
                    session: to.to_owned(),
                    socket: agent.launch.holder_socket,
                    holder_pid,
                })),
                _ => Err(NotTyped::NotRunning { session: to.to_owned() }.status()),
            },
            None => Err(NotTyped::NotRunning { session: to.to_owned() }.status()),
        }
    }
}

fn outcome(outcome: Outcome, answer: String) -> pb::AskSessionResponse {
    pb::AskSessionResponse {
        outcome: outcome as i32,
        answer,
    }
}

fn session_of(to: Option<&pb::SessionId>) -> Result<String, Status> {
    to.map(|to| to.value.clone())
        .filter(|to| !to.is_empty())
        .ok_or_else(|| status::missing("to"))
}

/// `text` as one line: a line break would submit part of it, and a control
/// character could drive the agent's screen. Refused when empty or too long.
fn checked(field: &str, text: &str) -> Result<String, Status> {
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let line = line.trim();
    if line.is_empty() {
        return Err(status::missing(field));
    }
    if line.len() > TEXT_MAX {
        return Err(status::invalid_field(
            field,
            Problem::TooLong,
            format!("the {field} is {} bytes, more than the {TEXT_MAX} a note or question takes", line.len()),
            "shorten it, or write the long part to a file in shared/ and say where it is",
        ));
    }
    Ok(line.to_owned())
}

/// Who a note or question names as its sender: no paths or branches (D743).
fn sender(caller: &Caller) -> String {
    let named = match caller {
        Caller::Agent(agent) => format!("session \"{}\" ({})", agent.title, agent.session.id.as_str()),
        Caller::Script(script) => format!("the script \"{}\"", script.name),
        Caller::User(_) | Caller::Peer => "the user".to_owned(),
    };
    named.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

/// The key the holder remembers the typing by, so a retried call types
/// nothing twice: from the caller and its idempotency key, or fresh.
fn typing_key(caller: &Caller, key: Option<&pb::IdempotencyKey>) -> String {
    let Some(key) = key.filter(|key| !key.value.is_empty()) else {
        return format!("tp5/{}", uuid::Uuid::new_v4().simple());
    };
    let who = match caller {
        Caller::Agent(agent) => agent.session.id.as_str().to_owned(),
        Caller::Script(script) => script.id.clone(),
        Caller::User(_) | Caller::Peer => "user".to_owned(),
    };
    let digest = Sha256::digest(format!("{who}\0{}", key.value).as_bytes());
    let hex: String = digest.iter().take(16).map(|byte| format!("{byte:02x}")).collect();
    format!("tp5/{hex}")
}

/// Waits until the agent's turn is over (SE-23): not mid-turn and not
/// waiting on an Inbox request of its own. An agent whose turns can't be
/// known is taken as ready, as Type takes it. False if `deadline` passes.
async fn turn_over(reached: &Reached, deadline: Instant) -> bool {
    let mut changes = super::turn_changes();
    loop {
        match super::running_turn(&reached.session, reached.holder_pid) {
            pb::SessionStatus::Working | pb::SessionStatus::NeedsInput => {}
            _ => return true,
        }
        match tokio::time::timeout_at(deadline, changes.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => return false,
        }
    }
}

/// Types `text`, then presses return, each once for `key`, once the agent
/// reads keys itself and the input gate lets text in (D762).
async fn type_message(reached: &Reached, key: &str, text: &str) -> Result<(), NotTyped> {
    let session = || reached.session.clone();
    let holder = |source: HolderError| match source {
        HolderError::Outdated { .. } => NotTyped::Outdated { session: session(), source },
        source => NotTyped::Holder { session: session(), source },
    };
    let socket: &Path = &reached.socket;
    match tokio::time::timeout(KEYS_WAIT + REACH, humata2_holder::await_keys(socket, KEYS_WAIT)).await {
        Ok(Ok(_)) => {}
        Ok(Err(source)) => return Err(holder(source)),
        Err(_) => return Err(NotTyped::Unreachable { session: session() }),
    }
    input_gate::guard(socket)
        .await
        .map_err(|source| NotTyped::Gate { session: session(), source })?;
    let mut watch = match tokio::time::timeout(REACH, humata2_holder::watch_output(socket)).await {
        Ok(Ok(watch)) => Some(watch),
        Ok(Err(_)) | Err(_) => None,
    };
    let typed = humata2_holder::keyed_input(socket, &format!("{key}/text"), text.as_bytes())
        .await
        .map_err(holder)?;
    // BUG-T1: the return waits for the agent to show the text; it goes on
    // after the wait either way, as Type does.
    if let (KeyedInput::Written, Some(watch)) = (typed, watch.as_mut()) {
        let _ = watch.shown(text.as_bytes(), SHOWN_WAIT).await;
    }
    tokio::time::sleep(RETURN_PAUSE).await;
    humata2_holder::keyed_input(socket, &format!("{key}/return"), b"\r")
        .await
        .map_err(holder)?;
    Ok(())
}

fn not_yours(id: &str) -> Status {
    status::failure(
        Code::PermissionDenied,
        pb::Error {
            message: format!("question {id} was asked of another session, so only that session's agent answers it"),
            fix: "answer only the questions typed into your own terminal; to tell the asker something, send it a note".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

fn nothing_waits(id: &str) -> Status {
    status::not_found(
        pb::EntityKind::Unspecified,
        id,
        format!("no question {id} waits for an answer: it was answered or declined, it expired, or the call that asked it ended"),
        "if the asker still needs it, send your answer as a note to the session that asked (session_note, or `humata session note`)",
    )
}
