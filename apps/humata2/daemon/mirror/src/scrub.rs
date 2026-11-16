//! Replacing host variable values with their names (LD-21).

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
