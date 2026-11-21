//! Wire-compatible proto3 Rust types, ID generators, and domain structs (SPEC.md §4.1).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum UTF-8 scalar count for an L0 `Claim::statement` (SPEC.md §3.3 MOD-1).
pub const MAX_STATEMENT_CHARS: usize = 280;

// ---------------------------------------------------------------------------
// ID generation
// ---------------------------------------------------------------------------

fn format_time_ordered_id(prefix: &str, now_ms: i64) -> String {
    let ts = u64::try_from(now_ms).unwrap_or(0) & 0x0000_FFFF_FFFF_FFFF;
    let rand_bytes = *Uuid::new_v4().as_bytes();
    format!(
        "{prefix}_{ts:012x}_{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        rand_bytes[0], rand_bytes[1], rand_bytes[2], rand_bytes[3], rand_bytes[4], rand_bytes[5],
    )
}

/// Generate a time-ordered claim identifier `clm_<12-hex-ms>_<12-hex-rand>`.
pub fn new_claim_id(now_ms: i64) -> String {
    format_time_ordered_id("clm", now_ms)
}

/// Generate a time-ordered event identifier `evt_<12-hex-ms>_<12-hex-rand>`.
pub fn new_event_id(now_ms: i64) -> String {
    format_time_ordered_id("evt", now_ms)
}

// ---------------------------------------------------------------------------
// Clock abstraction (deterministic testing without sleeps)
// ---------------------------------------------------------------------------

/// Wall-clock source returning Unix epoch milliseconds.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}

/// Production clock backed by [`SystemTime::now`].
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

/// Deterministic clock for in-process tests.
#[derive(Debug)]
pub struct ManualClock {
    now_ms: AtomicI64,
}

impl ManualClock {
    pub fn new(initial_ms: i64) -> Self {
        Self {
            now_ms: AtomicI64::new(initial_ms),
        }
    }

    pub fn set(&self, ms: i64) {
        self.now_ms.store(ms, Ordering::SeqCst);
    }

