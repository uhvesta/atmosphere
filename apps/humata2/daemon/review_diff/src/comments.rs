//! A session's comments for its agent and the batches of them being typed
//! into its terminal (SPEC SM-7, SM-9–SM-21, DK-9, SE-23, RV-27).
//!
//! This is plain data and rules: no IO, no clock, no git. Callers pass the
//! time in as Unix seconds. Every method that changes a comment bumps its
//! revision, so a caller holding an older copy can be told it is stale.

use std::fmt;

use crate::{FileRef, Side};

/// Where a comment points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentAnchor {
    pub file: FileRef,
    pub side: Side,
    /// 1-based, inclusive. 0..0 is the whole file.
    pub first: u32,
    pub last: u32,
    /// The anchored lines as they were when the comment was made.
    pub quoted: Vec<String>,
    /// For a comment made in a diff, what it was compared against as the user
    /// saw it named; empty otherwise (RV-27).
    pub base: String,
    /// The blob the lines were read from; may be empty.
    pub blob: String,
    /// SM-7: the passage is gone; the user re-attaches or drops the comment.
    pub detached: bool,
}

/// Where a comment is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentStatus {
    Draft,
    Waiting,
    Sent,
    Addressed,
    Resolved,
    /// A draft for a pull request that GitHub took (PR-14). Never edited
    /// or deleted here.
    Posted,
}

impl fmt::Display for CommentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Draft => "draft",
            Self::Waiting => "waiting to send",
            Self::Sent => "sent",
            Self::Addressed => "addressed",
            Self::Resolved => "resolved",
            Self::Posted => "posted",
        })
    }
}

/// Who wrote a reply or resolved a comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommentAuthor {
    User,
    /// The agent's name, for example "claude".
    Agent(String),
}

/// One message in a comment's thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentReply {
    /// "r-<n>", numbered within the comment and never reused.
    pub id: String,
    pub by: CommentAuthor,
    pub text: String,
    pub at: i64,
}

/// Who resolved a comment, and when.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentStamp {
    pub by: CommentAuthor,
    pub at: i64,
}

/// What became of a sent comment's lines (SM-18).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addressed {
    pub at: i64,
    /// The anchored lines after the change; empty when they were deleted.
    pub now: Vec<String>,
}

/// A comment of the user's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserComment {
    /// "c-<n>", n from 1, never reused even after a drop.
    pub id: String,
    pub anchor: CommentAnchor,
    pub text: String,
    pub status: CommentStatus,
    pub created_at: i64,
    /// Starts at 1, +1 on every change to this comment.
    pub revision: u64,
    pub sent_at: Option<i64>,
    pub addressed: Option<Addressed>,
    pub resolved: Option<CommentStamp>,
    /// SM-20: set while archived; the status is left as it was.
    pub archived: Option<i64>,
    pub replies: Vec<CommentReply>,
    /// Waiting, and Humata can't tell whether an earlier try reached the
    /// terminal.
    pub unconfirmed: bool,
    /// Who wrote it. A comment for the agent is the user's.
    pub author: CommentAuthor,
    /// D284: the pull request this is a draft for, as its repository key
    /// (`owner/name`) and number. Such a comment is never typed into a
    /// terminal; it is posted to GitHub on Submit.
    pub pull_request: Option<(String, u32)>,
    /// PR-14: the link to the posted comment; set with `Posted`.
    pub posted_url: Option<String>,
    /// PR-16: the lines changed since the draft was written.
    pub outdated: bool,
    /// D287: a submit was cut short after GitHub may have taken this draft,
    /// and GitHub lists no review that holds it. Still a draft.
    pub post_unconfirmed: bool,
}

/// How far typing a delivery got, as far as Humata knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryState {
    Sending,
    Unconfirmed,
}

/// A batch of comments being typed into a terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    /// "d-<n>", never reused.
    pub key: String,
    /// Comment ids in message order.
    pub comments: Vec<String>,
    /// The message exactly as it is typed.
    pub text: String,
    pub state: DeliveryState,
    pub made_at: i64,
    /// The terminal tab it was typed into, "" until known.
    pub tab: String,
    /// The instance of the holder process that held that tab's terminal
    /// when the batch was begun, 0 until known or if it can't say. After a
    /// restart only that same process can say the batch never arrived.
    pub holder: u64,
}

/// What became of typing a delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    Yes,
    No,
    Unknown,
}

