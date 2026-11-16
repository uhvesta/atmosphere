#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "No preferred terminal codec was configured. Supply at least one registered versioned codec ID before saving state."
    )]
    NoPreference,
    #[error(
        "Terminal {what} needs {actual} bytes, above its {limit}-byte limit. Reduce retained state or explicitly configure a compatible bounded policy."
    )]
    Limit {
        what: &'static str,
        actual: u64,
        limit: u64,
    },
    #[error(
        "Terminal compression size arithmetic overflowed. Reduce the supplied state or configured limits before retrying."
    )]
    SizeOverflow,
    #[error(
        "Could not allocate {bytes} bytes for terminal compression. Reduce retained history or close unused terminals; preserve the last valid checkpoint."
    )]
    Allocation {
        bytes: usize,
        #[source]
        source: std::collections::TryReserveError,
    },
    #[error(
        "Terminal compression envelope version {actual} is unsupported. Update host/viewer together or restore a compatible backup."
    )]
    Version { actual: u8 },
    #[error(
        "Terminal codec identifiers need 1–64 ASCII letters, digits, hyphens, underscores or dots. Select a registered versioned codec ID."
    )]
    CodecId,
    #[error(
        "Terminal codec identifier is not UTF-8. Preserve the damaged checkpoint and request or restore a valid copy."
    )]
    CodecUtf8 {
        #[source]
        source: std::str::Utf8Error,
    },
    #[error(
        "Terminal codec {codec} is not registered. Register a compatible codec or select an existing version explicitly."
    )]
    UnknownCodec { codec: String },
    #[error(
        "Terminal codec {codec} is already registered. Use a new versioned ID instead of replacing an existing decoder."
    )]
    DuplicateCodec { codec: String },
    #[error(
        "Terminal compression header is invalid ({reason}). Preserve the checkpoint and request or restore a compatible copy."
    )]
    Header { reason: &'static str },
    #[error(
        "Terminal {what} reported {actual} bytes, expected {expected}. The payload or codec is incompatible; preserve the last valid state and request a fresh checkpoint."
    )]
    Progress {
        what: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error(
        "Terminal {what} checksum is {actual:#010x}, expected {expected:#010x}. Preserve the damaged file and restore a compatible backup or request a fresh checkpoint."
    )]
    Checksum {
        what: &'static str,
        expected: u32,
        actual: u32,
    },
    #[error(
        "LZ4 could not compress the terminal state. Preserve the last checkpoint and reduce history or select a compatible codec."
    )]
    Lz4Encode {
        #[source]
        source: lz4_flex::block::CompressError,
    },
    #[error(
        "LZ4 terminal state is malformed or exceeds its decoded size. Preserve the last valid checkpoint and request a fresh copy."
    )]
    Lz4Decode {
        #[source]
        source: lz4_flex::block::DecompressError,
    },
    #[error(
        "Zlib could not compress the terminal state. Preserve the last checkpoint and reduce history or select a compatible codec."
    )]
    ZlibEncode {
        #[source]
        source: flate2::CompressError,
    },
    #[error(
        "Zlib terminal state is damaged. Preserve the last valid checkpoint and request a fresh copy."
    )]
    ZlibDecode {
        #[source]
        source: flate2::DecompressError,
    },
    #[error(
        "Terminal codec {codec} could not finish {operation} in its bounded buffer. Preserve the last checkpoint and check codec compatibility or retained state limits."
    )]
    Incomplete {
        codec: &'static str,
        operation: &'static str,
        consumed: usize,
        written: usize,
    },
    #[error(
        "Terminal codec {codec} failed to {operation}. Preserve the last valid state and repair or replace this explicitly registered codec."
    )]
    Plugin {
        codec: String,
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
}