    pub fn advance_ms(&self, delta_ms: i64) -> i64 {
        self.now_ms.fetch_add(delta_ms, Ordering::SeqCst) + delta_ms
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Proto3 enums (SPEC.md §4.1)
// ---------------------------------------------------------------------------

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum ClaimKind {
    Unspecified = 0,
    Axiom = 1,
    Fact = 2,
    Decision = 3,
    Solution = 4,
    DeadEnd = 5,
    Warning = 6,
    Preference = 7,
    Question = 8,
    Handoff = 9,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum ClaimStatus {
    Unspecified = 0,
    Active = 1,
    Superseded = 2,
    Invalidated = 3,
    Retracted = 4,
    Expired = 5,
    Quarantined = 6,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum PredicateOp {
    Unspecified = 0,
    Eq = 1,
    In = 2,
    Glob = 3,
    SemverReq = 4,
    AncestorOf = 5,
    NotAncestorOf = 6,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum EdgeRel {
    Unspecified = 0,
    Supersedes = 1,
    Contradicts = 2,
    Supports = 3,
    DerivedFrom = 4,
    Answers = 5,
    ContrastsWith = 6,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum PrincipalKind {
    Unspecified = 0,
    Owner = 1,
    Agent = 2,
    Import = 3,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum OutcomeKind {
    Unspecified = 0,
    Helpful = 1,
    Harmful = 2,
    NotApplicable = 3,
    Corroborated = 4,
    Disputed = 5,
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum Language {
    Unspecified = 0,
    Rust = 1,
    Go = 2,
    Python = 3,
    Java = 4,
    Bazel = 5,
    Starlark = 6,
    Swift = 7,
    Polyglot = 8,
    Other = 9,
}

impl Language {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unknown",
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Python => "python",
            Self::Java => "java",
            Self::Bazel => "bazel",
            Self::Starlark => "starlark",
            Self::Swift => "swift",
            Self::Polyglot => "polyglot",
            Self::Other => "other",
        }
    }
}

impl std::str::FromStr for Language {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        Ok(match normalized.as_str() {
            "" | "unknown" | "unspecified" => Self::Unspecified,
            "rust" | "rs" => Self::Rust,
            "go" | "golang" => Self::Go,
            "python" | "py" => Self::Python,
            "java" => Self::Java,
            "bazel" => Self::Bazel,
            "starlark" | "bzl" => Self::Starlark,
            "swift" => Self::Swift,
            "polyglot" => Self::Polyglot,
            _ => Self::Other,
        })
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum Ecosystem {
    Unspecified = 0,
    Cargo = 1,
    Go = 2,
    Python = 3,
    Maven = 4,
    Bazel = 5,
    Swift = 6,
    Polyglot = 7,
    Other = 8,
}

impl Ecosystem {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unknown",
            Self::Cargo => "cargo",
            Self::Go => "go",
            Self::Python => "python",
            Self::Maven => "maven",
            Self::Bazel => "bazel",
            Self::Swift => "swift",
            Self::Polyglot => "polyglot",
            Self::Other => "other",
        }
    }
}

impl std::str::FromStr for Ecosystem {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        Ok(match normalized.as_str() {
            "" | "unknown" | "unspecified" => Self::Unspecified,
            "cargo" | "rust" | "crates" => Self::Cargo,
            "go" | "gomod" | "golang" => Self::Go,
            "python" | "py" | "pypi" | "pip" | "uv" | "poetry" => Self::Python,
            "maven" | "java" | "gradle" => Self::Maven,
            "bazel" | "bzlmod" => Self::Bazel,
            "swift" | "swiftpm" => Self::Swift,
            "polyglot" => Self::Polyglot,
            _ => Self::Other,
        })
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, prost::Enumeration, Serialize, Deserialize,
)]
#[repr(i32)]
pub enum LockfileKind {
    Unspecified = 0,
    CargoLock = 1,
    GoMod = 2,
    PyUvLock = 3,
    PyPoetryLock = 4,
    PyRequirements = 5,
    JavaPom = 6,
    JavaGradleLock = 7,
    BazelModuleLock = 8,
    SwiftPackageResolved = 9,
    Other = 10,
}

impl LockfileKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unspecified => "unspecified",
            Self::CargoLock => "cargo_lock",
            Self::GoMod => "go_mod",
            Self::PyUvLock => "py_uv_lock",
            Self::PyPoetryLock => "py_poetry_lock",
            Self::PyRequirements => "py_requirements",
            Self::JavaPom => "java_pom",
            Self::JavaGradleLock => "java_gradle_lock",
            Self::BazelModuleLock => "bazel_module_lock",
            Self::SwiftPackageResolved => "swift_package_resolved",
            Self::Other => "other",
        }
    }
}

impl std::str::FromStr for LockfileKind {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = s.trim().to_ascii_lowercase();
        Ok(match normalized.as_str() {
            "" | "unknown" | "unspecified" => Self::Unspecified,
            "cargo_lock" | "cargo.lock" => Self::CargoLock,
            "go_mod" | "go.mod" => Self::GoMod,
            "py_uv_lock" | "uv.lock" => Self::PyUvLock,
            "py_poetry_lock" | "poetry.lock" => Self::PyPoetryLock,
            "py_requirements" | "requirements.txt" => Self::PyRequirements,
            "java_pom" | "pom.xml" => Self::JavaPom,
            "java_gradle_lock" | "gradle.lockfile" => Self::JavaGradleLock,
            "bazel_module_lock" | "module.bazel.lock" => Self::BazelModuleLock,
            "swift_package_resolved" | "package.resolved" => Self::SwiftPackageResolved,
            _ => Self::Other,
        })
    }
}

