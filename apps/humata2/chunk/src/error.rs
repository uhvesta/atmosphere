//! What can go wrong with a chunk or the compression settings.

use std::io;
use std::path::PathBuf;

use crate::chunk::CHUNK_MAX;
use crate::codec::Codec;

/// A chunk that couldn't be encoded or decoded.
#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error(
        "a chunk of {len} bytes is over the {CHUNK_MAX}-byte limit; this is a bug in the sender, which must split data into chunks of at most {CHUNK_MAX} bytes"
    )]
    TooBig { len: usize },

    #[error(
        "the chunk is encoded with codec number {wire}, which this build of Humata doesn't know; update Humata on this machine to match the sender, or set compression to Off for the sending machine"
    )]
    UnknownCodec { wire: i32 },

    #[error(
        "a {codec} chunk says it holds {claimed} bytes, over the {CHUNK_MAX}-byte limit; the chunk is damaged or the sender is faulty. Ask for the data again from this chunk's offset"
    )]
    SizeOutOfRange { codec: Codec, claimed: u64 },

    #[error(
        "a {codec} chunk is damaged and doesn't decompress; ask for the data again from this chunk's offset. If it happens again, set compression to Off for this machine and report it"
    )]
    Corrupt {
        codec: Codec,
        #[source]
        source: DecodeFailure,
    },

    #[error(
        "a {codec} chunk decompressed to {actual} bytes, not the {expected} its header names; it is damaged. Ask for the data again from this chunk's offset"
    )]
    WrongSize {
        codec: Codec,
        expected: u32,
        actual: usize,
    },

    #[error(
        "a {codec} chunk's checksum doesn't match its bytes (CRC-32 {actual:08x}, expected {expected:08x}); it was damaged on the way. Ask for the data again from this chunk's offset"
    )]
    ChecksumMismatch {
        codec: Codec,
        expected: u32,
        actual: u32,
    },

    #[error(
        "a {codec} chunk carries no checksum, which every compressed chunk must; the sender is faulty. Update Humata on the sending machine, or set compression to Off for it"
    )]
    NoChecksum { codec: Codec },

    #[error("couldn't start the zstd {doing} (the system may be out of memory); free some memory and try again")]
    ZstdContext {
        doing: &'static str,
        #[source]
        source: io::Error,
    },

    #[error("couldn't compress a chunk with zstd; send it again with compression set to Off and report this")]
    Compress {
        #[source]
        source: io::Error,
    },
}

/// A frame (compressed data at rest) that couldn't be written or read.
/// `origin` names the file or stream it came from.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error(
        "{origin} isn't Humata compressed data (it doesn't start with the frame header); it was written by something else or is damaged. Restore it from a backup or remove it, and report it if Humata wrote it"
    )]
    NotAFrame { origin: String },

    #[error(
        "{origin} ends inside its header, so it was cut short (a full disk or a crash while writing); restore it from a backup or remove it"
    )]
    Truncated { origin: String },

    #[error(
        "{origin} uses header format {format}, which this build of Humata doesn't read; it was written by a newer Humata. Update Humata on this machine; don't edit or remove the file, the newer build needs it"
    )]
    UnknownFormat { origin: String, format: u8 },

    #[error(
        "{origin} is compressed with codec id {id}, which this build of Humata doesn't know, so it isn't read rather than guessed at; it was written by a newer Humata. Update Humata on this machine; don't remove the file"
    )]
    UnknownCodec { origin: String, id: u8 },

    #[error(
        "{origin} says it decodes to {claimed} bytes, over the {max_len} this reader allows; it is damaged or isn't what the reader expected. Restore it from a backup or remove it"
    )]
    TooLarge { origin: String, claimed: u64, max_len: u64 },

    #[error("{origin} is damaged: its {codec} data doesn't decompress. Restore it from a backup or remove it")]
    Corrupt {
        origin: String,
        codec: Codec,
        #[source]
        source: DecodeFailure,
    },

    #[error(
        "{origin} is damaged: its bytes don't match the length or checksum in its header. Restore it from a backup or remove it"
    )]
    Damaged { origin: String },

    #[error("couldn't compress a message of type {kind} with zstd; write it with compression set to Off and report this")]
    Compress {
        kind: u16,
        #[source]
        source: io::Error,
    },
}

/// Why a compressed chunk didn't decompress.
#[derive(Debug, thiserror::Error)]
pub enum DecodeFailure {
    #[error(transparent)]
    Lz4(#[from] lz4_flex::block::DecompressError),
    #[error(transparent)]
    Zstd(#[from] io::Error),
}

/// The compression settings file couldn't be used.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("couldn't read the compression settings in {path}; check its permissions, or delete it to use Auto")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error(
        "the compression settings in {path} aren't valid: each `compression` is \"auto\", \"lz4\", \"zstd\" or \"off\", globally or under [machines.<machine id>]. Fix the file, or delete it to use Auto"
    )]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}
