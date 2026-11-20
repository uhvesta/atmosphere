//! Capability-root router mapping `cap_<id>` prefixes to clamped [`CallerContext`]s.
//!
//! Unknown or revoked capability IDs are always rejected with [`VfsError::Eacces`].

use std::collections::BTreeMap;
use std::sync::RwLock;

use fusefs_vfs::{CallerContext, VfsError, VfsResult, VirtualPath};
use serde::{Deserialize, Serialize};

/// Required prefix for capability root directory segments.
pub const CAPABILITY_PREFIX: &str = "cap_";

/// Metadata describing a registered capability root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityEntry {
    pub cap_id: String,
    pub root_path: VirtualPath,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub read_only: bool,
    pub revoked: bool,
}

/// Result of routing a request through [`CapabilityRouter`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedRequest {
    pub cap_id: String,
    pub caller: CallerContext,
    pub clamped_path: VirtualPath,
    pub read_only: bool,
}

/// Thread-safe table mapping `cap_<id>` root prefixes to clamped VFS roots.
#[derive(Debug, Default)]
pub struct CapabilityRouter {
    entries: RwLock<BTreeMap<String, CapabilityEntry>>,
}

impl CapabilityRouter {
    /// Creates an empty capability router.
    pub fn new() -> Self {
        Self::default()
    }

    /// Validates that `cap_id` is a well-formed `cap_<id>` single path segment.
    pub fn validate_cap_id(cap_id: &str) -> VfsResult<()> {
        if !cap_id.starts_with(CAPABILITY_PREFIX) {
            return Err(VfsError::Einval(format!(
                "capability id '{cap_id}' must start with '{CAPABILITY_PREFIX}'"
            )));
        }
        let suffix = &cap_id[CAPABILITY_PREFIX.len()..];
        if suffix.is_empty()
            || cap_id.contains('/')
            || cap_id.as_bytes().contains(&0)
            || suffix == "."
            || suffix == ".."
        {
            return Err(VfsError::Einval(format!(
                "invalid capability id '{cap_id}'"
            )));
        }
        Ok(())
    }

    /// Registers or replaces a capability root mapping.
    pub fn register(
        &self,
        cap_id: impl Into<String>,
        root_path: VirtualPath,
        turn_id: Option<String>,
        read_only: bool,
    ) -> VfsResult<CapabilityEntry> {
        let cap_id = cap_id.into();
        Self::validate_cap_id(&cap_id)?;
        let entry = CapabilityEntry {
            cap_id: cap_id.clone(),
            root_path,
            turn_id,
            read_only,
            revoked: false,
        };
        let mut entries = self.entries.write().expect("capability router lock poisoned");
        entries.insert(cap_id, entry.clone());
        Ok(entry)
    }

    /// Revokes a single capability by ID. Returns `true` if it existed and was active.
    pub fn revoke_capability(&self, cap_id: &str) -> bool {
        let mut entries = self.entries.write().expect("capability router lock poisoned");
        if let Some(entry) = entries.get_mut(cap_id)
            && !entry.revoked
        {
            entry.revoked = true;
            return true;
        }
        false
    }

    /// Revokes all capabilities associated with `turn_id` and returns their IDs.
    pub fn revoke_turn(&self, turn_id: &str) -> Vec<String> {
        let mut entries = self.entries.write().expect("capability router lock poisoned");
        let mut revoked = Vec::new();
        for entry in entries.values_mut() {
            if entry.turn_id.as_deref() == Some(turn_id) && !entry.revoked {
                entry.revoked = true;
                revoked.push(entry.cap_id.clone());
            }
        }
        revoked
    }

    /// Returns all currently active (non-revoked) capability entries.
    pub fn list_active(&self) -> Vec<CapabilityEntry> {
        let entries = self.entries.read().expect("capability router lock poisoned");
        entries
            .values()
            .filter(|e| !e.revoked)
            .cloned()
            .collect()
    }

    /// Returns all registered capability entries (including revoked ones).
    pub fn list_all(&self) -> Vec<CapabilityEntry> {
        let entries = self.entries.read().expect("capability router lock poisoned");
        entries.values().cloned().collect()
    }

