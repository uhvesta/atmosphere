//! The message a batch of comments is typed into the agent's terminal as
//! (SM-9, frame 43), and the bracketed paste it travels in. This is the
//! one place the daemon writes that text, whatever view the comments were
//! made in (SM-13).
//!
//! Comment text and quoted code come from people and from files, so
//! [`bracketed_paste`] removes everything that could end the paste early
//! or drive the terminal.

use crate::Side;

/// Most quoted lines a message carries per comment.
const QUOTE_LIMIT: usize = 12;

/// One comment of a batch, as the message needs it.
pub struct MessagePart<'a> {
    /// The comment's id (e.g. `c-12`), included in the header so the agent can
    /// call `comment_reply` / `comment_resolve` directly without `comments_list`.
    pub id: Option<&'a str>,
    /// The repo, when the session has more than one (SM-9); None otherwise.
    pub repo: Option<&'a str>,
    /// File path, or empty for a session-level overall comment.
    pub path: &'a str,
    /// 1-based inclusive lines; 0..0 is the whole file (or whole session when `path` is empty).
    pub first: u32,
    pub last: u32,
    /// Set for a comment made in a diff (RV-27).
    pub diff: Option<DiffContext<'a>>,
    /// The quoted code; used only with `diff` on a file comment.
    pub quoted: &'a [String],
    pub text: &'a str,
}

/// The side and base a diff comment was made against.
pub struct DiffContext<'a> {
    pub side: Side,
    pub base: &'a str,
}

/// The message for `parts`, in the order given (SM-9, frame 43).
#[must_use]
pub fn compose(parts: &[MessagePart<'_>]) -> String {
    parts.iter().map(one).collect::<Vec<_>>().join("\n\n")
}

fn one(part: &MessagePart<'_>) -> String {
    let mut out = String::from("[comment");
    if let Some(id) = part.id.filter(|id| !id.is_empty()) {
        out.push(' ');
        out.push_str(id);
    }
    out.push_str(" on ");
    if part.path.is_empty() {
        out.push_str("session]\n");
        out.push_str(part.text.trim_end());
        return out;
    }
    if let Some(repo) = part.repo {
        out.push_str(repo);
        out.push('/');
    }
    out.push_str(part.path);
    if part.first != 0 || part.last != 0 {
        if part.first == part.last {
            out.push_str(&format!(":{}", part.first));
        } else {
            out.push_str(&format!(":{}-{}", part.first, part.last));
        }
    }
    if let Some(diff) = &part.diff {
        out.push_str(" · ");
        out.push_str(diff.side.name());
        if !diff.base.is_empty() {
            out.push_str(" · base ");
            out.push_str(diff.base);
        }
    }
    out.push_str("]\n");
    if part.diff.is_some() {
        for line in part.quoted.iter().take(QUOTE_LIMIT) {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
        if part.quoted.len() > QUOTE_LIMIT {
            out.push_str(&format!(
                "> ({} more lines)\n",
                part.quoted.len() - QUOTE_LIMIT
            ));
        }
    }
    out.push_str(part.text.trim_end());
    out
}

/// `message` as one bracketed paste, safe to write to a terminal.
#[must_use]
pub fn bracketed_paste(message: &str) -> Vec<u8> {
    let mut clean = String::with_capacity(message.len());
    let mut chars = message.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    clean.push('\n');
                }
            }
            '\n' | '\t' => clean.push(c),
            c if c.is_control() => {}
            c => clean.push(c),
        }
    }
    let mut out = Vec::with_capacity(clean.len() + 12);
    out.extend_from_slice(b"\x1b[200~");
    out.extend_from_slice(clean.as_bytes());
    out.extend_from_slice(b"\x1b[201~");
    out
}
