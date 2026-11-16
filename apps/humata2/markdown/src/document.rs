//! The document model: a Markdown source split into the same top-level blocks
//! the text view draws, each with its bytes, lines, plain text and the parts a
//! comment can attach to. Appending text re-parses only from the last block.

use std::ops::Range;

use markdown::mdast::Node;

/// Lines of the source, 1-based and inclusive at both ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

impl LineRange {
    /// Whether `line` is one of these lines.
    pub fn contains(&self, line: usize) -> bool {
        self.start <= line && line <= self.end
    }
}

/// What a top-level block is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Heading {
        level: u8,
    },
    Paragraph,
    /// A paragraph that holds only an image.
    Image,
    List {
        ordered: bool,
        /// Whether any item is a task (`- [ ]` or `- [x]`).
        tasks: bool,
    },
    Quote,
    Table,
    Rule,
    Code {
        language: Option<String>,
        /// Whether the closing fence has arrived; false while it streams.
        closed: bool,
    },
    Mermaid {
        closed: bool,
    },
    Math,
    Html,
    /// A link reference definition (`[id]: url`), which draws nothing.
    Definition,
    Footnote,
    /// YAML front matter between `---` lines at the very start.
    Frontmatter,
    /// Anything else, including a source the parser rejected.
    Other,
}

impl BlockKind {
    /// A short, human name for labels ("heading", "table", ...).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Heading { .. } => "heading",
            Self::Paragraph => "paragraph",
            Self::Image => "image",
            Self::List { tasks: true, .. } => "task list",
            Self::List { .. } => "list",
            Self::Quote => "quote",
            Self::Table => "table",
            Self::Rule => "rule",
            Self::Code { .. } => "code block",
            Self::Mermaid { .. } => "diagram",
            Self::Math => "formula",
            Self::Html => "HTML block",
            Self::Definition => "link definition",
            Self::Footnote => "footnote",
            Self::Frontmatter => "front matter",
            Self::Other => "block",
        }
    }
}

/// A part of a block a comment can attach to on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockPart {
    /// A top-level item of a list, 0-based.
    ListItem {
        index: usize,
        checked: Option<bool>,
        bytes: Range<usize>,
        text: String,
    },
    /// A cell of a table; row 0 is the header.
    TableCell {
        row: usize,
        column: usize,
        bytes: Range<usize>,
        text: String,
    },
}

impl BlockPart {
    pub fn bytes(&self) -> Range<usize> {
        match self {
            Self::ListItem { bytes, .. } | Self::TableCell { bytes, .. } => bytes.clone(),
        }
    }

    /// The part's text, without markup.
    pub fn text(&self) -> &str {
        match self {
            Self::ListItem { text, .. } | Self::TableCell { text, .. } => text,
        }
    }
}

/// An image a block shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRef {
    pub url: String,
    pub alt: String,
    pub bytes: Range<usize>,
}

/// One top-level block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    /// The block's bytes in the source.
    pub bytes: Range<usize>,
    pub lines: LineRange,
    /// The text a reader sees, without markup.
    pub text: String,
    pub parts: Vec<BlockPart>,
    pub images: Vec<ImageRef>,
    /// A hash of the block's source with runs of whitespace collapsed, so a
    /// block whose only change is whitespace keeps its fingerprint (SM-16).
    pub fingerprint: u64,
}

/// What the last update to a [`Document`] parsed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParseStats {
    /// Whether the whole source was parsed.
    pub full: bool,
    /// Bytes handed to the parser.
    pub reparsed_bytes: usize,
    /// Blocks kept from before the update without parsing them again.
    pub reused_blocks: usize,
    /// Blocks the update produced by parsing.
    pub reparsed_blocks: usize,
}

/// A Markdown source and its top-level blocks.
#[derive(Clone, Debug)]
pub struct Document {
    source: String,
    blocks: Vec<Block>,
    /// Byte offsets where each line starts; `line_starts[0]` is 0.
    line_starts: Vec<usize>,
    stats: ParseStats,
}

