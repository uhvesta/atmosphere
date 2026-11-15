//! Standard base64, only as much as host keys need: decoding a key from
//! `known_hosts` and printing a fingerprint the way OpenSSH does.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The bytes `text` encodes, with or without `=` padding; none when it
/// isn't base64.
pub(crate) fn decode(text: &str) -> Option<Vec<u8>> {
    let text = text.strip_suffix("==").or_else(|| text.strip_suffix('=')).unwrap_or(text);
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = ALPHABET.iter().position(|&a| a == byte)?;
        acc = (acc << 6) | u32::try_from(value).ok()?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
            acc &= (1 << bits) - 1;
        }
    }
    // A lone final character, or stray bits, isn't anything an encoder
    // writes.
    (bits < 6 && acc == 0).then_some(out)
}

/// `bytes` as base64 without padding, as in `SHA256:…` fingerprints.
pub(crate) fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .fold(0u32, |acc, &byte| (acc << 8) | u32::from(byte))
            << (8 * (3 - chunk.len()));
        for index in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(n >> (18 - 6 * index)) as usize & 63]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_length() {
        let bytes: Vec<u8> = (0..=255u8).collect();
        for len in 0..40 {
            let encoded = encode(&bytes[..len]);
            assert_eq!(decode(&encoded).as_deref(), Some(&bytes[..len]), "{len}");
        }
    }

    #[test]
    fn reads_padded_text() {
        assert_eq!(decode("aGk=").as_deref(), Some(&b"hi"[..]));
        assert_eq!(decode("aA==").as_deref(), Some(&b"h"[..]));
        assert_eq!(encode(b"hi"), "aGk");
    }

    #[test]
    fn refuses_what_isnt_base64() {
        assert_eq!(decode("a"), None);
        assert_eq!(decode("ab$c"), None);
        assert_eq!(decode("aGl="), None);
    }
}
