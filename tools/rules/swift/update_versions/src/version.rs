//! Dotted release numbers such as `6.2.1` or `15.4`, ordered numerically.

use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A version as written upstream, ordered by its numeric parts, so `6.2 <
/// 6.2.1 < 6.10`. The catalogs are keyed by it, which keeps them in version
/// order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Version {
    text: String,
    parts: Vec<u32>,
}

impl Version {
    /// `None` unless `text` is dot-separated numbers.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let parts = text
            .split('.')
            .map(|part| part.parse().ok())
            .collect::<Option<Vec<u32>>>()?;
        Some(Version {
            text: text.to_owned(),
            parts,
        })
    }

    pub(crate) fn major(&self) -> u32 {
        self.parts[0]
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts
            .cmp(&other.parts)
            .then_with(|| self.text.cmp(&other.text))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Version::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is not a dotted version")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn orders_numerically() {
        assert!(v("6.2") < v("6.2.1"));
        assert!(v("6.9") < v("6.10"));
        assert!(v("15.5") < v("26.5"));
        assert_eq!(v("26.5").major(), 26);
    }

    #[test]
    fn rejects_non_numeric_versions() {
        assert_eq!(Version::parse("main"), None);
        assert_eq!(Version::parse("6.2-dev"), None);
        assert_eq!(Version::parse(""), None);
    }
}
