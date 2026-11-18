//! Pull request descriptions: the PR template (stack.toml's `template`, else
//! the built-in one), filled from a topic's first commit, then the stack.

use std::path::PathBuf;

use sha2::{Digest as _, Sha256};

use crate::config::{Config, ConfigError};
use crate::history::{self, StackLine};
use crate::plan::Topic;
use crate::trailers::Message;

/// The template used when stack.toml names none.
pub(crate) const BUILTIN_TEMPLATE: &str = include_str!("default_template.md");

/// The marker that ends every description stack writes, with a hash of the
/// text above it: `<!-- stack:body sha=<16 hex> -->`.
const BODY_MARKER_PREFIX: &str = "<!-- stack:body sha=";

/// A parsed PR template: its `## ` sections in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Template {
    /// None when it is the built-in template.
    pub(crate) path: Option<PathBuf>,
    pub(crate) text: String,
    pub(crate) sections: Vec<Section>,
}

/// One `## Name` heading of the template. It is required when the lines
/// under it (before the next heading) contain `<!-- required -->`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    pub(crate) name: String,
    pub(crate) required: bool,
}

impl Template {
    /// The file stack.toml's `template` names, in the config folder, else
    /// the built-in template. A named file that can't be read is an error.
    pub(crate) fn load(config: &Config) -> Result<Template, ConfigError> {
        match config.template_path() {
            Some(path) => {
                let text = std::fs::read_to_string(&path).map_err(|source| ConfigError::Template {
                    path: path.clone(),
                    source,
                })?;
                Ok(Template::parse(&text, Some(path)))
            }
            None => Ok(Template::parse(BUILTIN_TEMPLATE, None)),
        }
    }

    /// Reads the `## ` headings (exactly two `#` then a space) of `text`.
    pub(crate) fn parse(text: &str, path: Option<PathBuf>) -> Template {
        let (_, headings) = split_sections(text);
        let sections = headings
            .into_iter()
            .map(|(name, section_text)| Section {
                required: section_text.contains("<!-- required -->"),
                name,
            })
            .collect();
        Template {
            path,
            text: text.to_owned(),
            sections,
        }
    }

    /// The required sections the message's body leaves out or leaves empty.
    /// A body's sections are its `## ` headings; text before the first
    /// heading is the first template section (Summary). Names compare
    /// case-insensitively.
    pub(crate) fn missing_required(&self, message: &Message) -> Vec<String> {
        let (filled, _) = self.filled(&message.body);
        self.sections
            .iter()
            .filter(|section| section.required)
            .filter(|section| !filled.iter().any(|(name, _)| *name == section.name))
            .map(|section| section.name.clone())
            .collect()
    }

    /// The (name, text) pairs the body fills, keyed by template section name
    /// in template order (empty texts left out), and the body's own sections
    /// that the template doesn't have, in body order.
    fn filled(&self, body: &str) -> (Vec<(String, String)>, Vec<(String, String)>) {
        let (lead, sections) = split_sections(body);
        let mut slots: Vec<(String, String)> = self
            .sections
            .iter()
            .map(|section| (section.name.clone(), String::new()))
            .collect();
        if let Some((_, text)) = slots.first_mut() {
            join_into(text, &lead);
        }
        let mut extra = Vec::new();
        for (name, text) in sections {
            if let Some((_, slot_text)) = slots
                .iter_mut()
                .find(|(slot, _)| same_name(slot, &name))
            {
                join_into(slot_text, &text);
            } else if !text.is_empty() {
                extra.push((name, text));
            }
        }
        let filled: Vec<(String, String)> = slots
            .into_iter()
            .filter(|(_, text)| !text.is_empty())
            .collect();
        (filled, extra)
    }

