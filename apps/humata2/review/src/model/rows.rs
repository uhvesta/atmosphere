//! The review as rows: the file list's rows, and the body's rows that the
//! pane mounts lazily, a screenful at a time.

use std::collections::{BTreeMap, HashMap};

use super::anchor::LineRange;
use super::agent::NoteId;
use super::comments::{CommentBook, CommentId, Status};
use super::memory::{ReviewMemory, Viewed};
use super::review::{Content, FileChange, FileKey, Review, Side};

/// A row of the file list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListRow {
    Repo {
        repo: String,
        files: usize,
        collapsed: bool,
    },
    /// The file at this index of [`Review::files`].
    File(usize),
    /// A group of an agent-prepared review: its index in the view's groups,
    /// or none for the files it didn't place.
    Group { group: Option<usize>, files: usize },
}

/// Whether `file` passes the filters that are on.
pub fn passes(file: &FileChange, memory: &ReviewMemory, book: &CommentBook) -> bool {
    let filters = memory.filters;
    let viewed = memory.viewed(&file.key, &file.blob);
    (!filters.changed || viewed == Viewed::ChangedSince)
        && (!filters.unviewed || viewed != Viewed::Yes)
        && (!filters.commented
            || book
                .on_file(&file.key)
                .any(|comment| comment.status != Status::Resolved))
}

/// The files in the order shown, in sections: by repo, or by an
/// agent-prepared review's groups while one shows. Each section is its
/// repo or group row and its file indexes, before filters.
fn sections(review: &Review, memory: &ReviewMemory) -> Vec<(ListRow, Vec<usize>)> {
    if let Some(showing) = &memory.prepared {
        return showing
            .view
            .order(review)
            .into_iter()
            .map(|(group, files)| {
                (
                    ListRow::Group {
                        group,
                        files: files.len(),
                    },
                    files,
                )
            })
            .collect();
    }
    review
        .repos()
        .into_iter()
        .map(|repo| {
            let files: Vec<usize> = review
                .files()
                .iter()
                .enumerate()
                .filter(|(_, file)| file.key.repo == repo)
                .map(|(index, _)| index)
                .collect();
            (
                ListRow::Repo {
                    repo: repo.to_owned(),
                    files: files.len(),
                    collapsed: memory.collapsed_repos.contains(repo),
                },
                files,
            )
        })
        .collect()
}

/// The file list: each repo (or prepared group), then its files that pass
/// the filters.
pub fn list_rows(review: &Review, memory: &ReviewMemory, book: &CommentBook) -> Vec<ListRow> {
    let mut rows = Vec::new();
    for (mut head, files) in sections(review, memory) {
        let files: Vec<usize> = files
            .into_iter()
            .filter(|&index| passes(&review.files()[index], memory, book))
            .collect();
        let collapsed = match &mut head {
            ListRow::Repo { files: count, collapsed, .. } => {
                *count = files.len();
                *collapsed
            }
            ListRow::Group { files: count, .. } => {
                *count = files.len();
                false
            }
            ListRow::File(_) => false,
        };
        rows.push(head);
        if !collapsed {
            rows.extend(files.into_iter().map(ListRow::File));
        }
    }
    rows
}

/// A row of the review body.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BodyRow {
    /// A file's header, with its name, counts and viewed mark.
    File(usize),
    /// A hunk's `@@` header.
    Hunk { file: usize, hunk: usize },
    /// A diff line.
    Line { file: usize, hunk: usize, line: usize },
    /// A comment card (a comment, question or GitHub draft), under the
    /// line it ends on.
    Comment { file: usize, id: CommentId },
    /// A prepared review's group heading, with its title.
    Group(Option<usize>),
    /// The agent's note for the user (PR-23).
    AgentNote { file: usize, note: NoteId },
    /// The comment box, under the selection.
    Composer { file: usize },
    /// An image change.
    Image(usize),
    /// A block of a markdown file's rendered view.
    Block { file: usize, block: usize },
    /// Something said instead of content, such as a binary's sizes.
    Note(usize),
}

