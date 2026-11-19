//! Shared agent-watch entrypoint (`watch::agent`).

use crate::Tell;

/// Spawns a lightweight watcher placeholder for `session` on `tell`.
pub fn agent(_tell: Tell, _session: String) {}
