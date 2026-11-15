//! The codecs, and their one-byte ids at rest and on the wire
//! (`humata.v1.ChunkCodec`).

use std::fmt;

/// How a chunk's bytes are encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    /// As they are. What a peer that names no codec sends and expects.
    None,
    /// LZ4 block format: fast both ways, the default locally and on fast
    /// links.
    Lz4,
    /// A Zstandard frame: smaller, the default on slow links.
    Zstd,
}

impl Codec {
    /// Every codec this build can encode and decode, best first.
    pub const ALL: [Codec; 3] = [Codec::Lz4, Codec::Zstd, Codec::None];

    /// Its one-byte id, the same at rest (in a frame) and on the wire (as
    /// `humata.v1.ChunkCodec`). An id is never reused for another codec.
    pub fn id(self) -> u8 {
        match self {
            Codec::None => 0,
            Codec::Lz4 => 1,
            Codec::Zstd => 2,
        }
    }

    /// The codec an id names, if this build knows it.
    pub fn from_id(id: u8) -> Option<Codec> {
        match id {
            0 => Some(Codec::None),
            1 => Some(Codec::Lz4),
            2 => Some(Codec::Zstd),
            _ => None,
        }
    }

    /// Its `humata.v1.ChunkCodec` number: its id.
    pub fn wire(self) -> i32 {
        i32::from(self.id())
    }

    /// The codec a `humata.v1.ChunkCodec` number names, if this build
    /// knows it.
    pub fn from_wire(wire: i32) -> Option<Codec> {
        u8::try_from(wire).ok().and_then(Codec::from_id)
    }

    /// The codecs in a peer's list that this build knows, in its order.
    /// A newer peer may list codecs this build has never heard of; they are
    /// left out rather than refused.
    pub fn known(wire: &[i32]) -> Vec<Codec> {
        wire.iter().filter_map(|&value| Codec::from_wire(value)).collect()
    }

    /// The list to send a peer: this build's codecs, as numbers.
    pub fn wire_list(codecs: &[Codec]) -> Vec<i32> {
        codecs.iter().map(|codec| codec.wire()).collect()
    }
}

impl fmt::Display for Codec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Codec::None => "uncompressed",
            Codec::Lz4 => "lz4",
            Codec::Zstd => "zstd",
        })
    }
}
