//! Selections mapped to source, and comments anchored to them. An anchor
//! remembers the text it covers and some text on either side, so it finds its
//! passage again after the document changes (SM-6, SM-7, SM-13).

use std::ops::Range;

use crate::document::{BlockKind, BlockPart, Document, LineRange};

/// What a selection or comment is attached to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AnchorTarget {
    /// A run of text, which may cross blocks.
    Text,
    /// A whole top-level block.
    Block { index: usize },
    /// One top-level item of the list at `block`.
    ListItem { block: usize, index: usize },
    /// One cell of the table at `block`; row 0 is the header.
    TableCell {
        block: usize,
        row: usize,
        column: usize,
    },
    /// The Mermaid diagram at `block`.
    Diagram { block: usize },
}

/// A selection, mapped to the document's source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSelection {
    pub bytes: Range<usize>,
    pub lines: LineRange,
    /// The selected source, exactly.
    pub source: String,
    /// The text the reader saw selected, without markup.
    pub quote: String,
    pub target: AnchorTarget,
}

impl Document {
    /// The selection of `bytes` of the source, which the reader saw as `quote`.
    /// `None` when the range isn't in the source.
    pub fn selection(&self, bytes: Range<usize>, quote: impl Into<String>) -> Option<SourceSelection> {
        let source = self.source().get(bytes.clone())?.to_owned();
        Some(SourceSelection {
            lines: self.lines_of(&bytes),
            bytes,
            source,
            quote: quote.into(),
            target: AnchorTarget::Text,
        })
    }

    /// The selection of a whole block: a Mermaid block selects as a diagram.
    pub fn block_selection(&self, index: usize) -> Option<SourceSelection> {
        let block = self.blocks().get(index)?;
        let mut selection = self.selection(block.bytes.clone(), block.text.clone())?;
        selection.target = match block.kind {
            BlockKind::Mermaid { .. } => AnchorTarget::Diagram { block: index },
            _ => AnchorTarget::Block { index },
        };
        Some(selection)
    }

    /// The selection of one part (list item or table cell) of a block.
    pub fn part_selection(&self, block: usize, part: usize) -> Option<SourceSelection> {
        let found = self.blocks().get(block)?.parts.get(part)?;
        let mut selection = self.selection(found.bytes(), found.text())?;
        selection.target = match found {
            BlockPart::ListItem { index, .. } => AnchorTarget::ListItem {
                block,
                index: *index,
            },
            BlockPart::TableCell { row, column, .. } => AnchorTarget::TableCell {
                block,
                row: *row,
                column: *column,
            },
        };
        Some(selection)
    }
}

/// How much text on each side of a passage an anchor keeps.
const CONTEXT_BYTES: usize = 48;

/// A passage of a document, found again after the document changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    /// Where the passage was when anchored.
    pub bytes: Range<usize>,
    pub lines: LineRange,
    /// The passage's source when anchored.
    pub source: String,
    /// What the reader saw.
    pub quote: String,
    pub target: AnchorTarget,
    /// Up to 48 bytes of source before the passage, and after it.
    pub before: String,
    pub after: String,
}

/// Where an anchor's passage is now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnchorStatus {
    /// The passage is here, unchanged but for whitespace.
    Current { bytes: Range<usize>, lines: LineRange },
    /// The passage's surroundings are here but its text changed.
    Outdated { bytes: Range<usize>, lines: LineRange },
    /// Neither the passage nor its surroundings are left (SM-7: offer
    /// Re-attach or Drop).
    Gone,
}

impl AnchorStatus {
    /// Where the passage is, unless it is gone.
    pub fn bytes(&self) -> Option<Range<usize>> {
        match self {
            Self::Current { bytes, .. } | Self::Outdated { bytes, .. } => Some(bytes.clone()),
            Self::Gone => None,
        }
    }
}

