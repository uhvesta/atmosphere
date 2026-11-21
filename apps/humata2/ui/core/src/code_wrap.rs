//! Monospace code fragments: retain source bytes while wrapping whole graphemes.
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub struct Fragment {
    pub display: String,
    pub source: Range<usize>,
}

/// A scrolling line, with four-column tab stops.
pub fn unwrapped(text: &str) -> Fragment {
    let mut display = String::new();
    let mut column = 0;
    for grapheme in text.graphemes(true) {
        if grapheme == "\t" {
            let count = 4 - column % 4;
            display.push_str(&" ".repeat(count));
            column += count;
        } else {
            display.push_str(grapheme);
            column += grapheme.width();
        }
    }
    Fragment {
        display,
        source: 0..text.len(),
    }
}

/// Whether a grapheme is a good word/punctuation boundary to wrap *after*
/// before falling back to an arbitrary character split.
fn is_wrap_break_after(grapheme: &str) -> bool {
    matches!(
        grapheme.as_bytes(),
        [b' '
            | b'\t'
            | b'.'
            | b','
            | b';'
            | b':'
            | b'('
            | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b'/'
            | b'\\'
            | b'-'
            | b'='
            | b'+'
            | b'*'
            | b'|'
            | b'?']
    )
}

/// Wrap at cell boundaries, preferring whitespace/punctuation boundaries before
/// falling back to character splits, and never splitting inside a grapheme.
/// Deep indentation is limited to half the viewport so the first visual row
/// always contains code. At least four cells are needed to fit a tab or a wide
/// grapheme.
pub fn wrapped(text: &str, columns: usize) -> Vec<Fragment> {
    let columns = columns.max(4);
    let mut indent_end = 0;
    let mut logical_column = 0;
    for (at, ch) in text.char_indices() {
        match ch {
            ' ' => logical_column += 1,
            '\t' => logical_column += 4 - logical_column % 4,
            _ => break,
        }
        indent_end = at + ch.len_utf8();
    }
    if indent_end == text.len() {
        return vec![Fragment {
            display: String::new(),
            source: 0..text.len(),
        }];
    }
    let indent_limit = logical_column.min(columns / 2);
    let mut indent = indent_limit;
    let mut fragments = Vec::new();
    let mut source_start = 0;
    let mut current = String::new();
    let mut current_width = 0;
    // Last whitespace/punctuation break point within `current`:
    // `(display_byte_len, visual_width, source_byte_end)`.
    let mut last_break: Option<(usize, usize, usize)> = None;
    for (offset, grapheme) in text[indent_end..].grapheme_indices(true) {
        let at = indent_end + offset;
        let (rendered, width) = if grapheme == "\t" {
            let count = 4 - logical_column % 4;
            (" ".repeat(count), count)
        } else {
            (grapheme.to_owned(), grapheme.width())
        };
        logical_column += width;
        if current_width > 0 && current_width + width > columns - indent {
            if let Some((break_disp, break_width, break_src)) = last_break
                && break_disp > 0
                && (!fragments.is_empty()
                    || current[..break_disp].chars().any(|c| !c.is_whitespace()))
            {
                let head = &current[..break_disp];
                let tail = current[break_disp..].to_owned();
                fragments.push(Fragment {
                    display: format!("{}{head}", " ".repeat(indent)),
                    source: source_start..break_src,
                });
                source_start = break_src;
                current = tail;
                current_width -= break_width;
                last_break = None;
                indent = indent_limit.min(columns.saturating_sub(current_width));
            }
            if current_width > 0 && current_width + width > columns - indent {
                fragments.push(Fragment {
                    display: format!("{}{current}", " ".repeat(indent)),
                    source: source_start..at,
                });
                source_start = at;
                current.clear();
                current_width = 0;
                last_break = None;
            }
        }
        if current_width == 0 {
            indent = indent_limit.min(columns.saturating_sub(width));
        }
        current.push_str(&rendered);
        current_width += width;
        if is_wrap_break_after(grapheme) {
            last_break = Some((current.len(), current_width, at + grapheme.len()));
        }
    }
    fragments.push(Fragment {
        display: format!("{}{current}", " ".repeat(indent)),
        source: source_start..text.len(),
    });
    fragments
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapped_code_keeps_source_and_graphemes_without_an_indent_only_first_row() {
        for source in [
            "        self.db.insert_legacy(&entry).await?;",
            "\t\t        long_identifier_without_spaces",
            "    account_👩‍💻_e\u{301}_界_ledger",
            "    let\tamount = 100;",
        ] {
            let parts = wrapped(source, 12);
            assert!(parts[0].display.chars().any(|c| !c.is_whitespace()));
            let boundaries: std::collections::BTreeSet<_> = source
                .grapheme_indices(true)
                .map(|(at, _)| at)
                .chain(std::iter::once(source.len()))
                .collect();
            let mut restored = String::new();
            for part in parts {
                assert!(part.display.width() <= 12);
                assert!(
                    boundaries.contains(&part.source.start)
                        && boundaries.contains(&part.source.end)
                );
                restored.push_str(&source[part.source]);
            }
            assert_eq!(restored, source);
        }
    }
    #[test]
    fn blank_lines_and_small_viewports_terminate_and_scrolling_keeps_tab_stops() {
        assert_eq!(wrapped("", 0).len(), 1);
        assert_eq!(wrapped("   \t", 8).len(), 1);
        assert!(!wrapped("    identifier", 0)[0].display.trim().is_empty());
        let line = unwrapped("\ta\tb");
        assert_eq!(line.display, "    a   b");
        assert_eq!(line.source, 0..4);
    }
    #[test]
    fn wrapped_code_prefers_whitespace_and_punctuation_before_char_splits() {
        let parts = wrapped("    let total = price + tax;", 14);
        let displayed: Vec<&str> = parts.iter().map(|p| p.display.as_str()).collect();
        assert_eq!(displayed, vec!["    let total ", "    = price + ", "    tax;"]);

        let dotted = wrapped("  self.db.insert(&entry)", 14);
        let dotted_display: Vec<&str> = dotted.iter().map(|p| p.display.as_str()).collect();
        assert_eq!(dotted_display, vec!["  self.db.", "  insert(", "  &entry)"]);

        // An unbroken identifier longer than the budget still falls back to char splits.
        let long = wrapped("  supercalifragilistic", 10);
        assert!(long.len() >= 3);
        assert!(long.iter().all(|p| p.display.width() <= 10));
        let restored: String = long.iter().map(|p| &"  supercalifragilistic"[p.source.clone()]).collect();
        assert_eq!(restored, "  supercalifragilistic");
    }
}