/// What ListComments asks for.
#[derive(Clone, Copy, Debug)]
pub struct CommentFilter<'a> {
    pub statuses: &'a [CommentStatus],
    pub file: Option<&'a FileRef>,
    pub archived: bool,
}

/// Everything a ledger is saved as and restored from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LedgerParts {
    pub comments: Vec<UserComment>,
    pub deliveries: Vec<Delivery>,
    pub last_comment: u64,
    pub last_delivery: u64,
}

/// Why a comment change was refused.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CommentError {
    #[error("there is no comment {id}; it may have been dropped. Refresh the list and pick another")]
    Unknown { id: String },
    #[error("there is no delivery {key}; it was already settled. Refresh the comments to see where they are")]
    UnknownDelivery { key: String },
    #[error(
        "comment {id} changed since you read it (revision {actual}, you had {expected}); read it again and repeat the change"
    )]
    Stale {
        id: String,
        expected: u64,
        actual: u64,
    },
    #[error("comment {id} is {status}; only a draft can be edited or sent. Reopen it first if it is resolved")]
    NotDraft { id: String, status: CommentStatus },
    #[error("comment {id} has no text; write something before sending it")]
    Blank { id: String },
    #[error("there is nothing to send; pick at least one comment with text")]
    NothingToSend,
    #[error("comment {id} is being typed into the terminal; wait for it to finish, or take the batch back first")]
    InDelivery { id: String },
    #[error("comment {id} is {status}, but only a comment waiting to send can be taken back")]
    NotWaiting { id: String, status: CommentStatus },
    #[error("comment {id} is {status}; this needs a comment that was already typed into the terminal")]
    NotSent { id: String, status: CommentStatus },
    #[error("resolving comment {id} needs a reply that says what was done; add one and try again")]
    ReplyNeeded { id: String },
    #[error("comment {id} is already resolved; reopen it if more work is needed")]
    AlreadyResolved { id: String },
    #[error("comment {id} is {status}, not done; only a resolved or addressed comment can be reopened")]
    NotResolved { id: String, status: CommentStatus },
    #[error("comment {id} has lost its lines; re-attach it to a passage or drop it before sending")]
    Detached { id: String },
    #[error("comment {id} is archived; unarchive it first")]
    Archived { id: String },
    #[error("delivery {key} is not waiting on an answer; only an unconfirmed delivery can be sent again")]
    NotUnconfirmed { key: String },
    #[error("comment {id} is a draft for a pull request, not for the agent; it is posted to GitHub from Submit review")]
    ForPullRequest { id: String },
    #[error("comment {id} was already posted to GitHub; change or delete it there")]
    AlreadyPosted { id: String },
    #[error("comment {id} was written by someone else; only its author changes it")]
    NotYours { id: String },
}

/// A session's comments and the deliveries in flight.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommentLedger {
    comments: Vec<UserComment>,
    deliveries: Vec<Delivery>,
    last_comment: u64,
    last_delivery: u64,
}

fn blank(text: &str) -> bool {
    text.trim().is_empty()
}