impl BodyRow {
    pub fn file(&self) -> usize {
        match *self {
            Self::File(file)
            | Self::Image(file)
            | Self::Note(file)
            | Self::Hunk { file, .. }
            | Self::Line { file, .. }
            | Self::Comment { file, .. }
            | Self::Composer { file }
            | Self::AgentNote { file, .. }
            | Self::Block { file, .. } => file,
            // A heading belongs to no file; it sorts before the group's first.
            Self::Group(_) => usize::MAX,
        }
    }
}

/// Where the open comment box is: the file and the lines it's for.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ComposerAt {
    pub file: FileKey,
    pub side: Side,
    pub lines: LineRange,
}

/// The new-side source lines each rendered block of a markdown file comes
/// from, for files shown rendered, in reading order. A removed block, shown
/// struck through, comes from no new lines.
pub type RenderedLayout = HashMap<usize, Vec<Option<LineRange>>>;

/// The body's rows. Files that don't pass the filters, files in collapsed
/// repos and the contents of collapsed files are left out. A comment goes
/// under the line it ends on; one whose line isn't shown, or that's
/// outdated, goes under its file's header. While an agent-prepared review
/// shows, files follow its groups, each under its heading, and its notes go
/// where comments would.
pub fn body_rows(
    review: &Review,
    memory: &ReviewMemory,
    book: &CommentBook,
    rendered: &RenderedLayout,
    composer: Option<&ComposerAt>,
) -> Vec<BodyRow> {
    let mut rows = Vec::new();
    for (head, files) in sections(review, memory) {
        let files: Vec<usize> = files
            .into_iter()
            .filter(|&index| passes(&review.files()[index], memory, book))
            .collect();
        match head {
            ListRow::Repo { collapsed: true, .. } => continue,
            ListRow::Group { group, .. } if !files.is_empty() => rows.push(BodyRow::Group(group)),
            _ => {}
        }
        for index in files {
            file_rows(&mut rows, review, memory, book, rendered, composer, index);
        }
    }
    rows
}

/// A card under a line: a comment, a question or an agent's note.
#[derive(Clone, Copy)]
enum Card {
    Comment(CommentId),
    Note(NoteId),
}

impl Card {
    fn row(self, file: usize) -> BodyRow {
        match self {
            Self::Comment(id) => BodyRow::Comment { file, id },
            Self::Note(note) => BodyRow::AgentNote { file, note },
        }
    }
}

