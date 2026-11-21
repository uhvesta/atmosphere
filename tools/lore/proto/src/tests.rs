//! Behavioral unit tests for `lore_proto`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use prost::Message;

use crate::error::{FrameError, LoreError};
use crate::frame::{
    ARCHIVE_CODEC_ZSTD, ARCHIVE_MAGIC, BLOB_COMPRESS_MIN_BYTES, BLOB_MAGIC_RAW, BLOB_MAGIC_ZSTD,
    CHECKSUM_ALG_CRC32FAST, FORMAT_VERSION, HOT_MAGIC, RECORD_HEADER_LEN, SEGMENT_HEADER_LEN,
    TailStatus, blob_sha256_hex, decode_blob_bytes, decode_raw_record_frames,
    decode_segment_frames, decode_zstd_frame, encode_blob_bytes, encode_record_frame,
    encode_zstd_frame, verify_archive_header, verify_segment_header, write_archive_header,
    write_segment_header,
};
use crate::types::{
    BlobRef, Claim, ClaimForgotten, ClaimKind, ClaimStatus, ClaimTier, ClaimTransitioned,
    ClaimUpserted, Clock, Ecosystem, EdgeAdded, EdgeRel, FeedbackRecorded, FileDigestCheck, L0Card,
    Language, LockfileKind, LockfileSemverCheck, LogEvent, ManualClock, OriginSnapshot,
    OutcomeKind, PathExistsCheck, PathOrErrorOverlap, PeerPresence, PeerRepoMount, Predicate,
    PredicateEval, PredicateOp, Principal, PrincipalKind, SymbolPresentCheck, SystemClock,
    Verifier, VerifierStatus, log_event, new_claim_id, new_event_id, verifier,
};

fn sample_principal() -> Principal {
    Principal {
        kind: PrincipalKind::Agent as i32,
        id: "agent-01".to_string(),
        session_id: "sess-123".to_string(),
        agent_kind: "claude".to_string(),
        model: "claude-opus".to_string(),
        machine_id: "mac-host".to_string(),
    }
}

fn sample_claim(id: &str, ts_ms: i64) -> Claim {
    let mut attrs = BTreeMap::new();
    attrs.insert("ticket".to_string(), "HUM-42".to_string());
    attrs.insert("subsystem".to_string(), "proto".to_string());

    Claim {
        id: id.to_string(),
        kind: ClaimKind::Solution as i32,
        status: ClaimStatus::Active as i32,
        created_at_ms: ts_ms,
        updated_at_ms: ts_ms,
        valid_from_ms: 0,
        valid_until_ms: 0,
        review_after_ms: ts_ms + 86_400_000,
        statement: "Always run Bazel builds through tools/bq/bq".to_string(),
        body: "Sharing the Bazel daemon queue avoids lock contention across agents.".to_string(),
        dead_ends: vec!["Running cargo directly bypasses Bazel aspects".to_string()],
        keywords: vec!["bazel".to_string(), "bq".to_string()],
        attrs,
        scope: "repo:github.com/org/repo".to_string(),
        about: vec![
            "repo:github.com/org/repo".to_string(),
            "tool:bazel".to_string(),
        ],
        applies_when: vec![
            Predicate {
                axis: "os".to_string(),
                op: PredicateOp::Eq as i32,
                value: "macos".to_string(),
            },
            Predicate {
                axis: "branch_glob".to_string(),
                op: PredicateOp::Glob as i32,
                value: "release/*".to_string(),
            },
        ],
        author: Some(sample_principal()),
        tainted: false,
        origin: Some(OriginSnapshot {
            machine_id: "mac-host".to_string(),
            repo_id: "github.com/org/repo".to_string(),
            branch: "main".to_string(),
            head_commit: "deadbeef".to_string(),
            os: "macos".to_string(),
            arch: "aarch64".to_string(),
        }),
        verifiers: vec![
            Verifier {
                check: Some(verifier::Check::FileDigest(FileDigestCheck {
                    repo_id: "github.com/org/repo".to_string(),
                    path: "AGENTS.md".to_string(),
                    start_line: 1,
                    end_line: 20,
                    sha256: "abc123".to_string(),
                })),
            },
            Verifier {
                check: Some(verifier::Check::PathExists(PathExistsCheck {
                    repo_id: "github.com/org/repo".to_string(),
                    path: "tools/bq/bq".to_string(),
                    expect_exists: true,
                })),
            },
            Verifier {
                check: Some(verifier::Check::SymbolPresent(SymbolPresentCheck {
                    repo_id: "github.com/org/repo".to_string(),
                    path: "tools/lore/proto/src/frame.rs".to_string(),
                    symbol: "encode_record_frame".to_string(),
                    language: Language::Rust as i32,
                })),
            },
            Verifier {
                check: Some(verifier::Check::LockfileSemver(LockfileSemverCheck {
                    ecosystem: Ecosystem::Cargo as i32,
                    lockfile_kind: LockfileKind::CargoLock as i32,
                    package_name: "prost".to_string(),
                    semver_req: "^0.14".to_string(),
                })),
            },
            Verifier {
                check: Some(verifier::Check::BlobRef(BlobRef {
                    sha256: "0123456789abcdef".to_string(),
                    role: "stderr".to_string(),
                    summary: "build output".to_string(),
                })),
            },
        ],
        pinned: true,
        importance: 1.5,
    }
}