impl CommentLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_parts(parts: LedgerParts) -> Self {
        Self {
            comments: parts.comments,
            deliveries: parts.deliveries,
            last_comment: parts.last_comment,
            last_delivery: parts.last_delivery,
        }
    }

    #[must_use]
    pub fn parts(&self) -> LedgerParts {
        LedgerParts {
            comments: self.comments.clone(),
            deliveries: self.deliveries.clone(),
            last_comment: self.last_comment,
            last_delivery: self.last_delivery,
        }
    }

    #[must_use]
    pub fn comments(&self) -> &[UserComment] {
        &self.comments
    }

    #[must_use]
    pub fn comment(&self, id: &str) -> Option<&UserComment> {
        self.comments.iter().find(|c| c.id == id)
    }

    #[must_use]
    pub fn deliveries(&self) -> &[Delivery] {
        &self.deliveries
    }

    #[must_use]
    pub fn delivery(&self, key: &str) -> Option<&Delivery> {
        self.deliveries.iter().find(|d| d.key == key)
    }

    fn index(&self, id: &str) -> Result<usize, CommentError> {
        self.comments
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| CommentError::Unknown { id: id.to_owned() })
    }

    fn in_delivery(&self, id: &str) -> bool {
        self.deliveries
            .iter()
            .any(|d| d.comments.iter().any(|c| c == id))
    }

    fn check_revision(&self, i: usize, expected: Option<u64>) -> Result<(), CommentError> {
        let c = &self.comments[i];
        match expected {
            Some(e) if e != c.revision => Err(CommentError::Stale {
                id: c.id.clone(),
                expected: e,
                actual: c.revision,
            }),
            _ => Ok(()),
        }
    }

    fn refuse_in_delivery(&self, i: usize) -> Result<(), CommentError> {
        let id = &self.comments[i].id;
        if self.in_delivery(id) {
            Err(CommentError::InDelivery { id: id.clone() })
        } else {
            Ok(())
        }
    }

    /// A new draft (SM-9). Any text, even empty, is a draft.
    pub fn add(&mut self, anchor: CommentAnchor, text: &str, at: i64) -> &UserComment {
        self.last_comment += 1;
        self.comments.push(UserComment {
            id: format!("c-{}", self.last_comment),
            anchor,
            text: text.to_owned(),
            status: CommentStatus::Draft,
            created_at: at,
            revision: 1,
            sent_at: None,
            addressed: None,
            resolved: None,
            archived: None,
            replies: Vec::new(),
            unconfirmed: false,
            author: CommentAuthor::User,
            pull_request: None,
            posted_url: None,
            outdated: false,
            post_unconfirmed: false,
        });
        &self.comments[self.comments.len() - 1]
    }

    /// A new draft for pull request `number` of `repo` (`owner/name`), by
    /// `author` (D284). Never typed into a terminal.
    pub fn add_for_pull_request(
        &mut self,
        anchor: CommentAnchor,
        text: &str,
        author: CommentAuthor,
        repo: &str,
        number: u32,
        at: i64,
    ) -> &UserComment {
        self.add(anchor, text, at);
        let last = self.comments.len() - 1;
        let c = &mut self.comments[last];
        c.author = author;
        c.pull_request = Some((repo.to_owned(), number));
        &self.comments[last]
    }

    /// A review chat note becomes a draft for pull request `pr` (its
    /// repository key and number), keeping its own `id` (D284: in place,
    /// never a copy). A comment already holding `id` is returned as it is.
    pub fn adopt_note(
        &mut self,
        id: &str,
        anchor: CommentAnchor,
        text: &str,
        author: CommentAuthor,
        pr: (&str, u32),
        at: i64,
    ) -> &UserComment {
        if let Some(i) = self.comments.iter().position(|c| c.id == id) {
            return &self.comments[i];
        }
        self.add_for_pull_request(anchor, text, author, pr.0, pr.1, at);
        let last = self.comments.len() - 1;
        // `add` took the next `c-<n>`; the note keeps its own name and the
        // number is not spent.
        self.last_comment -= 1;
        id.clone_into(&mut self.comments[last].id);
        &self.comments[last]
    }

    /// GitHub took the draft (PR-14, D287).
    pub fn mark_posted(&mut self, id: &str, url: &str) -> Result<&UserComment, CommentError> {
        let i = self.index(id)?;
        let c = &mut self.comments[i];
        c.status = CommentStatus::Posted;
        c.posted_url = Some(url.to_owned());
        c.post_unconfirmed = false;
        c.revision += 1;
        Ok(&self.comments[i])
    }

    /// A submit was cut short and GitHub lists no review holding the draft
    /// (D287), or the user settled that. Returns whether anything changed.
    pub fn set_post_unconfirmed(&mut self, id: &str, unconfirmed: bool) -> bool {
        let Ok(i) = self.index(id) else {
            return false;
        };
        let c = &mut self.comments[i];
        if c.post_unconfirmed == unconfirmed || c.status != CommentStatus::Draft {
            return false;
        }
        c.post_unconfirmed = unconfirmed;
        c.revision += 1;
        true
    }

    /// The lines of a draft for a pull request changed (PR-16). Returns
    /// whether anything changed.
    pub fn set_outdated(&mut self, id: &str, outdated: bool) -> bool {
        let Ok(i) = self.index(id) else {
            return false;
        };
        let c = &mut self.comments[i];
        if c.outdated == outdated || c.pull_request.is_none() {
            return false;
        }
        c.outdated = outdated;
        c.revision += 1;
        true
    }

    fn refuse_posted(&self, i: usize) -> Result<(), CommentError> {
        let c = &self.comments[i];
        if c.status == CommentStatus::Posted {
            Err(CommentError::AlreadyPosted { id: c.id.clone() })
        } else {
            Ok(())
        }
    }

    fn refuse_pull_request(&self, i: usize) -> Result<(), CommentError> {
        let c = &self.comments[i];
        if c.pull_request.is_some() {
            Err(CommentError::ForPullRequest { id: c.id.clone() })
        } else {
            Ok(())
        }
    }

    /// Changes a draft's text.
    pub fn edit(
        &mut self,
        id: &str,
        text: &str,
        expected: Option<u64>,
    ) -> Result<&UserComment, CommentError> {
        let i = self.index(id)?;
        self.check_revision(i, expected)?;
        self.refuse_posted(i)?;
        let c = &self.comments[i];
        if c.archived.is_some() {
            return Err(CommentError::Archived { id: c.id.clone() });
        }
        if c.status != CommentStatus::Draft {
            return Err(CommentError::NotDraft {
                id: c.id.clone(),
                status: c.status,
            });
        }
        let c = &mut self.comments[i];
        c.text = text.to_owned();
        c.revision += 1;
        Ok(&self.comments[i])
    }

    /// Deletes a comment for good (SM-21: Drop deletes, archive keeps).
    pub fn remove(&mut self, id: &str, expected: Option<u64>) -> Result<UserComment, CommentError> {
        let i = self.index(id)?;
        self.check_revision(i, expected)?;
        self.refuse_posted(i)?;
        self.refuse_in_delivery(i)?;
        Ok(self.comments.remove(i))
    }

    /// Points a comment at a new passage (SM-7).
    pub fn reattach(
        &mut self,
        id: &str,
        anchor: CommentAnchor,
        expected: Option<u64>,
    ) -> Result<&UserComment, CommentError> {
        let i = self.index(id)?;
        self.check_revision(i, expected)?;
        self.refuse_posted(i)?;
        self.refuse_in_delivery(i)?;
        let c = &mut self.comments[i];
        c.anchor = CommentAnchor {
            detached: false,
            ..anchor
        };
        c.outdated = false;
        c.revision += 1;
        Ok(&self.comments[i])
    }

    fn sendable_indices(&self, ids: &[String]) -> Result<Vec<usize>, CommentError> {
        let mut out: Vec<usize> = Vec::new();
        for id in ids {
            let i = self.index(id)?;
            if !out.contains(&i) {
                out.push(i);
            }
        }
        if out.is_empty() {
            return Err(CommentError::NothingToSend);
        }
        for &i in &out {
            self.refuse_pull_request(i)?;
            let c = &self.comments[i];
            if c.archived.is_some() {
                return Err(CommentError::Archived { id: c.id.clone() });
            }
            if !matches!(c.status, CommentStatus::Draft | CommentStatus::Waiting) {
                return Err(CommentError::NotDraft {
                    id: c.id.clone(),
                    status: c.status,
                });
            }
            if c.anchor.detached {
                return Err(CommentError::Detached { id: c.id.clone() });
            }
            if self.in_delivery(&c.id) {
                return Err(CommentError::InDelivery { id: c.id.clone() });
            }
            if blank(&c.text) {
                return Err(CommentError::Blank { id: c.id.clone() });
            }
        }
        Ok(out)
    }

    /// The checks `begin` makes, without changing anything. Duplicate ids
    /// are dropped, keeping the first position.
    pub fn sendable(&self, ids: &[String]) -> Result<Vec<&UserComment>, CommentError> {
        Ok(self
            .sendable_indices(ids)?
            .into_iter()
            .map(|i| &self.comments[i])
            .collect())
    }

    /// Starts typing a batch. All or nothing.
    pub fn begin(
        &mut self,
        ids: &[String],
        text: String,
        at: i64,
    ) -> Result<&Delivery, CommentError> {
        let indices = self.sendable_indices(ids)?;
        if blank(&text) {
            return Err(CommentError::NothingToSend);
        }
        let mut members = Vec::with_capacity(indices.len());
        for i in indices {
            let c = &mut self.comments[i];
            c.status = CommentStatus::Waiting;
            c.unconfirmed = false;
            c.revision += 1;
            members.push(c.id.clone());
        }
        self.last_delivery += 1;
        self.deliveries.push(Delivery {
            key: format!("d-{}", self.last_delivery),
            comments: members,
            text,
            state: DeliveryState::Sending,
            made_at: at,
            tab: String::new(),
            holder: 0,
        });
        Ok(&self.deliveries[self.deliveries.len() - 1])
    }

    fn delivery_index(&self, key: &str) -> Result<usize, CommentError> {
        self.deliveries
            .iter()
            .position(|d| d.key == key)
            .ok_or_else(|| CommentError::UnknownDelivery {
                key: key.to_owned(),
            })
    }

    /// Records the terminal tab a delivery is typed into and the holder
    /// instance that holds it.
    pub fn set_target(&mut self, key: &str, tab: &str, holder: u64) -> Result<(), CommentError> {
        let i = self.delivery_index(key)?;
        self.deliveries[i].tab = tab.to_owned();
        self.deliveries[i].holder = holder;
        Ok(())
    }

    /// Records what became of typing a delivery; returns the comment ids it
    /// touched.
    pub fn settle(
        &mut self,
        key: &str,
        typed: Typed,
        at: i64,
    ) -> Result<Vec<String>, CommentError> {
        let i = self.delivery_index(key)?;
        let ids = self.deliveries[i].comments.clone();
        match typed {
            Typed::Unknown => self.deliveries[i].state = DeliveryState::Unconfirmed,
            Typed::Yes | Typed::No => {
                self.deliveries.remove(i);
            }
        }
        let mut touched = Vec::new();
        for id in ids {
            let Some(c) = self.comments.iter_mut().find(|c| c.id == id) else {
                continue;
            };
            match typed {
                Typed::Yes => {
                    c.status = CommentStatus::Sent;
                    c.sent_at = Some(at);
                    c.unconfirmed = false;
                }
                Typed::No => c.unconfirmed = false,
                Typed::Unknown => c.unconfirmed = true,
            }
            c.revision += 1;
            touched.push(id);
        }
        Ok(touched)
    }

    /// Deliveries whose fate nobody knows: the ones still Sending. After a
    /// restart these are the ones to ask about.
    #[must_use]
    pub fn in_doubt(&self) -> Vec<&Delivery> {
        self.deliveries
            .iter()
            .filter(|d| d.state == DeliveryState::Sending)
            .collect()
    }

    /// The user chose to send an unconfirmed delivery again.
    pub fn retry(&mut self, key: &str) -> Result<&Delivery, CommentError> {
        let i = self.delivery_index(key)?;
        if self.deliveries[i].state != DeliveryState::Unconfirmed {
            return Err(CommentError::NotUnconfirmed {
                key: key.to_owned(),
            });
        }
        self.deliveries[i].state = DeliveryState::Sending;
        Ok(&self.deliveries[i])
    }

    /// Turns waiting comments back into drafts (SE-23 Remove); returns every
    /// comment id that changed.
    pub fn take_back(&mut self, ids: &[String]) -> Result<Vec<String>, CommentError> {
        let mut picked: Vec<usize> = Vec::new();
        for id in ids {
            let i = self.index(id)?;
            let c = &self.comments[i];
            if c.status != CommentStatus::Waiting {
                return Err(CommentError::NotWaiting {
                    id: c.id.clone(),
                    status: c.status,
                });
            }
            let sending = self.deliveries.iter().any(|d| {
                d.state == DeliveryState::Sending && d.comments.iter().any(|m| *m == c.id)
            });
            if sending {
                return Err(CommentError::InDelivery { id: c.id.clone() });
            }
            if !picked.contains(&i) {
                picked.push(i);
            }
        }
        let mut changed: Vec<String> = Vec::new();
        for i in picked {
            let id = self.comments[i].id.clone();
            if changed.contains(&id) {
                continue;
            }
            let batch = self
                .deliveries
                .iter()
                .position(|d| d.comments.iter().any(|m| *m == id));
            let members = match batch {
                Some(b) => self.deliveries.remove(b).comments,
                None => vec![id],
            };
            for m in members {
                if let Some(c) = self.comments.iter_mut().find(|c| c.id == m) {
                    c.status = CommentStatus::Draft;
                    c.unconfirmed = false;
                    c.revision += 1;
                    if !changed.contains(&m) {
                        changed.push(m);
                    }
                }
            }
        }
        Ok(changed)
    }

    /// Resolves a comment (SM-11, SM-18, SM-19).
    pub fn resolve(
        &mut self,
        id: &str,
        by: CommentAuthor,
        reply: Option<&str>,
        at: i64,
        expected: Option<u64>,
    ) -> Result<&UserComment, CommentError> {
        let i = self.index(id)?;
        self.check_revision(i, expected)?;
        self.refuse_pull_request(i)?;
        let c = &self.comments[i];
        let reply = reply.filter(|r| !blank(r));
        match &by {
            CommentAuthor::User => {
                if c.status == CommentStatus::Resolved {
                    return Ok(&self.comments[i]);
                }
                if c.status == CommentStatus::Waiting {
                    self.refuse_in_delivery(i)?;
                }
            }
            CommentAuthor::Agent(_) => {
                if c.archived.is_some() {
                    return Err(CommentError::Archived { id: c.id.clone() });
                }
                if c.status == CommentStatus::Resolved {
                    return Err(CommentError::AlreadyResolved { id: c.id.clone() });
                }
                if !matches!(c.status, CommentStatus::Sent | CommentStatus::Addressed) {
                    return Err(CommentError::NotSent {
                        id: c.id.clone(),
                        status: c.status,
                    });
                }
                if reply.is_none() {
                    return Err(CommentError::ReplyNeeded { id: c.id.clone() });
                }
            }
        }
        let c = &mut self.comments[i];
        if let Some(text) = reply {
            let n = c.replies.len() + 1;
            c.replies.push(CommentReply {
                id: format!("r-{n}"),
                by: by.clone(),
                text: text.to_owned(),
                at,
            });
        }
        c.status = CommentStatus::Resolved;
        c.resolved = Some(CommentStamp { by, at });
        c.unconfirmed = false;
        c.revision += 1;
        Ok(&self.comments[i])
    }

    /// Puts a done (resolved or addressed) comment back to a draft (SM-11, D851); replies stay.
    pub fn reopen(
        &mut self,
        id: &str,
        expected: Option<u64>,
    ) -> Result<&UserComment, CommentError> {
        let i = self.index(id)?;
        self.check_revision(i, expected)?;
        let c = &mut self.comments[i];
        if !matches!(c.status, CommentStatus::Resolved | CommentStatus::Addressed) {
            return Err(CommentError::NotResolved {
                id: c.id.clone(),
                status: c.status,
            });
        }
        c.status = CommentStatus::Draft;
        c.resolved = None;
        c.addressed = None;
        c.sent_at = None;
        c.revision += 1;
        Ok(&self.comments[i])
    }

    /// Adds to the thread of a comment that was typed (DK-9).
    pub fn reply(
        &mut self,
        id: &str,
        by: CommentAuthor,
        text: &str,
        at: i64,
    ) -> Result<&CommentReply, CommentError> {
        let i = self.index(id)?;
        let c = &mut self.comments[i];
        if !matches!(
            c.status,
            CommentStatus::Sent | CommentStatus::Addressed | CommentStatus::Resolved
        ) {
            return Err(CommentError::NotSent {
                id: c.id.clone(),
                status: c.status,
            });
        }
        if c.archived.is_some() {
            return Err(CommentError::Archived { id: c.id.clone() });
        }
        if blank(text) {
            return Err(CommentError::Blank { id: c.id.clone() });
        }
        let n = c.replies.len() + 1;
        c.replies.push(CommentReply {
            id: format!("r-{n}"),
            by,
            text: text.to_owned(),
            at,
        });
        c.revision += 1;
        let last = c.replies.len() - 1;
        Ok(&self.comments[i].replies[last])
    }

    /// Sets a comment aside without deleting it (SM-20).
    pub fn archive(&mut self, id: &str, at: i64) -> Result<&UserComment, CommentError> {
        let i = self.index(id)?;
        if self.comments[i].archived.is_some() {
            return Ok(&self.comments[i]);
        }
        self.refuse_in_delivery(i)?;
        let c = &mut self.comments[i];
        if c.status == CommentStatus::Waiting {
            c.status = CommentStatus::Draft;
            c.unconfirmed = false;
        }
        c.archived = Some(at);
        c.revision += 1;
        Ok(&self.comments[i])
    }

    /// Brings an archived comment back (SM-21).
    pub fn unarchive(&mut self, id: &str) -> Result<&UserComment, CommentError> {
        let i = self.index(id)?;
        let c = &mut self.comments[i];
        if c.archived.is_some() {
            c.archived = None;
            c.revision += 1;
        }
        Ok(&self.comments[i])
    }

    /// The passage only moved to these lines (SM-7, SM-16). Returns whether
    /// anything changed.
    pub fn moved(&mut self, id: &str, first: u32, last: u32) -> bool {
        let Ok(i) = self.index(id) else {
            return false;
        };
        let c = &mut self.comments[i];
        if c.anchor.first == first && c.anchor.last == last && !c.anchor.detached {
            return false;
        }
        c.anchor.first = first;
        c.anchor.last = last;
        c.anchor.detached = false;
        c.revision += 1;
        true
    }

    /// The user changed the anchored lines themselves: they are now `now`,
    /// at `first..=last`, and the comment stays as it was, a sent one sent.
    /// Only a sent comment follows the new text; a draft or waiting one is
    /// [`changed`](Self::changed). Returns whether anything changed.
    pub fn rebaseline(&mut self, id: &str, first: u32, last: u32, now: Vec<String>) -> bool {
        let Ok(i) = self.index(id) else {
            return false;
        };
        let c = &mut self.comments[i];
        if c.archived.is_some() || c.status != CommentStatus::Sent {
            return false;
        }
        c.anchor.first = first;
        c.anchor.last = last;
        c.anchor.quoted = now;
        c.anchor.detached = false;
        c.revision += 1;
        true
    }

    /// The anchored lines changed but are still there, at `first..=last`
    /// (SM-7, SM-16, SM-17). A sent comment becomes addressed at those
    /// lines. Returns whether anything changed.
    pub fn changed(&mut self, id: &str, first: u32, last: u32, now: Vec<String>, at: i64) -> bool {
        let Ok(i) = self.index(id) else {
            return false;
        };
        let c = &mut self.comments[i];
        if c.archived.is_some() {
            return false;
        }
        match c.status {
            CommentStatus::Sent => {
                c.status = CommentStatus::Addressed;
                c.anchor.first = first;
                c.anchor.last = last;
                c.addressed = Some(Addressed { at, now });
            }
            CommentStatus::Draft | CommentStatus::Waiting => {
                if c.anchor.detached {
                    return false;
                }
                c.anchor.detached = true;
            }
            CommentStatus::Addressed | CommentStatus::Resolved | CommentStatus::Posted => return false,
        }
        c.revision += 1;
        true
    }

    /// The anchored lines are gone (SM-7, D190). A sent comment returns to a
    /// detached draft, sent_at and addressed cleared, so it is sent anew once
    /// re-attached; a draft or waiting comment is detached. Returns whether
    /// anything changed.
    pub fn gone(&mut self, id: &str) -> bool {
        let Ok(i) = self.index(id) else {
            return false;
        };
        let c = &mut self.comments[i];
        if c.archived.is_some() {
            return false;
        }
        match c.status {
            CommentStatus::Sent => {
                c.status = CommentStatus::Draft;
                c.anchor.detached = true;
                c.sent_at = None;
                c.addressed = None;
            }
            CommentStatus::Draft | CommentStatus::Waiting => {
                if c.anchor.detached {
                    return false;
                }
                c.anchor.detached = true;
            }
            CommentStatus::Addressed | CommentStatus::Resolved | CommentStatus::Posted => return false,
        }
        c.revision += 1;
        true
    }

    /// Comments oldest first, as asked (SM-12, SM-21).
    #[must_use]
    pub fn list<'a>(&'a self, filter: &CommentFilter<'_>) -> Vec<&'a UserComment> {
        self.comments
            .iter()
            .filter(|c| c.archived.is_some() == filter.archived)
            .filter(|c| {
                if filter.statuses.is_empty() {
                    filter.archived || c.status != CommentStatus::Resolved
                } else {
                    filter.statuses.contains(&c.status)
                }
            })
            .filter(|c| match filter.file {
                None => true,
                Some(f) => {
                    c.anchor.file.path == f.path && (f.repo.is_empty() || c.anchor.file.repo == f.repo)
                }
            })
            .collect()
    }

    /// Drafts for the agent that are not archived or detached (SM-15).
    /// Drafts for a pull request are not counted.
    #[must_use]
    pub fn draft_count(&self) -> u32 {
        let n = self
            .comments
            .iter()
            .filter(|c| {
                c.status == CommentStatus::Draft
                    && c.archived.is_none()
                    && !c.anchor.detached
                    && c.pull_request.is_none()
            })
            .count();
        u32::try_from(n).unwrap_or(u32::MAX)
    }
}
