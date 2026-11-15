//! The exact text a batch of comments reaches the agent as. The preview shows
//! this text and the queue sends this text, so what you saw is what it gets.

use std::fmt::Write as _;

use super::anchor::Anchor;
use super::review::Side;

/// The most quoted lines each comment carries; the rest are counted.
const QUOTE_LINES: usize = 6;

/// One comment as it goes into a message.
pub struct Part<'a> {
    pub anchor: &'a Anchor,
    pub body: &'a str,
    /// A question (Ask): answered, not acted on.
    pub question: bool,
    pub outdated: bool,
}

/// The message for `parts`: a header with the count, then each file with its
/// base, then each comment's lines, quote and text. Files and lines are in
/// order, so the same comments always make the same message. Questions are
/// marked, and the agent is told to answer them without changing anything.
pub fn compose(parts: &mut [Part<'_>]) -> String {
    parts.sort_by(|a, b| {
        (&a.anchor.file, a.anchor.side, a.anchor.lines).cmp(&(
            &b.anchor.file,
            b.anchor.side,
            b.anchor.lines,
        ))
    });
    let mut out = String::new();
    let questions = parts.iter().filter(|part| part.question).count();
    let _ = match (questions, parts.len() - questions) {
        (0, n) => writeln!(out, "Review comments from you ({n}):"),
        (n, 0) => writeln!(
            out,
            "Questions from you about this code ({n}). Answer them; don't change any files to do so:"
        ),
        (q, c) => writeln!(
            out,
            "Review comments ({c}) and questions ({q}) from you. Answer the questions without changing files for them:"
        ),
    };
    let mut file = None;
    for part in parts.iter() {
        let anchor = part.anchor;
        if file != Some((&anchor.file, &anchor.base)) {
            file = Some((&anchor.file, &anchor.base));
            let _ = writeln!(out);
            let _ = writeln!(out, "{} (at {})", anchor.file, short(&anchor.base));
        }
        let lines = anchor.lines;
        let mut place = if lines.is_file() {
            "the whole file".to_owned()
        } else if lines.first == lines.last {
            format!("line {}", lines.first)
        } else {
            format!("lines {}-{}", lines.first, lines.last)
        };
        if anchor.side == Side::Old {
            place.push_str(" (old side)");
        }
        if let Some(rendered) = &anchor.rendered {
            let _ = write!(place, ", rendered as \"{}\"", one_line(&rendered.text, 60));
        }
        if part.outdated {
            place.push_str(", the code there has changed since");
        }
        let _ = writeln!(out, "  {place}:");
        for line in anchor.quoted.iter().take(QUOTE_LINES) {
            let _ = writeln!(out, "  > {line}");
        }
        if anchor.quoted.len() > QUOTE_LINES {
            let _ = writeln!(
                out,
                "  > ({} more lines)",
                anchor.quoted.len() - QUOTE_LINES
            );
        }
        for (index, line) in part.body.trim().lines().enumerate() {
            if part.question && index == 0 {
                let _ = writeln!(out, "  Question: {line}");
            } else {
                let _ = writeln!(out, "  {line}");
            }
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// A revision's first seven characters.
fn short(revision: &str) -> &str {
    revision.get(..7).unwrap_or(revision)
}

/// `text` on one line, cut to `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max {
        joined
    } else {
        let mut cut: String = joined.chars().take(max).collect();
        cut.push('…');
        cut
    }
}