impl Anchor {
    /// Anchor a selection in the document it was made in.
    pub fn new(document: &Document, selection: &SourceSelection) -> Self {
        let source = document.source();
        let start = floor_boundary(source, selection.bytes.start.saturating_sub(CONTEXT_BYTES));
        let end = ceil_boundary(source, (selection.bytes.end + CONTEXT_BYTES).min(source.len()));
        Self {
            bytes: selection.bytes.clone(),
            lines: selection.lines,
            source: selection.source.clone(),
            quote: selection.quote.clone(),
            target: selection.target.clone(),
            before: source[start..selection.bytes.start].to_owned(),
            after: source[selection.bytes.end..end].to_owned(),
        }
    }

    /// Find the passage in `document`, which may have changed since.
    pub fn resolve(&self, document: &Document) -> AnchorStatus {
        let text = document.source();
        let current = |bytes: Range<usize>| AnchorStatus::Current {
            lines: document.lines_of(&bytes),
            bytes,
        };
        if text.get(self.bytes.clone()) == Some(self.source.as_str()) {
            return current(self.bytes.clone());
        }
        if !self.source.is_empty() {
            if let Some(start) = self.best_exact(text) {
                return current(start..start + self.source.len());
            }
            if let Some(bytes) = find_normalized(text, &self.source, self.bytes.start) {
                return current(bytes);
            }
        }
        for keep in [CONTEXT_BYTES, CONTEXT_BYTES / 2, CONTEXT_BYTES / 4] {
            if let Some(bytes) = self.between_context(text, keep) {
                if bytes.is_empty() {
                    return AnchorStatus::Gone;
                }
                return AnchorStatus::Outdated {
                    lines: document.lines_of(&bytes),
                    bytes,
                };
            }
        }
        // Only one side's surroundings are left, as when the passage was
        // reworded and the paragraph after it deleted: the block on that side
        // is the passage when it still shares half its words.
        for keep in [CONTEXT_BYTES, CONTEXT_BYTES / 2, CONTEXT_BYTES / 4] {
            if let Some(bytes) = self.one_sided(document, keep) {
                return AnchorStatus::Outdated {
                    lines: document.lines_of(&bytes),
                    bytes,
                };
            }
        }
        AnchorStatus::Gone
    }

    /// The reworded passage next to the surroundings on one side, nearest
    /// its old place: from the end of the text before it to the end of that
    /// block, or from the start of the block to the text after it.
    fn one_sided(&self, document: &Document, keep: usize) -> Option<Range<usize>> {
        let text = document.source();
        let before = tail(&self.before, keep);
        let after = head(&self.after, keep);
        let mut candidates = Vec::new();
        if !before.trim().is_empty() {
            for (at, _) in text.match_indices(before) {
                let start = at + before.len();
                let first = trim_range(text, start..text.len()).start;
                if let Some(block) = document.block_at(first).and_then(|index| document.blocks().get(index))
                    && block.bytes.end > first
                {
                    candidates.push(start..block.bytes.end);
                }
            }
        }
        if !after.trim().is_empty() {
            for (end, _) in text.match_indices(after) {
                let last = trim_range(text, 0..end).end;
                if let Some(block) = last
                    .checked_sub(1)
                    .and_then(|at| document.block_at(at))
                    .and_then(|index| document.blocks().get(index))
                    && block.bytes.end >= last
                {
                    candidates.push(block.bytes.start..end);
                }
            }
        }
        candidates
            .into_iter()
            .map(|bytes| trim_range(text, bytes))
            .filter(|bytes| !bytes.is_empty() && shares_half_its_words(&self.source, &text[bytes.clone()]))
            .min_by_key(|bytes| bytes.start.abs_diff(self.bytes.start))
    }

    /// The occurrence of the passage whose surroundings match best, nearest
    /// its old place on a tie.
    fn best_exact(&self, text: &str) -> Option<usize> {
        text.match_indices(self.source.as_str())
            .map(|(start, _)| {
                let end = start + self.source.len();
                let before = common_suffix(&text[..start], &self.before);
                let after = common_prefix(&text[end..], &self.after);
                let distance = start.abs_diff(self.bytes.start);
                (before + after, std::cmp::Reverse(distance), start)
            })
            .max()
            .map(|(_, _, start)| start)
    }

