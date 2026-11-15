//! The document model, anchors and diffs, without a window.

use humata2_markdown::{
    Anchor, AnchorStatus, AnchorTarget, BlockChange, BlockKind, BlockPart, Comment, Document, LineRange, diff,
};

const GALLERY: &str = "---\ntitle: Gallery\n---\n\n# Plan\n\nSome *emphasis*, `code` and a [link](https://example.com).\n\n- [x] done\n- [ ] next\n\n> A quote.\n\n| Name | Value |\n| ---- | ----- |\n| a    | 1     |\n| b    | 2     |\n\n---\n\n```rust\nfn main() {}\n```\n\n```mermaid\nflowchart TD\n  A --> B\n```\n\n![logo](images/logo.png)\n\nThe end.\n";

fn kinds(document: &Document) -> Vec<BlockKind> {
    document.blocks().iter().map(|block| block.kind.clone()).collect()
}

/// SM-3
#[test]
fn a_gallery_document_splits_into_the_blocks_a_reader_sees() {
    let document = Document::new(GALLERY);
    assert_eq!(
        kinds(&document),
        vec![
            BlockKind::Frontmatter,
            BlockKind::Heading { level: 1 },
            BlockKind::Paragraph,
            BlockKind::List {
                ordered: false,
                tasks: true
            },
            BlockKind::Quote,
            BlockKind::Table,
            BlockKind::Rule,
            BlockKind::Code {
                language: Some("rust".into()),
                closed: true
            },
            BlockKind::Mermaid { closed: true },
            BlockKind::Image,
            BlockKind::Paragraph,
        ]
    );
    let heading = &document.blocks()[1];
    assert_eq!(heading.text.trim(), "Plan");
    assert_eq!(heading.lines, LineRange { start: 5, end: 5 });
    let image = &document.blocks()[9];
    assert_eq!(image.images[0].url, "images/logo.png");
    assert_eq!(image.images[0].alt, "logo");
}

/// SM-3
#[test]
fn every_block_covers_its_own_source() {
    let document = Document::new(GALLERY);
    let mut last_end = 0;
    for block in document.blocks() {
        assert!(block.bytes.start >= last_end, "blocks overlap at {:?}", block.bytes);
        assert!(block.bytes.end <= GALLERY.len());
        assert_eq!(document.lines_of(&block.bytes), block.lines);
        last_end = block.bytes.end;
    }
}

/// SM-3
#[test]
fn streaming_a_document_at_any_split_ends_with_the_same_blocks_as_parsing_it_whole() {
    let whole = Document::new(GALLERY);
    for split in (0..=GALLERY.len()).filter(|at| GALLERY.is_char_boundary(*at)) {
        let mut streamed = Document::new(&GALLERY[..split]);
        streamed.append(&GALLERY[split..]);
        assert_eq!(streamed, whole, "split at byte {split}");
    }
}

/// SM-3
#[test]
fn streaming_byte_by_byte_ends_with_the_same_blocks_as_parsing_it_whole() {
    let body = GALLERY.split_once("---\n\n").map(|(_, body)| body).unwrap_or(GALLERY);
    let whole = Document::new(body);
    let mut streamed = Document::default();
    let mut at = 0;
    while at < body.len() {
        let mut next = at + 1;
        while !body.is_char_boundary(next) {
            next += 1;
        }
        streamed.append(&body[at..next]);
        at = next;
    }
    assert_eq!(streamed, whole);
}

/// SM-3
#[test]
fn appending_reparses_only_the_last_block() {
    let mut document = Document::new("# One\n\nTwo\n\nThree");
    document.append(" more");
    let stats = document.stats();
    assert!(!stats.full);
    assert_eq!(stats.reused_blocks, 2);
    assert_eq!(stats.reparsed_blocks, 1);
    assert_eq!(stats.reparsed_bytes, "Three more".len());
}

/// SM-3
#[test]
fn a_link_definition_makes_appending_reparse_the_whole_document() {
    let mut document = Document::new("See [the spec][spec].\n\n");
    document.append("[spec]: https://example.com\n");
    assert!(document.stats().full);
    assert_eq!(document, Document::new("See [the spec][spec].\n\n[spec]: https://example.com\n"));
}

