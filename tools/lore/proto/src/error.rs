//! Explicit, structured error types for `lore` (SPEC.md §5.3).

use std::path::PathBuf;

/// Result alias using [`LoreError`].
pub type Result<T> = std::result::Result<T, LoreError>;

/// Low-level framing, header, checksum, and compression errors.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("segment header is truncated: expected {expected} bytes, found {actual}")]
    HeaderTooShort { expected: usize, actual: usize },

    #[error("segment magic bytes do not match expected format")]
    InvalidMagic,

    #[error("unsupported segment format version {version} (expected {expected})")]
    UnsupportedVersion { version: u16, expected: u16 },

    #[error("unsupported checksum or codec algorithm id {algorithm}")]
    UnsupportedAlgorithm { algorithm: u8 },

    #[error("header CRC-32 mismatch: expected {expected:#010x}, computed {actual:#010x}")]
    HeaderChecksumMismatch { expected: u32, actual: u32 },

    #[error("record payload length {len} exceeds maximum {max} bytes")]
    PayloadTooLarge { len: usize, max: usize },

    #[error("record CRC-32 mismatch at seq {seq}: expected {expected:#010x}, computed {actual:#010x}")]
    RecordChecksumMismatch { seq: u64, expected: u32, actual: u32 },

    #[error("non-monotonic record sequence number: previous {prev}, got {got}")]
    NonMonotonicSeq { prev: u64, got: u64 },

    #[error("truncated record frame: expected at least {expected} bytes, found {actual}")]
    TruncatedFrame { expected: usize, actual: usize },

    #[error("invalid blob magic prefix (expected LRB0 or LRBZ)")]
    InvalidBlobMagic,

    #[error("protobuf decode failed: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),

    #[error("protobuf encode failed: {0}")]
    ProtobufEncode(#[from] prost::EncodeError),

    #[error("I/O or compression error: {0}")]
    Io(#[from] std::io::Error),
}

fn format_disabled_reason(reason: Option<&str>) -> String {
    match reason.map(str::trim).filter(|s| !s.is_empty()) {
        Some(r) => format!(" (reason: {r})"),
        None => String::new(),
    }
}

/// Top-level error enum shared across `lore` crates.
#[derive(Debug, thiserror::Error)]
pub enum LoreError {
    #[error(
        "lore is intentionally disabled (memory.enabled = false){}. Do not attempt to re-enable, debug, or work around lore; proceed with your task without it.",
        format_disabled_reason(reason.as_deref())
    )]
    Disabled { reason: Option<String> },

    #[error(
        "Claim statement is {len} chars (max {max}). Move details into `body` and keep `statement` to a single headline."
    )]
    StatementTooLong { len: usize, max: usize },

    #[error(
        "Refused to store potential {pattern_kind} secret in `{field}`. Remove the secret before recording; only pass `--redact` if you have verified the surrounding text contains no other credentials."
    )]
    SecretDetected { pattern_kind: String, field: String },

    #[error(
        "Tainted session cannot perform {attempted} on claim {claim_id}. Confirm the claim from a clean session or as the owner before promoting."
    )]
    TaintedPromotionDenied { claim_id: String, attempted: String },

    #[error(
        "Checksum mismatch in {} at byte {offset} (seq {seq}). Do NOT delete the log; run `lore verify` to inspect damaged frames or restore from backup before running `lore reindex`.",
        path.display()
    )]
    CorruptSegment {
        path: PathBuf,
        offset: u64,
        seq: u64,
        #[source]
        source: FrameError,
    },

    #[error(
        "Failed to decompress cold archive {} for {claim_id}. Run `lore verify --archives` to check integrity.",
        segment.display()
    )]
    ArchiveHydrationFailed {
        claim_id: String,
        segment: PathBuf,
        #[source]
        source: FrameError,
    },

    #[error(
        "Mac Host bridge ({endpoint}) is unreachable; write buffered={buffered}. Ensure Humata is running on the Host; buffered writes will drain automatically on reconnect."
    )]
    HostUnreachable {
        endpoint: String,
        buffered: bool,
        #[source]
        source: std::io::Error,
    },

    #[error("Claim `{id}` was not found. Check the ID with `lore recall` or `lore recall --include-history`.")]
    ClaimNotFound { id: String },

    #[error(
        "Invalid scope `{scope}`. Expected one of `global`, `channel:<id>`, `repo:<id>`, `machine:<id>`, `dir:<machine>:<hash>`, or `session:<id>`."
    )]
    InvalidScope { scope: String },

    #[error(
        "Invalid predicate `{axis} {op} {value}`. Check the operator and value syntax (e.g. valid glob or semver requirement)."
    )]
    InvalidPredicate {
        axis: String,
        op: String,
        value: String,
    },

    #[error(
        "Timed out after {timeout_ms}ms waiting for write lock {}. Ensure no stuck `lore` process holds the lock before retrying.",
        path.display()
    )]
    LockContention { path: PathBuf, timeout_ms: u64 },

    #[error("I/O error on {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("SQLite error during `{op}`: {reason}. If the projection index is damaged, run `lore reindex` to rebuild it from the authoritative logs.")]
    Sqlite { op: String, reason: String },

    #[error("Failed to decode protobuf payload ({context}): {source}")]
    Decode {
        context: String,
        #[source]
        source: prost::DecodeError,
    },

    #[error("Failed to encode protobuf payload ({context}): {source}")]
    Encode {
        context: String,
        #[source]
        source: prost::EncodeError,
    },
}
