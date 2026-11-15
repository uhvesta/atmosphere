//! A rendered diff: the new document's blocks, each marked unchanged,
//! inserted or changed against the old document, and the old blocks that are
//! gone. Blocks match by their whitespace-blind fingerprints, so a change in
//! whitespace alone is no change (SM-5, SM-16).

use std::ops::Range;

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::document::{BlockKind, Document, LineRange};

/// How one block of the new document differs from the old.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockChange {
    Unchanged {
        old_index: usize,
    },
    Inserted,
    /// The block replaces `old_index`. `inserted` lists the new source's
    /// bytes, in document coordinates, that aren't in the old block.
    Changed {
        old_index: usize,
        inserted: Vec<Range<usize>>,
    },
}

/// A block of the old document that the new one doesn't have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletedBlock {
    pub old_index: usize,
    /// The new block it was before; the block count when it was last.
    pub before: usize,
    pub kind: BlockKind,
    pub source: String,
    pub text: String,
    /// Its lines in the old document.
    pub lines: LineRange,
}

/// The difference between two versions of a document, for drawing the new one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenderedDiff {
    /// One entry per block of the new document.
    pub blocks: Vec<BlockChange>,
    pub deleted: Vec<DeletedBlock>,
}

impl RenderedDiff {
    /// Whether the versions differ in anything but whitespace.
    pub fn is_empty(&self) -> bool {
        self.deleted.is_empty()
            && self
                .blocks
                .iter()
                .all(|change| matches!(change, BlockChange::Unchanged { .. }))
    }
}

/// Compare two versions of a document.
pub fn diff(old: &Document, new: &Document) -> RenderedDiff {
    let old_prints: Vec<u64> = old.blocks().iter().map(|b| b.fingerprint).collect();
    let new_prints: Vec<u64> = new.blocks().iter().map(|b| b.fingerprint).collect();
    let mut result = RenderedDiff {
        blocks: vec![BlockChange::Inserted; new_prints.len()],
        deleted: Vec::new(),
    };
    let delete = |result: &mut RenderedDiff, old_index: usize, before: usize| {
        let block = &old.blocks()[old_index];
        result.deleted.push(DeletedBlock {
            old_index,
            before,
            kind: block.kind.clone(),
            source: old.source()[block.bytes.clone()].to_owned(),
            text: block.text.clone(),
            lines: block.lines,
        });
    };
    for op in capture_diff_slices(Algorithm::Myers, &old_prints, &new_prints) {
        match op {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for ix in 0..len {
                    result.blocks[new_index + ix] = BlockChange::Unchanged {
                        old_index: old_index + ix,
                    };
                }
            }
            DiffOp::Insert { .. } => {}
            DiffOp::Delete {
                old_index,
                old_len,
                new_index,
            } => {
                for ix in 0..old_len {
                    delete(&mut result, old_index + ix, new_index);
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                // Pair an old block with a new one only when they are the
                // same kind of block and share most of their words; an
                // unrelated replacement is a deletion and an insertion.
                let (old_end, new_end) = (old_index + old_len, new_index + new_len);
                let (mut at_old, mut at_new) = (old_index, new_index);
                while at_old < old_end && at_new < new_end {
                    if alike(old, at_old, new, at_new) {
                        let old_block = &old.blocks()[at_old];
                        let new_block = &new.blocks()[at_new];
                        result.blocks[at_new] = BlockChange::Changed {
                            old_index: at_old,
                            inserted: inserted_words(
                                &old.source()[old_block.bytes.clone()],
                                &new.source()[new_block.bytes.clone()],
                                new_block.bytes.start,
                            ),
                        };
                        at_old += 1;
                        at_new += 1;
                    } else if (at_new + 1..new_end).any(|later| alike(old, at_old, new, later)) {
                        // The old block changed into a later new one; this
                        // new one is inserted before it.
                        at_new += 1;
                    } else {
                        delete(&mut result, at_old, at_new);
                        at_old += 1;
                    }
                }
                for ix in at_old..old_end {
                    delete(&mut result, ix, new_end);
                }
            }
        }
    }
    result
}

/// Whether old block `old_index` and new block `new_index` are one block
/// edited: the same kind, sharing at least half their words.
fn alike(old: &Document, old_index: usize, new: &Document, new_index: usize) -> bool {
    let (old_block, new_block) = (&old.blocks()[old_index], &new.blocks()[new_index]);
    if std::mem::discriminant(&old_block.kind) != std::mem::discriminant(&new_block.kind) {
        return false;
    }
    let old_source = &old.source()[old_block.bytes.clone()];
    let new_source = &new.source()[new_block.bytes.clone()];
    let old_words: Vec<&str> = words(old_source).into_iter().map(|r| &old_source[r]).collect();
    let new_words: Vec<&str> = words(new_source).into_iter().map(|r| &new_source[r]).collect();
    let longest = old_words.len().max(new_words.len());
    if longest == 0 {
        return true;
    }
    let shared: usize = capture_diff_slices(Algorithm::Myers, &old_words, &new_words)
        .iter()
        .map(|op| match op {
            DiffOp::Equal { len, .. } => *len,
            _ => 0,
        })
        .sum();
    shared * 2 >= longest
}

/// The words of `new` that aren't in `old`, as byte ranges shifted by
/// `offset`, adjacent ones merged.
fn inserted_words(old: &str, new: &str, offset: usize) -> Vec<Range<usize>> {
    let old_words = words(old);
    let new_words = words(new);
    let old_text: Vec<&str> = old_words.iter().map(|r| &old[r.clone()]).collect();
    let new_text: Vec<&str> = new_words.iter().map(|r| &new[r.clone()]).collect();
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for op in capture_diff_slices(Algorithm::Myers, &old_text, &new_text) {
        let (start, len) = match op {
            DiffOp::Insert {
                new_index, new_len, ..
            }
            | DiffOp::Replace {
                new_index, new_len, ..
            } => (new_index, new_len),
            _ => continue,
        };
        let first = new_words[start].start + offset;
        let last = new_words[start + len - 1].end + offset;
        match ranges.last_mut() {
            Some(previous) if new[previous.end - offset..first - offset].trim().is_empty() => {
                previous.end = last;
            }
            _ => ranges.push(first..last),
        }
    }
    ranges
}

/// Words of `text`: runs of letters and digits, and each other character
/// that isn't whitespace on its own.
pub(crate) fn words(text: &str) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut run: Option<usize> = None;
    for (at, ch) in text.char_indices() {
        if ch.is_alphanumeric() || ch == '_' {
            run.get_or_insert(at);
            continue;
        }
        if let Some(start) = run.take() {
            words.push(start..at);
        }
        if !ch.is_whitespace() {
            words.push(at..at + ch.len_utf8());
        }
    }
    if let Some(start) = run {
        words.push(start..text.len());
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SM-5
    #[test]
    fn words_split_on_whitespace_and_punctuation() {
        let text = "fn foo(a_b) é";
        let found: Vec<&str> = words(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(found, ["fn", "foo", "(", "a_b", ")", "é"]);
    }

    /// SM-5
    #[test]
    fn adjacent_inserted_words_merge_into_one_range() {
        let ranges = inserted_words("one four", "one two three four", 10);
        assert_eq!(ranges, vec![14..23]);
    }
}