    /// The description for a topic, from its first commit's body:
    /// - each template section the body fills, in template order, as
    ///   `## Name` then its text; text before the body's first heading goes
    ///   in the first template section; sections the body leaves empty are
    ///   left out;
    /// - then the body's own sections the template doesn't have, in body order;
    /// - then, when the topic has more than one commit, `## Commits` with one
    ///   `- <subject>` line per commit;
    /// - then, when the stack has more than one pull request, `## Stack`
    ///   with the tree from the trunk, this pull request marked;
    /// - then a blank line and `<!-- stack:body sha=<hash> -->`, where hash is
    ///   the first 16 hex digits of the SHA-256 of everything above it with
    ///   `\r` removed and surrounding whitespace trimmed.
    pub(crate) fn render(&self, topic: &Topic, trunk: &str, stack: &[StackLine]) -> String {
        let body = topic
            .commits
            .first()
            .map_or("", |commit| commit.message.body.as_str());
        let (filled, extra) = self.filled(body);
        let mut parts: Vec<String> = filled
            .iter()
            .chain(extra.iter())
            .map(|(name, text)| format!("## {name}\n\n{text}"))
            .collect();
        if topic.commits.len() > 1 {
            let subjects: Vec<String> = topic
                .commits
                .iter()
                .map(|commit| format!("- {}", commit.message.subject))
                .collect();
            parts.push(format!("## Commits\n\n{}", subjects.join("\n")));
        }
        if stack.len() > 1 {
            let lines: Vec<String> = stack.iter().map(history::stack_line).collect();
            parts.push(format!("## Stack\n\nInto `{trunk}`:\n\n{}", lines.join("\n")));
        }
        let above = parts.join("\n\n");
        if above.is_empty() {
            return format!("{}\n", marker(&above));
        }
        format!("{above}\n\n{}\n", marker(&above))
    }
}

/// Whether a live description was changed since stack wrote it: true when it
/// has no body marker, or the marker's hash doesn't match the text above it
/// (hashed the same way as `render`, so GitHub's `\r\n` doesn't count).
pub(crate) fn edited_since_upload(live: &str) -> bool {
    let cleaned = live.replace('\r', "");
    let lines: Vec<&str> = cleaned.lines().collect();
    let found = lines
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, line)| marker_hash(line).map(|hash| (index, hash)));
    let Some((index, stored)) = found else {
        return true;
    };
    let above = lines
        .iter()
        .take(index)
        .copied()
        .collect::<Vec<&str>>()
        .join("\n");
    hash_of(&above) != stored
}

/// The body marker line that ends a description, for the text above it.
fn marker(above: &str) -> String {
    format!("{BODY_MARKER_PREFIX}{} -->", hash_of(above))
}

/// The hash inside a body marker line, when the line is one.
fn marker_hash(line: &str) -> Option<&str> {
    line.strip_prefix(BODY_MARKER_PREFIX)?.strip_suffix(" -->")
}

/// The first 16 lowercase hex digits of the SHA-256 of the normalised text.
fn hash_of(text: &str) -> String {
    let digest = Sha256::digest(normalise(text).as_bytes());
    digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The text as it is hashed: without `\r`, and without surrounding whitespace.
fn normalise(text: &str) -> String {
    text.replace('\r', "").trim().to_owned()
}

/// Splits `text` into its lead (the text before the first `## ` heading) and
/// the `(heading name, section text)` pairs in order. Both are trimmed.
fn split_sections(text: &str) -> (String, Vec<(String, String)>) {
    let mut lead: Vec<&str> = Vec::new();
    let mut sections: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("## ") {
            sections.push((name.trim().to_owned(), Vec::new()));
        } else if let Some((_, lines)) = sections.last_mut() {
            lines.push(line);
        } else {
            lead.push(line);
        }
    }
    let sections: Vec<(String, String)> = sections
        .into_iter()
        .map(|(name, lines)| (name, lines.join("\n").trim().to_owned()))
        .collect();
    (lead.join("\n").trim().to_owned(), sections)
}

