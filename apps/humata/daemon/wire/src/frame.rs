use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::WireError;
use crate::pb;

/// The largest envelope either side accepts. Both sides are humatad or the
/// app, so this only stops a garbled length from taking all memory; it is
/// room for a long session's whole history or backup, with the code it
/// committed, in one response.
pub const MAX_FRAME: usize = 1024 * 1024 * 1024;

/// The length prefix's top bit, set when the body is compressed with zstd.
const COMPRESSED: u32 = 1 << 31;

/// Bodies at least this long are compressed for a peer that reads zstd.
/// Shorter ones rarely shrink enough to pay for it.
const COMPRESS_FROM: usize = 1024;

/// Bodies at least this long are compressed, and decompressed and decoded,
/// on a thread that may block, so a long history or backup doesn't hold up
/// the executor. Shorter ones take less time than handing them over.
const APART_FROM: usize = 64 * 1024;

/// The next envelope, or None if the stream ended between envelopes.
pub async fn read<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<pb::Envelope>, WireError> {
    let mut length = [0_u8; 4];
    match reader.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(source) => return Err(WireError::Read { source }),
    }
    let prefix = u32::from_be_bytes(length);
    let compressed = prefix & COMPRESSED != 0;
    let length = (prefix & !COMPRESSED) as usize;
    if length > MAX_FRAME {
        return Err(WireError::TooLarge {
            length,
            max: MAX_FRAME,
        });
    }
    let mut body = vec![0_u8; length];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|source| WireError::Truncated { length, source })?;
    let envelope = if length >= APART_FROM {
        tokio::task::spawn_blocking(move || decode(&body, compressed))
            .await
            .map_err(|source| WireError::Stopped { source })??
    } else {
        decode(&body, compressed)?
    };
    Ok(Some(envelope))
}

/// The envelope in a frame's `body`.
fn decode(body: &[u8], compressed: bool) -> Result<pb::Envelope, WireError> {
    let decoded = if compressed {
        pb::Envelope::decode(decompress(body)?.as_slice())
    } else {
        pb::Envelope::decode(body)
    };
    decoded.map_err(|source| WireError::Decode { source })
}

/// The body of a compressed envelope, refusing one that would grow past
/// [`MAX_FRAME`].
fn decompress(body: &[u8]) -> Result<Vec<u8>, WireError> {
    use std::io::Read;
    let decoder =
        ruzstd::decoding::StreamingDecoder::new(body).map_err(|source| WireError::Decompress {
            source: std::io::Error::other(source),
        })?;
    let mut plain = Vec::new();
    decoder
        .take(MAX_FRAME as u64 + 1)
        .read_to_end(&mut plain)
        .map_err(|source| WireError::Decompress { source })?;
    if plain.len() > MAX_FRAME {
        return Err(WireError::TooLarge {
            length: plain.len(),
            max: MAX_FRAME,
        });
    }
    Ok(plain)
}

/// Writes `envelope`, compressed with zstd if `compress` and it is long
/// enough to gain from it.
pub async fn write<W: AsyncWrite + Unpin>(
    writer: &mut W,
    envelope: &pb::Envelope,
    compress: bool,
) -> Result<(), WireError> {
    let body = envelope.encode_to_vec();
    let apart = body.len() >= APART_FROM;
    let (body, flag) = if !compress || body.len() < COMPRESS_FROM {
        (body, 0)
    } else if apart {
        tokio::task::spawn_blocking(move || shrunk(body))
            .await
            .map_err(|source| WireError::Stopped { source })?
    } else {
        shrunk(body)
    };
    let length = body.len();
    let prefix = u32::try_from(length)
        .ok()
        .filter(|_| length <= MAX_FRAME)
        .ok_or(WireError::TooLarge {
            length,
            max: MAX_FRAME,
        })?
        | flag;
    let written = if apart {
        // Not copied once more to put the prefix before it.
        match writer.write_all(&prefix.to_be_bytes()).await {
            Ok(()) => writer.write_all(&body).await,
            Err(error) => Err(error),
        }
    } else {
        let mut frame = Vec::with_capacity(4 + length);
        frame.extend_from_slice(&prefix.to_be_bytes());
        frame.extend_from_slice(&body);
        writer.write_all(&frame).await
    };
    written.map_err(|source| WireError::Write { source })?;
    writer
        .flush()
        .await
        .map_err(|source| WireError::Write { source })
}

/// `body` compressed with zstd, and the prefix's flag that says so; or as
/// it is, with no flag, if that isn't smaller.
fn shrunk(body: Vec<u8>) -> (Vec<u8>, u32) {
    let smaller = ruzstd::encoding::compress_to_vec(
        body.as_slice(),
        ruzstd::encoding::CompressionLevel::Fastest,
    );
    if smaller.len() < body.len() {
        (smaller, COMPRESSED)
    } else {
        (body, 0)
    }
}