fn file_rows(
    rows: &mut Vec<BodyRow>,
    review: &Review,
    memory: &ReviewMemory,
    book: &CommentBook,
    rendered: &RenderedLayout,
    composer: Option<&ComposerAt>,
    index: usize,
) {
    let file = &review.files()[index];
    rows.push(BodyRow::File(index));
    if memory.collapsed.contains(&file.key) {
        return;
    }
    // Each card and where it ends: (side, last line), or none for the file.
    let mut cards: Vec<(Card, Option<(Side, u32)>)> = Vec::new();
    for comment in book
        .on_file(&file.key)
        .filter(|c| memory.show_resolved || c.status != Status::Resolved)
    {
        let at = (!comment.outdated && !comment.anchor.lines.is_file())
            .then_some((comment.anchor.side, comment.anchor.lines.last));
        cards.push((Card::Comment(comment.id), at));
    }
    for note in memory.agent.notes.iter().filter(|note| note.file == file.key) {
        let at = note
            .at
            .as_ref()
            .filter(|_| !note.outdated)
            .map(|at| (at.side, at.lines.last));
        cards.push((Card::Note(note.id), at));
    }
    let composer = composer.filter(|at| at.file == file.key);
    match (&file.content, rendered.get(&index)) {
        (Content::Markdown(_), Some(blocks)) if memory.rendered.contains(&file.key) => {
            rendered_rows(rows, index, blocks, &cards, composer);
        }
        (Content::Text(diff) | Content::Markdown(diff), _) => {
            // Where each card and the composer go: after the line on their
            // side that they end on.
            let mut after: BTreeMap<(Side, u32), Vec<Card>> = BTreeMap::new();
            let shown: std::collections::HashSet<(Side, u32)> = diff
                .lines()
                .flat_map(|line| {
                    [
                        line.old.map(|n| (Side::Old, n)),
                        line.new.map(|n| (Side::New, n)),
                    ]
                })
                .flatten()
                .collect();
            for (card, at) in &cards {
                match at {
                    Some(at) if shown.contains(at) => after.entry(*at).or_default().push(*card),
                    _ => rows.push(card.row(index)),
                }
            }
            let composer_at = composer.map(|at| (at.side, at.lines.last));
            if composer_at.is_some_and(|at| !shown.contains(&at)) {
                rows.push(BodyRow::Composer { file: index });
            }
            for (h, hunk) in diff.hunks.iter().enumerate() {
                rows.push(BodyRow::Hunk { file: index, hunk: h });
                for (l, line) in hunk.lines.iter().enumerate() {
                    rows.push(BodyRow::Line {
                        file: index,
                        hunk: h,
                        line: l,
                    });
                    // A context line is on both sides; cards on either side
                    // of it go under it, once.
                    let mut places: Vec<(Side, u32)> = Vec::with_capacity(2);
                    if let Some(n) = line.old.filter(|_| line.kind != super::LineKind::Added) {
                        places.push((Side::Old, n));
                    }
                    if let Some(n) = line.new {
                        places.push((Side::New, n));
                    }
                    for place in &places {
                        if let Some(cards) = after.remove(place) {
                            rows.extend(cards.into_iter().map(|card| card.row(index)));
                        }
                    }
                    if composer_at.is_some_and(|at| at == (line.side(), line.number(line.side()).unwrap_or(0))) {
                        rows.push(BodyRow::Composer { file: index });
                    }
                }
            }
        }
        (Content::Image(_), _) => {
            rows.extend(cards.iter().map(|(card, _)| card.row(index)));
            if composer.is_some() {
                rows.push(BodyRow::Composer { file: index });
            }
            rows.push(BodyRow::Image(index));
        }
        (Content::Binary { .. }, _) => {
            rows.extend(cards.iter().map(|(card, _)| card.row(index)));
            if composer.is_some() {
                rows.push(BodyRow::Composer { file: index });
            }
            rows.push(BodyRow::Note(index));
        }
    }
}

fn rendered_rows(
    rows: &mut Vec<BodyRow>,
    file: usize,
    blocks: &[Option<LineRange>],
    cards: &[(Card, Option<(Side, u32)>)],
    composer: Option<&ComposerAt>,
) {
    // A card goes after the block holding its last new line; one in no
    // block goes first.
    let block_of = |line: u32| {
        blocks
            .iter()
            .position(|range| range.is_some_and(|range| range.contains(line)))
    };
    let mut after: BTreeMap<usize, Vec<Card>> = BTreeMap::new();
    for (card, at) in cards {
        match at.and_then(|(side, line)| (side == Side::New).then(|| block_of(line)).flatten()) {
            Some(block) => after.entry(block).or_default().push(*card),
            None => rows.push(card.row(file)),
        }
    }
    let composer_block = composer.and_then(|at| block_of(at.lines.last));
    if composer.is_some() && composer_block.is_none() {
        rows.push(BodyRow::Composer { file });
    }
    for block in 0..blocks.len() {
        rows.push(BodyRow::Block { file, block });
        if let Some(cards) = after.remove(&block) {
            rows.extend(cards.into_iter().map(|card| card.row(file)));
        }
        if composer_block == Some(block) {
            rows.push(BodyRow::Composer { file });
        }
    }
}
