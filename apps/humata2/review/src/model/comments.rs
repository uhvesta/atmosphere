//! One session's review comments and the queue that sends them to its agent.
//!
//! A comment is a draft until it's queued. Queuing freezes the exact text
//! the agent will get into a *delivery* with an idempotency key. A delivery
//! is sent by [`CommentBook::next_dispatch`] when the agent can take it,
//! and settled by [`CommentBook::finish`]. The key never changes, so every
//! retry, and every resend after a restart, is the same message the terminal
//! types at most once: a comment reaches the agent exactly once.
//!
//! The book is plain data. The pane saves a [`CommentBook`] snapshot before
//! anything it says is done, and before every send.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

use serde::{Deserialize, Serialize};

use super::anchor::{Anchor, LineRange, Placement, passage_gone, place};
use super::message::{Part, compose};
use super::review::{FileKey, SessionId, Side};
use crate::error::{DeliveryError, ReviewError};

/// A comment, numbered within its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CommentId(pub u64);

impl std::fmt::Display for CommentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// A send of one or more comments, numbered within its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DeliveryId(pub u64);

impl std::fmt::Display for DeliveryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Where a comment is on its way to the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Status {
    /// Written, not sent.
    Draft,
    /// In a delivery waiting to be typed.
    Queued,
    /// Being typed into the terminal.
    Sending,
    /// The agent has it.
    Sent,
    /// Its delivery failed; Retry sends the same message again.
    Failed,
    /// The agent changed the lines it points at after it was sent.
    Addressed,
    /// Done with; hidden unless resolved comments are shown.
    Resolved,
}

impl Status {
    /// Whether the comment is still yours to send.
    pub fn unsent(self) -> bool {
        matches!(self, Self::Draft | Self::Queued | Self::Sending | Self::Failed)
    }
}

/// What a comment is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Kind {
    /// For the session's agent to act on.
    #[default]
    Comment,
    /// A question for the agent about the code (Ask, PR-19): it's told to
    /// answer without changing anything.
    Question,
    /// A draft for GitHub (PR-9, PR-11), posted only when the user submits
    /// the review. It never goes to the agent.
    GitHub,
}

/// One comment: for the agent, a question for it, or a GitHub draft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: CommentId,
    #[serde(default)]
    pub kind: Kind,
    /// Who wrote it, if not the user, such as the agent whose note it was.
    #[serde(default)]
    pub by: Option<String>,
    pub anchor: Anchor,
    pub body: String,
    pub status: Status,
    /// The code it points at changed before it was sent: re-attach or drop.
    pub outdated: bool,
    /// The delivery it's in while queued, sending or failed.
    pub delivery: Option<DeliveryId>,
}

/// What the agent's terminal can take right now, from the terminal's events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AgentState {
    /// Not heard from yet, such as just after a restart.
    Unknown,
    /// Waiting for input: messages are typed now.
    Idle,
    /// In a turn: messages wait for it to end.
    Working,
    /// Probably idle, from its output only: messages wait, Send now types them.
    ProbablyIdle,
    /// Its machine is offline: messages wait for it to reconnect.
    Disconnected,
    /// The agent exited: nothing can be typed until it's resumed.
    Exited,
}

/// Why a queued delivery hasn't been sent yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Hold {
    /// Next to go.
    Ready,
    /// The agent is in a turn; it's typed when the turn ends.
    Busy,
    /// The turn's end isn't certain; Send now types it anyway.
    TurnEndUncertain,
    /// The machine is offline; it's sent when it reconnects.
    Disconnected,
    /// The terminal's state isn't known yet.
    Unknown,
}

/// Why a delivery failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Failure {
    /// The agent exited; resume it, then retry.
    Exited,
    /// The terminal refused it.
    Refused { reason: String },
}

/// Where a delivery is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryState {
    Queued { hold: Hold, send_now: bool },
    Sending { attempt: u32 },
    Sent,
    Failed { failure: Failure },
}

/// One message to the agent: the exact text, the comments it carries and the
/// key the terminal types it under at most once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub id: DeliveryId,
    pub key: String,
    pub comments: Vec<CommentId>,
    pub text: String,
    pub state: DeliveryState,
    /// Times it was handed to the terminal, counting resends after restarts.
    pub attempts: u32,
    /// It may have been typed already: it was in flight when the app stopped
    /// or the link dropped. It's resent under its key, which the terminal
    /// types at most once, and it can't be taken back until then.
    #[serde(default)]
    pub in_doubt: bool,
}

