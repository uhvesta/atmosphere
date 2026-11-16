//! Finding a comment's quoted lines again in its file as it is now
//! (SM-7, SM-16): still in place, moved unchanged, or changed.

/// Where a comment's quoted lines are in the file as it is now.
pub enum Placement {
    /// Still at the lines the anchor names.
    Same,
    /// Unchanged but at other lines (SM-7): 1-based inclusive.
    Moved { first: u32, last: u32 },
    /// Changed or gone (SM-16): what is at the anchor's lines now (empty
    /// when the file is shorter or gone).
    Changed { now: Vec<String> },
}

/// A line without its whitespace, so a change that only alters whitespace
/// does not count (SM-16), as `git diff -w` reads it.
fn squash(line: &str) -> String {
    line.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Finds `quoted` (the anchored lines as they were, at 1-based `first`) in
/// `lines`, the file's lines now.
#[must_use]
pub fn place(quoted: &[String], first: u32, lines: &[&str]) -> Placement {
    let len = quoted.len();
    if len == 0 {
        return Placement::Same;
    }
    let want: Vec<String> = quoted.iter().map(|line| squash(line)).collect();
    let now: Vec<String> = lines.iter().map(|line| squash(line)).collect();
    let at = |start: usize| len <= now.len() - start.min(now.len()) && now[start..start + len] == want[..];
    let start = (first as usize).saturating_sub(1);
    if first != 0 && start <= now.len() && at(start) {
        return Placement::Same;
    }
    if len <= now.len() {
        let found = (0..=now.len() - len)
            .filter(|s| now[*s..*s + len] == want[..])
            .min_by_key(|s| (s + 1).abs_diff(first as usize));
        if let Some(s) = found {
            return Placement::Moved {
                first: u32::try_from(s + 1).unwrap_or(u32::MAX),
                last: u32::try_from(s + len).unwrap_or(u32::MAX),
            };
        }
    }
    let from = start.min(lines.len());
    let to = (start + len).min(lines.len());
    Placement::Changed {
        now: lines[from..to].iter().map(|line| (*line).to_string()).collect(),
    }
}
