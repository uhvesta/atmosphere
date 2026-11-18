//! Commit messages: subject, body and the trailer paragraph.

/// A commit message split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Message {
    /// The first line, trimmed.
    pub(crate) subject: String,
    /// Everything after the subject paragraph, without the trailer paragraph,
    /// trimmed. Empty when there is none.
    pub(crate) body: String,
    /// The trailers of the last paragraph, in order.
    pub(crate) trailers: Vec<Trailer>,
}

/// One `Key: value` line of the trailer paragraph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Trailer {
    /// The key, lowercased (`Topic` and `topic` are the same).
    pub(crate) key: String,
    /// The value, trimmed. Never empty: a key with no value is dropped.
    pub(crate) value: String,
}

impl Message {
    /// Splits a raw commit message (as `git log --format=%B` prints it).
    ///
    /// The trailer paragraph is the last paragraph (paragraphs are separated
    /// by blank lines) when it is not the subject paragraph and every one of
    /// its lines looks like `Key: value`, where Key starts with a letter and
    /// has only letters, digits and `-`. Otherwise the message has no
    /// trailers and that paragraph stays in the body.
    pub(crate) fn parse(raw: &str) -> Message {
        let normalised = raw.replace("\r\n", "\n");
        let mut paragraphs = split_paragraphs(normalised.trim()).into_iter();
        let Some(first) = paragraphs.next() else {
            return Message {
                subject: String::new(),
                body: String::new(),
                trailers: Vec::new(),
            };
        };

        let mut first_lines = first.into_iter();
        let subject = first_lines.next().unwrap_or_default().trim().to_string();
        let mut body_paragraphs: Vec<Vec<&str>> = Vec::new();
        let subject_rest: Vec<&str> = first_lines.collect();
        if !subject_rest.is_empty() {
            body_paragraphs.push(subject_rest);
        }

        let mut rest: Vec<Vec<&str>> = paragraphs.collect();
        let parsed = rest.last().and_then(|last| {
            last.iter()
                .copied()
                .map(parse_trailer_line)
                .collect::<Option<Vec<_>>>()
        });
        let trailers: Vec<Trailer> = match parsed {
            Some(pairs) => {
                rest.pop();
                pairs
                    .into_iter()
                    .filter(|(_, value)| !value.is_empty())
                    .map(|(key, value)| Trailer { key, value })
                    .collect()
            }
            None => Vec::new(),
        };
        body_paragraphs.extend(rest);

        let body = body_paragraphs
            .iter()
            .map(|paragraph| paragraph.join("\n"))
            .collect::<Vec<_>>()
            .join("\n\n");
        Message {
            subject,
            body: body.trim().to_string(),
            trailers,
        }
    }

    /// The values of every trailer with this key (lowercase), in order.
    pub(crate) fn values<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.trailers
            .iter()
            .filter(move |trailer| trailer.key == key)
            .map(|trailer| trailer.value.as_str())
    }
}

/// Splits text into paragraphs: runs of non-blank lines, separated by lines
/// that are blank after trimming.
fn split_paragraphs(text: &str) -> Vec<Vec<&str>> {
    let mut paragraphs: Vec<Vec<&str>> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current);
    }
    paragraphs
}

/// Reads one `Key: value` line. The key must start with an ASCII letter and
/// hold only ASCII letters, digits and `-`, with no space before the colon.
/// Returns the lowercased key and the trimmed value, or None when the line is
/// not a trailer.
fn parse_trailer_line(line: &str) -> Option<(String, String)> {
    let (key, rest) = line.split_once(':')?;
    let starts_with_letter = key.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
    let key_is_valid =
        starts_with_letter && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    key_is_valid.then(|| (key.to_ascii_lowercase(), rest.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trailer(key: &str, value: &str) -> Trailer {
        Trailer {
            key: key.to_string(),
            value: value.to_string(),
        }
    }

    #[test]
    fn subject_only_has_no_body_or_trailers() {
        let message = Message::parse("Add the parser\n");
        assert_eq!(message.subject, "Add the parser");
        assert_eq!(message.body, "");
        assert!(message.trailers.is_empty());
    }

    #[test]
    fn a_single_paragraph_is_the_subject_even_when_it_looks_like_a_trailer() {
        let message = Message::parse("Topic: parser");
        assert_eq!(message.subject, "Topic: parser");
        assert_eq!(message.body, "");
        assert!(message.trailers.is_empty());
    }

    #[test]
    fn body_and_trailers_are_split_from_the_last_paragraph() {
        let message = Message::parse(
            "Add the parser\n\nThe old one panicked on empty input.\n\nTopic: parser\nRelative: lexer",
        );
        assert_eq!(message.subject, "Add the parser");
        assert_eq!(message.body, "The old one panicked on empty input.");
        assert_eq!(
            message.trailers,
            vec![trailer("topic", "parser"), trailer("relative", "lexer")]
        );
    }

    #[test]
    fn a_markdown_last_paragraph_stays_in_the_body() {
        let message = Message::parse("Add the parser\n\n## Testing\nbazel test //...");
        assert_eq!(message.subject, "Add the parser");
        assert_eq!(message.body, "## Testing\nbazel test //...");
        assert!(message.trailers.is_empty());
    }

    #[test]
    fn keys_are_lowercased_and_values_keep_their_case() {
        let message = Message::parse("Add the parser\n\nTOPIC: Parser\nRelative: Lexer");
        assert_eq!(
            message.trailers,
            vec![trailer("topic", "Parser"), trailer("relative", "Lexer")]
        );
    }

    #[test]
    fn trailers_with_empty_values_are_dropped() {
        let message = Message::parse("Add the parser\n\nTopic: parser\nLabels:\nReviewers:   ");
        assert_eq!(message.trailers, vec![trailer("topic", "parser")]);
    }

    #[test]
    fn crlf_line_endings_parse_like_lf() {
        let message =
            Message::parse("Add the parser\r\n\r\nThe body.\r\n\r\nTopic: parser\r\n");
        assert_eq!(message.subject, "Add the parser");
        assert_eq!(message.body, "The body.");
        assert_eq!(message.trailers, vec![trailer("topic", "parser")]);
    }

    #[test]
    fn a_two_line_subject_paragraph_continues_into_the_body() {
        let message =
            Message::parse("Fix the parser on\nlong input\n\nThe body.\n\nTopic: parser");
        assert_eq!(message.subject, "Fix the parser on");
        assert_eq!(message.body, "long input\n\nThe body.");
        assert_eq!(message.trailers, vec![trailer("topic", "parser")]);
    }

    #[test]
    fn empty_and_blank_input_has_no_parts() {
        for raw in ["", "  \n\t\n  "] {
            let message = Message::parse(raw);
            assert_eq!(message.subject, "");
            assert_eq!(message.body, "");
            assert!(message.trailers.is_empty());
        }
    }
}