impl Default for Document {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl PartialEq for Document {
    /// Two documents are equal when their sources and blocks are, however
    /// each was parsed.
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source && self.blocks == other.blocks
    }
}

impl Document {
    /// Parse a whole source.
    pub fn new(source: impl Into<String>) -> Self {
        let mut document = Self {
            source: source.into(),
            blocks: Vec::new(),
            line_starts: Vec::new(),
            stats: ParseStats::default(),
        };
        document.reparse_all();
        document
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The block that is a byline: a paragraph of exactly one emphasis span
    /// directly under a level-1 heading. A document page draws it quietly,
    /// with the file's name after it.
    pub fn byline(&self) -> Option<usize> {
        self.blocks.iter().position(|block| {
            if !matches!(block.kind, BlockKind::Paragraph) {
                return false;
            }
            let before = self.source.get(..block.bytes.start).unwrap_or_default();
            if !follows_title(before) {
                return false;
            }
            let Some(text) = self.source.get(block.bytes.clone()) else {
                return false;
            };
            byline_text(text).is_some()
        })
    }

    /// What the last update parsed.
    pub fn stats(&self) -> ParseStats {
        self.stats
    }

    /// Append streamed text, re-parsing only from the start of the last block.
    ///
    /// A document with link or footnote definitions is parsed whole, since a
    /// definition changes how earlier blocks read.
    pub fn append(&mut self, text: &str) {
        if text.is_empty() {
            self.stats = ParseStats::default();
            return;
        }
        let old_len = self.source.len();
        self.source.push_str(text);
        self.extend_line_starts(old_len);
        let defines = |kind: &BlockKind| matches!(kind, BlockKind::Definition | BlockKind::Footnote);
        let Some(last) = self.blocks.last() else {
            self.reparse_all();
            return;
        };
        // Front matter only counts at the very start, so a source that may
        // be opening it is parsed whole until it closes.
        let opening_frontmatter =
            self.source.starts_with("---") && self.blocks.first().is_none_or(|b| b.kind != BlockKind::Frontmatter);
        if opening_frontmatter || self.blocks.iter().any(|block| defines(&block.kind)) || may_define(text) {
            self.reparse_all();
            return;
        }
        let from = self.line_starts[self.line_index(last.bytes.start)];
        let first_line = self.line_index(from) + 1;
        let keep = self.blocks.len() - 1;
        self.blocks.truncate(keep);
        let tail = parse_blocks(&self.source[from..], from, first_line, &self.line_starts);
        if tail.iter().any(|block| defines(&block.kind)) {
            self.reparse_all();
            return;
        }
        self.stats = ParseStats {
            full: false,
            reparsed_bytes: self.source.len() - from,
            reused_blocks: keep,
            reparsed_blocks: tail.len(),
        };
        self.blocks.extend(tail);
    }

    /// Replace the source; when the new one extends the old, this appends.
    pub fn set_source(&mut self, source: &str) {
        if source == self.source {
            self.stats = ParseStats::default();
            return;
        }
        if !self.source.is_empty()
            && let Some(delta) = source.strip_prefix(self.source.as_str())
        {
            let delta = delta.to_owned();
            self.append(&delta);
            return;
        }
        self.source.clear();
        self.source.push_str(source);
        self.reparse_all();
    }

    /// The 1-based line `offset` is on.
    pub fn line_of(&self, offset: usize) -> usize {
        self.line_index(offset) + 1
    }

    /// The lines `bytes` covers. An empty range covers the line it is on; a
    /// range ending right after a newline doesn't take in the next line.
    pub fn lines_of(&self, bytes: &Range<usize>) -> LineRange {
        let start = self.line_of(bytes.start);
        let last = if bytes.end > bytes.start {
            bytes.end - 1
        } else {
            bytes.start
        };
        LineRange {
            start,
            end: self.line_of(last).max(start),
        }
    }

    /// The bytes of the 1-based, inclusive `lines`, without the last newline.
    pub fn line_bytes(&self, lines: LineRange) -> Range<usize> {
        let start_ix = lines.start.saturating_sub(1).min(self.line_starts.len() - 1);
        let start = self.line_starts[start_ix];
        let end = match self.line_starts.get(lines.end) {
            Some(next) => next.saturating_sub(1).max(start),
            None => self.source.len(),
        };
        start..end
    }

    /// The block holding `offset`, or the last block starting before it.
    pub fn block_at(&self, offset: usize) -> Option<usize> {
        let after = self.blocks.partition_point(|block| block.bytes.start <= offset);
        after.checked_sub(1)
    }

    /// The list item of block `block` that holds the 1-based `line`, as an
    /// index into the block's `parts`. A continuation line belongs to the item
    /// it continues. `None` when the block isn't a list or no item spans the
    /// line, so a paragraph or a heading never has an item.
    pub fn item_at_line(&self, block: usize, line: usize) -> Option<usize> {
        let block = self.blocks.get(block)?;
        block.parts.iter().position(|part| {
            matches!(part, BlockPart::ListItem { .. }) && self.lines_of(&part.bytes()).contains(line)
        })
    }

    /// The blocks `bytes` touches.
    pub fn blocks_in(&self, bytes: &Range<usize>) -> Range<usize> {
        let start = self
            .blocks
            .partition_point(|block| block.bytes.end <= bytes.start);
        let end = self
            .blocks
            .partition_point(|block| block.bytes.start < bytes.end.max(bytes.start + 1));
        start..end.max(start)
    }

    fn line_index(&self, offset: usize) -> usize {
        self.line_starts
            .partition_point(|start| *start <= offset)
            .saturating_sub(1)
    }

    fn extend_line_starts(&mut self, from: usize) {
        if self.line_starts.is_empty() {
            self.line_starts.push(0);
        }
        let bytes = self.source.as_bytes();
        for (ix, byte) in bytes.iter().enumerate().skip(from) {
            if *byte == b'\n' {
                self.line_starts.push(ix + 1);
            }
        }
    }

    fn reparse_all(&mut self) {
        self.line_starts.clear();
        self.extend_line_starts(0);
        self.blocks = parse_blocks(&self.source, 0, 1, &self.line_starts);
        self.stats = ParseStats {
            full: true,
            reparsed_bytes: self.source.len(),
            reused_blocks: 0,
            reparsed_blocks: self.blocks.len(),
        };
    }
}

/// Whether appended text could hold the start of a definition, which changes
/// how earlier blocks parse: `[label]:` or `[^note]:` at a line's start.
fn may_define(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim_start().starts_with('[') && line.contains("]:"))
}

