//! The review chat (PR-22–PR-26, D59): the agent in a PR review helps the
//! user understand the change; it doesn't review it. It reads what the
//! user is looking at (`review_context`), moves the view (`review_focus`),
//! leaves notes for the user (`review_note`, never posted) and orders the
//! files (`review_prepare`). Each change applies at once and can be undone
//! from the chat; **Back** returns to where the user was before the agent
//! last moved the view.
//!
//! [`ReviewChat`] is one review's state as plain data. The daemon keeps it
//! per session, feeds it the view the app reports, and persists its view
//! and notes (PR-26); the app applies the [`Change`]s it hands out.

use std::collections::BTreeMap;

use crate::model::Side;
use crate::plan::{FileRef, Group, Note, NoteState, NoteTarget, ReviewPlan};
use crate::spec::DiffSpec;

/// PR-25: "Agent may: Focus, Notes, Order", all on by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentSwitches {
    pub focus: bool,
    pub notes: bool,
    pub order: bool,
}

impl Default for AgentSwitches {
    fn default() -> Self {
        Self {
            focus: true,
            notes: true,
            order: true,
        }
    }
}

/// One switch of [`AgentSwitches`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Switch {
    Focus,
    Notes,
    Order,
}

impl Switch {
    /// Its label in the Info panel.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Focus => "Focus",
            Self::Notes => "Notes",
            Self::Order => "Order",
        }
    }
}

/// The review chat's tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChatTool {
    Context,
    Focus,
    Note,
    NoteEdit,
    NoteDrop,
    Prepare,
}

impl ChatTool {
    /// The switch that gates it; `review_context` has none.
    #[must_use]
    pub fn switch(self) -> Option<Switch> {
        match self {
            Self::Context => None,
            Self::Focus => Some(Switch::Focus),
            Self::Note | Self::NoteEdit | Self::NoteDrop => Some(Switch::Notes),
            Self::Prepare => Some(Switch::Order),
        }
    }

    /// The MCP tool's name, as the chat line shows it.
    #[must_use]
    pub fn tool_name(self) -> &'static str {
        match self {
            Self::Context => "review_context",
            Self::Focus => "review_focus",
            Self::Note => "review_note",
            Self::NoteEdit => "review_note_edit",
            Self::NoteDrop => "review_note_drop",
            Self::Prepare => "review_prepare",
        }
    }
}

impl AgentSwitches {
    /// Whether the user lets the agent use `tool` (PR-25).
    ///
    /// # Errors
    ///
    /// [`ChatError::SwitchOff`] naming the switch the user turned off; the
    /// daemon sends it as `Denied` with `SOURCE_REVIEW_SWITCH_OFF`.
    pub fn allow(self, tool: ChatTool) -> Result<(), ChatError> {
        let on = match tool.switch() {
            None => true,
            Some(Switch::Focus) => self.focus,
            Some(Switch::Notes) => self.notes,
            Some(Switch::Order) => self.order,
        };
        match tool.switch() {
            Some(switch) if !on => Err(ChatError::SwitchOff {
                tool: tool.tool_name(),
                switch,
            }),
            _ => Ok(()),
        }
    }
}

/// Inclusive line numbers on one side of a file's diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lines {
    pub first: u32,
    pub last: u32,
}

/// Where the user is: a file, the lines on screen and what is selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub file: FileRef,
    pub side: Side,
    pub visible: Option<Lines>,
    pub selection: Option<Lines>,
}

/// A labelled range the agent highlighted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub file: FileRef,
    pub side: Side,
    pub lines: Lines,
    pub label: String,
}

/// RV-29: a file folded as viewed at this content hash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewedFile {
    pub file: FileRef,
    pub blob: String,
}

/// The review's view as the app reports it, and as it persists (PR-26).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewState {
    pub spec: DiffSpec,
    pub open: Option<Place>,
    pub order: Vec<FileRef>,
    pub groups: Vec<Group>,
    pub viewed: Vec<ViewedFile>,
    pub highlights: Vec<Highlight>,
}

/// A note for the user (PR-23): anchored like a plan's notes, so it is
/// marked outdated when its lines change (RV-34).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserNote {
    pub id: String,
    /// The agent's name, shown with the note ("note by Claude Code").
    pub author: String,
    pub note: Note,
    /// The change that made it, for Undo.
    pub undo_id: String,
    pub outdated: bool,
}