fn sample_event(seq: u64, ts_ms: i64) -> LogEvent {
    let claim_id = new_claim_id(ts_ms);
    let payload = match seq % 5 {
        1 => log_event::Payload::ClaimUpserted(ClaimUpserted {
            claim: Some(sample_claim(&claim_id, ts_ms)),
        }),
        2 => log_event::Payload::ClaimTransitioned(ClaimTransitioned {
            claim_id,
            new_status: ClaimStatus::Superseded as i32,
            reason: "replaced by newer benchmark".to_string(),
        }),
        3 => log_event::Payload::EdgeAdded(EdgeAdded {
            src_id: claim_id,
            rel: EdgeRel::Supersedes as i32,
            dst_id: new_claim_id(ts_ms - 1000),
            note: "supersession edge".to_string(),
        }),
        4 => log_event::Payload::FeedbackRecorded(FeedbackRecorded {
            claim_id,
            outcome: OutcomeKind::Corroborated as i32,
            note: "verified on linux".to_string(),
        }),
        _ => log_event::Payload::ClaimForgotten(ClaimForgotten {
            claim_id,
            reason: "owner requested erasure".to_string(),
        }),
    };

    LogEvent {
        seq,
        event_id: new_event_id(ts_ms),
        recorded_at_ms: ts_ms,
        actor: Some(sample_principal()),
        payload: Some(payload),
    }
}

#[test]
fn proto3_round_trip_all_event_variants() {
    for seq in 1..=5 {
        let ev = sample_event(seq, 1_700_000_000_000 + (seq as i64) * 1000);
        let encoded = ev.encode_to_vec();
        let decoded = LogEvent::decode(encoded.as_slice()).expect("decode LogEvent");
        assert_eq!(decoded, ev);
    }
}

#[test]
fn jsonl_serde_round_trip_is_deterministic() {
    let events: Vec<LogEvent> = (1..=5)
        .map(|seq| sample_event(seq, 1_700_000_000_000 + (seq as i64) * 500))
        .collect();

    let mut lines = Vec::new();
    for ev in &events {
        let line1 = serde_json::to_string(ev).expect("serialize JSONL line");
        let line2 = serde_json::to_string(ev).expect("serialize deterministically");
        assert_eq!(line1, line2);
        assert!(!line1.contains('\n'));
        lines.push(line1);
    }

    let decoded: Vec<LogEvent> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("deserialize JSONL line"))
        .collect();
    assert_eq!(decoded, events);
}

