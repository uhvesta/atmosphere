//! `humata2_rpc_kit`: shared gRPC middleware, logging, link proof, sync marker,
//! compression, and zero-service RPC helpers for `humatad2` (SPLIT §3.1, §3.2;
//! SPLIT_PHASE3_DAEMON §2.1 Step 6a).

pub mod admin;
pub mod compression;
pub mod helpers;
pub mod link_proof;
pub mod log;
pub mod rfc3339;
pub mod sync;

pub use helpers::{
    API_MAJOR, API_MINOR, Answers, Gated, REMEMBERED_MAX, Role, SOCKET_PATH_MAX,
    answer_key_for_actor, blocking, caller_data, chain, decode_caller_data, error, error_of,
    failed, failure, internal, invalid_field, lock, missing, not_built as not_built_status,
    not_found, now_ms, status_error_of, timestamp_ms,
};