    /// The text between the last `keep` bytes of the old surroundings before
    /// the passage and the first `keep` after it. The start and end of the
    /// document stand in for surroundings the anchor didn't have.
    fn between_context(&self, text: &str, keep: usize) -> Option<Range<usize>> {
        let before = tail(&self.before, keep);
        let after = head(&self.after, keep);
        if before.trim().is_empty() && after.trim().is_empty() {
            return None;
        }
        let starts: Vec<usize> = if before.is_empty() {
            vec![0]
        } else {
            text.match_indices(before).map(|(at, _)| at + before.len()).collect()
        };
        let mut best: Option<(usize, Range<usize>)> = None;
        for start in starts {
            let end = if after.is_empty() {
                Some(text.len())
            } else {
                text[start..].find(after).map(|at| start + at)
            };
            let Some(end) = end else { continue };
            let distance = start.abs_diff(self.bytes.start);
            if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                best = Some((distance, start..end));
            }
        }
        best.map(|(_, bytes)| trim_range(text, bytes))
    }
}

/// A comment on a passage of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comment {
    /// Chosen by the app; the view reports it back when the comment is
    /// activated.
    pub id: String,
    pub anchor: Anchor,
    pub body: String,
}

fn floor_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

fn ceil_boundary(text: &str, mut at: usize) -> usize {
    while !text.is_char_boundary(at) {
        at += 1;
    }
    at
}

fn tail(text: &str, keep: usize) -> &str {
    &text[ceil_boundary(text, text.len().saturating_sub(keep))..]
}

fn head(text: &str, keep: usize) -> &str {
    &text[..floor_boundary(text, keep.min(text.len()))]
}

fn common_suffix(a: &str, b: &str) -> usize {
    a.bytes()
        .rev()
        .zip(b.bytes().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

fn common_prefix(a: &str, b: &str) -> usize {
    a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count()
}

/// Whether at least half the words of `old` are still in `new`, each word
/// counted once per time it appears.
fn shares_half_its_words(old: &str, new: &str) -> bool {
    let words = |text: &str| -> Vec<String> {
        text.split(|ch: char| !ch.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .map(str::to_lowercase)
            .collect()
    };
    let old = words(old);
    let mut new = words(new);
    if old.is_empty() {
        return false;
    }
    let kept = old
        .iter()
        .filter(|word| {
            new.iter()
                .position(|candidate| candidate == *word)
                .map(|at| new.swap_remove(at))
                .is_some()
        })
        .count();
    kept * 2 >= old.len()
}

fn trim_range(text: &str, bytes: Range<usize>) -> Range<usize> {
    let slice = &text[bytes.clone()];
    let start = bytes.start + (slice.len() - slice.trim_start().len());
    let end = bytes.end - (slice.len() - slice.trim_end().len());
    start..end.max(start)
}

/// Find `needle` in `text` with every run of whitespace in either read as a
/// single space, nearest `near`. Returns the bytes of `text` it covers.
fn find_normalized(text: &str, needle: &str, near: usize) -> Option<Range<usize>> {
    let needle: String = needle.split_whitespace().collect::<Vec<_>>().join(" ");
    if needle.is_empty() {
        return None;
    }
    // The normalized text, and for each of its bytes the original byte.
    let mut normal = String::with_capacity(text.len());
    let mut origin = Vec::with_capacity(text.len());
    let mut in_space = false;
    for (at, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if !in_space {
                normal.push(' ');
                origin.push(at);
            }
            in_space = true;
        } else {
            let mut buffer = [0; 4];
            for _ in ch.encode_utf8(&mut buffer).bytes() {
                origin.push(at);
            }
            normal.push(ch);
            in_space = false;
        }
    }
    normal
        .match_indices(needle.as_str())
        .map(|(start, matched)| {
            let first = origin[start];
            let last = origin[start + matched.len() - 1];
            let end = last + text[last..].chars().next().map_or(1, char::len_utf8);
            first..end
        })
        .min_by_key(|bytes| bytes.start.abs_diff(near))
}