/// A delivery to type now: hand it to the terminal, then pass the result to
/// [`CommentBook::finish`] with the same `attempt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    pub delivery: DeliveryId,
    pub attempt: u32,
    pub key: String,
    pub text: String,
}

/// What the terminal did with a delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    /// It typed it now.
    Typed,
    /// It had already typed a message with that key, so it didn't again.
    AlreadyTyped,
}

/// One session's comments and the queue that sends them to its agent's
/// terminal (in a PR review, the review chat, PR-22).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentBook {
    session: SessionId,
    /// Makes this book's keys unlike any other book's, even one for the same
    /// session made again from nothing.
    epoch: String,
    next_comment: u64,
    next_delivery: u64,
    comments: Vec<Comment>,
    deliveries: Vec<Delivery>,
    agent: AgentState,
}

impl CommentBook {
    pub fn new(session: SessionId) -> Self {
        Self {
            session,
            epoch: epoch(),
            next_comment: 1,
            next_delivery: 1,
            comments: Vec::new(),
            deliveries: Vec::new(),
            agent: AgentState::Unknown,
        }
    }

    /// The book as it was saved, after a restart. A delivery that was being
    /// typed when the app stopped is in doubt: it's queued again under its
    /// key, so the terminal types it if it hadn't and only says so if it
    /// had, and its comments stay sending until then. The agent's state is
    /// unknown until the terminal says.
    pub fn restored(mut self) -> Self {
        self.agent = AgentState::Unknown;
        for delivery in &mut self.deliveries {
            if let DeliveryState::Sending { .. } = delivery.state {
                delivery.state = DeliveryState::Queued {
                    hold: Hold::Unknown,
                    send_now: false,
                };
                delivery.in_doubt = true;
            }
        }
        self.rehold();
        self
    }

    pub fn session(&self) -> &SessionId {
        &self.session
    }

    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    pub fn comment(&self, id: CommentId) -> Option<&Comment> {
        self.comments.iter().find(|comment| comment.id == id)
    }

    pub fn deliveries(&self) -> &[Delivery] {
        &self.deliveries
    }

    pub fn delivery(&self, id: DeliveryId) -> Option<&Delivery> {
        self.deliveries.iter().find(|delivery| delivery.id == id)
    }

    pub fn agent(&self) -> AgentState {
        self.agent
    }

