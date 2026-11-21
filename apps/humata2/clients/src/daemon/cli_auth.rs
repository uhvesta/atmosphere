//! Pending and live `humata login` sessions and script tokens (`CliAuthService`, SETTINGSFIX).

use crate::model::{Capability, ChannelId, MachineId};

/// A `humata login` waiting for the user's Allow or Deny in the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingCliLogin {
    /// The daemon's pending login id (e.g. `cl_...`).
    pub id: String,
    /// The terminal description, e.g. `"Terminal, ttys004"`.
    pub terminal: String,
    /// How many hours the login asks to last.
    pub hours: u32,
    /// When the login was asked, in milliseconds since the Unix epoch (0 if unknown).
    pub asked_at_unix_ms: i64,
}

/// An active `humata login` terminal session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliSession {
    /// The daemon's session id.
    pub id: String,
    /// The machine the login is on.
    pub machine: MachineId,
    /// The terminal description, e.g. `"Terminal, ttys004"`.
    pub terminal: String,
    /// When the login expires, in milliseconds since the Unix epoch (0 if unknown).
    pub expires_at_unix_ms: i64,
}

/// A script token minted via `humata token create`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptToken {
    /// The token's id.
    pub id: String,
    /// The token's human-readable name.
    pub name: String,
    /// Capabilities held by the token.
    pub capabilities: Vec<Capability>,
    /// Channels scoped to the token.
    pub channels: Vec<ChannelId>,
    /// When the token expires, in milliseconds since the Unix epoch (0 if unknown).
    pub expires_at_unix_ms: i64,
    /// When the token was last used, in milliseconds since the Unix epoch.
    pub last_used_at_unix_ms: Option<i64>,
}