/// The parse options of gpui-kit's text view as the view sets it up: GFM
/// with math and front matter.
fn parse_options() -> markdown::ParseOptions {
    let mut options = markdown::ParseOptions::gfm();
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    options.constructs.frontmatter = true;
    options
}

/// Parse `source`, which starts `offset` bytes and `first_line - 1` lines into
/// the document whose line starts are `line_starts`.
fn parse_blocks(source: &str, offset: usize, first_line: usize, line_starts: &[usize]) -> Vec<Block> {
    if source.trim().is_empty() {
        return Vec::new();
    }
    let line_of = |at: usize| {
        line_starts
            .partition_point(|start| *start <= at)
            .max(first_line)
    };
    let Ok(Node::Root(root)) = markdown::to_mdast(source, &parse_options()) else {
        // The parser rejects nothing GFM allows, but keep the text readable
        // if it ever does.
        let end = offset + source.len();
        return vec![Block {
            kind: BlockKind::Other,
            bytes: offset..end,
            lines: LineRange {
                start: first_line,
                end: line_of(end.saturating_sub(1)),
            },
            text: source.to_owned(),
            parts: Vec::new(),
            images: Vec::new(),
            fingerprint: fingerprint(source),
        }];
    };
    let definitions = definitions(&root.children);
    root.children
        .iter()
        .filter_map(|node| {
            let position = node.position()?;
            let local = position.start.offset..position.end.offset;
            let text_source = source.get(local.clone())?;
            let bytes = local.start + offset..local.end + offset;
            let kind = kind_of(node, text_source);
            let mut text = String::new();
            plain_text(node, &mut text);
            let mut images = Vec::new();
            collect_images(node, offset, &definitions, &mut images);
            Some(Block {
                parts: parts_of(node, offset),
                lines: LineRange {
                    start: line_of(bytes.start),
                    end: line_of(bytes.end.saturating_sub(1).max(bytes.start)),
                },
                text,
                images,
                fingerprint: fingerprint(text_source),
                kind,
                bytes,
            })
        })
        .collect()
}

