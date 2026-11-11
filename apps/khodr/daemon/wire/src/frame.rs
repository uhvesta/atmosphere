use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::WireError;
use crate::pb;

/// The largest envelope either side accepts: room for a long session's
/// whole history in one response.
pub const MAX_FRAME: usize = 64 * 1024 * 1024;

/// The next envelope, or None if the stream ended between envelopes.
pub async fn read<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Option<pb::Envelope>, WireError> {
    let mut length = [0_u8; 4];
    match reader.read_exact(&mut length).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(source) => return Err(WireError::Read { source }),
    }
    let length = u32::from_be_bytes(length) as usize;
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
    pb::Envelope::decode(body.as_slice())
        .map(Some)
        .map_err(|source| WireError::Decode { source })
}

pub async fn write<W: AsyncWrite + Unpin>(
    writer: &mut W,
    envelope: &pb::Envelope,
) -> Result<(), WireError> {
    let body = envelope.encode_to_vec();
    let length = body.len();
    let prefix = u32::try_from(length)
        .ok()
        .filter(|_| length <= MAX_FRAME)
        .ok_or(WireError::TooLarge {
            length,
            max: MAX_FRAME,
        })?;
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