/// SM-3
#[test]
fn an_open_fence_is_code_still_arriving_until_it_closes() {
    let mut document = Document::new("```mermaid\nflowchart TD\n");
    assert_eq!(kinds(&document), vec![BlockKind::Mermaid { closed: false }]);
    document.append("  A --> B\n```\n");
    assert_eq!(kinds(&document), vec![BlockKind::Mermaid { closed: true }]);
}

/// SM-3
#[test]
fn front_matter_counts_only_at_the_very_start() {
    let document = Document::new("# Title\n\n---\ntitle: no\n---\n");
    assert!(!kinds(&document).contains(&BlockKind::Frontmatter));
    let mut streamed = Document::new("--");
    streamed.append("-\ntitle: yes\n---\n\nBody\n");
    assert_eq!(kinds(&streamed), vec![BlockKind::Frontmatter, BlockKind::Paragraph]);
}

/// SM-6
#[test]
fn list_items_and_table_cells_are_parts_a_comment_can_target() {
    let document = Document::new(GALLERY);
    let list = &document.blocks()[3];
    let checked: Vec<Option<bool>> = list
        .parts
        .iter()
        .map(|part| match part {
            BlockPart::ListItem { checked, .. } => *checked,
            BlockPart::TableCell { .. } => None,
        })
        .collect();
    assert_eq!(checked, vec![Some(true), Some(false)]);

    let table = 5;
    let cells = &document.blocks()[table].parts;
    let b_value = cells
        .iter()
        .position(|part| matches!(part, BlockPart::TableCell { row: 2, column: 1, .. }))
        .expect("the table has a cell in row 2, column 1");
    let selection = document.part_selection(table, b_value).expect("the cell selects");
    assert_eq!(selection.quote, "2");
    assert_eq!(selection.source, "2");
    assert_eq!(
        selection.target,
        AnchorTarget::TableCell {
            block: table,
            row: 2,
            column: 1
        }
    );
    assert_eq!(selection.lines, LineRange { start: 17, end: 17 });
}

/// SM-6
#[test]
fn a_diagram_selects_as_a_diagram_with_its_lines() {
    let document = Document::new(GALLERY);
    let selection = document.block_selection(8).expect("the diagram selects");
    assert_eq!(selection.target, AnchorTarget::Diagram { block: 8 });
    assert_eq!(selection.lines, LineRange { start: 25, end: 28 });
    assert!(selection.source.starts_with("```mermaid"));
}

/// SM-6
#[test]
fn a_text_selection_maps_to_its_source_bytes_and_lines() {
    let document = Document::new(GALLERY);
    let start = GALLERY.find("*emphasis*").expect("the gallery has emphasis");
    let selection = document
        .selection(start..start + "*emphasis*".len(), "emphasis")
        .expect("the range is in the source");
    assert_eq!(selection.source, "*emphasis*");
    assert_eq!(selection.quote, "emphasis");
    assert_eq!(selection.lines, LineRange { start: 7, end: 7 });
    assert!(document.selection(0..GALLERY.len() + 1, "").is_none());
}

fn anchor_on(document: &Document, passage: &str) -> Anchor {
    let start = document.source().find(passage).expect("the passage is in the document");
    let selection = document
        .selection(start..start + passage.len(), passage)
        .expect("the range is in the source");
    Anchor::new(document, &selection)
}

/// SM-7
#[test]
fn an_anchor_follows_its_passage_when_text_is_added_before_it() {
    let old = Document::new("# Plan\n\nShip the parser first.\n\nThen the view.\n");
    let anchor = anchor_on(&old, "Then the view.");
    let new = Document::new("# Plan\n\nAn intro.\n\nShip the parser first.\n\nThen the view.\n");
    let start = new.source().find("Then the view.").expect("still there");
    assert_eq!(
        anchor.resolve(&new),
        AnchorStatus::Current {
            bytes: start..start + "Then the view.".len(),
            lines: LineRange { start: 7, end: 7 },
        }
    );
}

/// SM-7, SM-16
#[test]
fn a_whitespace_only_change_keeps_an_anchor_current() {
    let old = Document::new("Ship the parser first, then the view.\n");
    let anchor = anchor_on(&old, "the parser first, then");
    let new = Document::new("Ship  the parser\nfirst, then the view.\n");
    assert!(matches!(anchor.resolve(&new), AnchorStatus::Current { .. }));
}

