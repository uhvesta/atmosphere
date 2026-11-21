//! Wire-compatible schema, explicit errors, and binary framing for `lore`.

pub mod error;
pub mod frame;
pub mod types;

#[cfg(test)]
mod tests;

pub use error::{FrameError, LoreError, Result};
pub use frame::{
    ARCHIVE_CODEC_ZSTD, ARCHIVE_MAGIC, ARCHIVE_ZSTD_LEVEL, ArchiveHeader, BLOB_COMPRESS_MIN_BYTES,
    BLOB_MAGIC_RAW, BLOB_MAGIC_ZSTD, CHECKSUM_ALG_CRC32FAST, FORMAT_VERSION, HOT_MAGIC,
    MAX_RECORD_PAYLOAD_LEN, RECORD_HEADER_LEN, SEGMENT_HEADER_LEN, SegmentHeader, TailStatus,
    blob_sha256_hex, decode_blob_bytes, decode_raw_record_frames, decode_segment_frames,
    decode_zstd_frame, encode_blob_bytes, encode_record_frame, encode_zstd_frame,
    verify_archive_header, verify_segment_header, write_archive_header, write_segment_header,
};
pub use types::{
    BlobRef, Claim, ClaimForgotten, ClaimKind, ClaimStatus, ClaimTier, ClaimTransitioned,
    ClaimUpserted, Clock, Ecosystem, EdgeAdded, EdgeRel, FeedbackRecorded, FileDigestCheck, L0Card,
    Language, LockfileKind, LockfileSemverCheck, LogEvent, MAX_STATEMENT_CHARS, ManualClock,
    OriginSnapshot, OutcomeKind, PathExistsCheck, PathOrErrorOverlap, PeerPresence, PeerRepoMount,
    Predicate, PredicateEval, PredicateOp, Principal, PrincipalKind, SymbolPresentCheck,
    SystemClock, Verifier, VerifierStatus, log_event, new_claim_id, new_event_id, verifier,
};
