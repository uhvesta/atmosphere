//! A session's read-only Conversation tab (D186): the host mirror's entries
//! around a search hit, landing on the hit's entry.
use humata2_clients::history::Turn as Entry;
use serde::{Deserialize, Serialize};

/// How many turns either side of the hit the first read covers.
pub const AROUND: u32 = 2;
/// How many turns Show earlier turns and Show later turns each add.
pub const STEP: u32 = 5;

/// Where a Conversation tab is aimed: the hit's turn and entry. Saved in
/// `SessionView`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationTarget {
    /// The hit's turn, counted from 1; 0 is what came before the first prompt.
    pub turn: u32,
    /// The hit's entry, `history::Turn::seq`.
    pub seq: u64,
}

/// Which read a reply answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationRead {
    /// The first read: the target's turn ± AROUND.
    Around,
    /// STEP turns before what is shown.
    Earlier,
    /// STEP turns after what is shown.
    Later,
}

/// What a session's Conversation tab has read. Not saved: re-read on restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conversation {
    /// Where it is aimed.
    pub target: ConversationTarget,
    /// The first turn read so far.
    pub first: u32,
    /// The last turn asked for so far.
    pub last: u32,
    /// What was read, in `seq` order, no duplicates.
    pub entries: Vec<Entry>,
    /// The read on its way, if any.
    pub loading: Option<ConversationRead>,
    /// The last failure, for the tab's error and Retry.
    pub error: Option<String>,
    /// The read Retry repeats, with its range.
    pub failed: Option<(ConversationRead, u32, u32)>,
    /// A Later read came back empty: nothing later yet.
    pub at_end: bool,
}

impl Conversation {
    /// A conversation aimed at `target`, nothing read yet: first and last
    /// are the Around window.
    pub(crate) fn new(target: ConversationTarget) -> Self {
        Self {
            target,
            first: target.turn.saturating_sub(AROUND),
            last: target.turn.saturating_add(AROUND),
            entries: Vec::new(),
            loading: None,
            error: None,
            failed: None,
            at_end: false,
        }
    }

    /// The hit's entry, if it was read.
    #[must_use]
    pub fn marked(&self) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.seq == self.target.seq)
    }

    /// Whether Show earlier turns shows: something comes before what is shown.
    #[must_use]
    pub const fn can_show_earlier(&self) -> bool {
        self.first > 0
    }

    /// Whether Show later turns shows: no Later read came back empty.
    #[must_use]
    pub const fn can_show_later(&self) -> bool {
        !self.at_end
    }

    /// The turns a read of this kind asks for, from what is shown now;
    /// `None` when there is nothing to ask for.
    pub(crate) fn range(&self, read: ConversationRead) -> Option<(u32, u32)> {
        match read {
            ConversationRead::Around => Some((
                self.target.turn.saturating_sub(AROUND),
                self.target.turn.saturating_add(AROUND),
            )),
            ConversationRead::Earlier => {
                (self.first > 0).then(|| (self.first.saturating_sub(STEP), self.first.saturating_sub(1)))
            }
            ConversationRead::Later => {
                Some((self.last.saturating_add(1), self.last.saturating_add(STEP)))
            }
        }
    }

    /// Merges a successful reply. Around replaces what was read; Earlier and
    /// Later add what is new, in `seq` order. The window moves to the range
    /// asked for, and a Later read that comes back empty ends the list.
    pub(crate) fn merge(&mut self, read: ConversationRead, first: u32, last: u32, entries: Vec<Entry>) {
        let reply_empty = entries.is_empty();
        if read == ConversationRead::Around {
            self.entries.clear();
        }
        for entry in entries {
            if !self.entries.iter().any(|known| known.seq == entry.seq) {
                self.entries.push(entry);
            }
        }
        self.entries.sort_by_key(|entry| entry.seq);
        match read {
            ConversationRead::Around => {
                self.first = first;
                self.last = last;
            }
            ConversationRead::Earlier => self.first = first,
            ConversationRead::Later => {
                self.last = last;
                if reply_empty {
                    self.at_end = true;
                }
            }
        }
        self.error = None;
        self.failed = None;
        self.loading = None;
    }
}
