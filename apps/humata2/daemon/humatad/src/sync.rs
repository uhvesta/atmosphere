//! The sync marker (MP-15): when Humata takes over a terminal, the shell's
//! own output comes first, so whatever carries the gRPC stream next
//! (`bridge` or `serve --stdio`) prints `HUMATA_SYNC_<nonce>` on a line of
//! its own before it, and the Mac drops every byte up to it.

use tokio::io::{AsyncWrite, AsyncWriteExt};

/// The longest nonce `--sync` takes.
const NONCE_MAX: usize = 64;

/// Checks a `--sync` nonce: 1 to 64 letters and digits, so it can't break
/// the marker's line or need quoting in the shell that typed it.
pub(crate) fn parse_nonce(nonce: &str) -> Result<String, String> {
    if !nonce.is_empty() && nonce.len() <= NONCE_MAX && nonce.bytes().all(|b| b.is_ascii_alphanumeric()) {
        Ok(nonce.to_owned())
    } else {
        Err(format!("a sync nonce is 1 to {NONCE_MAX} letters and digits"))
    }
}

/// Writes the marker for `nonce` and flushes it, so it is out before any
/// byte of the stream.
pub(crate) async fn write_marker(
    output: &mut (impl AsyncWrite + Unpin),
    nonce: &str,
) -> std::io::Result<()> {
    output
        .write_all(format!("HUMATA_SYNC_{nonce}\n").as_bytes())
        .await?;
    output.flush().await
}
