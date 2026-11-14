//! Agents starting sessions of their own and writing to other sessions,
//! with the humata tools start_session and send_message. Each goes by what
//! the session's project lets its agents do: nothing, only what the person
//! allows from the inbox one at a time, or anything within its limits.

use humata_wire::pb;

use std::sync::PoisonError;

use super::{Daemon, refusal};
use crate::error::{AgentSessionsError, DaemonError, StoreError};
use crate::sender;

/// The longest message one session's agent may send another, in bytes.
/// Each waits whole in the inbox and is read whole by its session.
const MESSAGE_LIMIT: usize = 32 * 1024;

/// The most messages to other sessions one session's agent may have
/// waiting for the person at once.
const WAITING_LIMIT: u32 = 8;

/// The most messages one session's agent may send straight to other
/// sessions, where its project allows that without asking, since the
/// person last wrote to it. Two agents answering each other stop here.
const UNATTENDED_LIMIT: u32 = 20;

/// The longest name an agent may give a session it starts, in characters.
/// It shows in the sidebar and the inbox; the detail goes in the message.
const NAME_LIMIT: usize = 80;

/// Fails when `text` is longer than an agent's message may be.
fn short_enough(text: &str) -> Result<(), AgentSessionsError> {
    if text.len() > MESSAGE_LIMIT {
        return Err(AgentSessionsError::MessageTooLong {
            bytes: text.len(),
            limit: MESSAGE_LIMIT,
        });
    }
    Ok(())
}

/// Whether `request` asks to start a session or to write to one, which the
/// person allows rather than answers.
pub(super) fn asks_for_session(request: &pb::AgentRequest) -> bool {
    matches!(
        request.kind(),
        pb::AgentRequestKind::StartSession | pb::AgentRequestKind::SendMessage
    )
}

/// What `request` asks for, to finish "your request to".
fn asked(request: &pb::AgentRequest) -> String {
    match request.kind() {
        pb::AgentRequestKind::StartSession => format!("start session {:?}", request.name),
        _ => format!(
            "write to session {:?} ({})",
            request.name, request.target_session_id
        ),
    }
}

/// The person's `reply` to `request` as its agent reads it: for a session
/// to start or a message to send, it says the reply declines that.
pub(super) fn declined_with(request: &pb::AgentRequest, reply: &str) -> String {
    if asks_for_session(request) {
        format!(
            "The person declined your request to {}, and says:\n\n{reply}",
            asked(request)
        )
    } else {
        reply.to_owned()
    }
}

/// A session whose agent asks, and what its project lets it do.
struct Caller {
    session: pb::SessionInfo,
    allowed: pb::AgentSessions,
    /// Every session, as listed when it asked.
    sessions: Vec<pb::SessionInfo>,
}

impl Caller {
    /// The session it may write to as `target`: one it started, or, where
    /// its project lets agents write to others, any in its project.
    fn target(&self, target: &str) -> Result<&pb::SessionInfo, DaemonError> {
        if target == self.session.id {
            return Err(AgentSessionsError::Itself.into());
        }
        let found = self
            .sessions
            .iter()
            .find(|session| session.id == target)
            .ok_or_else(|| StoreError::UnknownSession {
                id: target.to_owned(),
            })?;
        if found.started_by_session_id == self.session.id {
            return Ok(found);
        }
        if !self.allowed.message_others {
            return Err(AgentSessionsError::NotOwnChild {
                target: target.to_owned(),
            }
            .into());
        }
        if found.project_id != self.session.project_id {
            return Err(AgentSessionsError::AnotherProject {
                target: target.to_owned(),
            }
            .into());
        }
        Ok(found)
    }
}

impl Daemon {
    /// Whether `request`, from a client other than the app, decides what
    /// agents may do to sessions, which is the person's alone: allowing
    /// what one asked for, or changing a project's setting.
    pub(super) fn decides_agent_sessions(
        &self,
        request: &pb::request::Kind,
    ) -> Result<bool, DaemonError> {
        use pb::answer_request::Answer;
        match request {
            pb::request::Kind::UpdateProject(update) => Ok(update.agent_sessions.is_some()),
            pb::request::Kind::AnswerRequest(pb::AnswerRequest {
                request_id,
                answer: Some(Answer::Approve(_)),
            }) => match self.store.open_request(request_id) {
                Ok(asked) => Ok(asks_for_session(&asked)),
                // Answering it says it isn't there.
                Err(StoreError::UnknownRequest { .. }) => Ok(false),
                Err(failure) => Err(failure.into()),
            },
            _ => Ok(false),
        }
    }