/// One change the agent made, as the app applies it and the chat lists it
/// ("● review_focus refund.rs:140  [Undo]").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub undo_id: String,
    /// Increases by one per change in a review.
    pub seq: u64,
    pub tool: ChangeTool,
    /// For the chat line: `refund.rs:140`.
    pub summary: String,
    pub what: ChangeKind,
    /// The highlights after the change.
    pub highlights: Vec<Highlight>,
}

/// Which tool made a [`Change`], or `Undo` for the chat's Undo and the
/// view's Back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeTool {
    Tool(ChatTool),
    Undo { undid: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// Move the view here (or back here).
    Focus(Option<Place>),
    /// Show this note (new, edited or restored).
    Note(UserNote),
    /// Take this note away.
    NoteDropped(String),
    /// Lay the review out in this order with these groups.
    Order {
        order: Vec<FileRef>,
        groups: Vec<Group>,
    },
}

/// What `review_context` returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Context {
    pub view: ViewState,
    pub notes: Vec<UserNote>,
    pub switches: AgentSwitches,
}

/// Why a review chat call failed. The agent reads these as tool results,
/// so each says what to do instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChatError {
    #[error(
        "{tool} is turned off: the user turned off \"Agent may: {}\" for this review in the session's Info panel. Don't retry; answer in the chat instead, or ask the user to turn it back on if they want you to",
        switch.label()
    )]
    SwitchOff { tool: &'static str, switch: Switch },

    #[error(
        "{repo}/{path} isn't in this review's diff; call review_context to see the files it shows"
    )]
    NotInReview { repo: String, path: String },

    #[error(
        "there is no note {id} you can change; call review_context for the notes, and edit or drop only your own"
    )]
    NoSuchNote { id: String },

    #[error("nothing to undo for {undo_id}: it was undone already or isn't from this review")]
    NothingToUndo { undo_id: String },

    #[error("there is nowhere to go back to: the agent hasn't moved the view")]
    NothingBack,
}

/// What a change replaced, to undo it.
#[derive(Clone, Debug)]
enum Before {
    Focus {
        place: Option<Place>,
        highlights: Vec<Highlight>,
    },
    NoNote(String),
    Note(UserNote),
    Order {
        order: Vec<FileRef>,
        groups: Vec<Group>,
        added: Vec<String>,
    },
}

#[derive(Clone, Debug)]
struct Done {
    seq: u64,
    before: Before,
    undone: bool,
}

/// One review session's chat state.
#[derive(Clone, Debug, Default)]
pub struct ReviewChat {
    view: ViewState,
    switches: AgentSwitches,
    notes: BTreeMap<String, UserNote>,
    done: BTreeMap<String, Done>,
    changes: Vec<Change>,
    next_seq: u64,
    next_note: u64,
    /// Where the user was before the agent's first move since they last
    /// moved themselves, for Back.
    back: Option<Option<Place>>,
}

impl ReviewChat {
    /// A review's chat restored from what persisted (PR-26).
    #[must_use]
    pub fn restore(view: ViewState, switches: AgentSwitches, notes: Vec<UserNote>) -> Self {
        let next_note = notes.len() as u64;
        Self {
            view,
            switches,
            notes: notes.into_iter().map(|note| (note.id.clone(), note)).collect(),
            next_note,
            ..Self::default()
        }
    }

    /// The view the app reports, debounced while the user scrolls. A move
    /// of the user's own ends what Back returns to.
    pub fn report(&mut self, view: ViewState) {
        if view.open != self.view.open {
            self.back = None;
        }
        self.view = view;
    }

    pub fn set_switches(&mut self, switches: AgentSwitches) {
        self.switches = switches;
    }

    #[must_use]
    pub fn switches(&self) -> AgentSwitches {
        self.switches
    }

    #[must_use]
    pub fn view(&self) -> &ViewState {
        &self.view
    }

    /// The notes, oldest first.
    pub fn notes(&self) -> impl Iterator<Item = &UserNote> {
        self.notes.values()
    }

