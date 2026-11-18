//! The default repo search scope in `config/repos.toml` (RL-7, RL-8, D474):
//! reading it, and writing it back without losing the user's comments or
//! other keys.
//!
//! Only the top-level `owners` and `include_me` keys belong to the scope.
//! Every other line, including the tables, is left as it was when the scope
//! is written.

use std::collections::HashSet;

use crate::name::is_owner;

/// Which GitHub repos a search looks in when it isn't told otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The repos `gh` lists for you as owner, collaborator or org member
    /// (CH-3). This is the scope when the file lists no owners.
    YourRepos,
    /// The listed GitHub owners (orgs or users), plus your own repos when
    /// `include_me` is on.
    Owners {
        /// The owners, in the order listed, each once (case ignored).
        owners: Vec<String>,
        /// Whether the repos of your own `gh` login are searched too.
        include_me: bool,
    },
    /// Every repo on GitHub. It widens one search only and is never saved.
    AllOfGitHub,
}

/// What went wrong reading or writing the default scope.
#[derive(Debug, thiserror::Error)]
pub enum ScopeError {
    /// The file isn't TOML, so no scope can be read from it.
    #[error(
        "config/repos.toml isn't valid TOML, so the default search scope can't be read. Fix the syntax the error points at (usually a missing ], quote or comma), then save the file again"
    )]
    Toml {
        /// What the TOML parser reported.
        #[source]
        source: toml::de::Error,
    },

    /// `owners` is there but isn't a list.
    #[error(
        "config/repos.toml: `owners` must be a list of owner names in square brackets, such as owners = [\"bazelbuild\", \"buildfarm\"], but it is {found}. Put the names inside [ ] and separate them with commas"
    )]
    OwnersNotAList {
        /// The value as the file has it.
        found: String,
    },

    /// An entry in `owners` isn't text.
    #[error(
        "config/repos.toml: each owner in `owners` must be a name in quotes, such as \"bazelbuild\", but this entry is {found}. Put the name in quotes"
    )]
    OwnerNotText {
        /// The entry as the file has it.
        found: String,
    },

    /// An owner isn't a GitHub owner name.
    #[error(
        "config/repos.toml: {owner:?} isn't a GitHub owner name. Owner names use letters, digits and '-' only, up to 39 characters, and don't start with '-'. Write the org or user name as GitHub shows it, such as owners = [\"bazelbuild\"]"
    )]
    BadOwner {
        /// The owner as written.
        owner: String,
    },

    /// `include_me` isn't true or false.
    #[error(
        "config/repos.toml: `include_me` must be true or false, without quotes, such as include_me = true, but it is {found}. Change it to true or false"
    )]
    IncludeMeNotABool {
        /// The value as the file has it.
        found: String,
    },

    /// All of GitHub was asked to be saved as the default.
    #[error(
        "All of GitHub widens one search only, so it isn't saved to config/repos.toml. Save the owners you want as the default instead, such as owners = [\"bazelbuild\"], and pick All of GitHub for the one search that needs it"
    )]
    AllOfGitHubIsPerSearch,
}

impl Scope {
    /// Reads the scope from the text of `config/repos.toml`. A file with no
    /// `owners` key gives [`Scope::YourRepos`]. With owners, `include_me`
    /// defaults to on, and a listed owner given twice (in any case) counts
    /// once, where it first appeared. Unknown keys are ignored.
    pub fn read(text: &str) -> Result<Scope, ScopeError> {
        let table: toml::Table =
            toml::from_str(text).map_err(|source| ScopeError::Toml { source })?;
        let include_me = match table.get("include_me") {
            None => true,
            Some(value) => value
                .as_bool()
                .ok_or_else(|| ScopeError::IncludeMeNotABool {
                    found: value.to_string(),
                })?,
        };
        let Some(value) = table.get("owners") else {
            return Ok(Scope::YourRepos);
        };
        let items = value
            .as_array()
            .ok_or_else(|| ScopeError::OwnersNotAList {
                found: value.to_string(),
            })?;
        let mut names = Vec::new();
        for item in items.iter() {
            let name = item
                .as_str()
                .ok_or_else(|| ScopeError::OwnerNotText {
                    found: item.to_string(),
                })?;
            if !is_owner(name) {
                return Err(ScopeError::BadOwner {
                    owner: name.to_owned(),
                });
            }
            names.push(name.to_owned());
        }
        Ok(Scope::Owners {
            owners: unique_owners(&names),
            include_me,
        })
    }

