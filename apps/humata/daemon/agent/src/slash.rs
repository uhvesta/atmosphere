//! A message that runs a skill: `/name arguments`.
//!
//! Claude Code takes such a message as it is, and Copilot is asked to run
//! the command. A local model has no skills of its own, so the skill's file
//! is found in the folders the session works in and written out with the
//! arguments in place, as Claude Code writes one out: `$ARGUMENTS` is all of
//! them, `$ARGUMENTS[N]` and `$N` the Nth, counted from 0, and a skill that
//! names none gets them on a line at its end.

use std::path::{Path, PathBuf};

/// A SKILL.md larger than this isn't written out for a model.
const LARGEST: u64 = 1 << 20;

/// Where skills are kept in a repository, as the / list finds them.
const FOLDERS: [&str; 2] = [".claude/skills", ".github/skills"];

/// The name and what follows it when `text` is `/name` or `/name
/// arguments`; None for any other message, a path such as `/usr/bin`
/// included.
pub(crate) fn command(text: &str) -> Option<(&str, &str)> {
    let rest = text.trim().strip_prefix('/')?;
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let (name, arguments) = rest.split_at(end);
    let named = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'));
    named.then_some((name, arguments.trim()))
}

/// The SKILL.md of the skill `name` in the first of `roots` that has one:
/// by the name its front matter gives, else its folder's.
pub(crate) fn find(roots: &[PathBuf], name: &str) -> Option<PathBuf> {
    for root in roots {
        for folder in FOLDERS {
            let Ok(entries) = std::fs::read_dir(root.join(folder)) else {
                continue;
            };
            let mut skills: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
            skills.sort();
            for skill in skills {
                let file = skill.join("SKILL.md");
                let small = std::fs::metadata(&file)
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= LARGEST);
                if !small {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let named = front_matter_name(&text).or_else(|| {
                    skill
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                });
                if named.as_deref() == Some(name) {
                    return Some(file);
                }
            }
        }
    }
    None
}

/// The skill in `file` written out for `arguments`; None if it can't be
/// read, as when it was deleted since it was listed.
pub(crate) fn read(file: &Path, arguments: &str) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    Some(expand(body(&text), arguments))
}

/// What the model is given in answer to `/name arguments`: that the person
/// ran it, and the skill's text to follow.
pub(crate) fn told(name: &str, arguments: &str, skill: &str) -> String {
    let ran = if arguments.is_empty() {
        format!("/{name}")
    } else {
        format!("/{name} {arguments}")
    };
    format!(
        "The person ran {ran}. That is the skill below: do what it says now.\n\n{}",
        skill.trim()
    )
}

/// `text` without its front matter.
fn body(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---") else {
        return text;
    };
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        // The first line is what's left of the opening one.
        if at > 0 && line.trim() == "---" {
            return rest[at + line.len()..].trim_start_matches(['\r', '\n']);
        }
        at += line.len();
    }
    text
}

/// The `name` in `text`'s front matter, if it gives one.
fn front_matter_name(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let Some(value) = line.strip_prefix("name:") else {
            continue;
        };
        let value = value.trim();
        let value = ['"', '\'']
            .iter()
            .find_map(|&quote| value.strip_prefix(quote)?.strip_suffix(quote))
            .unwrap_or(value);
        return (!value.is_empty()).then(|| value.to_owned());
    }
    None
}

/// `skill` with `arguments` put in for its placeholders, or on a line at
/// its end when it has none.
pub(crate) fn expand(skill: &str, arguments: &str) -> String {
    let each = split(arguments);
    let mut expanded = String::with_capacity(skill.len() + arguments.len());
    let mut placed = false;
    let mut rest = skill;
    while let Some(at) = rest.find('$') {
        expanded.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if let Some(indexed) = after.strip_prefix("ARGUMENTS[")
            && let Some(close) = indexed.find(']')
            && !indexed[..close].is_empty()
            && indexed[..close].bytes().all(|byte| byte.is_ascii_digit())
        {
            expanded.push_str(nth(&each, &indexed[..close]));
            rest = &indexed[close + 1..];
            placed = true;
        } else if let Some(all) = after.strip_prefix("ARGUMENTS") {
            expanded.push_str(arguments);
            rest = all;
            placed = true;
        } else if digits > 0 {
            expanded.push_str(nth(&each, &after[..digits]));
            rest = &after[digits..];
            placed = true;
        } else {
            expanded.push('$');
            rest = after;
        }
    }
    expanded.push_str(rest);
    if !placed && !arguments.is_empty() {
        expanded.truncate(expanded.trim_end().len());
        expanded.push_str("\n\nARGUMENTS: ");
        expanded.push_str(arguments);
    }
    expanded
}

