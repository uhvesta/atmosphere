//! Code-aware FTS5 pre-tokenizer (`ST-6`) and deterministic error-signature
//! normalizer (`AW-3`, `HOOK-2`).

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use sha2::{Digest, Sha256};

/// Pre-tokenizes code and prose for SQLite FTS5 `unicode61` indexing (`ST-6`).
///
/// Splits `camelCase`, `PascalCase`, `snake_case`, `::`, `/`, `.`, and `-`
/// while preserving the original lowercased compound token so both exact symbol
/// lookups and sub-token queries match.
pub fn tokenize_code(text: &str) -> String {
    tokenize_code_terms(text).join(" ")
}

/// Returns the ordered sequence of lowercased tokens produced by
/// [`tokenize_code`].
pub fn tokenize_code_terms(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split_whitespace() {
        let trimmed = raw.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '_' && c != ':' && c != '/' && c != '.' && c != '-'
        });
        if trimmed.is_empty() {
            continue;
        }

        let mut local_seen = HashSet::new();
        let mut push_local = |tok: String| {
            if !tok.is_empty() && local_seen.insert(tok.clone()) {
                out.push(tok);
            }
        };

        // Preserve original token (lowercased)
        push_local(trimmed.to_ascii_lowercase());

        // Split on structural delimiters `::`, `/`, `.`, `-`
        for seg in split_delimiters(trimmed) {
            if seg.is_empty() {
                continue;
            }
            push_local(seg.to_ascii_lowercase());

            // Split on snake_case `_`
            for snake in seg.split('_') {
                if snake.is_empty() {
                    continue;
                }
                push_local(snake.to_ascii_lowercase());

                // Split on camelCase / PascalCase / acronym boundaries
                for part in split_camel_case(snake) {
                    push_local(part.to_ascii_lowercase());
                }
            }
        }
    }
    out
}

/// Extracts deterministic `err:<sig>` error signatures from compiler, linker,
/// build-tool, or runtime `stderr` output.
///
/// Paths, line/column numbers, hex addresses, UUIDs, timestamps, and numeric
/// literals are normalized so two sessions hitting the same failure produce the
/// same `err:<sig>` key.
pub fn normalize_error_signature(stderr: &str) -> Vec<String> {
    let mut sigs = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |sig: String| {
        if seen.insert(sig.clone()) {
            sigs.push(sig);
        }
    };

    // 1. Explicit Rust compiler error codes: `error[E0382]`
    for cap in rustc_code_re().captures_iter(stderr) {
        if let Some(code) = cap.get(1) {
            push(format!("err:rustc:{}", code.as_str().to_ascii_lowercase()));
        }
    }

    // 2. Common POSIX errno names in error lines
    for cap in errno_re().captures_iter(stderr) {
        if let Some(code) = cap.get(1) {
            push(format!("err:errno:{}", code.as_str().to_ascii_lowercase()));
        }
    }

    // 3. Normalized error / panic / fatal lines
    for line in stderr.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        let is_error_line = lower.starts_with("error")
            || lower.starts_with("fatal:")
            || lower.contains("panicked at")
            || lower.contains("undefined reference")
            || lower.contains("unresolved import")
            || lower.contains("cannot find ")
            || lower.contains("permission denied")
            || lower.contains("no such file or directory")
            || lower.contains("failed:");

        if !is_error_line {
            continue;
        }

        let norm = normalize_error_line(trimmed);
        if norm.is_empty() {
            continue;
        }
        let digest = short_sha256(&norm);
        push(format!("err:sig:{digest}"));
    }

    sigs
}

fn split_delimiters(token: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    for chunk in token.split("::") {
        for sub in chunk.split(['/', '.', '-']) {
            if !sub.is_empty() {
                parts.push(sub);
            }
        }
    }
    parts
}