    /// Returns `text` (the current `config/repos.toml`) with this scope
    /// written into it. Comments, other keys and tables are kept as they
    /// are. The `owners` and `include_me` lines are replaced where they are,
    /// keeping a trailing comment on the line; missing ones are added before
    /// the first table. A scope with no owners takes the `owners` key out.
    /// Reading the result back gives this scope.
    ///
    /// Fails with nothing written when the scope can't be saved: all of
    /// GitHub, an owner that isn't a GitHub name, or a `text` that isn't
    /// TOML.
    pub fn write_into(&self, text: &str) -> Result<String, ScopeError> {
        let saved = match self {
            Scope::AllOfGitHub => return Err(ScopeError::AllOfGitHubIsPerSearch),
            Scope::YourRepos => None,
            Scope::Owners {
                owners,
                include_me,
            } => {
                if let Some(owner) = owners.iter().find(|owner| !is_owner(owner.as_str())) {
                    return Err(ScopeError::BadOwner {
                        owner: owner.to_string(),
                    });
                }
                Some((unique_owners(owners), *include_me))
            }
        };
        let _table: toml::Table =
            toml::from_str(text).map_err(|source| ScopeError::Toml { source })?;

        let mut body = text.to_owned();
        if !body.is_empty() && !body.ends_with('\n') {
            body.push('\n');
        }
        let lines: Vec<&str> = body.split_inclusive('\n').collect();
        let (statements, header) = top_level(&lines);
        let find = |key: &str| statements.iter().find(|statement| statement.key == key);

        let mut edits = Vec::new();
        match saved {
            None => {
                if let Some(statement) = find("owners") {
                    edits.push(Edit {
                        start: statement.first,
                        end: statement.last + 1,
                        text: render(&statement.comments, None, statement.trailing.as_deref()),
                    });
                }
            }
            Some((owners, include_me)) => {
                let insert_at = insertion_point(&lines, header);
                edits.push(put(find("owners"), &owners_line(&owners), insert_at));
                edits.push(put(
                    find("include_me"),
                    &format!("include_me = {include_me}"),
                    insert_at,
                ));
            }
        }
        Ok(apply(&lines, edits))
    }
}

/// The owners with repeats dropped, case ignored, the first kept.
fn unique_owners(names: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    names
        .iter()
        .filter(|name| seen.insert(name.to_ascii_lowercase()))
        .cloned()
        .collect()
}

/// The `owners = [...]` line for `owners`, with each name as a TOML string.
fn owners_line(owners: &[String]) -> String {
    let items: Vec<String> = owners
        .iter()
        .map(|owner| toml::Value::String(owner.clone()).to_string())
        .collect();
    format!("owners = [{}]", items.join(", "))
}

/// How one line of the file starts, for finding the top-level keys.
struct LineInfo {
    /// The line starts outside any string and outside any open `[` or `{`,
    /// so it begins a key, a table header, a comment or a blank line.
    boundary: bool,
    /// The line is a table header, `[name]` or `[[name]]`.
    header: bool,
    /// The byte offset where a comment starts on the line, if it has one.
    comment: Option<usize>,
}

/// Where a string ends, as the scanner reads the file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    Outside,
    Basic,
    Literal,
    MultiBasic,
    MultiLiteral,
}

/// Works out, for each line, whether it starts a new statement and where a
/// comment starts, following strings and brackets across lines.
fn scan_lines(lines: &[&str]) -> Vec<LineInfo> {
    let mut quote = Quote::Outside;
    let mut depth: i64 = 0;
    let mut infos = Vec::with_capacity(lines.len());
    for line in lines {
        let bytes = line.as_bytes();
        let boundary = quote == Quote::Outside && depth == 0;
        let mut comment = None;
        let mut i = 0;
        while i < bytes.len() {
            let rest = &bytes[i..];
            match quote {
                Quote::Outside => {
                    if bytes[i] == b'#' {
                        comment = Some(i);
                        break;
                    }
                    if rest.starts_with(b"\"\"\"") {
                        quote = Quote::MultiBasic;
                        i += 3;
                        continue;
                    }
                    if rest.starts_with(b"'''") {
                        quote = Quote::MultiLiteral;
                        i += 3;
                        continue;
                    }
                    match bytes[i] {
                        b'"' => quote = Quote::Basic,
                        b'\'' => quote = Quote::Literal,
                        b'[' | b'{' => depth += 1,
                        b']' | b'}' => depth -= 1,
                        _ => {}
                    }
                }
                Quote::Basic => match bytes[i] {
                    b'\\' => i += 1,
                    b'"' => quote = Quote::Outside,
                    _ => {}
                },
                Quote::Literal => {
                    if bytes[i] == b'\'' {
                        quote = Quote::Outside;
                    }
                }
                Quote::MultiBasic => {
                    if bytes[i] == b'\\' {
                        i += 1;
                    } else if rest.starts_with(b"\"\"\"") {
                        quote = Quote::Outside;
                        i += 3;
                        continue;
                    }
                }
                Quote::MultiLiteral => {
                    if rest.starts_with(b"'''") {
                        quote = Quote::Outside;
                        i += 3;
                        continue;
                    }
                }
            }
            i += 1;
        }
        // A basic or literal string can't run past its line.
        if matches!(quote, Quote::Basic | Quote::Literal) {
            quote = Quote::Outside;
        }
        infos.push(LineInfo {
            boundary,
            header: boundary && line.trim_start().starts_with('['),
            comment,
        });
    }
    infos
}

