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
use crate::stack::Stack;

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
    /// The stack the review is told as (D800), if it has one.
    pub stack: Option<Stack>,
    /// The title of the layer showing; `None` is the whole stack.
    pub layer: Option<String>,
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
    /// A later diff marked a note outdated or moved it (RV-34); it has no
    /// undo id, since nobody made it.
    Diff,
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
        stack: Option<Stack>,
        layer: Option<String>,
    },
    /// Show this layer (`None` is the whole stack).
    Layer(Option<String>),
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

    #[error(
        "nothing to undo for {undo_id}: it was undone already, is older than the review's last 256 changes, or isn't from this review"
    )]
    NothingToUndo { undo_id: String },

    #[error(
        "{}",
        no_such_layer_message(layer, known)
    )]
    NoSuchLayer { layer: String, known: Vec<String> },

    #[error("there is nowhere to go back to: the agent hasn't moved the view")]
    NothingBack,
}

fn no_such_layer_message(layer: &str, known: &[String]) -> String {
    if known.is_empty() {
        "this review shows no stack; send one with review_prepare".to_owned()
    } else {
        format!(
            "the stack has no layer titled \"{layer}\"; its layers are {}. Call review_context for the stack",
            known.join(", ")
        )
    }
}

/// The title a layer has in `stack`, matching trimmed and ignoring case.
fn layer_title(stack: &Stack, wanted: &str) -> Option<String> {
    let wanted = wanted.trim().to_lowercase();
    stack
        .layers
        .iter()
        .find(|layer| layer.title.trim().to_lowercase() == wanted)
        .map(|layer| layer.title.clone())
}

/// At most this many changes can be undone; the oldest is forgotten
/// first, so a saved review stays small.
const UNDO_MAX: usize = 256;

/// What a change replaced, to undo it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Replaced {
    /// The place open before and the agent's highlights.
    Focus {
        place: Option<Place>,
        highlights: Vec<Highlight>,
    },
    /// No note: the change added this one.
    NoNote(String),
    /// The note as it was before an edit or a drop.
    Note(UserNote),
    /// The order and groups before a plan, and the notes it added.
    Order {
        order: Vec<FileRef>,
        groups: Vec<Group>,
        added: Vec<String>,
        stack: Option<Stack>,
        layer: Option<String>,
    },
    /// The layer showing before the agent moved it.
    Layer { layer: Option<String> },
}

/// A change Undo can take back, as it is saved with the review so Undo
/// still works after a restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Undoable {
    pub undo_id: String,
    pub seq: u64,
    pub undone: bool,
    pub before: Replaced,
}

