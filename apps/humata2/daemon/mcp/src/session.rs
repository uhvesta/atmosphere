//! The session token the holder puts in the agent's environment (MCP.md §1).

use std::env;
use std::fmt;

use crate::error::TokenError;

/// The variable the holder sets for the agent and everything it starts.
pub const TOKEN_VAR: &str = "HUMATA_SESSION_TOKEN";

/// A session's token: new on every launch and resume. The daemon binds the
/// connection to that session only after checking that this process
/// descends from the session's holder.
///
/// Never printed: `Debug` hides it, and it is not part of any idempotency
/// key, result or error.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionToken(String);

impl SessionToken {
    /// A token from its text. Fails when the text is empty or only spaces.
    pub fn new(value: impl Into<String>) -> Result<Self, TokenError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(TokenError::Empty { var: TOKEN_VAR });
        }
        Ok(Self(value))
    }

    /// The token from [`TOKEN_VAR`].
    pub fn from_env() -> Result<Self, TokenError> {
        match env::var_os(TOKEN_VAR) {
            None => Err(TokenError::Missing { var: TOKEN_VAR }),
            Some(value) => match value.into_string() {
                Ok(text) => Self::new(text),
                Err(_) => Err(TokenError::NotUnicode { var: TOKEN_VAR }),
            },
        }
    }

    /// The token's text, for the daemon connection to present.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionToken(hidden)")
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionToken, TOKEN_VAR};
    use crate::error::TokenError;

    /// MCP.md §1
    #[test]
    fn an_empty_or_blank_token_is_refused() {
        for text in ["", "   ", "\n"] {
            assert_eq!(
                SessionToken::new(text),
                Err(TokenError::Empty { var: TOKEN_VAR })
            );
        }
    }

    /// MCP.md §1: the token is never printed.
    #[test]
    fn debug_output_hides_the_token() {
        let token = SessionToken::new("secret-1234").expect("a token");
        assert!(!format!("{token:?}").contains("secret-1234"));
    }
}