/// Whether the text before a block ends with a level-1 heading line, so the
/// block comes directly under the title.
pub(crate) fn follows_title(before: &str) -> bool {
    let last = before.trim_end().lines().next_back().unwrap_or_default();
    let line = last.trim_start_matches(' ');
    if last.len() - line.len() > 3 {
        return false;
    }
    let bytes = line.as_bytes();
    bytes.first() == Some(&b'#') && (bytes.len() == 1 || matches!(bytes[1], b' ' | b'\t'))
}

/// The text, without markup, of a block whose source is one paragraph that
/// holds one emphasis span and nothing else but blank text; none otherwise.
/// It takes the source, not a parsed node, because each engine's text view
/// parses with its own copy of the Markdown crate.
pub(crate) fn byline_text(source: &str) -> Option<String> {
    let Ok(Node::Root(root)) = markdown::to_mdast(source, &parse_options()) else {
        return None;
    };
    let [node] = root.children.as_slice() else {
        return None;
    };
    if !emphasis_only(node) {
        return None;
    }
    let mut text = String::new();
    plain_text(node, &mut text);
    Some(text.trim().to_owned())
}

fn emphasis_only(node: &Node) -> bool {
    let Node::Paragraph(paragraph) = node else {
        return false;
    };
    let mut visible = paragraph.children.iter().filter(|child| match child {
        Node::Text(text) => !text.value.trim().is_empty(),
        _ => true,
    });
    matches!((visible.next(), visible.next()), (Some(Node::Emphasis(_)), None))
}

fn kind_of(node: &Node, source: &str) -> BlockKind {
    match node {
        Node::Heading(heading) => BlockKind::Heading {
            level: heading.depth,
        },
        Node::Paragraph(paragraph) => {
            let mut visible = paragraph.children.iter().filter(|child| match child {
                Node::Text(text) => !text.value.trim().is_empty(),
                _ => true,
            });
            match (visible.next(), visible.next()) {
                (Some(Node::Image(_) | Node::ImageReference(_)), None) => BlockKind::Image,
                _ => BlockKind::Paragraph,
            }
        }
        Node::List(list) => BlockKind::List {
            ordered: list.ordered,
            tasks: list.children.iter().any(|item| match item {
                Node::ListItem(item) => item.checked.is_some(),
                _ => false,
            }),
        },
        Node::Blockquote(_) => BlockKind::Quote,
        Node::Table(_) => BlockKind::Table,
        Node::ThematicBreak(_) => BlockKind::Rule,
        Node::Code(code) => {
            let closed = fence_closed(source);
            match code.lang.as_deref() {
                Some(lang) if lang.eq_ignore_ascii_case("mermaid") => BlockKind::Mermaid { closed },
                lang => BlockKind::Code {
                    language: lang.map(str::to_owned),
                    closed,
                },
            }
        }
        Node::Math(_) => BlockKind::Math,
        Node::Html(_) => BlockKind::Html,
        Node::Definition(_) => BlockKind::Definition,
        Node::FootnoteDefinition(_) => BlockKind::Footnote,
        Node::Yaml(_) => BlockKind::Frontmatter,
        _ => BlockKind::Other,
    }
}

