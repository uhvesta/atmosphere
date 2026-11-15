//! One session's review state, and the snapshot of it that's saved.

use serde::{Deserialize, Serialize};

use super::anchor::{Anchor, RenderedSpan, quote};
use super::agent::{self, AgentAction, NoteId, Spot, Switch};
use super::comments::{CommentBook, CommentId, Dispatch, Kind};
use super::memory::ReviewMemory;
use super::prepared::PreparedReview;
use super::review::{FileChange, Review, SessionId, Side};
use super::rows::ComposerAt;
use crate::error::{ActionError, ReviewError};

/// The comment box while it's open: where, what's typed so far, and the
/// comment it edits, if any. Saved with the session, so a restart keeps it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposerDraft {
    pub at: ComposerAt,
    pub text: String,
    pub editing: Option<CommentId>,
    /// What was selected, when it was opened on a rendered markdown view.
    pub rendered: Option<RenderedSpan>,
    /// A comment for the agent, a question for it, or a GitHub draft.
    #[serde(default)]
    pub kind: Kind,
    /// The agent's note it was opened from, dropped once it's a draft.
    #[serde(default)]
    pub note: Option<NoteId>,
}

/// Everything saved for one session's review. `revision` grows with every
/// change; a store keeps the newest it was given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSnapshot {
    pub revision: u64,
    pub book: CommentBook,
    pub memory: ReviewMemory,
    pub composer: Option<ComposerDraft>,
}

impl ReviewSnapshot {
    pub fn session(&self) -> &SessionId {
        self.book.session()
    }
}

/// One session's review: its changes, comments, queue and what the pane
/// remembers. Sessions share nothing, so a draft never shows in another.
#[derive(Clone, Debug)]
pub struct SessionReview {
    review: Review,
    book: CommentBook,
    memory: ReviewMemory,
    composer: Option<ComposerDraft>,
    revision: u64,
}

impl SessionReview {
    pub fn new(session: SessionId) -> Self {
        Self {
            review: Review::default(),
            book: CommentBook::new(session),
            memory: ReviewMemory::default(),
            composer: None,
            revision: 0,
        }
    }

    /// The review as it was saved, after a restart. Its changes come from
    /// the daemon again with [`SessionReview::set_review`].
    pub fn restore(snapshot: ReviewSnapshot) -> Self {
        Self {
            review: Review::default(),
            book: snapshot.book.restored(),
            memory: snapshot.memory,
            composer: snapshot.composer,
            revision: snapshot.revision,
        }
    }

