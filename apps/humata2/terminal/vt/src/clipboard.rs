//! What a program asks of the clipboard through OSC 52, and the answer the
//! host sends back. The terminal never reads or writes a clipboard itself:
//! it raises events and the host, which knows whether the session is local
//! or remote and what the user allowed, decides.

use crate::ffi;

/// Which clipboard a program named in OSC 52.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipboardTarget {
    /// The system clipboard (`c`).
    Standard,
    /// The selection clipboard (`s`), which X11 keeps apart from the
    /// primary selection.
    Selection,
    /// The primary selection (`p`), filled by selecting text on X11 and
    /// Wayland.
    Primary,
}

impl ClipboardTarget {
    pub(crate) fn from_location(location: ffi::GhosttyClipboardLocation) -> Self {
        match location {
            ffi::CLIPBOARD_LOCATION_SELECTION => ClipboardTarget::Selection,
            ffi::CLIPBOARD_LOCATION_PRIMARY => ClipboardTarget::Primary,
            _ => ClipboardTarget::Standard,
        }
    }

    fn letter(self) -> u8 {
        match self {
            ClipboardTarget::Standard => b'c',
            ClipboardTarget::Selection => b's',
            ClipboardTarget::Primary => b'p',
        }
    }
}

/// The OSC 52 answer to a program that asked to read `target`, to write to
/// the pty: `text` when the user allowed it, or an empty clipboard when
/// they refused (`None`), which is what a program sees from a terminal that
/// doesn't share its clipboard.
pub fn clipboard_read_reply(target: ClipboardTarget, text: Option<&str>) -> Vec<u8> {
    let mut reply = b"\x1b]52;".to_vec();
    reply.push(target.letter());
    reply.push(b';');
    base64_into(text.unwrap_or_default().as_bytes(), &mut reply);
    reply.extend_from_slice(b"\x1b\\");
    reply
}

/// Appends `bytes` in standard base64 with padding, as OSC 52 carries it.
fn base64_into(bytes: &[u8], out: &mut Vec<u8>) {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let n = (u32::from(b0) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        out.push(ALPHABET[(n >> 18) as usize & 63]);
        out.push(ALPHABET[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63]
        } else {
            b'='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63]
        } else {
            b'='
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base64(bytes: &[u8]) -> String {
        let mut out = Vec::new();
        base64_into(bytes, &mut out);
        String::from_utf8(out).unwrap_or_default()
    }

    #[test]
    fn base64_matches_the_rfc_4648_examples() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
