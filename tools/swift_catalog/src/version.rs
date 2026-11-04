//! Dotted release numbers such as `6.2.1` or `15.4`, ordered numerically.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A release number, so `6.2 < 6.2.1 < 6.10`. The catalogs are keyed by it,
/// which keeps them in version order. Parsing and ordering come from the
/// `versions` crate; this wrapper only (de)serializes it as a plain string,
/// so it can be a JSON object key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version(versions::Version);

impl Version {
    /// `None` unless `text` is a version that starts with a number.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let version = versions::Version::new(text)?;
        version.nth(0)?;
        Some(Version(version))
    }

    pub(crate) fn major(&self) -> u32 {
        self.0
            .nth(0)
            .expect("parse checked the first part is a number")
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Version::parse(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("{text:?} is not a version number")))
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
    fn round_trips_as_written() {
        assert_eq!(v("6.2.1").to_string(), "6.2.1");
        assert_eq!(v("15.4").to_string(), "15.4");
    }

    #[test]
    fn rejects_non_versions() {
        assert_eq!(Version::parse("main"), None);
        assert_eq!(Version::parse(""), None);
    }
}
