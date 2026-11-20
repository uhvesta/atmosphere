//! Normalized virtual paths with strict `..` capability-root clamping.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{VfsError, VfsResult, VfsTarget};

/// A strictly normalized, absolute virtual filesystem path.
///
/// Invariants:
/// - Always starts with `'/'`.
/// - Never contains NUL (`'\0'`) bytes.
/// - Never contains duplicate `'/'`, `.` segments, or `..` segments.
/// - Never ends with `'/'` unless it is the root path `"/"`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualPath(String);

impl VirtualPath {
    /// Returns the root virtual path `"/"`.
    pub fn root() -> Self {
        Self("/".to_string())
    }

    /// Parses and normalizes a raw path against the filesystem root `"/"`.
    ///
    /// - Rejects NUL bytes (`VfsError::Einval`).
    /// - Collapses `.` and duplicate `/`.
    /// - Clamps `..` at `"/"`.
    pub fn parse(raw: &str) -> VfsResult<Self> {
        Self::resolve_from_segments(&[], 0, raw)
    }

    /// Parses and normalizes a raw path with `..` clamped at `capability_root`.
    ///
    /// - Rejects NUL bytes (`VfsError::Einval`).
    /// - If `raw` is an absolute path that begins with `capability_root`, walking
    ///   `..` can never pop above `capability_root`.
    /// - If `raw` is a relative path or an absolute path outside `capability_root`,
    ///   it is resolved starting at `capability_root` with `..` clamped at
    ///   `capability_root` so the resulting path can never escape `capability_root`.
    pub fn parse_clamped(raw: &str, capability_root: &VirtualPath) -> VfsResult<Self> {
        validate_no_nul(raw)?;
        let cap_segs = capability_root.segment_vec();
        let min_depth = cap_segs.len();

        if raw.starts_with('/') {
            let raw_segs: Vec<&str> = raw.split('/').filter(|s| !s.is_empty()).collect();
            if raw_segs.len() >= cap_segs.len() && raw_segs[..cap_segs.len()] == cap_segs[..] {
                let mut stack: Vec<String> = cap_segs.iter().map(|s| (*s).to_string()).collect();
                apply_segments(&mut stack, min_depth, &raw_segs[cap_segs.len()..]);
                return Ok(Self::from_segments(&stack));
            }
        }

        Self::resolve_from_segments(&cap_segs, min_depth, raw)
    }

    /// Resolves `raw` relative to `self`, clamping `..` at `"/"`.
    pub fn join(&self, raw: &str) -> VfsResult<Self> {
        self.join_clamped(raw, &Self::root())
    }

    /// Resolves `raw` relative to `self`, clamping `..` at `capability_root`.
    ///
    /// If `self` is not within `capability_root`, resolution begins at `capability_root`.
    /// If `raw` starts with `'/'`, it is resolved via [`Self::parse_clamped`].
    pub fn join_clamped(&self, raw: &str, capability_root: &VirtualPath) -> VfsResult<Self> {
        validate_no_nul(raw)?;
        if raw.starts_with('/') {
            return Self::parse_clamped(raw, capability_root);
        }
        let cap_segs = capability_root.segment_vec();
        let min_depth = cap_segs.len();
        let base_segs = if self.is_within(capability_root) {
            self.segment_vec()
        } else {
            cap_segs
        };
        Self::resolve_from_segments(&base_segs, min_depth, raw)
    }

    /// Returns the normalized string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns `true` if this path is the root `"/"`.
    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }

    /// Returns an iterator over path segments (empty for `"/"`).
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|s| !s.is_empty())
    }

    /// Returns the parent directory path, or `None` for `"/"`.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        match self.0.rfind('/') {
            Some(0) => Some(Self::root()),
            Some(idx) => Some(Self(self.0[..idx].to_string())),
            None => None,
        }
    }

    /// Returns the parent directory path clamped at `capability_root`.
    ///
    /// If `self` is at or outside `capability_root`, returns `capability_root.clone()`.
    pub fn parent_clamped(&self, capability_root: &VirtualPath) -> Self {
        if !self.is_within(capability_root) || self == capability_root {
            return capability_root.clone();
        }
        self.parent().unwrap_or_else(|| capability_root.clone())
    }

    /// Returns the final path component, or `None` for `"/"`.
    pub fn file_name(&self) -> Option<&str> {
        if self.is_root() {
            return None;
        }
        self.0.rsplit('/').next()
    }

    /// Returns `true` if `self` is equal to `ancestor` or strictly inside `ancestor`.
    ///
    /// Matching is segment-aware: `"/work/repo2"` is **not** within `"/work/repo"`.
    pub fn is_within(&self, ancestor: &VirtualPath) -> bool {
        if ancestor.is_root() {
            return true;
        }
        if self.0 == ancestor.0 {
            return true;
        }
        self.0
            .strip_prefix(&ancestor.0)
            .is_some_and(|rest| rest.starts_with('/'))
    }

    /// Strips `prefix` from `self` and returns the remainder as a normalized `VirtualPath`.
    pub fn strip_prefix(&self, prefix: &VirtualPath) -> Option<Self> {
        if !self.is_within(prefix) {
            return None;
        }
        if prefix.is_root() {
            return Some(self.clone());
        }
        if self.0 == prefix.0 {
            return Some(Self::root());
        }
        Some(Self(self.0[prefix.0.len()..].to_string()))
    }

    fn segment_vec(&self) -> Vec<&str> {
        self.segments().collect()
    }

    fn resolve_from_segments(base: &[&str], min_depth: usize, raw: &str) -> VfsResult<Self> {
        validate_no_nul(raw)?;
        let mut stack: Vec<String> = base.iter().map(|s| (*s).to_string()).collect();
        let raw_segs: Vec<&str> = raw.split('/').filter(|s| !s.is_empty()).collect();
        apply_segments(&mut stack, min_depth, &raw_segs);
        Ok(Self::from_segments(&stack))
    }

    fn from_segments(segs: &[String]) -> Self {
        if segs.is_empty() {
            Self::root()
        } else {
            Self(format!("/{}", segs.join("/")))
        }
    }
}