/// SM-7
#[test]
fn an_anchor_whose_text_changed_between_the_same_surroundings_is_outdated() {
    let old = Document::new("# Plan\n\nShip the parser first.\n\n## Later\n");
    let anchor = anchor_on(&old, "Ship the parser first.");
    let new = Document::new("# Plan\n\nShip the view first.\n\n## Later\n");
    let start = new.source().find("Ship the view first.").expect("the new text is there");
    assert_eq!(
        anchor.resolve(&new),
        AnchorStatus::Outdated {
            bytes: start..start + "Ship the view first.".len(),
            lines: LineRange { start: 3, end: 3 },
        }
    );
}

/// SM-7
#[test]
fn a_reworded_passage_beside_a_deleted_paragraph_is_outdated() {
    let old = Document::new("# Plan\n\nShip the parser first.\n\n## Later\n\nThen the view.\n");
    let reworded = anchor_on(&old, "Ship the parser first.");
    let deleted = anchor_on(&old, "Then the view.");
    let new = Document::new("# Plan\n\nShip the view first.\n\n## Later\n");
    let start = new.source().find("Ship the view first.").expect("the new text is there");
    assert_eq!(
        reworded.resolve(&new),
        AnchorStatus::Outdated {
            bytes: start..start + "Ship the view first.".len(),
            lines: LineRange { start: 3, end: 3 },
        }
    );
    assert_eq!(deleted.resolve(&new), AnchorStatus::Gone);
}

/// SM-7
#[test]
fn a_deleted_paragraph_between_paragraphs_that_remain_is_gone() {
    let old = Document::new("Keep the first.\n\nDrop the middle one.\n\nKeep the last.\n");
    let anchor = anchor_on(&old, "Drop the middle one.");
    let new = Document::new("Keep the first.\n\nKeep the last.\n");
    assert_eq!(anchor.resolve(&new), AnchorStatus::Gone);
}

/// SM-7
#[test]
fn an_anchor_whose_passage_and_surroundings_are_gone_is_gone() {
    let old = Document::new("# Plan\n\nShip the parser first.\n\n## Later\n");
    let anchor = anchor_on(&old, "Ship the parser first.");
    let new = Document::new("Something else entirely.\n");
    assert_eq!(anchor.resolve(&new), AnchorStatus::Gone);
}

/// SM-13
#[test]
fn a_comment_keeps_its_anchor_and_body() {
    let document = Document::new("Text to discuss.\n");
    let comment = Comment {
        id: "c1".into(),
        anchor: anchor_on(&document, "discuss"),
        body: "Why?".into(),
    };
    assert_eq!(comment.anchor.resolve(&document).bytes(), Some(8..15));
}

/// SM-5
#[test]
fn a_diff_marks_inserted_changed_and_deleted_blocks() {
    let old = Document::new("# Plan\n\nKeep this.\n\nChange this line.\n\nDrop this.\n");
    let new = Document::new("# Plan\n\nKeep this.\n\nChange this sentence.\n\nA new paragraph.\n");
    let changes = diff(&old, &new);
    assert!(matches!(changes.blocks[0], BlockChange::Unchanged { old_index: 0 }));
    assert!(matches!(changes.blocks[1], BlockChange::Unchanged { old_index: 1 }));
    let BlockChange::Changed { old_index, inserted } = &changes.blocks[2] else {
        panic!("the third block changed: {:?}", changes.blocks[2]);
    };
    assert_eq!(*old_index, 2);
    let inserted: Vec<&str> = inserted.iter().map(|range| &new.source()[range.clone()]).collect();
    assert_eq!(inserted, vec!["sentence"]);
    // "Drop this." and "A new paragraph." don't match at all: one went, one came.
    assert!(matches!(changes.blocks[3], BlockChange::Inserted));
    assert_eq!(changes.deleted.len(), 1);
    assert_eq!(changes.deleted[0].text.trim(), "Drop this.");
    assert_eq!(changes.deleted[0].before, 3);
}

/// SM-16
#[test]
fn a_whitespace_only_change_is_no_change_in_a_diff() {
    let old = Document::new("# Plan\n\nKeep this.\n");
    let new = Document::new("#  Plan\n\nKeep   this.\n\n");
    assert!(diff(&old, &new).is_empty());
}

/// SM-5
#[test]
fn a_block_deleted_at_the_end_is_placed_after_the_last_block() {
    let old = Document::new("One.\n\nTwo.\n");
    let new = Document::new("One.\n");
    let changes = diff(&old, &new);
    assert_eq!(changes.deleted.len(), 1);
    assert_eq!(changes.deleted[0].before, 1);
}
