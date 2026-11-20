//! Subtree write leases and per-file optimistic generation concurrency checks.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

use crate::error::{VfsError, VfsResult};
use crate::path::VirtualPath;
use crate::types::CallerContext;

/// Unique identifier for a granted [`WriteLease`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LeaseId(pub u64);

impl fmt::Display for LeaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "lease:{}", self.0)
    }
}

/// Time-bounded write lease scoped to a caller PID and virtual subtree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteLease {
    pub id: LeaseId,
    pub caller_pid: u32,
    pub subtree: VirtualPath,
    pub expires_at_ms: u64,
    pub turn_id: Option<String>,
    pub revoked: bool,
}

impl WriteLease {
    /// Returns `true` if the lease has not been revoked and has not expired at `now_ms`.
    pub fn is_active_at(&self, now_ms: u64) -> bool {
        !self.revoked && now_ms < self.expires_at_ms
    }

    /// Returns `true` if the lease has reached or passed its expiration timestamp.
    pub fn is_expired_at(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }
}

#[derive(Debug, Default)]
struct LeaseTableInner {
    next_id: u64,
    leases: BTreeMap<LeaseId, WriteLease>,
}

/// Thread-safe table of subtree [`WriteLease`] grants.
#[derive(Debug, Default)]
pub struct LeaseTable {
    inner: RwLock<LeaseTableInner>,
}

impl LeaseTable {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(LeaseTableInner {
                next_id: 1,
                leases: BTreeMap::new(),
            }),
        }
    }

    /// Grants a new write lease for `caller_pid` on `subtree` expiring at `expires_at_ms`.
    pub fn grant(
        &self,
        caller_pid: u32,
        subtree: VirtualPath,
        expires_at_ms: u64,
        turn_id: Option<String>,
    ) -> WriteLease {
        let mut guard = self.inner.write().expect("lease table lock poisoned");
        let id = LeaseId(guard.next_id);
        guard.next_id = guard.next_id.saturating_add(1);
        let lease = WriteLease {
            id,
            caller_pid,
            subtree,
            expires_at_ms,
            turn_id,
            revoked: false,
        };
        guard.leases.insert(id, lease.clone());
        lease
    }

    /// Checks whether `caller` holds an active write lease covering `path` at `now_ms`.
    ///
    /// Returns the matching [`LeaseId`] on success, or [`VfsError::Eacces`] if no active
    /// lease covers `path`.
    pub fn authorize_write(
        &self,
        caller: &CallerContext,
        path: &VirtualPath,
        now_ms: u64,
    ) -> VfsResult<LeaseId> {
        let guard = self.inner.read().expect("lease table lock poisoned");
        let mut saw_expired_or_revoked = false;

        for lease in guard.leases.values() {
            if lease.caller_pid != caller.pid {
                continue;
            }
            if !path.is_within(&lease.subtree) {
                continue;
            }
            if lease.is_active_at(now_ms) {
                return Ok(lease.id);
            }
            saw_expired_or_revoked = true;
        }

        if saw_expired_or_revoked {
            Err(VfsError::Eacces(format!(
                "write lease for pid {} on '{}' is expired or revoked",
                caller.pid, path
            )))
        } else {
            Err(VfsError::Eacces(format!(
                "no active write lease for pid {} covering '{}'",
                caller.pid, path
            )))
        }
    }

    /// Revokes a single lease by [`LeaseId`]. Returns `true` if the lease existed and was
    /// transitioned to revoked.
    pub fn revoke_lease(&self, lease_id: LeaseId) -> bool {
        let mut guard = self.inner.write().expect("lease table lock poisoned");
        if let Some(lease) = guard.leases.get_mut(&lease_id)
            && !lease.revoked
        {
            lease.revoked = true;
            return true;
        }
        false
    }

    /// Revokes all non-revoked leases tagged with `turn_id`. Returns the number revoked.
    pub fn revoke_turn(&self, turn_id: &str) -> usize {
        let mut guard = self.inner.write().expect("lease table lock poisoned");
        let mut count = 0;
        for lease in guard.leases.values_mut() {
            if !lease.revoked && lease.turn_id.as_deref() == Some(turn_id) {
                lease.revoked = true;
                count += 1;
            }
        }
        count
    }

    /// Returns `Some(true)` if the lease exists and is expired at `now_ms`,
    /// `Some(false)` if it exists and has not expired, or `None` if unknown.
    pub fn is_expired(&self, lease_id: LeaseId, now_ms: u64) -> Option<bool> {
        let guard = self.inner.read().expect("lease table lock poisoned");
        guard.leases.get(&lease_id).map(|l| l.is_expired_at(now_ms))
    }

    /// Removes expired or revoked leases from the table and returns how many were pruned.
    pub fn prune_inactive(&self, now_ms: u64) -> usize {
        let mut guard = self.inner.write().expect("lease table lock poisoned");
        let before = guard.leases.len();
        guard.leases.retain(|_, l| l.is_active_at(now_ms));
        before - guard.leases.len()
    }

    /// Looks up a lease by ID.
    pub fn get(&self, lease_id: LeaseId) -> Option<WriteLease> {
        let guard = self.inner.read().expect("lease table lock poisoned");
        guard.leases.get(&lease_id).cloned()
    }

    /// Lists all active leases at `now_ms`.
    pub fn active_leases(&self, now_ms: u64) -> Vec<WriteLease> {
        let guard = self.inner.read().expect("lease table lock poisoned");
        guard
            .leases
            .values()
            .filter(|l| l.is_active_at(now_ms))
            .cloned()
            .collect()
    }
}

