//! Humata 2's chunks on a link (SPEC TR-11, D56, D65).
//!
//! Anything that can be large moves in chunks of at most [`CHUNK_MAX`]
//! bytes, each compressed on its own and naming its codec in its header, so
//! a stream can change codec between chunks and resume at any chunk's
//! offset. A chunk that doesn't shrink, or one from data known not to (an
//! image, an archive), goes as it is.
//!
//! Which codec a stream uses is [`choose`]n from the user's [`Preference`]
//! (global or per machine, in [`Settings`]), the kind of [`Link`] and the
//! codecs the other end said it supports.
//!
//! At rest, compressed data is a frame ([`encode_framed`],
//! [`decode_framed`]): a binary header naming the codec, the message type
//! and its format version by number, then the bytes (TR-13, D60).

mod chunk;
mod codec;
mod error;
mod frame;
mod policy;
mod settings;
mod sniff;

pub use chunk::{CHUNK_MAX, Chunk, Decoder, Encoder, Header};
pub use codec::Codec;
pub use error::{ChunkError, DecodeFailure, FrameError, SettingsError};
pub use frame::{FRAME_MAGIC, FrameHeader, Framed, decode_framed, encode_framed, peek_framed};
pub use policy::{Link, Preference, SLOW_LINK_BYTES_PER_SECOND, choose, ranked};
pub use settings::{SETTINGS_FILE, Settings};
