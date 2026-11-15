//! Data that is already compressed, told by its first bytes, so a stream of
//! it skips trying to compress every chunk.

/// File signatures of formats that are compressed already: images, audio
/// and video, archives and compressed streams. Each is (offset, bytes).
const SIGNATURES: &[(usize, &[u8])] = &[
    (0, b"\x89PNG\r\n\x1a\n"),
    (0, b"\xff\xd8\xff"),        // JPEG
    (0, b"GIF87a"),
    (0, b"GIF89a"),
    (8, b"WEBP"),                // RIFF....WEBP
    (4, b"ftyp"),                // MP4, MOV, HEIC, AVIF
    (0, b"\x1a\x45\xdf\xa3"),    // Matroska, WebM
    (0, b"OggS"),
    (0, b"fLaC"),
    (0, b"ID3"),                 // MP3
    (0, b"wOF2"),                // WOFF2
    (0, b"PK\x03\x04"),          // zip, jar, docx, xlsx, apk, ipa
    (0, b"\x1f\x8b"),            // gzip, .tar.gz
    (0, b"\x28\xb5\x2f\xfd"),    // zstd
    (0, b"\xfd7zXZ\x00"),        // xz
    (0, b"BZh"),                 // bzip2
    (0, b"7z\xbc\xaf\x27\x1c"),  // 7-Zip
    (0, b"\x04\x22\x4d\x18"),    // lz4 frame
    (0, b"Rar!\x1a\x07"),
];

/// Whether `start`, the first bytes of a file or stream, names a format
/// that is compressed already.
pub(crate) fn already_compressed(start: &[u8]) -> bool {
    SIGNATURES
        .iter()
        .any(|(at, signature)| start.get(*at..*at + signature.len()) == Some(*signature))
}

#[cfg(test)]
mod tests {
    use super::already_compressed;

    #[test]
    fn images_and_archives_are_recognised() {
        assert!(already_compressed(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"));
        assert!(already_compressed(b"\xff\xd8\xff\xe0\0\x10JFIF"));
        assert!(already_compressed(b"PK\x03\x04\x14\0\0\0"));
        assert!(already_compressed(b"RIFF\x24\0\0\0WEBPVP8 "));
        assert!(already_compressed(b"\0\0\0\x20ftypheic"));
    }

    #[test]
    fn text_and_short_input_are_not() {
        assert!(!already_compressed(b"fn main() {}\n"));
        assert!(!already_compressed(b""));
        assert!(!already_compressed(b"\x1f"));
    }
}