#[test]
fn ids_are_time_ordered_and_well_formed() {
    let earlier = new_claim_id(1_700_000_000_000);
    let later = new_claim_id(1_700_000_000_500);
    assert!(earlier.starts_with("clm_"));
    assert!(later.starts_with("clm_"));
    assert_eq!(earlier.len(), 4 + 12 + 1 + 12);
    assert!(earlier < later, "claim IDs should sort chronologically");

    let evt_a = new_event_id(1_700_000_000_000);
    let evt_b = new_event_id(1_700_000_000_000);
    assert!(evt_a.starts_with("evt_"));
    assert_eq!(evt_a.len(), 4 + 12 + 1 + 12);
    assert_ne!(evt_a, evt_b, "random suffix should disambiguate same-ms IDs");
}

#[test]
fn clock_and_domain_types_round_trip() {
    let manual = ManualClock::new(10_000);
    assert_eq!(manual.now_ms(), 10_000);
    assert_eq!(manual.advance_ms(5_000), 15_000);
    manual.set(42);
    assert_eq!(manual.now_ms(), 42);

    let sys = SystemClock;
    assert!(sys.now_ms() > 0);

    assert_eq!(ClaimTier::from_i32(ClaimTier::Hot.as_i32()), Some(ClaimTier::Hot));
    assert_eq!(
        ClaimTier::from_i32(ClaimTier::ColdArchived.as_i32()),
        Some(ClaimTier::ColdArchived)
    );
    assert_eq!(ClaimTier::from_i32(99), None);

    let card = L0Card {
        id: new_claim_id(1000),
        kind: ClaimKind::Decision,
        status: ClaimStatus::Active,
        statement: "Use proto3 for WAL".to_string(),
        trust_badge: "owner".to_string(),
        verifier_badge: VerifierStatus::Verified,
        scope: "global".to_string(),
        score: 0.95,
        has_body: true,
        has_dead_ends: false,
        pinned: true,
        updated_at_ms: 1000,
    };
    let card_json = serde_json::to_string(&card).expect("serialize L0Card");
    assert_eq!(serde_json::from_str::<L0Card>(&card_json).unwrap(), card);

    let presence = PeerPresence {
        session_id: "s1".to_string(),
        channel_id: Some("ch1".to_string()),
        machine_id: "mac".to_string(),
        agent_kind: "agy".to_string(),
        mounted_repos: vec![PeerRepoMount {
            repo_id: "github.com/org/repo".to_string(),
            mount_subpath: ".".to_string(),
            branch: Some("main".to_string()),
            head_sha: Some("abc".to_string()),
            dirty_count: 2,
        }],
        intent: Some("implementing proto".to_string()),
        hypothesis: None,
        touched_paths: vec!["tools/lore/proto/src/frame.rs".to_string()],
        recent_failures: vec!["err:e0433".to_string()],
        updated_at_ms: 1000,
        ttl_ms: 900_000,
    };
    let p_json = serde_json::to_string(&presence).expect("serialize PeerPresence");
    assert_eq!(
        serde_json::from_str::<PeerPresence>(&p_json).unwrap(),
        presence
    );

    let overlap = PathOrErrorOverlap {
        peer_session_id: "s1".to_string(),
        peer_machine_id: "mac".to_string(),
        peer_agent_kind: "agy".to_string(),
        repo_id: Some("github.com/org/repo".to_string()),
        overlapping_paths: vec!["tools/lore/proto/src/frame.rs".to_string()],
        overlapping_errors: vec!["err:e0433".to_string()],
    };
    let o_json = serde_json::to_string(&overlap).expect("serialize overlap");
    assert_eq!(
        serde_json::from_str::<PathOrErrorOverlap>(&o_json).unwrap(),
        overlap
    );

    assert_ne!(PredicateEval::Match, PredicateEval::Mismatch);
    assert_ne!(PredicateEval::Unknown, PredicateEval::Match);
}