fn split_camel_case(s: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    if chars.len() <= 1 {
        return if s.is_empty() { vec![] } else { vec![s] };
    }

    let mut parts = Vec::new();
    let mut start = 0;
    for i in 1..chars.len() {
        let prev = chars[i - 1].1;
        let curr = chars[i].1;
        let next = chars.get(i + 1).map(|c| c.1);

        let lower_to_upper = prev.is_lowercase() && curr.is_uppercase();
        let acronym_end =
            prev.is_uppercase() && curr.is_uppercase() && next.is_some_and(char::is_lowercase);
        let alpha_to_digit = prev.is_alphabetic() && curr.is_ascii_digit();
        let digit_to_alpha = prev.is_ascii_digit() && curr.is_alphabetic();

        if lower_to_upper || acronym_end || alpha_to_digit || digit_to_alpha {
            let idx = chars[i].0;
            if idx > start {
                parts.push(&s[start..idx]);
                start = idx;
            }
        }
    }
    if start < s.len() {
        parts.push(&s[start..]);
    }
    parts
}

fn normalize_error_line(line: &str) -> String {
    let s = timestamp_re().replace_all(line, "<ts>");
    let s = uuid_re().replace_all(&s, "<uuid>");
    let s = hex_addr_re().replace_all(&s, "<hex>");
    let s = path_with_loc_re().replace_all(&s, "<path>");
    let s = abs_path_re().replace_all(&s, "<path>");
    let s = number_re().replace_all(&s, "<n>");
    let collapsed: Vec<&str> = s.split_whitespace().collect();
    collapsed.join(" ").to_ascii_lowercase()
}

fn short_sha256(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(16);
    for &b in &digest[..8] {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn rustc_code_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"error\[(E\d{4})\]").unwrap())
}

fn errno_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b(EPERM|ENOENT|EACCES|EEXIST|EISDIR|ENOTDIR|ECONNREFUSED|ETIMEDOUT|EADDRINUSE|EPIPE|ENOMEM|ENOSPC)\b")
            .unwrap()
    })
}

fn timestamp_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?\b")
            .unwrap()
    })
}

fn uuid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b")
            .unwrap()
    })
}

fn hex_addr_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b0x[0-9a-fA-F]+\b").unwrap())
}

fn path_with_loc_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:[A-Za-z0-9_./-]+/)+[A-Za-z0-9_.-]+:\d+(?::\d+)?").unwrap())
}

fn abs_path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?:/[A-Za-z0-9_.-]+){2,}").unwrap())
}

fn number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b\d+\b").unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_camel_snake_and_delimiters_while_preserving_originals() {
        let terms = tokenize_code_terms("lore_context::HTTPServer/foo-bar.bazQux");
        assert!(terms.contains(&"lore_context::httpserver/foo-bar.bazqux".to_string()));
        assert!(terms.contains(&"lore_context".to_string()));
        assert!(terms.contains(&"lore".to_string()));
        assert!(terms.contains(&"context".to_string()));
        assert!(terms.contains(&"http".to_string()));
        assert!(terms.contains(&"server".to_string()));
        assert!(terms.contains(&"foo".to_string()));
        assert!(terms.contains(&"bar".to_string()));
        assert!(terms.contains(&"baz".to_string()));
        assert!(terms.contains(&"qux".to_string()));
    }

    #[test]
    fn normalizes_error_signatures_deterministically() {
        let err1 = "2026-11-21T10:00:01Z error[E0382]: borrow of moved value at /tmp/run-1/src/main.rs:42:13";
        let err2 = "2026-11-21T11:30:55Z error[E0382]: borrow of moved value at /Users/dev/proj/src/main.rs:99:7";

        let s1 = normalize_error_signature(err1);
        let s2 = normalize_error_signature(err2);
        assert!(s1.contains(&"err:rustc:e0382".to_string()));
        assert_eq!(s1, s2);
    }

    #[test]
    fn extracts_errno_signatures() {
        let s = normalize_error_signature("fatal: failed to open .git/HEAD: EPERM (Permission denied)");
        assert!(s.contains(&"err:errno:eperm".to_string()));
        assert!(s.iter().any(|x| x.starts_with("err:sig:")));
    }
}
