//! The keychain, for host variables' values and tokens (HO-7, HO-13).
//! Values never leave it except to the daemon that needs them.

/// Why the keychain refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeychainError {
    /// The person didn't allow access.
    #[error(
        "the keychain didn't allow access to {name}. Allow Humata in the prompt, or in Keychain Access, and try again"
    )]
    Denied {
        /// The item.
        name: String,
    },
}

/// Secrets by name.
pub trait Keychain: Send + Sync + 'static {
    /// Whether a value is stored for `name`; the value itself isn't read.
    ///
    /// # Errors
    ///
    /// [`KeychainError::Denied`] when access is refused.
    fn has(&self, name: &str) -> Result<bool, KeychainError>;
    /// Stores `value` for `name`.
    ///
    /// # Errors
    ///
    /// [`KeychainError::Denied`] when access is refused.
    fn set(&self, name: &str, value: &str) -> Result<(), KeychainError>;
}
