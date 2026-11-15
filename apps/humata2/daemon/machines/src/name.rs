//! Names: machines, provider tables, params and host variables.

use std::fmt;

use crate::error::NameError;

/// The longest machine name, so it fits a DNS label and a hostname.
const MAX_LEN: usize = 63;

/// The variables every command gets from the Mac, so tools can find their
/// own config and binaries. Nothing else crosses but `HUMATA_*` and the host
/// variables a provider lists by name (MP-4).
pub(crate) const BASELINE: [&str; 9] = [
    "HOME",
    "PATH",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "SSH_AUTH_SOCK",
];

/// A machine's name: lowercase letters, digits and `-`, starting and ending
/// with a letter or digit, at most 63 characters. Every such name is a
/// single shell word, so `{name}` never needs quotes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MachineName(String);

impl MachineName {
    pub fn new(value: &str) -> Result<Self, NameError> {
        if value.is_empty() {
            return Err(NameError::Empty);
        }
        let len = value.chars().count();
        if len > MAX_LEN {
            return Err(NameError::TooLong {
                value: value.to_owned(),
                len,
            });
        }
        if let Some(character) = value
            .chars()
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'))
        {
            return Err(NameError::BadCharacter {
                value: value.to_owned(),
                character,
            });
        }
        if value.starts_with('-') || value.ends_with('-') {
            return Err(NameError::BadEdge {
                value: value.to_owned(),
            });
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MachineName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `[a-z][a-z0-9-]*`, at most 63: a provider's table name.
pub(crate) fn is_provider_name(value: &str) -> bool {
    let mut chars = value.chars();
    value.len() <= MAX_LEN
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// `[a-z][a-z0-9_]*`, at most 32: a param, exported as `HUMATA_<PARAM>`.
pub(crate) fn is_param_name(value: &str) -> bool {
    let mut chars = value.chars();
    value.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// `[A-Z_][A-Z0-9_]*`: a host variable a provider may receive.
pub(crate) fn is_env_name(value: &str) -> bool {
    let mut chars = value.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Names Humata sets itself, which a listed host variable can't replace.
pub(crate) fn is_reserved_env_name(value: &str) -> bool {
    value.starts_with("HUMATA_") || BASELINE.contains(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_names_are_single_lowercase_words() {
        assert!(MachineName::new("ci-box-1").is_ok());
        assert!(MachineName::new("a").is_ok());
        assert!(matches!(MachineName::new(""), Err(NameError::Empty)));
        assert!(matches!(
            MachineName::new("Box"),
            Err(NameError::BadCharacter { character: 'B', .. })
        ));
        assert!(matches!(
            MachineName::new("a b"),
            Err(NameError::BadCharacter { character: ' ', .. })
        ));
        assert!(matches!(
            MachineName::new("box;rm"),
            Err(NameError::BadCharacter { character: ';', .. })
        ));
        assert!(matches!(
            MachineName::new("-box"),
            Err(NameError::BadEdge { .. })
        ));
        assert!(matches!(
            MachineName::new(&"a".repeat(64)),
            Err(NameError::TooLong { len: 64, .. })
        ));
    }

    #[test]
    fn listed_variables_cannot_shadow_what_humata_sets() {
        assert!(is_env_name("HCLOUD_TOKEN"));
        assert!(!is_env_name("hcloud_token"));
        assert!(!is_env_name("1TOKEN"));
        assert!(is_reserved_env_name("HUMATA_NAME"));
        assert!(is_reserved_env_name("PATH"));
        assert!(!is_reserved_env_name("HCLOUD_TOKEN"));
    }
}