/// Validates an optional optimistic concurrency generation expectation.
///
/// Returns `Ok(())` when `expected` is `None` or matches `actual`; otherwise returns
/// [`VfsError::Estale`].
pub fn check_generation(expected: Option<u64>, actual: u64) -> VfsResult<()> {
    if let Some(exp) = expected
        && exp != actual
    {
        return Err(VfsError::Estale(format!(
            "generation mismatch: expected {exp}, actual {actual}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_grant_prefix_matching_and_eacces_outside_subtree() {
        let table = LeaseTable::new();
        let caller = CallerContext::new(42, 1000, 1000);
        let subtree = VirtualPath::parse("/work/repo").unwrap();

        let lease = table.grant(42, subtree.clone(), 10_000, Some("turn-1".into()));

        // Inside subtree succeeds.
        let inside = VirtualPath::parse("/work/repo/src/lib.rs").unwrap();
        assert_eq!(table.authorize_write(&caller, &inside, 5_000).unwrap(), lease.id);
        assert_eq!(
            table.authorize_write(&caller, &subtree, 5_000).unwrap(),
            lease.id
        );

        // Sibling with matching string prefix must be rejected.
        let sibling = VirtualPath::parse("/work/repo2/src/lib.rs").unwrap();
        let err = table.authorize_write(&caller, &sibling, 5_000).unwrap_err();
        assert!(matches!(err, VfsError::Eacces(_)));

        // Different PID must be rejected.
        let other_caller = CallerContext::new(99, 1000, 1000);
        let err = table
            .authorize_write(&other_caller, &inside, 5_000)
            .unwrap_err();
        assert!(matches!(err, VfsError::Eacces(_)));
    }

    #[test]
    fn lease_expiry_and_turn_revocation() {
        let table = LeaseTable::new();
        let caller = CallerContext::new(7, 1000, 1000);
        let path_a = VirtualPath::parse("/a/file.txt").unwrap();
        let path_b = VirtualPath::parse("/b/file.txt").unwrap();

        let lease_a = table.grant(
            7,
            VirtualPath::parse("/a").unwrap(),
            1_000,
            Some("turn-x".into()),
        );
        let lease_b = table.grant(
            7,
            VirtualPath::parse("/b").unwrap(),
            5_000,
            Some("turn-x".into()),
        );

        assert_eq!(table.is_expired(lease_a.id, 999), Some(false));
        assert_eq!(table.is_expired(lease_a.id, 1_000), Some(true));

        // Expired at t = 1_000.
        assert!(matches!(
            table.authorize_write(&caller, &path_a, 1_000),
            Err(VfsError::Eacces(_))
        ));
        // Still active at t = 1_000 for lease_b.
        assert_eq!(
            table.authorize_write(&caller, &path_b, 1_000).unwrap(),
            lease_b.id
        );

        // Revoke all leases for turn-x.
        assert_eq!(table.revoke_turn("turn-x"), 2);
        assert!(matches!(
            table.authorize_write(&caller, &path_b, 1_000),
            Err(VfsError::Eacces(_))
        ));
    }

    #[test]
    fn generation_check_returns_estale_on_mismatch() {
        assert!(check_generation(None, 5).is_ok());
        assert!(check_generation(Some(5), 5).is_ok());
        let err = check_generation(Some(4), 5).unwrap_err();
        assert!(matches!(err, VfsError::Estale(_)));
    }
}