#[derive(Clone, Debug)]
struct Done {
    seq: u64,
    before: Replaced,
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
    /// A review's chat restored from what persisted (PR-26). `last_seq`
    /// and `last_note` are [`ReviewChat::last_seq`] and
    /// [`ReviewChat::last_note`] as they were saved, so new changes and
    /// notes never reuse an id the app or the agent already holds, even
    /// one of a note since dropped.
    #[must_use]
    pub fn restore(
        view: ViewState,
        switches: AgentSwitches,
        notes: Vec<UserNote>,
        last_seq: u64,
        last_note: u64,
    ) -> Self {
        let next_note = notes
            .iter()
            .filter_map(|note| note.id.strip_prefix("note-")?.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .max(last_note);
        Self {
            view,
            switches,
            notes: notes.into_iter().map(|note| (note.id.clone(), note)).collect(),
            next_seq: last_seq,
            next_note,
            ..Self::default()
        }
    }

    /// The chat with the Undo history saved with it
    /// ([`ReviewChat::undoable`]); only the newest 256 are kept.
    #[must_use]
    pub fn with_undoable(mut self, history: Vec<Undoable>) -> Self {
        for entry in history {
            self.done.insert(
                entry.undo_id,
                Done {
                    seq: entry.seq,
                    before: entry.before,
                    undone: entry.undone,
                },
            );
        }
        self.forget_oldest();
        self
    }

    /// The changes Undo can still take back (and those already undone, so
    /// a second Undo still says so), oldest first, to save with the chat.
    #[must_use]
    pub fn undoable(&self) -> Vec<Undoable> {
        let mut history: Vec<Undoable> = self
            .done
            .iter()
            .map(|(undo_id, done)| Undoable {
                undo_id: undo_id.clone(),
                seq: done.seq,
                undone: done.undone,
                before: done.before.clone(),
            })
            .collect();
        history.sort_by_key(|entry| entry.seq);
        history
    }

    /// The number of the last change, to save with the chat.
    #[must_use]
    pub fn last_seq(&self) -> u64 {
        self.next_seq
    }

    /// The number of the last note given out, to save with the chat.
    #[must_use]
    pub fn last_note(&self) -> u64 {
        self.next_note
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
        let before = Replaced::Focus {
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

    /// Moves the view to a layer of the stack, or to the whole stack for
    /// `None`. The title is matched trimmed, ignoring case.
    ///
    /// # Errors
    ///
    /// [`ChatError::SwitchOff`] when Focus is off; [`ChatError::NoSuchLayer`]
    /// when there is no stack or it has no such layer.
    pub fn focus_layer(&mut self, layer: Option<String>) -> Result<Change, ChatError> {
        self.switches.allow(ChatTool::Focus)?;
        let title = match layer {
            None => None,
            Some(wanted) => {
                let found = self
                    .view
                    .stack
                    .as_ref()
                    .and_then(|stack| layer_title(stack, &wanted));
                match found {
                    Some(title) => Some(title),
                    None => {
                        let known = self.view.stack.as_ref().map_or_else(Vec::new, |stack| {
                            stack.layers.iter().map(|layer| layer.title.clone()).collect()
                        });
                        return Err(ChatError::NoSuchLayer {
                            layer: wanted,
                            known,
                        });
                    }
                }
            }
        };
        let before = Replaced::Layer {
            layer: std::mem::replace(&mut self.view.layer, title.clone()),
        };
        let summary = title.clone().unwrap_or_else(|| "Whole stack".to_owned());
        Ok(self.record(
            ChangeTool::Tool(ChatTool::Focus),
            summary,
            ChangeKind::Layer(title),
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
            Replaced::NoNote(id),
        ))
    }

    /// `review_note_edit`: new text for a note the agent wrote.
    ///
    /// # Errors
    ///
    /// [`ChatError::SwitchOff`] when Notes is off; [`ChatError::NoSuchNote`].
    pub fn edit_note(&mut self, id: &str, text: String) -> Result<Change, ChatError> {
        self.switches.allow(ChatTool::NoteEdit)?;
        let note = self
            .notes
            .get_mut(id)
            .ok_or_else(|| ChatError::NoSuchNote { id: id.to_owned() })?;
        let before = Replaced::Note(note.clone());
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
            Replaced::Note(note),
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
        let layer = match (&plan.stack, &self.view.layer) {
            (Some(stack), Some(showing)) => layer_title(stack, showing),
            _ => None,
        };
        let before = Replaced::Order {
            order: std::mem::replace(&mut self.view.order, plan.order.clone()),
            groups: std::mem::replace(&mut self.view.groups, plan.groups.clone()),
            added,
            stack: std::mem::replace(&mut self.view.stack, plan.stack.clone()),
            layer: std::mem::replace(&mut self.view.layer, layer.clone()),
        };
        let summary = format!("{} files, {} groups", plan.order.len(), plan.groups.len());
        Ok(self.record(
            ChangeTool::Tool(ChatTool::Prepare),
            summary,
            ChangeKind::Order {
                order: plan.order,
                groups: plan.groups,
                stack: plan.stack,
                layer,
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
            Replaced::Focus { place, highlights } => {
                self.view.highlights = highlights;
                self.view.open.clone_from(&place);
                let summary = place.as_ref().map_or_else(String::new, place_summary);
                (ChangeKind::Focus(place), summary)
            }
            Replaced::NoNote(id) => {
                self.notes.remove(&id);
                (ChangeKind::NoteDropped(id), String::new())
            }
            Replaced::Note(note) => {
                let summary = note_summary(&note.note);
                self.notes.insert(note.id.clone(), note.clone());
                (ChangeKind::Note(note), summary)
            }
            Replaced::Order {
                order,
                groups,
                added,
                stack,
                layer,
            } => {
                for id in added {
                    self.notes.remove(&id);
                }
                self.view.order.clone_from(&order);
                self.view.groups.clone_from(&groups);
                self.view.stack.clone_from(&stack);
                self.view.layer.clone_from(&layer);
                (
                    ChangeKind::Order {
                        order,
                        groups,
                        stack,
                        layer,
                    },
                    String::new(),
                )
            }
            Replaced::Layer { layer } => {
                self.view.layer.clone_from(&layer);
                let summary = layer.clone().unwrap_or_else(|| "Whole stack".to_owned());
                (ChangeKind::Layer(layer), summary)
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
    /// order), and moves the ones whose lines only moved. Returns a
    /// change for each note that went outdated or moved, for the app's
    /// feed; Undo can't take them back.
    pub fn refresh_notes(&mut self, states: &[Option<NoteState>]) -> Vec<Change> {
        let mut changed = Vec::new();
        for (note, state) in self.notes.values_mut().zip(states) {
            let before = note.clone();
            match state {
                Some(NoteState::Outdated) => note.outdated = true,
                Some(NoteState::Current { first, last }) => {
                    if let NoteTarget::Lines {
                        first: f, last: l, ..
                    } = &mut note.note.target
                    {
                        *f = *first;
                        *l = *last;
                    }
                }
                None | Some(NoteState::OnFile | NoteState::Gone) => {}
            }
            if *note != before {
                changed.push(note.clone());
            }
        }
        changed
            .into_iter()
            .map(|note| {
                self.next_seq += 1;
                self.push(Change {
                    undo_id: String::new(),
                    seq: self.next_seq,
                    tool: ChangeTool::Diff,
                    summary: note_summary(&note.note),
                    what: ChangeKind::Note(note),
                    highlights: self.view.highlights.clone(),
                })
            })
            .collect()
    }

    fn record(
        &mut self,
        tool: ChangeTool,
        summary: String,
        what: ChangeKind,
        before: Replaced,
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
        self.forget_oldest();
        self.push(Change {
            undo_id: id,
            seq,
            tool,
            summary,
            what,
            highlights: self.view.highlights.clone(),
        })
    }

    fn forget_oldest(&mut self) {
        while self.done.len() > UNDO_MAX {
            let oldest = self
                .done
                .iter()
                .min_by_key(|(_, done)| done.seq)
                .map(|(id, _)| id.clone());
            match oldest {
                Some(id) => self.done.remove(&id),
                None => break,
            };
        }
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
            layer: None,
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
        let changed = chat.refresh_notes(&[Some(NoteState::Outdated)]);
        assert!(
            matches!(changed.as_slice(), [Change { tool: ChangeTool::Diff, what: ChangeKind::Note(note), .. }] if note.outdated),
            "{changed:?}"
        );
        assert!(chat.refresh_notes(&[Some(NoteState::Outdated)]).is_empty());
        assert!(chat.notes().all(|note| note.outdated));
        assert_eq!(chat.changes_after(0).count(), 2, "the app's feed hears of it");
        assert!(chat.undo(&changed[0].undo_id).is_err(), "nobody can undo a diff");
    }

    #[test]
    fn a_chat_restored_with_its_history_can_still_undo_and_knows_what_was_undone() {
        let mut chat = ReviewChat::default();
        chat.report(ViewState {
            open: Some(at("a.rs", 1)),
            ..ViewState::default()
        });
        let moved = chat
            .focus(at("b.rs", 2), vec![], everywhere)
            .map(|change| change.undo_id.clone());
        let noted = chat
            .note("Claude Code", line_note("b.rs", 2), everywhere)
            .map(|change| change.undo_id.clone());
        let (Ok(moved), Ok(noted)) = (moved, noted) else {
            panic!("the agent's changes failed");
        };
        assert!(chat.undo(&noted).is_ok());
        let mut back = ReviewChat::restore(
            chat.view().clone(),
            chat.switches(),
            chat.notes().cloned().collect(),
            chat.last_seq(),
            chat.last_note(),
        )
        .with_undoable(chat.undoable());
        assert!(matches!(
            back.undo(&noted),
            Err(ChatError::NothingToUndo { .. })
        ));
        assert!(back.undo(&moved).is_ok());
        assert_eq!(back.view().open, Some(at("a.rs", 1)));
    }

    #[test]
    fn only_the_newest_changes_can_be_undone() {
        let mut chat = ReviewChat::default();
        let mut ids = Vec::new();
        for line in 0..=UNDO_MAX {
            let line = u32::try_from(line).unwrap_or(u32::MAX);
            match chat.focus(at("a.rs", line), vec![], everywhere) {
                Ok(change) => ids.push(change.undo_id),
                Err(error) => panic!("focus failed: {error}"),
            }
        }
        assert_eq!(chat.undoable().len(), UNDO_MAX);
        assert!(matches!(
            chat.undo(&ids[0]),
            Err(ChatError::NothingToUndo { .. })
        ));
        assert!(chat.undo(&ids[UNDO_MAX]).is_ok());
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