/// Whether a code block's source ends with its closing fence. An indented
/// code block has no fence and is always closed.
pub(crate) fn fence_closed(source: &str) -> bool {
    let first = source.lines().next().unwrap_or_default().trim_start();
    let fence_char = match first.chars().next() {
        Some(ch @ ('`' | '~')) => ch,
        _ => return true,
    };
    let fence_len = first.chars().take_while(|ch| *ch == fence_char).count();
    if fence_len < 3 {
        return true;
    }
    let mut lines = source.lines();
    lines.next();
    lines.next_back().is_some_and(|last| {
        let last = last.trim();
        last.chars().take_while(|ch| *ch == fence_char).count() >= fence_len
            && last.chars().all(|ch| ch == fence_char)
    })
}

fn definitions(children: &[Node]) -> Vec<(String, String)> {
    children
        .iter()
        .filter_map(|node| match node {
            Node::Definition(definition) => {
                Some((definition.identifier.to_lowercase(), definition.url.clone()))
            }
            _ => None,
        })
        .collect()
}

fn plain_text(node: &Node, out: &mut String) {
    match node {
        Node::Text(text) => out.push_str(&text.value),
        Node::InlineCode(code) => out.push_str(&code.value),
        Node::InlineMath(math) => out.push_str(&math.value),
        Node::Code(code) => out.push_str(&code.value),
        Node::Math(math) => out.push_str(&math.value),
        Node::Html(html) => out.push_str(&html.value),
        Node::Image(image) => out.push_str(&image.alt),
        Node::ImageReference(image) => out.push_str(&image.alt),
        Node::Break(_) => out.push('\n'),
        Node::Definition(definition) => out.push_str(&definition.url),
        _ => {
            let Some(children) = node.children() else {
                return;
            };
            let blocky = matches!(
                node,
                Node::List(_)
                    | Node::ListItem(_)
                    | Node::Blockquote(_)
                    | Node::Table(_)
                    | Node::FootnoteDefinition(_)
            );
            let separator = if matches!(node, Node::TableRow(_)) {
                "\t"
            } else {
                "\n"
            };
            for (ix, child) in children.iter().enumerate() {
                if ix > 0 && (blocky || matches!(node, Node::TableRow(_))) {
                    out.push_str(separator);
                }
                plain_text(child, out);
            }
        }
    }
}

fn text_of(node: &Node) -> String {
    let mut text = String::new();
    plain_text(node, &mut text);
    text
}

fn collect_images(node: &Node, offset: usize, definitions: &[(String, String)], out: &mut Vec<ImageRef>) {
    let bytes = || {
        node.position()
            .map(|p| p.start.offset + offset..p.end.offset + offset)
            .unwrap_or(offset..offset)
    };
    match node {
        Node::Image(image) => out.push(ImageRef {
            url: image.url.clone(),
            alt: image.alt.clone(),
            bytes: bytes(),
        }),
        Node::ImageReference(image) => {
            let id = image.identifier.to_lowercase();
            if let Some((_, url)) = definitions.iter().find(|(key, _)| *key == id) {
                out.push(ImageRef {
                    url: url.clone(),
                    alt: image.alt.clone(),
                    bytes: bytes(),
                });
            }
        }
        _ => {
            for child in node.children().into_iter().flatten() {
                collect_images(child, offset, definitions, out);
            }
        }
    }
}