/// The argument `index` names, counted from 0; nothing for one not given.
fn nth<'a>(each: &'a [String], index: &str) -> &'a str {
    index
        .parse::<usize>()
        .ok()
        .and_then(|index| each.get(index))
        .map_or("", String::as_str)
}

/// `arguments` apart at spaces, a quoted one kept whole without its quotes.
fn split(arguments: &str) -> Vec<String> {
    let mut each = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    for c in arguments.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    each.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        each.push(current);
    }
    each
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_a_slash_a_name_and_what_follows() {
        assert_eq!(command("/release"), Some(("release", "")));
        assert_eq!(
            command("  /release 1.2 now\nplease "),
            Some(("release", "1.2 now\nplease"))
        );
        assert_eq!(command("/plugin:lint src"), Some(("plugin:lint", "src")));
    }

    #[test]
    fn a_path_a_bare_slash_and_plain_text_are_no_command() {
        assert_eq!(command("/usr/bin/env is missing"), None);
        assert_eq!(command("/"), None);
        assert_eq!(command("/ release"), None);
        assert_eq!(command("run /release"), None);
        assert_eq!(command(""), None);
    }

    #[test]
    fn arguments_go_where_the_skill_names_them() {
        assert_eq!(
            expand("Release $ARGUMENTS today.", "1.2 beta"),
            "Release 1.2 beta today."
        );
        assert_eq!(
            expand(
                "From $0 to $1, then $ARGUMENTS[1] again; $2.",
                "main \"my branch\""
            ),
            "From main to my branch, then my branch again; ."
        );
    }

    #[test]
    fn a_skill_that_names_no_arguments_gets_them_at_its_end_and_none_adds_nothing() {
        let with = expand("Cut a release.\n", "1.2");
        assert!(with.starts_with("Cut a release."));
        assert!(with.ends_with("1.2"));
        assert_eq!(expand("Cut a release.\n", ""), "Cut a release.\n");
    }

    #[test]
    fn a_dollar_that_is_no_placeholder_stays() {
        assert_eq!(
            expand("Costs $ or $HOME, $ARGUMENTS", "x"),
            "Costs $ or $HOME, x"
        );
    }

    fn skill(root: &Path, folder: &str, text: &str) -> PathBuf {
        let folder = root.join(folder);
        std::fs::create_dir_all(&folder).unwrap_or_else(|error| panic!("{error}"));
        let file = folder.join("SKILL.md");
        std::fs::write(&file, text).unwrap_or_else(|error| panic!("{error}"));
        file
    }

    #[test]
    fn a_skill_is_found_by_its_front_matters_name_or_its_folders() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let named = skill(
            root.path(),
            ".claude/skills/folder",
            "---\nname: release\ndescription: Cut one\n---\nCut $ARGUMENTS.\n",
        );
        let unnamed = skill(root.path(), ".github/skills/lint", "Lint it.\n");
        let roots = [root.path().to_path_buf()];

        assert_eq!(find(&roots, "release"), Some(named.clone()));
        assert_eq!(find(&roots, "lint"), Some(unnamed));
        assert_eq!(find(&roots, "folder"), None);
        assert_eq!(find(&roots, "missing"), None);
        assert_eq!(read(&named, "1.2").as_deref(), Some("Cut 1.2.\n"));
    }

    #[test]
    fn the_first_folder_with_the_skill_wins() {
        let one = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let other = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        skill(one.path(), ".claude/skills/other", "Other.\n");
        let theirs = skill(other.path(), ".claude/skills/release", "Theirs.\n");
        let roots = [one.path().to_path_buf(), other.path().to_path_buf()];

        assert_eq!(find(&roots, "release"), Some(theirs));
    }
}