/// Section names compare case-insensitively.
fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Appends `text` to `slot`, after a blank line when the slot has text.
/// Empty text changes nothing.
fn join_into(slot: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !slot.is_empty() {
        slot.push_str("\n\n");
    }
    slot.push_str(text);
}

#[cfg(test)]
mod tests {
    use super::{
        BODY_MARKER_PREFIX, BUILTIN_TEMPLATE, Section, Template, edited_since_upload,
    };
    use crate::config::Config;
    use crate::history::StackLine;
    use crate::plan::{Commit, Topic};
    use crate::trailers::{Message, Trailer};

    const TEMPLATE_TEXT: &str =
        "## Summary\n<!-- required -->\n\n## Why\n<!-- required -->\n\n## Testing\n";

    fn message(subject: &str, body: &str) -> Message {
        Message {
            subject: subject.to_owned(),
            body: body.to_owned(),
            trailers: Vec::new(),
        }
    }

    fn commit(subject: &str, body: &str) -> Commit {
        Commit {
            sha: format!("{subject}-sha"),
            parent: "parent-sha".to_owned(),
            message: message(subject, body),
        }
    }

    fn topic(commits: Vec<Commit>) -> Topic {
        Topic {
            name: "topic".to_owned(),
            commits,
            relative: None,
            reviewers: Vec::new(),
            labels: Vec::new(),
            draft: false,
        }
    }

    fn section(name: &str, required: bool) -> Section {
        Section {
            name: name.to_owned(),
            required,
        }
    }

    #[test]
    fn parse_finds_the_sections_and_their_required_flags() {
        let text = "## Summary\n<!-- required -->\n### Details\n\n## Rollout\nBehind a flag.\n";
        let parsed = Template::parse(text, None);
        assert_eq!(
            parsed.sections,
            vec![section("Summary", true), section("Rollout", false)]
        );
        assert_eq!(parsed.text, text);
    }

