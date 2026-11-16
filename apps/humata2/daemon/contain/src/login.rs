//! How a contained Claude Code signs in (D217, the keychain gap). Humata
//! never copies `.credentials.json` and never duplicates or rewrites the
//! user's keychain login. In order: the usual keychain login, when this
//! Claude Code version is known to honour `CLAUDE_SECURESTORAGE_CONFIG_DIR`;
//! else a token or API key the user already set; else the launch is refused.

use crate::ContainError;

/// Claude Code versions tested to find the usual keychain login when
/// `CLAUDE_CONFIG_DIR` points at a copy and `CLAUDE_SECURESTORAGE_CONFIG_DIR`
/// is empty. The variable is undocumented, so a version is added here only
/// after it has been tried; any other version is unverified.
const VERIFIED_VERSIONS: &[&str] = &["2.1.325"];

/// The variables that carry a login the user set for the session themselves.
pub const TOKEN_VARIABLES: &[&str] = &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"];

/// Where a contained Claude Code finds its login.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    /// The usual keychain login.
    Keychain,
    /// A token or API key already in the launch environment.
    Token,
}

/// The version number in the output of `claude --version`, such as
/// `2.1.325 (Claude Code)`: the first word made of digits and dots.
pub fn parse_version(output: &str) -> Option<String> {
    output
        .split_whitespace()
        .find(|word| {
            word.split('.').count() >= 2
                && word.split('.').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(str::to_owned)
}

/// Picks the login for a contained launch of the installed Claude Code
/// `version` (`None` when it couldn't be read), given whether the user set a
/// token. Refuses when neither works: the launch must not fall back to
/// uncontained or prompt for a login.
pub fn choose_login(version: Option<&str>, token_set: bool) -> Result<Login, ContainError> {
    if version.is_some_and(|version| VERIFIED_VERSIONS.contains(&version)) {
        Ok(Login::Keychain)
    } else if token_set {
        Ok(Login::Token)
    } else {
        Err(ContainError::ClaudeLoginUnreachable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verified_version_uses_the_keychain_login() {
        assert_eq!(choose_login(Some("2.1.325"), false).unwrap(), Login::Keychain);
        assert_eq!(choose_login(Some("2.1.325"), true).unwrap(), Login::Keychain);
    }

    #[test]
    fn an_unknown_version_uses_a_token_the_user_set() {
        assert_eq!(choose_login(Some("9.9.9"), true).unwrap(), Login::Token);
        assert_eq!(choose_login(None, true).unwrap(), Login::Token);
    }

    #[test]
    fn an_unknown_version_without_a_token_is_refused() {
        assert!(matches!(
            choose_login(Some("9.9.9"), false),
            Err(ContainError::ClaudeLoginUnreachable)
        ));
        assert!(matches!(
            choose_login(None, false),
            Err(ContainError::ClaudeLoginUnreachable)
        ));
    }

    #[test]
    fn the_version_is_read_from_the_version_output() {
        assert_eq!(parse_version("2.1.325 (Claude Code)\n").as_deref(), Some("2.1.325"));
        assert_eq!(parse_version("Claude Code v2.1.325\n"), None);
        assert_eq!(parse_version("not a version"), None);
    }
}