    /// The session `id`, whose agent asks, with what its project lets it
    /// do; refused when that is nothing, and for a side chat, which only
    /// reads, whatever its project allows.
    async fn agent_caller(&self, id: &str) -> Result<Caller, DaemonError> {
        let sessions = self.list("").await?.sessions;
        let session = sessions
            .iter()
            .find(|session| session.id == id)
            .cloned()
            .ok_or_else(|| StoreError::UnknownSession { id: id.to_owned() })?;
        if session.read_only {
            return Err(AgentSessionsError::ReadOnly.into());
        }
        let project = Some(session.project_id.as_str()).filter(|project| !project.is_empty());
        let allowed = self.store.agent_sessions(project)?;
        if allowed.mode() == pb::AgentSessionsMode::Off {
            return Err(AgentSessionsError::Off.into());
        }
        Ok(Caller {
            session,
            allowed,
            sessions,
        })
    }

    /// Fails when `caller` may not start one more session: it is as deep
    /// as its project lets agents' sessions nest, or has as many as it may.
    /// `waiting` is how many of the starts it asked for count: all but the
    /// one being allowed.
    fn room_for_child(&self, caller: &Caller, waiting: u32) -> Result<(), DaemonError> {
        let depth = self.store.started_depth(&caller.session.id)?;
        if depth >= caller.allowed.max_depth {
            return Err(AgentSessionsError::TooDeep {
                depth,
                limit: caller.allowed.max_depth,
            }
            .into());
        }
        // Those archived or deleted since make room.
        let started = self.store.started_by(&caller.session.id)?;
        let kept = caller
            .sessions
            .iter()
            .filter(|session| !session.archived && started.contains(&session.id))
            .count();
        let children = u32::try_from(kept)
            .unwrap_or(u32::MAX)
            .saturating_add(waiting);
        if children >= caller.allowed.max_children {
            return Err(AgentSessionsError::TooManyChildren {
                limit: caller.allowed.max_children,
            }
            .into());
        }
        Ok(())
    }

    /// An agent's start_session: refused, kept for the person, or done, by
    /// its project's setting.
    pub(super) async fn agent_start_session(
        &self,
        start: pb::AgentStartSession,
    ) -> Result<pb::AgentActed, DaemonError> {
        let (name, text) = (start.name.trim(), start.text.trim());
        if name.is_empty() || text.is_empty() {
            return Err(AgentSessionsError::NoNameOrMessage.into());
        }
        let characters = name.chars().count();
        if characters > NAME_LIMIT {
            return Err(AgentSessionsError::NameTooLong {
                characters,
                limit: NAME_LIMIT,
            }
            .into());
        }
        short_enough(text)?;
        let (caller, child) = {
            // One start at a time, so each is counted with those before it:
            // the sessions are listed only once earlier starts are made.
            let _counted = self.agent_asks.lock().await;
            let caller = self.agent_caller(&start.session_id).await?;
            let waiting = self
                .store
                .open_of_kind(&caller.session.id, pb::AgentRequestKind::StartSession)?;
            self.room_for_child(&caller, waiting)?;
            if caller.allowed.mode() != pb::AgentSessionsMode::Allow {
                let request = self.store.add_request(
                    pb::AgentRequest {
                        session_id: caller.session.id.clone(),
                        kind: pb::AgentRequestKind::StartSession.into(),
                        text: text.to_owned(),
                        name: name.to_owned(),
                        ..Default::default()
                    },
                    "",
                )?;
                return Ok(pb::AgentActed {
                    request: Some(request),
                    session: None,
                });
            }
            let child = self.start_child(&caller.session, name).await?;
            (caller, child)
        };
        // The session is there whether or not its message reaches it, so a
        // failure names it, or the agent would start another.
        if let Err(source) = self
            .send_from(&caller.session, &child.id, text, String::new())
            .await
        {
            return Err(AgentSessionsError::StartedUnsent {
                name: child.name,
                id: child.id,
                source: Box::new(source),
            }
            .into());
        }
        Ok(pb::AgentActed {
            request: None,
            session: Some(child),
        })
    }