    /// The comments on `file`, in line order.
    pub fn on_file<'a>(&'a self, file: &'a FileKey) -> impl Iterator<Item = &'a Comment> + 'a {
        self.comments.iter().filter(move |c| &c.anchor.file == file)
    }

    /// Drafts for the agent, in the order they were written. GitHub drafts
    /// aren't: they never go to the agent.
    pub fn drafts(&self) -> Vec<CommentId> {
        self.comments
            .iter()
            .filter(|c| c.status == Status::Draft && c.kind != Kind::GitHub)
            .map(|c| c.id)
            .collect()
    }

    /// Comments not yet with the agent: drafts, queued, sending and failed.
    pub fn unsent(&self) -> usize {
        self.comments
            .iter()
            .filter(|c| c.status.unsent() && c.kind != Kind::GitHub)
            .count()
    }

    /// GitHub drafts, in the order they were written.
    pub fn github_drafts(&self) -> Vec<CommentId> {
        self.comments
            .iter()
            .filter(|c| c.kind == Kind::GitHub)
            .map(|c| c.id)
            .collect()
    }

    /// Adds a draft for the agent.
    pub fn add(&mut self, anchor: Anchor, body: &str) -> Result<CommentId, ReviewError> {
        self.add_as(Kind::Comment, None, anchor, body)
    }

    /// Adds a draft of `kind`, written by `by` (none for the user).
    pub fn add_as(
        &mut self,
        kind: Kind,
        by: Option<String>,
        anchor: Anchor,
        body: &str,
    ) -> Result<CommentId, ReviewError> {
        let body = body.trim();
        if body.is_empty() {
            return Err(ReviewError::EmptyComment);
        }
        let id = CommentId(self.next_comment);
        self.next_comment += 1;
        self.comments.push(Comment {
            id,
            kind,
            by,
            anchor,
            body: body.to_owned(),
            status: Status::Draft,
            outdated: false,
            delivery: None,
        });
        Ok(id)
    }

    /// Changes a comment's text. A queued or failed comment leaves its
    /// delivery and is a draft again, so nothing is sent that wasn't
    /// previewed.
    pub fn edit(&mut self, id: CommentId, body: &str) -> Result<(), ReviewError> {
        let body = body.trim();
        if body.is_empty() {
            return Err(ReviewError::EmptyComment);
        }
        self.take_back(id)?;
        let comment = self.comment_mut(id)?;
        comment.body = body.to_owned();
        Ok(())
    }

    /// Makes a draft a comment, a question or a GitHub draft.
    pub fn set_kind(&mut self, id: CommentId, kind: Kind) -> Result<(), ReviewError> {
        self.take_back(id)?;
        self.comment_mut(id)?.kind = kind;
        Ok(())
    }

    /// Drops a comment. One that's queued leaves its delivery first.
    pub fn delete(&mut self, id: CommentId) -> Result<(), ReviewError> {
        let comment = self.comment_mut(id)?;
        if comment.status.unsent() {
            self.take_back(id)?;
        }
        self.comments.retain(|comment| comment.id != id);
        Ok(())
    }

    /// Points a comment somewhere else, such as an outdated one at the code
    /// it's about now.
    pub fn reattach(&mut self, id: CommentId, anchor: Anchor) -> Result<(), ReviewError> {
        self.take_back(id)?;
        let comment = self.comment_mut(id)?;
        comment.anchor = anchor;
        comment.outdated = false;
        Ok(())
    }

    /// Marks a sent or addressed comment done.
    pub fn resolve(&mut self, id: CommentId) -> Result<(), ReviewError> {
        let comment = self.comment_mut(id)?;
        if matches!(comment.status, Status::Sent | Status::Addressed) {
            comment.status = Status::Resolved;
        }
        Ok(())
    }

    /// Makes a sent, addressed or resolved comment a draft again, to send
    /// once more.
    pub fn reopen(&mut self, id: CommentId) -> Result<(), ReviewError> {
        let comment = self.comment_mut(id)?;
        if matches!(
            comment.status,
            Status::Sent | Status::Addressed | Status::Resolved
        ) {
            comment.status = Status::Draft;
            comment.delivery = None;
        }
        Ok(())
    }

    /// The exact message `ids` would be sent as. Only drafts go in.
    pub fn preview(&self, ids: &[CommentId]) -> Option<String> {
        let mut parts: Vec<Part<'_>> = self
            .comments
            .iter()
            .filter(|c| c.status == Status::Draft && c.kind != Kind::GitHub && ids.contains(&c.id))
            .map(|c| Part {
                anchor: &c.anchor,
                body: &c.body,
                question: c.kind == Kind::Question,
                outdated: c.outdated,
            })
            .collect();
        (!parts.is_empty()).then(|| compose(&mut parts))
    }

    /// Queues `ids` as one message. Comments already on their way are left
    /// where they are, so pressing Send twice queues nothing twice: when
    /// none of `ids` is a draft but they're queued together, that delivery
    /// is returned.
    pub fn queue(&mut self, ids: &[CommentId]) -> Result<DeliveryId, ReviewError> {
        let drafts: Vec<CommentId> = self
            .comments
            .iter()
            .filter(|c| c.status == Status::Draft && c.kind != Kind::GitHub && ids.contains(&c.id))
            .map(|c| c.id)
            .collect();
        if drafts.is_empty() {
            let mut live = self
                .comments
                .iter()
                .filter(|c| ids.contains(&c.id))
                .filter_map(|c| c.delivery.filter(|_| c.status.unsent()));
            return match live.next() {
                Some(delivery) if live.all(|other| other == delivery) => Ok(delivery),
                _ => Err(ReviewError::NothingToSend),
            };
        }
        let text = self.preview(&drafts).ok_or(ReviewError::NothingToSend)?;
        let id = DeliveryId(self.next_delivery);
        self.next_delivery += 1;
        for comment in &mut self.comments {
            if drafts.contains(&comment.id) {
                comment.status = Status::Queued;
                comment.delivery = Some(id);
            }
        }
        self.deliveries.push(Delivery {
            id,
            key: format!("review:{}:{}:{}", self.session, self.epoch, id.0),
            comments: drafts,
            text,
            state: DeliveryState::Queued {
                hold: Hold::Ready,
                send_now: false,
            },
            attempts: 0,
            in_doubt: false,
        });
        self.rehold();
        Ok(id)
    }

    /// Queues every draft as one message.
    pub fn queue_all(&mut self) -> Result<DeliveryId, ReviewError> {
        let drafts = self.drafts();
        self.queue(&drafts)
    }

    /// Types a held delivery even though the agent may be in a turn.
    pub fn send_now(&mut self, id: DeliveryId) -> Result<(), ReviewError> {
        let delivery = self.delivery_mut(id)?;
        if let DeliveryState::Queued { send_now, .. } = &mut delivery.state {
            *send_now = true;
        }
        Ok(())
    }

    /// Takes a queued or failed delivery back: its comments are drafts again.
    pub fn cancel(&mut self, id: DeliveryId) -> Result<(), ReviewError> {
        let delivery = self.delivery_mut(id)?;
        if delivery.in_doubt {
            return Err(ReviewError::DeliveryInFlight { delivery: id });
        }
        match delivery.state {
            DeliveryState::Sending { .. } => {
                return Err(ReviewError::DeliveryInFlight { delivery: id });
            }
            DeliveryState::Sent => return Err(ReviewError::NoDelivery { delivery: id }),
            DeliveryState::Queued { .. } | DeliveryState::Failed { .. } => {}
        }
        let comments = delivery.comments.clone();
        self.deliveries.retain(|delivery| delivery.id != id);
        for comment in &mut self.comments {
            if comments.contains(&comment.id) {
                comment.status = Status::Draft;
                comment.delivery = None;
            }
        }
        Ok(())
    }

    /// Queues a failed delivery again, with the same text and key. Does
    /// nothing to one that isn't failed, so a double press retries once.
    pub fn retry(&mut self, id: DeliveryId) -> Result<(), ReviewError> {
        let delivery = self.delivery_mut(id)?;
        if let DeliveryState::Failed { .. } = delivery.state {
            delivery.state = DeliveryState::Queued {
                hold: Hold::Ready,
                send_now: false,
            };
            let status = if delivery.in_doubt {
                Status::Sending
            } else {
                Status::Queued
            };
            let comments = delivery.comments.clone();
            self.set_status(&comments, status);
            self.rehold();
        }
        Ok(())
    }

    /// The terminal said what the agent can take now.
    pub fn set_agent(&mut self, state: AgentState) {
        self.agent = state;
        self.rehold();
    }

    /// The next delivery to type, if the agent can take one: marks it
    /// sending. Deliveries go one at a time, in the order they were queued.
    /// With the agent exited, queued deliveries fail with a fix instead.
    pub fn next_dispatch(&mut self) -> Option<Dispatch> {
        if self
            .deliveries
            .iter()
            .any(|d| matches!(d.state, DeliveryState::Sending { .. }))
        {
            return None;
        }
        self.rehold();
        let agent = self.agent;
        let delivery = self.deliveries.iter_mut().find(|delivery| {
            matches!(delivery.state, DeliveryState::Queued { .. })
        })?;
        let DeliveryState::Queued { send_now, .. } = delivery.state else {
            return None;
        };
        let go = match agent {
            AgentState::Idle => true,
            AgentState::Working | AgentState::ProbablyIdle => send_now,
            AgentState::Unknown | AgentState::Disconnected | AgentState::Exited => false,
        };
        if !go {
            return None;
        }
        delivery.attempts += 1;
        let attempt = delivery.attempts;
        delivery.state = DeliveryState::Sending { attempt };
        let dispatch = Dispatch {
            delivery: delivery.id,
            attempt,
            key: delivery.key.clone(),
            text: delivery.text.clone(),
        };
        let comments = delivery.comments.clone();
        self.set_status(&comments, Status::Sending);
        Some(dispatch)
    }

    /// Puts a dispatch back in the queue untried, such as when it couldn't
    /// be saved as sending first, so it mustn't be typed.
    pub fn abandon(&mut self, id: DeliveryId, attempt: u32) {
        let Some(delivery) = self.deliveries.iter_mut().find(|d| d.id == id) else {
            return;
        };
        if delivery.state != (DeliveryState::Sending { attempt }) {
            return;
        }
        delivery.state = DeliveryState::Queued {
            hold: Hold::Ready,
            send_now: false,
        };
        delivery.attempts -= 1;
        let status = if delivery.in_doubt {
            Status::Sending
        } else {
            Status::Queued
        };
        let comments = delivery.comments.clone();
        self.set_status(&comments, status);
        self.rehold();
    }

    /// Settles a dispatch with what the terminal said. A success always
    /// counts, even late or after a failure, since the key means it was
    /// typed once. A failure only counts for the attempt still in flight.
    pub fn finish(
        &mut self,
        id: DeliveryId,
        attempt: u32,
        result: Result<Typed, DeliveryError>,
    ) {
        let Some(delivery) = self.deliveries.iter_mut().find(|d| d.id == id) else {
            return;
        };
        let comments = delivery.comments.clone();
        match result {
            Ok(_) => {
                if delivery.state == DeliveryState::Sent {
                    return;
                }
                delivery.state = DeliveryState::Sent;
                delivery.in_doubt = false;
                self.set_status(&comments, Status::Sent);
            }
            Err(error) => {
                if delivery.state != (DeliveryState::Sending { attempt }) {
                    return;
                }
                let in_doubt = match error {
                    DeliveryError::Exited { .. } | DeliveryError::Refused { .. } => false,
                    DeliveryError::Interrupted { .. } => true,
                    DeliveryError::Disconnected { .. } => delivery.in_doubt,
                };
                let (state, status, agent) = match error {
                    DeliveryError::Exited { .. } => (
                        DeliveryState::Failed {
                            failure: Failure::Exited,
                        },
                        Status::Failed,
                        Some(AgentState::Exited),
                    ),
                    DeliveryError::Refused { reason, .. } => (
                        DeliveryState::Failed {
                            failure: Failure::Refused { reason },
                        },
                        Status::Failed,
                        None,
                    ),
                    DeliveryError::Disconnected { .. } | DeliveryError::Interrupted { .. } => (
                        DeliveryState::Queued {
                            hold: Hold::Disconnected,
                            send_now: false,
                        },
                        if in_doubt {
                            Status::Sending
                        } else {
                            Status::Queued
                        },
                        Some(AgentState::Disconnected),
                    ),
                };
                delivery.state = state;
                delivery.in_doubt = in_doubt;
                self.set_status(&comments, status);
                if let Some(agent) = agent {
                    self.agent = agent;
                }
                self.rehold();
            }
        }
    }

    /// The file changed: each comment on it follows its quoted lines, using
    /// the `(number, text)` lines of each side now. A comment whose lines
    /// changed is outdated if unsent, or addressed if the agent had it. One
    /// the agent had whose passage is gone was addressed by nothing, so it
    /// goes back to a detached draft to re-attach and send again (SM-16).
    pub fn reanchor(
        &mut self,
        file: &FileKey,
        base: &str,
        old: &[(u32, &str)],
        new: &[(u32, &str)],
    ) {
        for comment in self.comments.iter_mut().filter(|c| &c.anchor.file == file) {
            if matches!(comment.status, Status::Addressed | Status::Resolved) {
                continue;
            }
            let lines = match comment.anchor.side {
                Side::Old => old,
                Side::New => new,
            };
            match place(&comment.anchor.quoted, comment.anchor.lines, lines) {
                Placement::Unchanged => {}
                Placement::Moved { lines } => {
                    comment.anchor.lines = lines;
                    comment.anchor.base = base.to_owned();
                }
                Placement::Outdated => {
                    let anchor = &comment.anchor;
                    if comment.status != Status::Sent {
                        comment.outdated = true;
                    } else if passage_gone(&anchor.quoted, anchor.lines, lines) {
                        comment.status = Status::Draft;
                        comment.delivery = None;
                        comment.outdated = true;
                    } else {
                        comment.status = Status::Addressed;
                    }
                }
            }
        }
    }

    /// Comments whose anchor covers `line` on `side` of `file`.
    pub fn at_line<'a>(
        &'a self,
        file: &'a FileKey,
        side: Side,
        line: u32,
    ) -> impl Iterator<Item = &'a Comment> + 'a {
        self.on_file(file).filter(move |c| {
            c.anchor.side == side && c.anchor.lines.last == line
        })
    }

    /// Whether a comment on `file` ends at a line in `range`.
    pub fn ends_in(&self, file: &FileKey, side: Side, range: LineRange) -> bool {
        self.on_file(file)
            .any(|c| c.anchor.side == side && range.contains(c.anchor.lines.last))
    }

    fn comment_mut(&mut self, id: CommentId) -> Result<&mut Comment, ReviewError> {
        self.comments
            .iter_mut()
            .find(|comment| comment.id == id)
            .ok_or(ReviewError::NoComment { id })
    }

    fn delivery_mut(&mut self, id: DeliveryId) -> Result<&mut Delivery, ReviewError> {
        self.deliveries
            .iter_mut()
            .find(|delivery| delivery.id == id)
            .ok_or(ReviewError::NoDelivery { delivery: id })
    }

    /// Makes a draft of a comment that's queued or failed, taking it out of
    /// its delivery, whose text is made again without it.
    fn take_back(&mut self, id: CommentId) -> Result<(), ReviewError> {
        let comment = self.comment_mut(id)?;
        match comment.status {
            Status::Draft => return Ok(()),
            Status::Sending => return Err(ReviewError::Sending { id }),
            Status::Sent | Status::Addressed | Status::Resolved => {
                return Err(ReviewError::AlreadySent { id });
            }
            Status::Queued | Status::Failed => {}
        }
        let delivery = comment.delivery;
        if let Some(delivery) = delivery
            && self.delivery(delivery).is_some_and(|d| d.in_doubt)
        {
            return Err(ReviewError::Sending { id });
        }
        let comment = self.comment_mut(id)?;
        comment.delivery = None;
        comment.status = Status::Draft;
        let Some(delivery) = delivery else {
            return Ok(());
        };
        let Some(index) = self.deliveries.iter().position(|d| d.id == delivery) else {
            return Ok(());
        };
        self.deliveries[index].comments.retain(|other| *other != id);
        let rest = self.deliveries[index].comments.clone();
        if rest.is_empty() {
            self.deliveries.remove(index);
            return Ok(());
        }
        let mut parts: Vec<Part<'_>> = self
            .comments
            .iter()
            .filter(|c| rest.contains(&c.id))
            .map(|c| Part {
                anchor: &c.anchor,
                body: &c.body,
                question: c.kind == Kind::Question,
                outdated: c.outdated,
            })
            .collect();
        let text = compose(&mut parts);
        self.deliveries[index].text = text;
        Ok(())
    }

    fn set_status(&mut self, ids: &[CommentId], status: Status) {
        for comment in &mut self.comments {
            if ids.contains(&comment.id) {
                comment.status = status;
            }
        }
    }

    /// Fits each queued delivery's hold to the agent's state. With the agent
    /// exited, queued deliveries fail: they can't be typed until it's
    /// resumed, and Retry sends them then.
    fn rehold(&mut self) {
        let agent = self.agent;
        let mut failed = Vec::new();
        for delivery in &mut self.deliveries {
            let DeliveryState::Queued { hold, .. } = &mut delivery.state else {
                continue;
            };
            let next = match agent {
                AgentState::Unknown => Some(Hold::Unknown),
                AgentState::Idle => Some(Hold::Ready),
                AgentState::Working => Some(Hold::Busy),
                AgentState::ProbablyIdle => Some(Hold::TurnEndUncertain),
                AgentState::Disconnected => Some(Hold::Disconnected),
                AgentState::Exited => None,
            };
            match next {
                Some(next) => *hold = next,
                None => {
                    delivery.state = DeliveryState::Failed {
                        failure: Failure::Exited,
                    };
                    failed.extend(delivery.comments.iter().copied());
                }
            }
        }
        self.set_status(&failed, Status::Failed);
    }
}

/// A value no other book shares: the process's random hash seed and the time.
fn epoch() -> String {
    let mut hasher = RandomState::new().build_hasher();
    if let Ok(since) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hasher.write_u128(since.as_nanos());
    }
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}