    /// `review_context`.
    #[must_use]
    pub fn context(&self) -> Context {
        Context {
            view: self.view.clone(),
            notes: self.notes.values().cloned().collect(),
            switches: self.switches,
        }
    }

    /// The changes after `seq`, for the app's feed (WatchReviewChanges).
    pub fn changes_after(&self, seq: u64) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(move |change| change.seq > seq)
    }

    /// `review_focus`: open `place` and replace the agent's highlights.
    /// `shown` says whether a file is in the review's diff.
    ///
    /// # Errors
    ///
    /// [`ChatError::SwitchOff`] when Focus is off; [`ChatError::NotInReview`]
    /// for a file the diff doesn't show.
    pub fn focus(
        &mut self,
        place: Place,
        highlights: Vec<Highlight>,
        shown: impl Fn(&FileRef) -> bool,
    ) -> Result<Change, ChatError> {
        self.switches.allow(ChatTool::Focus)?;
        for file in std::iter::once(&place.file).chain(highlights.iter().map(|h| &h.file)) {
            if !shown(file) {
                return Err(not_in_review(file));
            }
        }
        let before = Before::Focus {
            place: self.view.open.clone(),
            highlights: std::mem::replace(&mut self.view.highlights, highlights),
        };
        if self.back.is_none() {
            self.back = Some(self.view.open.clone());
        }
        let summary = place_summary(&place);
        self.view.open = Some(place.clone());
        Ok(self.record(
            ChangeTool::Tool(ChatTool::Focus),
            summary,
            ChangeKind::Focus(Some(place)),
            before,
        ))
    }

    /// `review_note`: a note for the user. Anchor it first
    /// ([`crate::Checkout::anchor_notes_blocking`]) so it can go outdated.
    ///
    /// # Errors
    ///
    /// [`ChatError::SwitchOff`] when Notes is off; [`ChatError::NotInReview`]
    /// for a file the diff doesn't show.
    pub fn note(
        &mut self,
        author: &str,
        note: Note,
        shown: impl Fn(&FileRef) -> bool,
    ) -> Result<Change, ChatError> {
        self.switches.allow(ChatTool::Note)?;
        if !shown(&note.file) {
            return Err(not_in_review(&note.file));
        }
        self.next_note += 1;
        let id = format!("note-{}", self.next_note);
        let undo_id = undo_id(self.next_seq + 1);
        let summary = note_summary(&note);
        let user_note = UserNote {
            id: id.clone(),
            author: author.to_owned(),
            note,
            undo_id,
            outdated: false,
        };
        self.notes.insert(id.clone(), user_note.clone());
        Ok(self.record(
            ChangeTool::Tool(ChatTool::Note),
            summary,
            ChangeKind::Note(user_note),
            Before::NoNote(id),
        ))
    }

    /// `review_note_edit`: new text for a note the agent wrote.
    ///
    /// # Errors
    ///
    /// [`ChatError::SwitchOff`] when Notes is off; [`ChatError::NoSuchNote`].
    pub fn edit_note(&mut self, id: &str, text: String) -> Result<Change, ChatError> {
        self.switches.allow(ChatTool::NoteEdit)?;
        let note = self.notes.get_mut(id).ok_or_else(|| ChatError::NoSuchNote {
            id: id.to_owned(),
        })?;
        let before = Before::Note(note.clone());
        note.note.text = text;
        let note = note.clone();
        let summary = note_summary(&note.note);
        Ok(self.record(
            ChangeTool::Tool(ChatTool::NoteEdit),
            summary,
            ChangeKind::Note(note),
            before,
        ))
    }

    /// `review_note_drop`.
    ///
    /// # Errors
    ///
    /// [`ChatError::SwitchOff`] when Notes is off; [`ChatError::NoSuchNote`].
    pub fn drop_note(&mut self, id: &str) -> Result<Change, ChatError> {
        self.switches.allow(ChatTool::NoteDrop)?;
        let note = self.notes.remove(id).ok_or_else(|| ChatError::NoSuchNote {
            id: id.to_owned(),
        })?;
        let summary = note_summary(&note.note);
        Ok(self.record(
            ChangeTool::Tool(ChatTool::NoteDrop),
            summary,
            ChangeKind::NoteDropped(id.to_owned()),
            Before::Note(note),
        ))
    }

    /// `review_prepare` in a review chat: the plan's order and groups
    /// apply at once (PR-24), and its notes become notes for the user.
    /// Check the plan first ([`ReviewPlan::check`]).
    ///
    /// # Errors
    ///
    /// [`ChatError::SwitchOff`] when Order is off.
    pub fn prepare(&mut self, author: &str, plan: ReviewPlan) -> Result<Change, ChatError> {
        self.switches.allow(ChatTool::Prepare)?;
        let undo = undo_id(self.next_seq + 1);
        let mut added = Vec::new();
        for note in plan.notes {
            self.next_note += 1;
            let id = format!("note-{}", self.next_note);
            added.push(id.clone());
            self.notes.insert(
                id.clone(),
                UserNote {
                    id,
                    author: author.to_owned(),
                    note,
                    undo_id: undo.clone(),
                    outdated: false,
                },
            );
        }
        let before = Before::Order {
            order: std::mem::replace(&mut self.view.order, plan.order.clone()),
            groups: std::mem::replace(&mut self.view.groups, plan.groups.clone()),
            added,
        };
        let summary = format!("{} files, {} groups", plan.order.len(), plan.groups.len());
        Ok(self.record(
            ChangeTool::Tool(ChatTool::Prepare),
            summary,
            ChangeKind::Order {
                order: plan.order,
                groups: plan.groups,
            },
            before,
        ))
    }

    /// The chat line's Undo: puts back what the change replaced.
    ///
    /// # Errors
    ///
    /// [`ChatError::NothingToUndo`] for an unknown or already undone change.
    pub fn undo(&mut self, undo_id: &str) -> Result<Change, ChatError> {
        let nothing = || ChatError::NothingToUndo {
            undo_id: undo_id.to_owned(),
        };
        let done = self.done.get_mut(undo_id).ok_or_else(nothing)?;
        if done.undone {
            return Err(nothing());
        }
        done.undone = true;
        let undid = done.seq;
        let before = done.before.clone();
        let (what, summary) = match before {
            Before::Focus { place, highlights } => {
                self.view.highlights = highlights;
                self.view.open.clone_from(&place);
                let summary = place.as_ref().map_or_else(String::new, place_summary);
                (ChangeKind::Focus(place), summary)
            }
            Before::NoNote(id) => {
                self.notes.remove(&id);
                (ChangeKind::NoteDropped(id), String::new())
            }
            Before::Note(note) => {
                let summary = note_summary(&note.note);
                self.notes.insert(note.id.clone(), note.clone());
                (ChangeKind::Note(note), summary)
            }
            Before::Order {
                order,
                groups,
                added,
            } => {
                for id in added {
                    self.notes.remove(&id);
                }
                self.view.order.clone_from(&order);
                self.view.groups.clone_from(&groups);
                (ChangeKind::Order { order, groups }, String::new())
            }
        };
        let seq = self.next_seq + 1;
        self.next_seq = seq;
        Ok(self.push(Change {
            undo_id: String::new(),
            seq,
            tool: ChangeTool::Undo { undid },
            summary,
            what,
            highlights: self.view.highlights.clone(),
        }))
    }

    /// The view's Back: where the user was before the agent last moved it
    /// (PR-24). The agent's highlights stay.
    ///
    /// # Errors
    ///
    /// [`ChatError::NothingBack`] when the agent hasn't moved the view since
    /// the user last did.
    pub fn back(&mut self) -> Result<Change, ChatError> {
        let place = self.back.take().ok_or(ChatError::NothingBack)?;
        self.view.open.clone_from(&place);
        let seq = self.next_seq + 1;
        self.next_seq = seq;
        let summary = place.as_ref().map_or_else(String::new, place_summary);
        Ok(self.push(Change {
            undo_id: String::new(),
            seq,
            tool: ChangeTool::Undo { undid: 0 },
            summary,
            what: ChangeKind::Focus(place),
            highlights: self.view.highlights.clone(),
        }))
    }

    /// Marks notes outdated from their states against the current diff
    /// ([`crate::Checkout::note_states_blocking`], in [`Self::notes`]
    /// order), and moves the ones whose lines only moved. Returns the ids
    /// that became outdated.
    pub fn refresh_notes(&mut self, states: &[Option<NoteState>]) -> Vec<String> {
        let mut newly = Vec::new();
        for (note, state) in self.notes.values_mut().zip(states) {
            match state {
                Some(NoteState::Outdated) if !note.outdated => {
                    note.outdated = true;
                    newly.push(note.id.clone());
                }
                Some(NoteState::Current { first, last }) => {
                    if let NoteTarget::Lines {
                        first: f, last: l, ..
                    } = &mut note.note.target
                    {
                        *f = *first;
                        *l = *last;
                    }
                }
                _ => {}
            }
        }
        newly
    }

    fn record(
        &mut self,
        tool: ChangeTool,
        summary: String,
        what: ChangeKind,
        before: Before,
    ) -> Change {
        let seq = self.next_seq + 1;
        self.next_seq = seq;
        let id = undo_id(seq);
        self.done.insert(
            id.clone(),
            Done {
                seq,
                before,
                undone: false,
            },
        );
        self.push(Change {
            undo_id: id,
            seq,
            tool,
            summary,
            what,
            highlights: self.view.highlights.clone(),
        })
    }

    fn push(&mut self, change: Change) -> Change {
        self.changes.push(change.clone());
        change
    }
}