    /// An agent's send_message: refused, kept for the person, or sent, by
    /// its project's setting.
    pub(super) async fn agent_send_message(
        &self,
        send: pb::AgentSendMessage,
    ) -> Result<pb::AgentActed, DaemonError> {
        let text = send.text.trim();
        if text.is_empty() {
            return Err(AgentSessionsError::NoText.into());
        }
        short_enough(text)?;
        let caller = self.agent_caller(&send.session_id).await?;
        let target = caller.target(&send.to_session_id)?;
        if caller.allowed.mode() != pb::AgentSessionsMode::Allow {
            // Counted and kept in one go, so messages sent together don't
            // each see room for themselves.
            let _counted = self.agent_asks.lock().await;
            let waiting = self
                .store
                .open_of_kind(&caller.session.id, pb::AgentRequestKind::SendMessage)?;
            if waiting >= WAITING_LIMIT {
                return Err(AgentSessionsError::TooManyWaiting { waiting }.into());
            }
            let request = self.store.add_request(
                pb::AgentRequest {
                    session_id: caller.session.id.clone(),
                    kind: pb::AgentRequestKind::SendMessage.into(),
                    text: text.to_owned(),
                    target_session_id: target.id.clone(),
                    name: target.name.clone(),
                    ..Default::default()
                },
                "",
            )?;
            return Ok(pb::AgentActed {
                request: Some(request),
                session: None,
            });
        }
        // Counted before it is sent, so ones sent together can't all pass,
        // and not counted after all if it didn't go.
        self.count_unattended(&caller.session.id)?;
        if let Err(failure) = self
            .send_from(&caller.session, &target.id, text, String::new())
            .await
        {
            self.uncount_unattended(&caller.session.id);
            return Err(failure);
        }
        Ok(pb::AgentActed {
            request: None,
            session: Some(target.clone()),
        })
    }

    /// Counts one more message `session`'s agent sends straight to another
    /// session; refused once it has sent as many as it may since the
    /// person last wrote to it.
    fn count_unattended(&self, session: &str) -> Result<(), AgentSessionsError> {
        let mut sent = self
            .agent_messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let count = sent.entry(session.to_owned()).or_insert(0);
        if *count >= UNATTENDED_LIMIT {
            return Err(AgentSessionsError::TooManyUnattended {
                sent: *count,
                limit: UNATTENDED_LIMIT,
            });
        }
        *count += 1;
        Ok(())
    }

    /// Takes back a count made for a message that wasn't sent after all.
    fn uncount_unattended(&self, session: &str) {
        let mut sent = self
            .agent_messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(count) = sent.get_mut(session) {
            *count = count.saturating_sub(1);
        }
    }

    /// The person wrote to `session`, so its agent may write to other
    /// sessions afresh.
    pub(super) fn person_wrote(&self, session: &str) {
        self.agent_messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(session);
    }

    /// Starts a session named `name` as `parent`'s child: where `parent`
    /// is, set up as it is, with none of its history.
    async fn start_child(
        &self,
        parent: &pb::SessionInfo,
        name: &str,
    ) -> Result<pb::SessionInfo, DaemonError> {
        let start = pb::StartSession {
            backend: parent.backend,
            // A chat works in its parent's folder; a project's session
            // gets its own copy of the project's repositories.
            working_directory: if parent.project_id.is_empty() {
                parent.working_directory.clone()
            } else {
                String::new()
            },
            model: parent.model.clone(),
            name: name.to_owned(),
            host: parent.host.clone(),
            approval_mode: parent.approval_mode,
            project_id: parent.project_id.clone(),
            item_id: parent.item_id.clone(),
            effort: parent.effort.clone(),
            experimental: parent.experimental,
            config_from_session_id: parent.id.clone(),
            ..Default::default()
        };
        let reply = Box::pin(self.answer(pb::request::Kind::StartSession(start))).await?;
        if let Some(refused) = refusal(&reply) {
            return Err(refused);
        }
        let humata_wire::Reply::Response(pb::response::Kind::SessionStarted(pb::SessionStarted {
            session: Some(mut child),
        })) = reply
        else {
            return Err(AgentSessionsError::NotStarted {
                name: name.to_owned(),
            }
            .into());
        };
        self.store.set_started_by(&child.id, &parent.id)?;
        child.started_by_session_id.clone_from(&parent.id);
        Ok(child)
    }

    /// Sends `text` to session `to` as from `from`'s agent. A non-empty
    /// `client_id` names the send, so one made again is sent once.
    async fn send_from(
        &self,
        from: &pb::SessionInfo,
        to: &str,
        text: &str,
        client_id: String,
    ) -> Result<(), DaemonError> {
        let text = sender::headed(&from.id, &from.name, text);
        self.tell(to, text, client_id).await
    }

    /// Sends `session` a message that isn't the person's: another agent's,
    /// or what became of something its own agent asked for. It waits its
    /// turn apart from what the person wrote, never joined to it. A
    /// non-empty `client_id` names it, so it's sent once.
    async fn tell(
        &self,
        session: &str,
        text: String,
        client_id: String,
    ) -> Result<(), DaemonError> {
        let message = pb::SendMessage {
            session_id: session.to_owned(),
            text,
            mode: pb::SendMode::EnqueueApart.into(),
            client_id,
            ..Default::default()
        };
        match refusal(&Box::pin(self.send_message(message)).await?) {
            Some(refused) => Err(refused),
            None => Ok(()),
        }
    }

