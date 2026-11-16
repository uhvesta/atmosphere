//! Replacing host variable values with their names (LD-21).

use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// Replaces a machine's host variable values with `‹NAME›` before anything
/// is mirrored. It holds the values, so it has no `Debug` on purpose.
#[derive(Clone, Default)]
pub struct Scrubber {
    /// (name, value), longest value first.
    vars: Vec<(String, String)>,
}

impl Scrubber {
    /// `vars` are the (name, value) pairs of the machine's host variables.
    /// Variables with an empty value are ignored.
    pub fn new(vars: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut vars: Vec<(String, String)> = vars
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .collect();
        vars.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
        Scrubber { vars }
    }

    /// `text` with every exact occurrence of a value replaced by `‹NAME›`,
    /// its variable's name. Longer values go first, so a value that holds
    /// another one is replaced whole, never in part.
    pub fn scrub(&self, text: &str) -> String {
        self.scrub_noting(text, &mut BTreeSet::new())
    }

    /// One line of a JSON conversation file with every value replaced, as
    /// one compact JSON line (no newline), and the names of the variables
    /// whose values were found. Every string, every object key and every
    /// number whose text holds a value is scrubbed (a number that does
    /// becomes the scrubbed string), after the line is parsed, so a value
    /// JSON escapes (a quote, a backslash, a control character) is found
    /// too. `None` when the line isn't JSON: it is never passed on unscrubbed.
    pub fn scrub_json_line(&self, line: &str) -> Option<(String, Vec<String>)> {
        let (value, names) = self.scrub_json(line)?;
        let text = serde_json::to_string(&value).ok()?;
        Some((text, names.into_iter().collect()))
    }

    /// The parsed, scrubbed line and the names that hit.
    pub(crate) fn scrub_json(&self, line: &str) -> Option<(Value, BTreeSet<String>)> {
        let value: Value = serde_json::from_str(line).ok()?;
        let mut names = BTreeSet::new();
        let value = self.scrub_value(value, &mut names);
        Some((value, names))
    }

    fn scrub_value(&self, value: Value, names: &mut BTreeSet<String>) -> Value {
        match value {
            Value::String(text) => Value::String(self.scrub_noting(&text, names)),
            Value::Number(number) => {
                let text = number.to_string();
                let scrubbed = self.scrub_noting(&text, names);
                if scrubbed == text {
                    Value::Number(number)
                } else {
                    Value::String(scrubbed)
                }
            }
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| self.scrub_value(item, names))
                    .collect(),
            ),
            Value::Object(fields) => {
                let mut scrubbed = Map::new();
                for (key, field) in fields {
                    let key = self.scrub_noting(&key, names);
                    scrubbed.insert(key, self.scrub_value(field, names));
                }
                Value::Object(scrubbed)
            }
            other => other,
        }
    }

    fn scrub_noting(&self, text: &str, names: &mut BTreeSet<String>) -> String {
        if self.vars.is_empty() {
            return text.to_owned();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(next) = rest.chars().next() {
            match self
                .vars
                .iter()
                .find(|(_, value)| rest.starts_with(value.as_str()))
            {
                Some((name, value)) => {
                    names.insert(name.clone());
                    out.push('‹');
                    out.push_str(name);
                    out.push('›');
                    rest = &rest[value.len()..];
                }
                None => {
                    out.push(next);
                    rest = &rest[next.len_utf8()..];
                }
            }
        }
        out
    }
}
