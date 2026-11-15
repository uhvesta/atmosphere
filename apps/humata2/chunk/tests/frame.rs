//! Frames (compressed data at rest) round-trip with every codec, name
//! their codec and message type, and refuse what they can't read.

use humata2_chunk::{Codec, FRAME_MAGIC, FrameError, decode_framed, encode_framed, peek_framed};

const KIND: u16 = 7;
const MAX: u64 = 64 << 20;

fn text(len: usize) -> Vec<u8> {
    let line = b"message TerminalSnapshot { uint64 offset = 1; bytes vt = 2; }\n";
    line.iter().copied().cycle().take(len).collect()
}

fn random(len: usize) -> Vec<u8> {
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

#[test]
fn every_codec_round_trips_and_the_header_names_it() {
    // Frames at rest are not limited to one chunk.
    let raw = text(3 << 20);
    for codec in Codec::ALL {
        let framed = encode_framed(codec, KIND, 3, &raw).expect("encodes");
        let decoded = decode_framed(&framed, "test frame", MAX).expect("decodes");
        assert_eq!(decoded.data, raw, "{codec}");
        assert_eq!(decoded.header.codec, codec);
        assert_eq!(decoded.header.kind, KIND);
        assert_eq!(decoded.header.version, 3);
        assert_eq!(decoded.header.raw_len, raw.len() as u64);
        if codec != Codec::None {
            assert!(framed.len() < raw.len() / 4, "{codec} shrinks text");
        }
        assert_eq!(peek_framed(&framed, "test frame").expect("a header"), decoded.header);
    }
}

#[test]
fn empty_and_incompressible_data_is_stored_as_it_is() {
    for raw in [Vec::new(), b"x".to_vec(), random(256 << 10)] {
        for codec in Codec::ALL {
            let framed = encode_framed(codec, KIND, 1, &raw).expect("encodes");
            let decoded = decode_framed(&framed, "test frame", MAX).expect("decodes");
            assert_eq!(decoded.data, raw);
            assert_eq!(decoded.header.codec, Codec::None);
        }
    }
}

#[test]
fn an_unknown_codec_is_refused_and_named() {
    let mut framed = encode_framed(Codec::Lz4, KIND, 1, &text(4096)).expect("encodes");
    framed[5] = 9;
    let error = decode_framed(&framed, "blobs/ab/cdef", MAX).expect_err("refused");
    assert!(
        matches!(&error, FrameError::UnknownCodec { origin, id: 9 } if origin == "blobs/ab/cdef"),
        "{error:?}"
    );
}

#[test]
fn a_newer_header_format_is_refused() {
    let mut framed = encode_framed(Codec::Zstd, KIND, 1, &text(4096)).expect("encodes");
    framed[4] = 2;
    assert!(matches!(
        decode_framed(&framed, "f", MAX),
        Err(FrameError::UnknownFormat { format: 2, .. })
    ));
}

#[test]
fn a_truncated_header_or_other_data_is_an_error() {
    let framed = encode_framed(Codec::Zstd, KIND, 1, &text(4096)).expect("encodes");
    for cut in [4, 5, 12, 21] {
        assert!(
            matches!(decode_framed(&framed[..cut], "f", MAX), Err(FrameError::Truncated { .. })),
            "cut at {cut}"
        );
    }
    for other in [&b""[..], b"\x89H2", b"{\"json\": true}", b"\x28\xb5\x2f\xfd plain zstd"] {
        assert!(matches!(decode_framed(other, "f", MAX), Err(FrameError::NotAFrame { .. })));
    }
    assert_eq!(&framed[..4], &FRAME_MAGIC);
}

#[test]
fn a_damaged_body_is_an_error_not_garbage() {
    let raw = text(1 << 20);
    for codec in Codec::ALL {
        let framed = encode_framed(codec, KIND, 1, &raw).expect("encodes");
        let mut damaged: Vec<Vec<u8>> = (0..32)
            .map(|step| {
                let mut bytes = framed.clone();
                let at = 22 + step * (bytes.len() - 22) / 32;
                bytes[at] ^= 0x5a;
                bytes
            })
            .collect();
        damaged.push(framed[..framed.len() - 1].to_vec());
        damaged.push(framed[..22].to_vec());
        for bytes in damaged {
            assert!(decode_framed(&bytes, "f", MAX).is_err(), "{codec}: damaged frame decoded");
        }
    }
}

#[test]
fn a_frame_over_the_readers_limit_is_refused() {
    let framed = encode_framed(Codec::Lz4, KIND, 1, &text(1 << 20)).expect("encodes");
    assert!(matches!(
        decode_framed(&framed, "f", 1 << 10),
        Err(FrameError::TooLarge { claimed, .. }) if claimed == 1 << 20
    ));
    let mut huge = framed;
    huge[10..18].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(matches!(decode_framed(&huge, "f", MAX), Err(FrameError::TooLarge { .. })));
}