fn parts_of(node: &Node, offset: usize) -> Vec<BlockPart> {
    let bytes_of = |node: &Node| {
        node.position()
            .map(|p| p.start.offset + offset..p.end.offset + offset)
    };
    match node {
        Node::List(list) => list
            .children
            .iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Node::ListItem(list_item) => Some(BlockPart::ListItem {
                    index,
                    checked: list_item.checked,
                    bytes: bytes_of(item)?,
                    text: text_of(item),
                }),
                _ => None,
            })
            .collect(),
        Node::Table(table) => table
            .children
            .iter()
            .enumerate()
            .flat_map(|(row, row_node)| {
                row_node
                    .children()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(move |(column, cell)| {
                        // A cell's content, without its pipes and padding;
                        // an empty cell is its whole span.
                        let children = cell.children().map(Vec::as_slice).unwrap_or_default();
                        let content = match (children.first(), children.last()) {
                            (Some(first), Some(last)) => Some(bytes_of(first)?.start..bytes_of(last)?.end),
                            _ => None,
                        };
                        Some(BlockPart::TableCell {
                            row,
                            column,
                            bytes: content.or_else(|| bytes_of(cell))?,
                            text: text_of(cell),
                        })
                    })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// FNV-1a over `source` with every run of whitespace read as one space and
/// leading and trailing whitespace dropped.
pub(crate) fn fingerprint(source: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut push = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    };
    let mut first = true;
    for word in source.split_whitespace() {
        if !first {
            push(b' ');
        }
        first = false;
        word.bytes().for_each(&mut push);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SM-3
    #[test]
    fn an_unclosed_fence_is_open_until_its_closing_fence_arrives() {
        assert!(!fence_closed("```rust\nfn main() {}\n"));
        assert!(!fence_closed("```rust"));
        assert!(fence_closed("```rust\nfn main() {}\n```"));
        assert!(fence_closed("````\n```\n````"));
        assert!(!fence_closed("````\n```\n"));
        assert!(fence_closed("    indented code"));
    }

    /// The byline sits directly under the title and is only emphasis.
    #[test]
    fn a_paragraph_of_one_emphasis_span_under_the_title_is_the_byline() {
        let document = Document::new("# Title\n\n*Draft by Ada*\n\nBody.\n");
        assert_eq!(document.byline(), Some(1));
    }

    #[test]
    fn emphasis_with_other_text_under_the_title_is_not_a_byline() {
        let document = Document::new("# Title\n\n*Draft* by Ada\n\nBody.\n");
        assert_eq!(document.byline(), None);
    }

    #[test]
    fn emphasis_that_is_not_under_the_title_is_not_a_byline() {
        let document = Document::new("# Title\n\nBody.\n\n*Draft by Ada*\n");
        assert_eq!(document.byline(), None);
        let document = Document::new("## Section\n\n*Draft by Ada*\n");
        assert_eq!(document.byline(), None);
    }

    /// A line of a list lights the item it is on, and a continuation line
    /// lights the item it continues.
    #[test]
    fn a_line_inside_a_list_item_finds_that_item() {
        let document = Document::new("# Title\n\n- one\n  more\n- two\n- three\n");
        assert_eq!(document.item_at_line(1, 3), Some(0));
        assert_eq!(document.item_at_line(1, 4), Some(0));
        assert_eq!(document.item_at_line(1, 5), Some(1));
        assert_eq!(document.item_at_line(1, 6), Some(2));
    }

    /// A paragraph or a heading has no list items, so a mark on it finds none.
    #[test]
    fn a_paragraph_has_no_item_but_a_list_line_has_one() {
        let document = Document::new("# Title\n\nA paragraph.\n\n- one\n- two\n");
        assert_eq!(document.item_at_line(0, 1), None);
        assert_eq!(document.item_at_line(1, 3), None);
        assert_eq!(document.item_at_line(2, 5), Some(0));
    }

    /// SM-16
    #[test]
    fn whitespace_alone_does_not_change_a_fingerprint() {
        assert_eq!(fingerprint("a  b\n c"), fingerprint("a b c "));
        assert_ne!(fingerprint("a b"), fingerprint("ab"));
    }
}
