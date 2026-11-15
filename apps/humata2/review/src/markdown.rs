//! A markdown file's rendered view, with what changed marked. The renderer
//! is a seam: the markdown crate (`humata2_markdown`) plugs in here, and
//! until it exists [`PlainBlocks`] splits the source into blocks itself.

use crate::model::LineRange;

/// What a rendered block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Heading(u8),
    Paragraph,
    ListItem,
    Quote,
    Code,
    Rule,
}

/// One block of rendered markdown and the source lines it comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// The text as it reads rendered, without markup.
    pub text: String,
    pub lines: LineRange,
}

/// Turns markdown source into blocks that know their source lines, so a
/// selection in the rendered view becomes a comment on source lines.
pub trait MarkdownRenderer: 'static {
    fn blocks(&self, source: &str) -> Vec<Block>;
}

/// How a rendered block changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    Same,
    Added,
    Changed,
    /// Only in the old file; shown struck through.
    Removed,
}

/// A block of the rendered diff: new blocks in order, with removed old
/// blocks where they were.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedBlock {
    pub block: Block,
    pub change: Change,
}

impl RenderedBlock {
    /// The new-side lines it comes from; none for a removed block.
    pub fn new_lines(&self) -> Option<LineRange> {
        (self.change != Change::Removed).then_some(self.block.lines)
    }
}

/// The rendered diff of `old` and `new`: blocks matched by kind and text,
/// in order. An unmatched new block facing an unmatched old one in the same
/// gap is changed; other unmatched new blocks are added and unmatched old
/// ones removed.
pub fn rendered_diff(renderer: &dyn MarkdownRenderer, old: &str, new: &str) -> Vec<RenderedBlock> {
    let old = renderer.blocks(old);
    let new = renderer.blocks(new);
    let same = |a: &Block, b: &Block| a.kind == b.kind && a.text == b.text;
    // Longest common subsequence of blocks.
    let (n, m) = (old.len(), new.len());
    let mut table = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i][j] = if same(&old[i], &new[j]) {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut out = Vec::with_capacity(m.max(n));
    let (mut i, mut j) = (0, 0);
    let mut gap_old: Vec<Block> = Vec::new();
    let mut gap_new: Vec<Block> = Vec::new();
    let flush = |out: &mut Vec<RenderedBlock>, gap_old: &mut Vec<Block>, gap_new: &mut Vec<Block>| {
        let paired = gap_old.len().min(gap_new.len());
        let mut olds = gap_old.drain(..);
        for (index, block) in gap_new.drain(..).enumerate() {
            let change = if index < paired {
                olds.next();
                Change::Changed
            } else {
                Change::Added
            };
            out.push(RenderedBlock { block, change });
        }
        out.extend(olds.map(|block| RenderedBlock {
            block,
            change: Change::Removed,
        }));
    };
    while i < n || j < m {
        if i < n && j < m && same(&old[i], &new[j]) {
            flush(&mut out, &mut gap_old, &mut gap_new);
            out.push(RenderedBlock {
                block: new[j].clone(),
                change: Change::Same,
            });
            i += 1;
            j += 1;
        } else if j < m && (i == n || table[i][j + 1] >= table[i + 1][j]) {
            gap_new.push(new[j].clone());
            j += 1;
        } else {
            gap_old.push(old[i].clone());
            i += 1;
        }
    }
    flush(&mut out, &mut gap_old, &mut gap_new);
    out
}

/// A small stand-in renderer: headings, list items, quotes, fenced code,
/// rules and paragraphs, one block each, with inline markup stripped.
pub struct PlainBlocks;

impl MarkdownRenderer for PlainBlocks {
    fn blocks(&self, source: &str) -> Vec<Block> {
        let lines: Vec<&str> = source.lines().collect();
        let mut blocks = Vec::new();
        let mut index = 0;
        let number = |index: usize| u32::try_from(index + 1).unwrap_or(u32::MAX);
        while index < lines.len() {
            let line = lines[index];
            let trimmed = line.trim();
            let start = index;
            if trimmed.is_empty() {
                index += 1;
                continue;
            }
            if trimmed.starts_with("```") {
                index += 1;
                let mut code = Vec::new();
                while index < lines.len() && !lines[index].trim().starts_with("```") {
                    code.push(lines[index]);
                    index += 1;
                }
                let end = index.min(lines.len().saturating_sub(1));
                index += 1;
                blocks.push(Block {
                    kind: BlockKind::Code,
                    text: code.join("\n"),
                    lines: LineRange::new(number(start), number(end)),
                });
                continue;
            }
            let heading = trimmed.chars().take_while(|c| *c == '#').count();
            let (kind, text) = if (1..=6).contains(&heading) && trimmed[heading..].starts_with(' ') {
                (
                    BlockKind::Heading(u8::try_from(heading).unwrap_or(6)),
                    trimmed[heading..].trim().to_owned(),
                )
            } else if trimmed == "---" || trimmed == "***" {
                (BlockKind::Rule, String::new())
            } else if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                (BlockKind::ListItem, rest.to_owned())
            } else if let Some(rest) = trimmed.strip_prefix('>') {
                (BlockKind::Quote, rest.trim().to_owned())
            } else {
                // A paragraph runs to the next blank line or block start.
                let mut text = vec![trimmed];
                while index + 1 < lines.len() {
                    let next = lines[index + 1].trim();
                    if next.is_empty()
                        || next.starts_with('#')
                        || next.starts_with("- ")
                        || next.starts_with("* ")
                        || next.starts_with('>')
                        || next.starts_with("```")
                    {
                        break;
                    }
                    text.push(next);
                    index += 1;
                }
                (BlockKind::Paragraph, text.join(" "))
            };
            blocks.push(Block {
                kind,
                text: strip_inline(&text),
                lines: LineRange::new(number(start), number(index)),
            });
            index += 1;
        }
        blocks
    }
}

/// Text without emphasis stars, backticks and link targets.
fn strip_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '`' => {}
            ']' if chars.peek() == Some(&'(') => {
                for c in chars.by_ref() {
                    if c == ')' {
                        break;
                    }
                }
            }
            '[' => {}
            c => out.push(c),
        }
    }
    out
}