fn validate_no_nul(raw: &str) -> VfsResult<()> {
    if raw.as_bytes().contains(&0) {
        return Err(VfsError::Einval {
            op: "parse path",
            target: VfsTarget::Path(raw.replace('\0', "\\0")),
        });
    }
    Ok(())
}

fn apply_segments(stack: &mut Vec<String>, min_depth: usize, segs: &[&str]) {
    for &seg in segs {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.len() > min_depth {
                    stack.pop();
                }
            }
            normal => stack.push(normal.to_string()),
        }
    }
}

impl fmt::Display for VirtualPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for VirtualPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for VirtualPath {
    type Err = VfsError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for VirtualPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for VirtualPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_collapses_dots_and_duplicate_slashes() {
        assert_eq!(VirtualPath::parse("").unwrap().as_str(), "/");
        assert_eq!(VirtualPath::parse("/").unwrap().as_str(), "/");
        assert_eq!(VirtualPath::parse("///a//b/./c/").unwrap().as_str(), "/a/b/c");
        assert_eq!(
            VirtualPath::parse("/a/b/../c/./d/..").unwrap().as_str(),
            "/a/c"
        );
        assert_eq!(
            VirtualPath::parse("../../../etc/passwd").unwrap().as_str(),
            "/etc/passwd"
        );
    }

    #[test]
    fn rejects_nul_bytes() {
        let err = VirtualPath::parse("/foo/\0bar").unwrap_err();
        assert!(matches!(err, VfsError::Einval { .. }));

        let cap = VirtualPath::parse("/sandbox").unwrap();
        let err = VirtualPath::parse_clamped("foo\0bar", &cap).unwrap_err();
        assert!(matches!(err, VfsError::Einval { .. }));
    }

    #[test]
    fn capability_root_clamps_dotdot_adversarial_traversal() {
        let cap = VirtualPath::parse("/workspaces/session-1").unwrap();

        // Relative traversal from inside capability root.
        let sub = VirtualPath::parse("/workspaces/session-1/src/lib").unwrap();
        let escaped = sub
            .join_clamped("../../../../../../etc/passwd", &cap)
            .unwrap();
        assert_eq!(escaped.as_str(), "/workspaces/session-1/etc/passwd");
        assert!(escaped.is_within(&cap));

        // Absolute path beginning inside capability root then traversing up.
        let abs_escaped = VirtualPath::parse_clamped(
            "/workspaces/session-1/a/b/../../../../../etc/shadow",
            &cap,
        )
        .unwrap();
        assert_eq!(abs_escaped.as_str(), "/workspaces/session-1/etc/shadow");
        assert!(abs_escaped.is_within(&cap));

        // Pure upward traversal stays pinned at capability root.
        let at_cap = sub.join_clamped("../../../../..", &cap).unwrap();
        assert_eq!(at_cap, cap);

        // Absolute path outside capability root is clamped inside capability root.
        let outside = VirtualPath::parse_clamped("/etc/passwd", &cap).unwrap();
        assert_eq!(outside.as_str(), "/workspaces/session-1/etc/passwd");
        assert!(outside.is_within(&cap));

        // Parent clamping at capability root stays at capability root.
        assert_eq!(cap.parent_clamped(&cap), cap);
        assert_eq!(
            VirtualPath::parse("/workspaces/session-1/a")
                .unwrap()
                .parent_clamped(&cap),
            cap
        );
    }

    #[test]
    fn segment_aware_is_within() {
        let repo = VirtualPath::parse("/work/repo").unwrap();
        let repo_file = VirtualPath::parse("/work/repo/src/main.rs").unwrap();
        let sibling = VirtualPath::parse("/work/repo2").unwrap();
        let sibling_file = VirtualPath::parse("/work/repo2/src/main.rs").unwrap();

        assert!(repo.is_within(&repo));
        assert!(repo_file.is_within(&repo));
        assert!(!sibling.is_within(&repo));
        assert!(!sibling_file.is_within(&repo));
        assert!(sibling.is_within(&VirtualPath::root()));
    }
}