fn undo_id(seq: u64) -> String {
    format!("change-{seq}")
}

fn not_in_review(file: &FileRef) -> ChatError {
    ChatError::NotInReview {
        repo: file.repo.clone(),
        path: file.path.clone(),
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn place_summary(place: &Place) -> String {
    let name = file_name(&place.file.path);
    match place.selection.or(place.visible) {
        Some(lines) => format!("{name}:{}", lines.first),
        None => name.to_owned(),
    }
}

fn note_summary(note: &Note) -> String {
    let name = file_name(&note.file.path);
    match note.target {
        NoteTarget::File => name.to_owned(),
        NoteTarget::Hunk { line, .. } => format!("{name}:{line}"),
        NoteTarget::Lines { first, .. } => format!("{name}:{first}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> FileRef {
        FileRef {
            repo: "api".to_owned(),
            path: path.to_owned(),
        }
    }

    fn at(path: &str, line: u32) -> Place {
        Place {
            file: file(path),
            side: Side::New,
            visible: None,
            selection: Some(Lines {
                first: line,
                last: line,
            }),
        }
    }

    fn line_note(path: &str, line: u32) -> Note {
        Note {
            file: file(path),
            target: NoteTarget::Lines {
                first: line,
                last: line,
                side: Side::New,
            },
            text: "rounds half away from zero".to_owned(),
            anchor: None,
        }
    }

    fn everywhere(_: &FileRef) -> bool {
        true
    }

    #[test]
    fn a_tool_whose_switch_is_off_is_denied_and_changes_nothing() {
        let mut chat = ReviewChat::default();
        chat.set_switches(AgentSwitches {
            notes: false,
            ..AgentSwitches::default()
        });
        let denied = chat.note("Claude Code", line_note("refund.rs", 140), everywhere);
        assert!(matches!(
            denied,
            Err(ChatError::SwitchOff {
                switch: Switch::Notes,
                ..
            })
        ));
        assert_eq!(chat.notes().count(), 0);
        assert!(chat.focus(at("refund.rs", 140), vec![], everywhere).is_ok());
        assert!(chat.switches().allow(ChatTool::Context).is_ok());
    }

    #[test]
    fn undoing_a_focus_returns_to_the_users_place_and_highlights() {
        let mut chat = ReviewChat::default();
        chat.report(ViewState {
            open: Some(at("ledger.rs", 3)),
            ..ViewState::default()
        });
        let highlight = Highlight {
            file: file("refund.rs"),
            side: Side::New,
            lines: Lines { first: 1, last: 2 },
            label: "rounding".to_owned(),
        };
        let id = chat
            .focus(at("refund.rs", 140), vec![highlight], everywhere)
            .map(|change| change.undo_id.clone());
        let Ok(id) = id else {
            panic!("focus failed: {id:?}");
        };
        assert_eq!(chat.view().open, Some(at("refund.rs", 140)));
        assert!(chat.undo(&id).is_ok());
        assert_eq!(chat.view().open, Some(at("ledger.rs", 3)));
        assert!(chat.view().highlights.is_empty());
        assert!(matches!(chat.undo(&id), Err(ChatError::NothingToUndo { .. })));
    }

    #[test]
    fn back_returns_to_before_the_agents_first_move_since_the_user_moved() {
        let mut chat = ReviewChat::default();
        chat.report(ViewState {
            open: Some(at("a.rs", 1)),
            ..ViewState::default()
        });
        assert!(chat.focus(at("b.rs", 2), vec![], everywhere).is_ok());
        assert!(chat.focus(at("c.rs", 3), vec![], everywhere).is_ok());
        assert!(chat.back().is_ok());
        assert_eq!(chat.view().open, Some(at("a.rs", 1)));
        assert!(matches!(chat.back(), Err(ChatError::NothingBack)));
    }

    #[test]
    fn a_focus_outside_the_diff_is_refused() {
        let mut chat = ReviewChat::default();
        let only_refund = |file: &FileRef| file.path == "refund.rs";
        assert!(matches!(
            chat.focus(at("other.rs", 1), vec![], only_refund),
            Err(ChatError::NotInReview { .. })
        ));
        assert_eq!(chat.view().open, None);
    }

    #[test]
    fn notes_can_be_edited_dropped_and_restored_by_undo() {
        let mut chat = ReviewChat::default();
        let id = chat
            .note("Claude Code", line_note("refund.rs", 140), everywhere)
            .ok()
            .and_then(|change| match &change.what {
                ChangeKind::Note(note) => Some(note.id.clone()),
                _ => None,
            });
        let Some(id) = id else {
            panic!("no note made");
        };
        assert!(chat.edit_note(&id, "half to even?".to_owned()).is_ok());
        let dropped = chat.drop_note(&id).map(|change| change.undo_id.clone());
        let Ok(dropped) = dropped else {
            panic!("drop failed");
        };
        assert_eq!(chat.notes().count(), 0);
        assert!(chat.undo(&dropped).is_ok());
        let restored: Vec<&str> = chat.notes().map(|note| note.note.text.as_str()).collect();
        assert_eq!(restored, ["half to even?"]);
        assert!(matches!(
            chat.drop_note("note-99"),
            Err(ChatError::NoSuchNote { .. })
        ));
    }

    #[test]
    fn a_prepared_order_applies_and_undo_restores_the_old_one() {
        let mut chat = ReviewChat::default();
        let plan = ReviewPlan {
            order: vec![file("b.rs"), file("a.rs")],
            groups: vec![Group {
                title: "Entry points".to_owned(),
                note: String::new(),
                first: file("b.rs"),
            }],
            notes: vec![line_note("a.rs", 1)],
            ..ReviewPlan::default()
        };
        let id = chat
            .prepare("Claude Code", plan)
            .map(|change| change.undo_id.clone());
        let Ok(id) = id else {
            panic!("prepare failed");
        };
        assert_eq!(chat.view().order, vec![file("b.rs"), file("a.rs")]);
        assert_eq!(chat.notes().count(), 1);
        assert!(chat.undo(&id).is_ok());
        assert!(chat.view().order.is_empty());
        assert_eq!(chat.notes().count(), 0);
    }

    #[test]
    fn a_note_whose_lines_changed_is_marked_outdated_once() {
        let mut chat = ReviewChat::default();
        assert!(chat.note("Claude Code", line_note("refund.rs", 140), everywhere).is_ok());
        assert_eq!(chat.refresh_notes(&[Some(NoteState::Outdated)]).len(), 1);
        assert!(chat.refresh_notes(&[Some(NoteState::Outdated)]).is_empty());
        assert!(chat.notes().all(|note| note.outdated));
    }

    #[test]
    fn the_feed_resumes_after_a_seq() {
        let mut chat = ReviewChat::default();
        assert!(chat.focus(at("a.rs", 1), vec![], everywhere).is_ok());
        assert!(chat.focus(at("b.rs", 1), vec![], everywhere).is_ok());
        let after_first: Vec<u64> = chat.changes_after(1).map(|change| change.seq).collect();
        assert_eq!(after_first, [2]);
    }
}