    /// The person allowed `request`: starts the session, or sends the
    /// message, it asks for, takes it out of the inbox and tells its agent.
    /// The project's limits hold here too. If the session can't be started
    /// or the message sent, the request stays, to allow again or decline.
    pub(super) async fn allow_agent_request(
        &self,
        request: &pb::AgentRequest,
    ) -> Result<(), DaemonError> {
        let told = format!("allowed-{}", request.id);
        if request.kind() == pb::AgentRequestKind::SendMessage {
            let session = self
                .list("")
                .await?
                .sessions
                .into_iter()
                .find(|session| session.id == request.session_id)
                .ok_or_else(|| StoreError::UnknownSession {
                    id: request.session_id.clone(),
                })?;
            // Named by the request, so allowing it again after its answer
            // was lost sends it once, for as long as the process that
            // hosts the target session lives. A session on this Mac is
            // hosted by humatad itself, so that isn't across its restart.
            self.send_from(
                &session,
                &request.target_session_id,
                &request.text,
                format!("agent-message-{}", request.id),
            )
            .await?;
            self.store.answer_request(&request.id)?;
            return self
                .tell(
                    &session.id,
                    sender::noticed(&format!(
                        "The person allowed it: your message to session {:?} ({}) was sent. read_session shows what it does with it.",
                        request.name, request.target_session_id
                    )),
                    told,
                )
                .await;
        }
        let (caller, child) = {
            // One start at a time. Allowed twice together, the second
            // waits here and then finds the request answered, so it starts
            // nothing; and the sessions are listed after any start before.
            let _counted = self.agent_asks.lock().await;
            self.store.open_request(&request.id)?;
            let sessions = self.list("").await?.sessions;
            let session = sessions
                .iter()
                .find(|session| session.id == request.session_id)
                .cloned()
                .ok_or_else(|| StoreError::UnknownSession {
                    id: request.session_id.clone(),
                })?;
            let project = Some(session.project_id.as_str()).filter(|project| !project.is_empty());
            let caller = Caller {
                allowed: self.store.agent_sessions(project)?,
                session,
                sessions,
            };
            // This request is one of those waiting.
            let waiting = self
                .store
                .open_of_kind(&caller.session.id, pb::AgentRequestKind::StartSession)?
                .saturating_sub(1);
            // Said to the person, who is the one allowing it, not to the
            // agent.
            self.room_for_child(&caller, waiting)
                .map_err(|refused| match refused {
                    DaemonError::AgentSessions(AgentSessionsError::TooManyChildren { limit }) => {
                        DaemonError::from(AgentSessionsError::AllowTooManyChildren {
                            name: caller.session.name.clone(),
                            limit,
                        })
                    }
                    DaemonError::AgentSessions(AgentSessionsError::TooDeep { depth, limit }) => {
                        DaemonError::from(AgentSessionsError::AllowTooDeep {
                            name: caller.session.name.clone(),
                            depth,
                            limit,
                        })
                    }
                    other => other,
                })?;
            let child = self.start_child(&caller.session, &request.name).await?;
            // Out of the inbox before its first message: allowed again, it
            // would start a second session.
            self.store.answer_request(&request.id)?;
            (caller, child)
        };
        let first = self
            .send_from(
                &caller.session,
                &child.id,
                &request.text,
                format!("agent-message-{}", request.id),
            )
            .await;
        let text = match &first {
            Ok(()) => format!(
                "The person allowed it: session {:?} is started and has your message. Its id is {}; read_session shows what it does, and send_message writes to it.",
                request.name, child.id
            ),
            Err(failure) => format!(
                "The person allowed it: session {:?} is started, with id {}, but your first message didn't reach it: {}. Send it again with send_message.",
                request.name,
                child.id,
                humata_wire::describe(failure)
            ),
        };
        self.tell(&caller.session.id, sender::noticed(&text), told)
            .await?;
        first
    }

    /// The person dismissed `request`: when it asks to start a session or
    /// to write to one, its agent is told it was declined, so it doesn't
    /// wait on it.
    pub(super) async fn decline_agent_request(&self, request: &pb::AgentRequest) {
        if !asks_for_session(request) {
            return;
        }
        // A session that can't be told, as one archived since, has no
        // agent waiting on the answer; the request leaves the inbox all
        // the same.
        let _ = self
            .tell(
                &request.session_id,
                sender::noticed(&format!(
                    "The person declined your request to {}. Carry on without it, or ask_person if you need to know why.",
                    asked(request)
                )),
                format!("declined-{}", request.id),
            )
            .await;
    }
}