#[test]
fn hot_segment_header_and_clean_frames_round_trip() {
    let mut seg = Vec::new();
    write_segment_header(&mut seg).expect("write header");
    assert_eq!(seg.len(), SEGMENT_HEADER_LEN);
    assert_eq!(&seg[0..8], &HOT_MAGIC);

    let hdr = verify_segment_header(&seg).expect("verify header");
    assert_eq!(hdr.format_ver, FORMAT_VERSION);
    assert_eq!(hdr.checksum_alg, CHECKSUM_ALG_CRC32FAST);
    assert_eq!(hdr.flags, 0);

    let events: Vec<LogEvent> = (1..=12)
        .map(|seq| sample_event(seq, 1_700_000_000_000 + seq as i64))
        .collect();

    let mut expected_offsets = Vec::new();
    for ev in &events {
        expected_offsets.push(seg.len() as u64);
        let frame = encode_record_frame(ev).expect("encode frame");
        seg.extend_from_slice(&frame);
    }

    let (decoded, status) = decode_segment_frames(&seg).expect("decode clean segment");
    assert_eq!(status, TailStatus::Clean);
    assert_eq!(decoded.len(), events.len());
    for ((off, ev), (exp_off, exp_ev)) in decoded.iter().zip(expected_offsets.iter().zip(events.iter())) {
        assert_eq!(off, exp_off);
        assert_eq!(ev, exp_ev);
    }
}

#[test]
fn hot_segment_detects_torn_tail_vs_mid_segment_corruption() {
    let mut seg = Vec::new();
    write_segment_header(&mut seg).unwrap();

    let mut frame_offsets = Vec::new();
    for seq in 1..=10u64 {
        frame_offsets.push(seg.len());
        let ev = sample_event(seq, 1_700_000_000_000 + seq as i64);
        seg.extend_from_slice(&encode_record_frame(&ev).unwrap());
    }

    let last_offset = frame_offsets[9];

    // 1. Truncate final frame mid-payload -> TruncatedTail at last valid frame boundary.
    let truncated_payload = &seg[..seg.len() - 5];
    let (records, status) = decode_segment_frames(truncated_payload).unwrap();
    assert_eq!(records.len(), 9);
    assert_eq!(
        status,
        TailStatus::TruncatedTail {
            valid_end_offset: last_offset as u64,
        }
    );

    // 2. Truncate final frame mid-header -> TruncatedTail at last valid frame boundary.
    let truncated_hdr = &seg[..last_offset + 7];
    let (records, status) = decode_segment_frames(truncated_hdr).unwrap();
    assert_eq!(records.len(), 9);
    assert_eq!(
        status,
        TailStatus::TruncatedTail {
            valid_end_offset: last_offset as u64,
        }
    );

    // 3. Flip 1 bit in final frame payload -> TruncatedTail (recoverable tail bit-flip).
    let mut tail_flipped = seg.clone();
    let last_idx = tail_flipped.len() - 1;
    tail_flipped[last_idx] ^= 0x01;
    let (records, status) = decode_segment_frames(&tail_flipped).unwrap();
    assert_eq!(records.len(), 9);
    assert_eq!(
        status,
        TailStatus::TruncatedTail {
            valid_end_offset: last_offset as u64,
        }
    );

    // 4. Flip 1 bit in middle frame (seq 5) -> MidSegmentCorrupt (must NOT silently truncate).
    let mid_offset = frame_offsets[4];
    let mut mid_flipped = seg.clone();
    mid_flipped[mid_offset + RECORD_HEADER_LEN + 2] ^= 0x80;
    let (records, status) = decode_segment_frames(&mid_flipped).unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!(
        status,
        TailStatus::MidSegmentCorrupt {
            offset: mid_offset as u64,
            seq: 5,
        }
    );

    // 5. Flip high bit of payload_len in middle frame (seq 5) so payload_len > MAX_RECORD_PAYLOAD_LEN
    //    (and exceeds segment length) -> MUST be classified as MidSegmentCorrupt, never TruncatedTail.
    let mut mid_len_inflated = seg.clone();
    mid_len_inflated[mid_offset + 3] ^= 0x80;
    let (records, status) = decode_segment_frames(&mid_len_inflated).unwrap();
    assert_eq!(records.len(), 4);
    assert_eq!(
        status,
        TailStatus::MidSegmentCorrupt {
            offset: mid_offset as u64,
            seq: 5,
        }
    );
}

