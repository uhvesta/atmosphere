//! Whether a run records snapshots or checks them.

use thiserror::Error;

/// The environment variable that picks the [`Mode`].
pub(crate) const VARIABLE: &str = "HUMATA2_SNAPSHOT";

/// Whether a run records snapshots or checks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Writes what each test produced as its snapshot.
    Record,
    /// Fails a test whose output differs from its snapshot. The default.
    Verify,
}

/// `HUMATA2_SNAPSHOT` holds something other than a mode.
#[derive(Debug, Error)]
pub enum ModeError {
    #[error(
        "HUMATA2_SNAPSHOT is {value:?}, which is not a snapshot mode. Set it to `verify` to \
         check snapshots (the same as leaving it unset), or to `record` to rewrite them from \
         what the tests produce"
    )]
    Unknown { value: String },
    #[error(
        "HUMATA2_SNAPSHOT is not valid UTF-8, so it can't name a snapshot mode. Set it to \
         `verify` or `record`, or unset it to verify"
    )]
    NotUnicode,
}

impl Mode {
    /// The mode `HUMATA2_SNAPSHOT` sets: [`Mode::Verify`] when it is unset
    /// or empty.
    ///
    /// # Errors
    ///
    /// [`ModeError`] when it is set to anything else.
    pub fn from_env() -> Result<Self, ModeError> {
        match std::env::var(VARIABLE) {
            Ok(value) => Self::parse(Some(&value)),
            Err(std::env::VarError::NotPresent) => Self::parse(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(ModeError::NotUnicode),
        }
    }

    fn parse(value: Option<&str>) -> Result<Self, ModeError> {
        match value.map(str::trim) {
            None | Some("") | Some("verify") => Ok(Self::Verify),
            Some("record") => Ok(Self::Record),
            Some(other) => Err(ModeError::Unknown {
                value: other.to_owned(),
            }),
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Record => "record",
            Self::Verify => "verify",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_empty_verifies() {
        assert_eq!(Mode::parse(None).ok(), Some(Mode::Verify));
        assert_eq!(Mode::parse(Some("")).ok(), Some(Mode::Verify));
        assert_eq!(Mode::parse(Some("verify")).ok(), Some(Mode::Verify));
    }

    #[test]
    fn record_records() {
        assert_eq!(Mode::parse(Some("record")).ok(), Some(Mode::Record));
        assert_eq!(Mode::parse(Some(" record\n")).ok(), Some(Mode::Record));
    }

    #[test]
    fn anything_else_is_an_error() {
        assert!(matches!(
            Mode::parse(Some("update")),
            Err(ModeError::Unknown { value }) if value == "update"
        ));
    }
}