    pub fn session(&self) -> &SessionId {
        self.book.session()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn review(&self) -> &Review {
        &self.review
    }

    pub fn book(&self) -> &CommentBook {
        &self.book
    }

    /// Shows an agent-prepared review the user opened (VW-8), its notes
    /// quoting their lines so they follow them (RV-32, RV-34).
    pub fn open_prepared(&mut self, view: PreparedReview) {
        let notes = self.memory.agent.add_prepared_notes(&self.review, &view);
        self.memory.open_prepared(view, notes);
        self.touch();
    }

    /// Applies a review chat agent's tool call (PR-23), with the user at
    /// `now`. Where the view should go, if it moves. Nothing changes on an
    /// error, which is the tool's answer.
    pub fn apply_agent(&mut self, action: AgentAction, now: Spot) -> Result<Option<Spot>, ActionError> {
        let moved = agent::apply(&self.review, &mut self.memory, action, now)?;
        self.touch();
        Ok(moved)
    }

    /// Takes back the agent's change `id` (PR-24). Where the view goes.
    pub fn undo_agent(&mut self, id: &str) -> Result<Option<Spot>, ActionError> {
        let moved = agent::undo(&mut self.memory, id)?;
        self.touch();
        Ok(moved)
    }

    /// Where the user was before the agent last moved the view, once.
    pub fn back(&mut self) -> Option<Spot> {
        let spot = self.memory.agent.back.take()?;
        self.memory.agent.selection = None;
        self.memory.current = spot.current.clone();
        self.touch();
        Some(spot)
    }

    /// Turns one of the "Agent may" switches on or off (PR-25).
    pub fn set_agent_may(&mut self, switch: Switch, on: bool) {
        if self.memory.agent.may.allows(switch) != on {
            self.memory.agent.may.set(switch, on);
            self.touch();
        }
    }

    /// Makes the agent's note a GitHub draft (PR-23, PR-11), marked as the
    /// agent's, and drops the note.
    pub fn note_to_draft(&mut self, note: NoteId) -> Result<CommentId, ReviewError> {
        let found = self.memory.agent.note(note).cloned().ok_or(ReviewError::NoNote { note })?;
        let anchor = self.note_anchor(&found);
        let id = self
            .book
            .add_as(Kind::GitHub, Some(found.agent.clone()), anchor, &found.text)?;
        self.memory.agent.remove_note(note);
        self.touch();
        Ok(id)
    }

    /// Drops the agent's note.
    pub fn drop_note(&mut self, note: NoteId) -> Result<(), ReviewError> {
        self.memory.agent.remove_note(note).ok_or(ReviewError::NoNote { note })?;
        self.touch();
        Ok(())
    }

    /// Opens the comment box on the agent's note, as a GitHub draft to
    /// change before adding it; adding it drops the note.
    pub fn edit_note(&mut self, note: NoteId) -> Result<(), ReviewError> {
        let found = self.memory.agent.note(note).cloned().ok_or(ReviewError::NoNote { note })?;
        let (side, lines) = found
            .at
            .as_ref()
            .filter(|_| !found.outdated)
            .map_or((Side::New, super::anchor::LineRange::FILE), |at| (at.side, at.lines));
        self.composer = Some(ComposerDraft {
            at: ComposerAt {
                file: found.file.clone(),
                side,
                lines,
            },
            text: found.text.clone(),
            editing: None,
            rendered: None,
            kind: Kind::GitHub,
            note: Some(note),
        });
        self.touch();
        Ok(())
    }

    fn note_anchor(&self, note: &super::agent::AgentNote) -> Anchor {
        let base = self.review.file(&note.file).map(|file| file.base.clone()).unwrap_or_default();
        match &note.at {
            Some(at) => Anchor {
                file: note.file.clone(),
                side: at.side,
                lines: at.lines,
                base,
                quoted: at.quoted.clone(),
                rendered: None,
            },
            None => Anchor {
                file: note.file.clone(),
                side: Side::New,
                lines: super::anchor::LineRange::FILE,
                base,
                quoted: Vec::new(),
                rendered: None,
            },
        }
    }

    pub fn memory(&self) -> &ReviewMemory {
        &self.memory
    }

    pub fn composer(&self) -> Option<&ComposerDraft> {
        self.composer.as_ref()
    }

    /// Changes the comments or queue; the change is saved.
    pub fn change<R>(&mut self, change: impl FnOnce(&mut CommentBook) -> R) -> R {
        let result = change(&mut self.book);
        self.touch();
        result
    }

    /// Taking work changes the queue only when a delivery can start.
    pub(crate) fn next_dispatch(&mut self) -> Option<Dispatch> {
        let dispatch = self.book.next_dispatch()?;
        self.touch();
        Some(dispatch)
    }

    /// Changes what the pane remembers; the change is saved.
    pub fn remember<R>(&mut self, change: impl FnOnce(&mut ReviewMemory) -> R) -> R {
        let result = change(&mut self.memory);
        self.touch();
        result
    }

    /// Marks a change to save.
    pub fn touch(&mut self) {
        self.revision += 1;
    }

    pub fn snapshot(&self) -> ReviewSnapshot {
        ReviewSnapshot {
            revision: self.revision,
            book: self.book.clone(),
            memory: self.memory.clone(),
            composer: self.composer.clone(),
        }
    }

    /// Takes the session's changes as they are now. Comments on files that
    /// changed follow their quoted lines; comments on files no longer
    /// changed have nothing left to point at.
    pub fn set_review(&mut self, review: Review) {
        let before = std::mem::replace(&mut self.review, review);
        let mut files: Vec<_> = self
            .book
            .comments()
            .iter()
            .map(|comment| comment.anchor.file.clone())
            .chain(self.memory.agent.notes.iter().map(|note| note.file.clone()))
            .chain(self.memory.agent.highlights.iter().map(|h| h.file.clone()))
            .collect();
        files.sort();
        files.dedup();
        for key in files {
            let now = self.review.file(&key);
            let then = before.file(&key);
            if now.map(|file| &file.blob) == then.map(|file| &file.blob) && then.is_some() {
                continue;
            }
            match now.and_then(FileChange::text) {
                Some(diff) => {
                    let base = now.map(|file| file.base.clone()).unwrap_or_default();
                    let old = diff.side_lines(Side::Old);
                    let new = diff.side_lines(Side::New);
                    self.book.reanchor(&key, &base, &old, &new);
                    self.memory.agent.reanchor(&key, &old, &new);
                }
                None => {
                    self.book.reanchor(&key, "", &[], &[]);
                    self.memory.agent.reanchor(&key, &[], &[]);
                }
            }
        }
        self.touch();
    }

    /// The anchor for `at` in this review, quoting its lines.
    pub fn anchor(&self, at: &ComposerAt, rendered: Option<RenderedSpan>) -> Option<Anchor> {
        let file = self.review.file(&at.file)?;
        if at.lines.is_file() {
            return Some(Anchor {
                file: at.file.clone(),
                side: at.side,
                lines: at.lines,
                base: file.base.clone(),
                quoted: Vec::new(),
                rendered: None,
            });
        }
        let diff = file.text()?;
        let lines = diff.side_lines(at.side);
        Some(Anchor {
            file: at.file.clone(),
            side: at.side,
            lines: at.lines,
            base: file.base.clone(),
            quoted: quote(&lines, at.lines),
            rendered,
        })
    }

    /// Opens the comment box at `at`, replacing one that's open.
    pub fn open_composer(&mut self, at: ComposerAt, rendered: Option<RenderedSpan>) {
        self.open_composer_as(Kind::Comment, at, rendered);
    }

    /// Opens the box for a comment, a question (Ask) or a GitHub draft.
    pub fn open_composer_as(&mut self, kind: Kind, at: ComposerAt, rendered: Option<RenderedSpan>) {
        self.composer = Some(ComposerDraft {
            at,
            text: String::new(),
            editing: None,
            rendered,
            kind,
            note: None,
        });
        self.touch();
    }

    /// Makes the open box a comment, a question or a GitHub draft, keeping
    /// its text.
    pub fn set_composer_kind(&mut self, kind: Kind) {
        if let Some(composer) = &mut self.composer
            && composer.kind != kind
        {
            composer.kind = kind;
            self.touch();
        }
    }

    /// Opens the comment box on a comment, to change its text.
    pub fn edit_composer(&mut self, id: CommentId) -> Result<(), ReviewError> {
        let comment = self.book.comment(id).ok_or(ReviewError::NoComment { id })?;
        self.composer = Some(ComposerDraft {
            at: ComposerAt {
                file: comment.anchor.file.clone(),
                side: comment.anchor.side,
                lines: comment.anchor.lines,
            },
            text: comment.body.clone(),
            editing: Some(id),
            rendered: comment.anchor.rendered.clone(),
            kind: comment.kind,
            note: None,
        });
        self.touch();
        Ok(())
    }

    pub fn set_composer_text(&mut self, text: String) {
        if let Some(composer) = &mut self.composer {
            if composer.text != text {
                composer.text = text;
                self.touch();
            }
        }
    }

    pub fn close_composer(&mut self) {
        if self.composer.take().is_some() {
            self.touch();
        }
    }

    /// Adds the comment box's text as a draft, or changes the comment it
    /// edits, and closes it. On an error it stays open with its text.
    pub fn submit_composer(&mut self) -> Result<CommentId, ReviewError> {
        let Some(draft) = self.composer.clone() else {
            return Err(ReviewError::EmptyComment);
        };
        let id = match draft.editing {
            Some(id) => {
                self.book.edit(id, &draft.text)?;
                self.book.set_kind(id, draft.kind)?;
                id
            }
            None => {
                let anchor = self
                    .anchor(&draft.at, draft.rendered.clone())
                    .unwrap_or_else(|| Anchor {
                        file: draft.at.file.clone(),
                        side: draft.at.side,
                        lines: draft.at.lines,
                        base: String::new(),
                        quoted: Vec::new(),
                        rendered: draft.rendered.clone(),
                    });
                let by = draft
                    .note
                    .and_then(|note| self.memory.agent.note(note))
                    .map(|note| note.agent.clone());
                self.book.add_as(draft.kind, by, anchor, &draft.text)?
            }
        };
        if let Some(note) = draft.note {
            self.memory.agent.remove_note(note);
        }
        self.composer = None;
        self.touch();
        Ok(id)
    }
}