#[test]
fn hot_segment_header_rejects_corrupt_header() {
    let mut hdr = Vec::new();
    write_segment_header(&mut hdr).unwrap();

    // Flip CRC byte.
    let mut bad_crc = hdr.clone();
    bad_crc[15] ^= 0x01;
    assert!(matches!(
        verify_segment_header(&bad_crc),
        Err(FrameError::HeaderChecksumMismatch { .. })
    ));

    // Corrupt magic.
    let mut bad_magic = hdr.clone();
    bad_magic[0] = b'X';
    assert!(matches!(
        verify_segment_header(&bad_magic),
        Err(FrameError::InvalidMagic)
    ));

    // Short header.
    assert!(matches!(
        verify_segment_header(&hdr[..10]),
        Err(FrameError::HeaderTooShort { .. })
    ));
}

#[test]
fn cold_zstd_archive_header_and_frames_round_trip_and_detect_bitflip() {
    let mut arch_hdr = Vec::new();
    write_archive_header(&mut arch_hdr).unwrap();
    assert_eq!(&arch_hdr[0..8], &ARCHIVE_MAGIC);
    let parsed = verify_archive_header(&arch_hdr).unwrap();
    assert_eq!(parsed.format_ver, FORMAT_VERSION);
    assert_eq!(parsed.codec, ARCHIVE_CODEC_ZSTD);

    let events: Vec<LogEvent> = (1..=25u64)
        .map(|seq| sample_event(seq, 1_700_000_000_000 + seq as i64))
        .collect();
    let mut raw_block = Vec::new();
    for ev in &events {
        raw_block.extend_from_slice(&encode_record_frame(ev).unwrap());
    }

    let compressed = encode_zstd_frame(&raw_block).expect("compress zstd frame");
    assert!(
        compressed.len() < raw_block.len(),
        "repetitive event block should compress smaller"
    );

    let decompressed = decode_zstd_frame(&compressed).expect("decompress zstd frame");
    assert_eq!(decompressed, raw_block);

    let decoded_records = decode_raw_record_frames(&decompressed).expect("decode raw frames");
    assert_eq!(decoded_records.len(), events.len());
    for ((_, got), want) in decoded_records.iter().zip(events.iter()) {
        assert_eq!(got, want);
    }

    // Flip a bit in the zstd frame (triggers XXH64 checksum failure or decode error).
    let mut corrupt = compressed.clone();
    let mid = corrupt.len() / 2;
    corrupt[mid] ^= 0xFF;
    assert!(decode_zstd_frame(&corrupt).is_err());
}