/// The key a line assigns, if it is `key = ...` (bare or in quotes).
fn key_of(line: &str) -> Option<String> {
    let rest = line.trim_start();
    let (key, after) = if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        (quoted[..end].to_owned(), &quoted[end + 1..])
    } else {
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(rest.len());
        if end == 0 {
            return None;
        }
        (rest[..end].to_owned(), &rest[end..])
    };
    after.trim_start().starts_with('=').then_some(key)
}

/// One top-level key in the file, with the lines it spans.
struct Statement {
    key: String,
    /// The first line of the key.
    first: usize,
    /// The last line of its value (the same as `first` for a one-line value).
    last: usize,
    /// Comments on the lines of the value before the last one.
    comments: Vec<String>,
    /// The comment on the last line, which stays with the key when it is
    /// rewritten.
    trailing: Option<String>,
}

/// The top-level keys before the first table, and the line of that first
/// table (or the number of lines when there is none).
fn top_level(lines: &[&str]) -> (Vec<Statement>, usize) {
    let infos = scan_lines(lines);
    let header = infos
        .iter()
        .position(|info| info.header)
        .unwrap_or(lines.len());
    let comment_at = |line: usize| {
        infos[line]
            .comment
            .map(|at| lines[line].get(at..).unwrap_or("").trim_end().to_owned())
    };
    let mut statements = Vec::new();
    let mut i = 0;
    while i < header {
        if !infos[i].boundary {
            i += 1;
            continue;
        }
        let Some(key) = key_of(lines[i]) else {
            i += 1;
            continue;
        };
        let mut last = i;
        while last + 1 < lines.len() && !infos[last + 1].boundary {
            last += 1;
        }
        statements.push(Statement {
            key,
            first: i,
            last,
            comments: (i..last).filter_map(&comment_at).collect(),
            trailing: comment_at(last),
        });
        i = last + 1;
    }
    (statements, header)
}

/// Where a new top-level key goes: just before the first table, after any
/// blank lines that come right before it.
fn insertion_point(lines: &[&str], header: usize) -> usize {
    let mut at = header;
    while at > 0 && lines.get(at - 1).is_some_and(|line| line.trim().is_empty()) {
        at -= 1;
    }
    at
}

/// A change to the file: lines `start..end` are replaced by `text`. An
/// insertion has `start == end`.
struct Edit {
    start: usize,
    end: usize,
    text: String,
}

/// The text that replaces a key: its comments above it, then the key line
/// (or nothing, when the key is taken out), then its trailing comment.
fn render(comments: &[String], body: Option<&str>, trailing: Option<&str>) -> String {
    let mut text = String::new();
    for comment in comments {
        text.push_str(comment);
        text.push('\n');
    }
    match body {
        Some(body) => {
            text.push_str(body);
            if let Some(trailing) = trailing {
                text.push(' ');
                text.push_str(trailing);
            }
            text.push('\n');
        }
        None => {
            if let Some(trailing) = trailing {
                text.push_str(trailing);
                text.push('\n');
            }
        }
    }
    text
}

/// The edit that sets a key: it replaces the key's lines when the key is
/// there, and inserts the line at `insert_at` when it isn't.
fn put(existing: Option<&Statement>, line: &str, insert_at: usize) -> Edit {
    match existing {
        Some(statement) => Edit {
            start: statement.first,
            end: statement.last + 1,
            text: render(
                &statement.comments,
                Some(line),
                statement.trailing.as_deref(),
            ),
        },
        None => Edit {
            start: insert_at,
            end: insert_at,
            text: format!("{line}\n"),
        },
    }
}

/// Applies the edits to the lines, in place order, and returns the file.
fn apply(lines: &[&str], mut edits: Vec<Edit>) -> String {
    edits.sort_by_key(|edit| (edit.start, edit.end));
    let mut out = String::new();
    let mut cursor = 0;
    for edit in edits {
        for line in lines.iter().take(edit.start).skip(cursor) {
            out.push_str(line);
        }
        out.push_str(&edit.text);
        cursor = edit.end.max(cursor);
    }
    for line in lines.iter().skip(cursor) {
        out.push_str(line);
    }
    out
}