    #[test]
    fn load_without_a_named_file_uses_the_builtin_template() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load_from(dir.path()).unwrap();
        let loaded = Template::load(&config).unwrap();
        assert_eq!(loaded.path, None);
        assert_eq!(loaded.text, BUILTIN_TEMPLATE);
    }

    #[test]
    fn load_reads_the_template_stack_toml_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("stack.toml"), "template = \"pr.md\"\n").unwrap();
        let file = dir.path().join("pr.md");
        std::fs::write(&file, "## Only\n<!-- required -->\n").unwrap();
        let config = Config::load_from(dir.path()).unwrap();
        let loaded = Template::load(&config).unwrap();
        assert_eq!(loaded.path, Some(file));
        assert_eq!(loaded.sections, vec![section("Only", true)]);
    }

    #[test]
    fn the_stack_is_listed_with_this_pull_request_marked() {
        let parsed = Template::parse(TEMPLATE_TEXT, None);
        let one = topic(vec![commit("add", "Lead.\n\n## Why\nBecause.\n")]);
        let line = |number: u64, current: bool| StackLine {
            depth: 0,
            number,
            title: format!("pr {number}"),
            current,
        };
        let alone = parsed.render(&one, "main", &[line(1, true)]);
        assert!(!alone.contains("## Stack"), "{alone}");
        let chained = parsed.render(&one, "main", &[line(1, false), line(2, true)]);
        assert!(chained.contains("## Stack"), "{chained}");
        assert!(!edited_since_upload(&chained));
        assert_ne!(alone, chained);
    }

    #[test]
    fn missing_required_counts_the_lead_and_reports_empty_and_absent_sections() {
        let parsed = Template::parse(TEMPLATE_TEXT, None);
        let empty_why = message("subject", "Lead as the summary.\n\n## Why\n");
        assert_eq!(parsed.missing_required(&empty_why), vec!["Why".to_owned()]);

        let nothing = message("subject", "");
        assert_eq!(
            parsed.missing_required(&nothing),
            vec!["Summary".to_owned(), "Why".to_owned()]
        );

        let no_summary = message("subject", "## WHY\nBecause.\n");
        assert_eq!(
            parsed.missing_required(&no_summary),
            vec!["Summary".to_owned()]
        );
    }

    #[test]
    fn render_puts_template_sections_first_in_template_order() {
        let parsed = Template::parse(TEMPLATE_TEXT, None);
        let body = "## Testing\nRan the suite.\n\n## Why\nBecause.\n\n## Summary\nIt adds a.";
        let rendered = parsed.render(&topic(vec![commit("subject", body)]), "main", &[]);
        assert!(rendered.starts_with(
            "## Summary\n\nIt adds a.\n\n## Why\n\nBecause.\n\n## Testing\n\nRan the suite.\n\n<!-- stack:body sha="
        ));
    }

    #[test]
    fn render_keeps_the_bodys_own_sections_after_the_template_ones() {
        let parsed = Template::parse("## Summary\n\n## Why\n", None);
        let body = "Lead.\n\n## Extra\nMore.\n\n## Why\nWhy text.";
        let rendered = parsed.render(&topic(vec![commit("subject", body)]), "main", &[]);
        assert!(rendered.starts_with(
            "## Summary\n\nLead.\n\n## Why\n\nWhy text.\n\n## Extra\n\nMore.\n\n<!-- stack:body sha="
        ));
    }

    #[test]
    fn render_lists_commits_only_for_a_topic_with_several() {
        let parsed = Template::parse("## Summary\n", None);
        let single = parsed.render(&topic(vec![commit("first", "Lead.")]), "main", &[]);
        assert!(!single.contains("## Commits"));

        let several = parsed.render(&topic(vec![
            commit("first", "Lead."),
            commit("second", ""),
        ]), "main", &[]);
        assert!(several.contains("## Commits\n\n- first\n- second\n\n<!-- stack:body sha="));
    }

    #[test]
    fn render_leaves_out_empty_sections_and_trailers() {
        let parsed = Template::parse("## Summary\n\n## Risks\n\n## Testing\n", None);
        let mut first = commit("first", "Lead.\n\n## Risks\n\n## Testing\nRan it.");
        first.message.trailers.push(Trailer {
            key: "relative".to_owned(),
            value: "ticket-42".to_owned(),
        });
        let rendered = parsed.render(&topic(vec![first]), "main", &[]);
        assert!(!rendered.contains("## Risks"));
        assert!(rendered.contains("## Testing\n\nRan it."));
        assert!(!rendered.contains("ticket-42"));
    }

    #[test]
    fn render_of_a_topic_without_commits_is_only_the_marker() {
        let parsed = Template::parse(TEMPLATE_TEXT, None);
        let rendered = parsed.render(&topic(Vec::new()), "main", &[]);
        assert!(rendered.starts_with(BODY_MARKER_PREFIX));
        assert!(!edited_since_upload(&rendered));
    }

    #[test]
    fn edited_since_upload_is_false_for_a_rendered_description() {
        let parsed = Template::parse(TEMPLATE_TEXT, None);
        let rendered =
            parsed.render(&topic(vec![commit("subject", "It adds a.\n\n## Why\nBecause.")]), "main", &[]);
        assert!(!edited_since_upload(&rendered));
        assert!(!edited_since_upload(&rendered.replace('\n', "\r\n")));
    }

    #[test]
    fn edited_since_upload_is_true_after_an_edit_above_the_marker() {
        let parsed = Template::parse(TEMPLATE_TEXT, None);
        let rendered = parsed.render(&topic(vec![commit("subject", "It adds a.")]), "main", &[]);
        let edited = rendered.replace("It adds a.", "It adds b.");
        assert!(edited_since_upload(&edited));
    }

    #[test]
    fn edited_since_upload_is_true_without_a_marker() {
        assert!(edited_since_upload("## Summary\n\nWritten by hand.\n"));
    }
}
