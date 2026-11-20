//! Replacing host variable values with their names (LD-21).

use std::borrow::Cow;
use std::collections::BTreeSet;

use serde_json::{Map, Value};

/// Replaces a machine's host variable values with `‹NAME›` before anything
/// is mirrored. It holds the values, so it has no `Debug` on purpose.
#[derive(Clone)]
pub struct Scrubber {
    /// (name, value), longest value first.
    vars: Vec<(String, String)>,
    /// Indices into `vars` grouped by the first byte of `value`, preserving
    /// longest-first order within each bucket. Empty when `vars` is empty.
    by_first: Option<Box<[Vec<usize>; 256]>>,
}

impl Default for Scrubber {
    fn default() -> Self {
        Self {
            vars: Vec::new(),
            by_first: None,
        }
    }
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
        let by_first = if vars.is_empty() {
            None
        } else {
            let mut buckets: Box<[Vec<usize>; 256]> = Box::new(std::array::from_fn(|_| Vec::new()));
            for (idx, (_, value)) in vars.iter().enumerate() {
                buckets[usize::from(value.as_bytes()[0])].push(idx);
            }
            Some(buckets)
        };
        Scrubber { vars, by_first }
    }

    /// `text` with every exact occurrence of a value replaced by `‹NAME›`,
    /// its variable's name. Longer values go first, so a value that holds
    /// another one is replaced whole, never in part.
    pub fn scrub(&self, text: &str) -> String {
        self.scrub_noting_cow(text, &mut BTreeSet::new()).into_owned()
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
        if self.by_first.is_none() {
            return value;
        }
        match value {
            Value::String(text) => match self.scrub_noting_cow(&text, names) {
                Cow::Borrowed(_) => Value::String(text),
                Cow::Owned(scrubbed) => Value::String(scrubbed),
            },
            Value::Number(number) => {
                let text = number.to_string();
                match self.scrub_noting_cow(&text, names) {
                    Cow::Borrowed(_) => Value::Number(number),
                    Cow::Owned(scrubbed) => Value::String(scrubbed),
                }
            }
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .map(|item| self.scrub_value(item, names))
                    .collect(),
            ),
            Value::Object(fields) => {
                let mut scrubbed = Map::with_capacity(fields.len());
                for (key, field) in fields {
                    let key = match self.scrub_noting_cow(&key, names) {
                        Cow::Borrowed(_) => key,
                        Cow::Owned(k) => k,
                    };
                    scrubbed.insert(key, self.scrub_value(field, names));
                }
                Value::Object(scrubbed)
            }
            other => other,
        }
    }

    fn scrub_noting_cow<'a>(&self, text: &'a str, names: &mut BTreeSet<String>) -> Cow<'a, str> {
        let Some(buckets) = self.by_first.as_deref() else {
            return Cow::Borrowed(text);
        };
        let bytes = text.as_bytes();
        let mut out: Option<String> = None;
        let mut last_copy = 0;
        let mut i = 0;
        while i < bytes.len() {
            let bucket = &buckets[usize::from(bytes[i])];
            if bucket.is_empty() {
                i += 1;
                continue;
            }
            // Valid UTF-8 values never start with a continuation byte (0x80..=0xBF),
            // so `i` is guaranteed to be a UTF-8 char boundary whenever `bucket` is non-empty.
            let rest = &bytes[i..];
            let matched = bucket
                .iter()
                .map(|&idx| &self.vars[idx])
                .find(|(_, value)| rest.starts_with(value.as_bytes()));
            if let Some((name, value)) = matched {
                let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
                buf.push_str(&text[last_copy..i]);
                names.insert(name.clone());
                buf.push('‹');
                buf.push_str(name);
                buf.push('›');
                i += value.len();
                last_copy = i;
            } else {
                i += 1;
            }
        }
        match out {
            None => Cow::Borrowed(text),
            Some(mut buf) => {
                buf.push_str(&text[last_copy..]);
                Cow::Owned(buf)
            }
        }
    }
}
