use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::WireError;
use crate::pb;

/// The largest envelope either side accepts: room for a long session's
/// whole history in one response.
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

/// The length prefix's top bit, set when the body is compressed with zstd.
const COMPRESSED: u32 = 1 << 31;

/// Bodies at least this long are compressed for a peer that reads zstd.
/// Shorter ones rarely shrink enough to pay for it.
const COMPRESS_FROM: usize = 1024;

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
    if compressed {
        body = decompress(&body)?;
    }
    pb::Envelope::decode(body.as_slice())
        .map(Some)
        .map_err(|source| WireError::Decode { source })
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
    let mut body = envelope.encode_to_vec();
    let mut flag = 0;
    if compress && body.len() >= COMPRESS_FROM {
        let smaller = ruzstd::encoding::compress_to_vec(
            body.as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        if smaller.len() < body.len() {
            body = smaller;
            flag = COMPRESSED;
        }
    }
    let length = body.len();
    let prefix = u32::try_from(length)
        .ok()
        .filter(|_| length <= MAX_FRAME)
        .ok_or(WireError::TooLarge {
            length,
            max: MAX_FRAME,
        })?
        | flag;
    let mut frame = Vec::with_capacity(4 + length);
    frame.extend_from_slice(&prefix.to_be_bytes());
    frame.extend_from_slice(&body);
    writer
        .write_all(&frame)
        .await
        .map_err(|source| WireError::Write { source })?;
    writer
        .flush()
        .await
        .map_err(|source| WireError::Write { source })
}
