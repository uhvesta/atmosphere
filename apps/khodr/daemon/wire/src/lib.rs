//! Khodr's protocol on a byte stream.
//!
//! Each message is one [`pb::Envelope`], preceded by its length as a 4-byte
//! big-endian integer. Both sides send a [`pb::Hello`] first and stop if the
//! other speaks a different protocol version. After that, a [`Client`] sends
//! requests and a [`serve`]d [`Service`] answers each with one response, or
//! with a response and a stream of items.

mod client;
mod error;
mod frame;
mod server;
mod socket;

pub use client::{Client, Stream};
pub use error::{ClientError, WireError};
pub use frame::{MAX_FRAME, read, write};
pub use server::{Reply, Service, ServiceFuture, serve};
pub use socket::{SOCKET_ENV, default_socket};

/// The protocol's messages, generated from `khodr/v1/khodr.proto`.
pub mod pb {
    pub use khodr_proto::khodr::v1::*;
}

use tokio::io::{AsyncRead, AsyncWrite};

/// Whether a request is answered with a stream rather than one response.
pub fn streams(request: &pb::request::Kind) -> bool {
    matches!(request, pb::request::Kind::WatchSession(_))
}

/// Sends our hello and reads the other side's.
pub(crate) async fn handshake<R, W>(
    reader: &mut R,
    writer: &mut W,
    role: pb::Role,
    program_version: &str,
) -> Result<pb::Hello, WireError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let ours = pb::Hello {
        protocol_version: pb::ProtocolVersion::Current.into(),
        role: role.into(),
        program_version: program_version.to_owned(),
    };
    let hello = pb::Envelope {
        id: 0,
        body: Some(pb::envelope::Body::Hello(ours)),
    };
    write(writer, &hello).await?;
    let theirs = match read(reader).await? {
        Some(pb::Envelope {
            body: Some(pb::envelope::Body::Hello(hello)),
            ..
        }) => hello,
        Some(_) => return Err(WireError::NoHello),
        None => return Err(WireError::ClosedBeforeHello),
    };
    let current = i32::from(pb::ProtocolVersion::Current);
    if theirs.protocol_version != current {
        return Err(WireError::VersionMismatch {
            ours: current,
            theirs: theirs.protocol_version,
            their_program: theirs.program_version,
        });
    }
    Ok(theirs)
}

/// An error response.
pub fn error(code: pb::ErrorCode, message: impl Into<String>) -> pb::response::Kind {
    pb::response::Kind::Error(pb::Error {
        code: code.into(),
        message: message.into(),
    })
}

/// An error and its causes on one line, for a person to read.
pub fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

#[cfg(test)]
mod tests;