#[test]
fn blob_framing_respects_size_and_savings_thresholds() {
    // Small blob (< 1 KiB): always raw with LRB0 prefix.
    let small = vec![b'a'; BLOB_COMPRESS_MIN_BYTES - 1];
    let enc_small = encode_blob_bytes(&small).unwrap();
    assert_eq!(&enc_small[0..4], &BLOB_MAGIC_RAW);
    assert_eq!(decode_blob_bytes(&enc_small).unwrap(), small);

    // Large compressible blob (>= 1 KiB): compressed with LRBZ prefix.
    let compressible = vec![b'x'; 4096];
    let enc_comp = encode_blob_bytes(&compressible).unwrap();
    assert_eq!(&enc_comp[0..4], &BLOB_MAGIC_ZSTD);
    assert!(enc_comp.len() < compressible.len());
    assert_eq!(decode_blob_bytes(&enc_comp).unwrap(), compressible);

    // Large incompressible blob (>= 1 KiB pseudo-random): falls back to LRB0 raw.
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut incompressible = Vec::with_capacity(4096);
    for _ in 0..4096 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        incompressible.push((state & 0xFF) as u8);
    }
    let enc_incomp = encode_blob_bytes(&incompressible).unwrap();
    assert_eq!(&enc_incomp[0..4], &BLOB_MAGIC_RAW);
    assert_eq!(decode_blob_bytes(&enc_incomp).unwrap(), incompressible);

    // Invalid blob magic rejected.
    assert!(matches!(
        decode_blob_bytes(b"NOPE1234"),
        Err(FrameError::InvalidBlobMagic)
    ));

    // SHA-256 hex digest is 64 lowercase hex chars and deterministic.
    let h1 = blob_sha256_hex(b"hello lore");
    let h2 = blob_sha256_hex(b"hello lore");
    assert_eq!(h1, h2);
    assert_eq!(h1.len(), 64);
    assert!(h1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

#[test]
fn error_variants_construct_and_preserve_sources() {
    use std::error::Error as _;

    let corrupt = LoreError::CorruptSegment {
        path: PathBuf::from("/tmp/seg-0001.pblog"),
        offset: 128,
        seq: 7,
        source: FrameError::RecordChecksumMismatch {
            seq: 7,
            expected: 1,
            actual: 2,
        },
    };
    assert!(corrupt.source().is_some());
    assert!(corrupt.to_string().contains("lore verify"));

    let disabled_plain = LoreError::Disabled { reason: None };
    let plain_msg = disabled_plain.to_string();
    assert!(plain_msg.contains("memory.enabled = false"));
    assert!(plain_msg.contains("Do not attempt to re-enable"));

    let disabled_reason = LoreError::Disabled {
        reason: Some("evaluating baseline without memory".to_string()),
    };
    let reason_msg = disabled_reason.to_string();
    assert!(reason_msg.contains("evaluating baseline without memory"));
    assert!(reason_msg.contains("Do not attempt to re-enable"));

    let host_err = LoreError::HostUnreachable {
        endpoint: "humatad.sock".to_string(),
        buffered: true,
        source: std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused"),
    };
    assert!(host_err.source().is_some());
}

#[test]
fn closed_language_ecosystem_and_lockfile_enums_parse_and_round_trip() {
    assert_eq!("rust".parse::<Language>().unwrap(), Language::Rust);
    assert_eq!("starlark".parse::<Language>().unwrap(), Language::Starlark);
    assert_eq!("polyglot".parse::<Language>().unwrap(), Language::Polyglot);
    assert_eq!("".parse::<Language>().unwrap(), Language::Unspecified);
    assert_eq!("unknown".parse::<Language>().unwrap(), Language::Unspecified);
    assert_eq!("cobol".parse::<Language>().unwrap(), Language::Other);
    assert_eq!(Language::Swift.as_str(), "swift");
    assert_eq!(Language::Unspecified.as_str(), "unknown");

    assert_eq!("cargo".parse::<Ecosystem>().unwrap(), Ecosystem::Cargo);
    assert_eq!("gradle".parse::<Ecosystem>().unwrap(), Ecosystem::Maven);
    assert_eq!("bzlmod".parse::<Ecosystem>().unwrap(), Ecosystem::Bazel);
    assert_eq!("".parse::<Ecosystem>().unwrap(), Ecosystem::Unspecified);
    assert_eq!("npm".parse::<Ecosystem>().unwrap(), Ecosystem::Other);
    assert_eq!(Ecosystem::Maven.as_str(), "maven");

    assert_eq!(
        "Cargo.lock".parse::<LockfileKind>().unwrap(),
        LockfileKind::CargoLock
    );
    assert_eq!(
        "MODULE.bazel.lock".parse::<LockfileKind>().unwrap(),
        LockfileKind::BazelModuleLock
    );
    assert_eq!(
        "Package.resolved".parse::<LockfileKind>().unwrap(),
        LockfileKind::SwiftPackageResolved
    );
    assert_eq!("".parse::<LockfileKind>().unwrap(), LockfileKind::Unspecified);
    assert_eq!("yarn.lock".parse::<LockfileKind>().unwrap(), LockfileKind::Other);
    assert_eq!(LockfileKind::PyUvLock.as_str(), "py_uv_lock");
}