    /// Resolves a `cap_<id>` identifier directly into a clamped [`CallerContext`]
    /// and its backing [`VirtualPath`] root.
    ///
    /// Rejects unknown or revoked capability IDs with [`VfsError::Eacces`].
    pub fn resolve_cap_id(
        &self,
        cap_id: &str,
        base_caller: CallerContext,
    ) -> VfsResult<(CallerContext, VirtualPath, bool)> {
        if !cap_id.starts_with(CAPABILITY_PREFIX) {
            return Err(VfsError::Eacces(format!(
                "unknown capability root '{cap_id}'"
            )));
        }
        let entries = self.entries.read().expect("capability router lock poisoned");
        let entry = entries
            .get(cap_id)
            .ok_or_else(|| VfsError::Eacces(format!("unknown capability root '{cap_id}'")))?;
        if entry.revoked {
            return Err(VfsError::Eacces(format!(
                "capability root '{cap_id}' has been revoked"
            )));
        }
        let caller = base_caller.with_capability_root(entry.root_path.as_str());
        Ok((caller, entry.root_path.clone(), entry.read_only))
    }

    /// Routes a raw path prefixed with `/cap_<id>/...` (or `cap_<id>/...`) into a
    /// clamped [`CallerContext`] and [`VirtualPath`].
    ///
    /// - Rejects NUL bytes with [`VfsError::Einval`].
    /// - Rejects missing, unknown, or revoked `cap_<id>` prefixes with [`VfsError::Eacces`].
    /// - Clamps all `..` segments strictly inside the capability's `root_path`.
    pub fn resolve(&self, raw_path: &str, base_caller: CallerContext) -> VfsResult<RoutedRequest> {
        if raw_path.as_bytes().contains(&0) {
            return Err(VfsError::Einval("path contains NUL byte".to_string()));
        }
        let trimmed = raw_path.trim_start_matches('/');
        let (first_seg, remainder) = match trimmed.split_once('/') {
            Some((head, tail)) => (head, tail),
            None => (trimmed, ""),
        };
        if first_seg.is_empty() || !first_seg.starts_with(CAPABILITY_PREFIX) {
            return Err(VfsError::Eacces(format!(
                "path '{raw_path}' does not start with a capability root"
            )));
        }

        let (caller, root_path, read_only) = self.resolve_cap_id(first_seg, base_caller)?;
        let clamped_path = VirtualPath::parse_clamped(remainder, &root_path)?;

        Ok(RoutedRequest {
            cap_id: first_seg.to_string(),
            caller,
            clamped_path,
            read_only,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusefs_vfs::error::errno;

    #[test]
    fn routes_and_clamps_capability_paths() {
        let router = CapabilityRouter::new();
        let root = VirtualPath::parse("/workspaces/session-a").unwrap();
        router
            .register("cap_turn1", root.clone(), Some("turn-1".into()), false)
            .unwrap();

        let base = CallerContext::new(4242, 501, 20);
        let routed = router
            .resolve("/cap_turn1/src/lib.rs", base.clone())
            .unwrap();
        assert_eq!(routed.cap_id, "cap_turn1");
        assert_eq!(routed.clamped_path.as_str(), "/workspaces/session-a/src/lib.rs");
        assert_eq!(
            routed.caller.capability_root.as_deref(),
            Some("/workspaces/session-a")
        );
        assert_eq!(routed.caller.pid, 4242);
        assert!(!routed.read_only);

        // Adversarial `..` traversal stays clamped inside capability root.
        let escaped = router
            .resolve("/cap_turn1/src/../../../../etc/passwd", base)
            .unwrap();
        assert_eq!(
            escaped.clamped_path.as_str(),
            "/workspaces/session-a/etc/passwd"
        );
        assert!(escaped.clamped_path.is_within(&root));
    }

    #[test]
    fn rejects_unknown_and_revoked_capabilities_with_eacces() {
        let router = CapabilityRouter::new();
        let base = CallerContext::new(100, 501, 20);

        let err = router
            .resolve("/cap_missing/file.txt", base.clone())
            .unwrap_err();
        assert_eq!(err.errno(), errno::EACCES);

        let err = router.resolve("/etc/passwd", base.clone()).unwrap_err();
        assert_eq!(err.errno(), errno::EACCES);

        router
            .register(
                "cap_t7",
                VirtualPath::parse("/work/repo").unwrap(),
                Some("turn-7".into()),
                true,
            )
            .unwrap();
        assert!(router.resolve("/cap_t7/README.md", base.clone()).is_ok());

        let revoked = router.revoke_turn("turn-7");
        assert_eq!(revoked, vec!["cap_t7".to_string()]);
        assert!(router.list_active().is_empty());

        let err = router.resolve("/cap_t7/README.md", base).unwrap_err();
        assert_eq!(err.errno(), errno::EACCES);
    }
}