// ---------------------------------------------------------------------------
// Proto3 messages (SPEC.md §4.1)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct Predicate {
    #[prost(string, tag = "1")]
    pub axis: String,
    #[prost(enumeration = "PredicateOp", tag = "2")]
    pub op: i32,
    #[prost(string, tag = "3")]
    pub value: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct Principal {
    #[prost(enumeration = "PrincipalKind", tag = "1")]
    pub kind: i32,
    #[prost(string, tag = "2")]
    pub id: String,
    #[prost(string, tag = "3")]
    pub session_id: String,
    #[prost(string, tag = "4")]
    pub agent_kind: String,
    #[prost(string, tag = "5")]
    pub model: String,
    #[prost(string, tag = "6")]
    pub machine_id: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct OriginSnapshot {
    #[prost(string, tag = "1")]
    pub machine_id: String,
    #[prost(string, tag = "2")]
    pub repo_id: String,
    #[prost(string, tag = "3")]
    pub branch: String,
    #[prost(string, tag = "4")]
    pub head_commit: String,
    #[prost(string, tag = "5")]
    pub os: String,
    #[prost(string, tag = "6")]
    pub arch: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct Verifier {
    #[prost(oneof = "verifier::Check", tags = "1, 2, 3, 4, 5")]
    pub check: Option<verifier::Check>,
}

pub mod verifier {
    use super::{
        BlobRef, Deserialize, FileDigestCheck, LockfileSemverCheck, PathExistsCheck, Serialize,
        SymbolPresentCheck,
    };

    #[derive(Clone, PartialEq, prost::Oneof, Serialize, Deserialize)]
    pub enum Check {
        #[prost(message, tag = "1")]
        FileDigest(FileDigestCheck),
        #[prost(message, tag = "2")]
        PathExists(PathExistsCheck),
        #[prost(message, tag = "3")]
        SymbolPresent(SymbolPresentCheck),
        #[prost(message, tag = "4")]
        LockfileSemver(LockfileSemverCheck),
        #[prost(message, tag = "5")]
        BlobRef(BlobRef),
    }
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct FileDigestCheck {
    #[prost(string, tag = "1")]
    pub repo_id: String,
    #[prost(string, tag = "2")]
    pub path: String,
    #[prost(uint32, tag = "3")]
    pub start_line: u32,
    #[prost(uint32, tag = "4")]
    pub end_line: u32,
    #[prost(string, tag = "5")]
    pub sha256: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct PathExistsCheck {
    #[prost(string, tag = "1")]
    pub repo_id: String,
    #[prost(string, tag = "2")]
    pub path: String,
    #[prost(bool, tag = "3")]
    pub expect_exists: bool,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct SymbolPresentCheck {
    #[prost(string, tag = "1")]
    pub repo_id: String,
    #[prost(string, tag = "2")]
    pub path: String,
    #[prost(string, tag = "3")]
    pub symbol: String,
    #[prost(enumeration = "Language", tag = "4")]
    pub language: i32,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct LockfileSemverCheck {
    #[prost(enumeration = "Ecosystem", tag = "1")]
    pub ecosystem: i32,
    #[prost(enumeration = "LockfileKind", tag = "2")]
    pub lockfile_kind: i32,
    #[prost(string, tag = "3")]
    pub package_name: String,
    #[prost(string, tag = "4")]
    pub semver_req: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct BlobRef {
    #[prost(string, tag = "1")]
    pub sha256: String,
    #[prost(string, tag = "2")]
    pub role: String,
    #[prost(string, tag = "3")]
    pub summary: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct Claim {
    // Facet 1: Identity & Lifecycle
    #[prost(string, tag = "1")]
    pub id: String,
    #[prost(enumeration = "ClaimKind", tag = "2")]
    pub kind: i32,
    #[prost(enumeration = "ClaimStatus", tag = "3")]
    pub status: i32,
    #[prost(int64, tag = "4")]
    pub created_at_ms: i64,
    #[prost(int64, tag = "5")]
    pub updated_at_ms: i64,
    #[prost(int64, tag = "6")]
    pub valid_from_ms: i64,
    #[prost(int64, tag = "7")]
    pub valid_until_ms: i64,
    #[prost(int64, tag = "8")]
    pub review_after_ms: i64,

    // Facet 2: Progressive Content
    #[prost(string, tag = "10")]
    pub statement: String,
    #[prost(string, tag = "11")]
    pub body: String,
    #[prost(string, repeated, tag = "12")]
    pub dead_ends: Vec<String>,
    #[prost(string, repeated, tag = "13")]
    pub keywords: Vec<String>,
    #[prost(btree_map = "string, string", tag = "14")]
    pub attrs: BTreeMap<String, String>,

    // Facet 3: Scope vs. Entities
    #[prost(string, tag = "20")]
    pub scope: String,
    #[prost(string, repeated, tag = "21")]
    pub about: Vec<String>,

    // Facet 4: Applicability
    #[prost(message, repeated, tag = "30")]
    pub applies_when: Vec<Predicate>,

    // Facet 5: Provenance, Taint & Verifiers
    #[prost(message, optional, tag = "40")]
    pub author: Option<Principal>,
    #[prost(bool, tag = "41")]
    pub tainted: bool,
    #[prost(message, optional, tag = "42")]
    pub origin: Option<OriginSnapshot>,
    #[prost(message, repeated, tag = "43")]
    pub verifiers: Vec<Verifier>,

    // Facet 6: Raw Signals
    #[prost(bool, tag = "50")]
    pub pinned: bool,
    #[prost(float, tag = "51")]
    pub importance: f32,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct LogEvent {
    #[prost(uint64, tag = "1")]
    pub seq: u64,
    #[prost(string, tag = "2")]
    pub event_id: String,
    #[prost(int64, tag = "3")]
    pub recorded_at_ms: i64,
    #[prost(message, optional, tag = "4")]
    pub actor: Option<Principal>,
    #[prost(oneof = "log_event::Payload", tags = "10, 11, 12, 13, 14")]
    pub payload: Option<log_event::Payload>,
}

pub mod log_event {
    use super::{
        ClaimForgotten, ClaimTransitioned, ClaimUpserted, Deserialize, EdgeAdded,
        FeedbackRecorded, Serialize,
    };

    #[derive(Clone, PartialEq, prost::Oneof, Serialize, Deserialize)]
    pub enum Payload {
        #[prost(message, tag = "10")]
        ClaimUpserted(ClaimUpserted),
        #[prost(message, tag = "11")]
        ClaimTransitioned(ClaimTransitioned),
        #[prost(message, tag = "12")]
        EdgeAdded(EdgeAdded),
        #[prost(message, tag = "13")]
        FeedbackRecorded(FeedbackRecorded),
        #[prost(message, tag = "14")]
        ClaimForgotten(ClaimForgotten),
    }
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaimUpserted {
    #[prost(message, optional, tag = "1")]
    pub claim: Option<Claim>,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaimTransitioned {
    #[prost(string, tag = "1")]
    pub claim_id: String,
    #[prost(enumeration = "ClaimStatus", tag = "2")]
    pub new_status: i32,
    #[prost(string, tag = "3")]
    pub reason: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeAdded {
    #[prost(string, tag = "1")]
    pub src_id: String,
    #[prost(enumeration = "EdgeRel", tag = "2")]
    pub rel: i32,
    #[prost(string, tag = "3")]
    pub dst_id: String,
    #[prost(string, tag = "4")]
    pub note: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct FeedbackRecorded {
    #[prost(string, tag = "1")]
    pub claim_id: String,
    #[prost(enumeration = "OutcomeKind", tag = "2")]
    pub outcome: i32,
    #[prost(string, tag = "3")]
    pub note: String,
}

#[derive(Clone, PartialEq, prost::Message, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaimForgotten {
    #[prost(string, tag = "1")]
    pub claim_id: String,
    #[prost(string, tag = "2")]
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Domain / projection / recall / live-awareness types
// ---------------------------------------------------------------------------

/// Storage tier for a claim in the SQLite projection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum ClaimTier {
    #[default]
    Hot = 0,
    ColdArchived = 1,
}

impl ClaimTier {
    pub fn as_i32(self) -> i32 {
        self as i32
    }

    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Hot),
            1 => Some(Self::ColdArchived),
            _ => None,
        }
    }
}

/// Result of running a claim's local citation verifiers on the calling machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum VerifierStatus {
    Verified,
    Drifted,
    UnverifiedLocal,
}

/// Tri-state evaluation of an `applies_when` predicate (SPEC.md §3.3 MOD-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PredicateEval {
    Match,
    Mismatch,
    Unknown,
}

/// Compact L0 card returned by `recall` (SPEC.md §3.6 RET-1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct L0Card {
    pub id: String,
    pub kind: ClaimKind,
    pub status: ClaimStatus,
    pub statement: String,
    pub trust_badge: String,
    pub verifier_badge: VerifierStatus,
    pub scope: String,
    pub score: f64,
    pub has_body: bool,
    pub has_dead_ends: bool,
    pub pinned: bool,
    pub updated_at_ms: i64,
}

/// Mounted repository snapshot reported in live peer presence (SPEC.md §3.2 TOP-7).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRepoMount {
    pub repo_id: String,
    pub mount_subpath: String,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub dirty_count: u32,
}

/// Ephemeral presence row for a live peer session (SPEC.md §3.7 AW-1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerPresence {
    pub session_id: String,
    pub channel_id: Option<String>,
    pub machine_id: String,
    pub agent_kind: String,
    pub mounted_repos: Vec<PeerRepoMount>,
    pub intent: Option<String>,
    pub hypothesis: Option<String>,
    pub touched_paths: Vec<String>,
    pub recent_failures: Vec<String>,
    pub updated_at_ms: i64,
    pub ttl_ms: i64,
}

/// Detected path or error-signature overlap with an active peer (SPEC.md §3.7 AW-3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathOrErrorOverlap {
    pub peer_session_id: String,
    pub peer_machine_id: String,
    pub peer_agent_kind: String,
    pub repo_id: Option<String>,
    pub overlapping_paths: Vec<String>,
    pub overlapping_errors: Vec<String>,
}
