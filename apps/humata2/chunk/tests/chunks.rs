//! Chunks round-trip with every codec, skip compression when it doesn't
//! pay, and refuse damaged input with an error.

use humata2_chunk::{CHUNK_MAX, Chunk, ChunkError, Codec, Decoder, Encoder, Header, Link, Preference, choose};

/// Text that compresses well: source-like lines.
fn text(len: usize) -> Vec<u8> {
    let line = b"fn transfer(offset: u64, codec: Codec) -> Result<Chunk, ChunkError> { todo() }\n";
    line.iter().copied().cycle().take(len).collect()
}

/// Bytes no codec shrinks: a xorshift stream.
fn random(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

fn round_trip(codec: Codec, raw: &[u8]) -> Chunk {
    let chunk = Encoder::new(codec).encode(raw.to_vec()).expect("encodes");
    let decoded = Decoder::new()
        .decode(chunk.header, chunk.data.clone())
        .expect("decodes");
    assert_eq!(decoded, raw, "{codec} round trip");
    chunk
}

#[test]
fn every_codec_round_trips_and_names_itself() {
    let raw = text(CHUNK_MAX);
    for codec in [Codec::Lz4, Codec::Zstd] {
        let chunk = round_trip(codec, &raw);
        assert_eq!(chunk.header.codec, codec);
        assert!(chunk.data.len() < raw.len() / 4, "{codec} shrinks text");
    }
    let chunk = round_trip(Codec::None, &raw);
    assert_eq!(chunk.header.codec, Codec::None);
    assert_eq!(chunk.data, raw);
}

#[test]
fn empty_and_tiny_chunks_round_trip_as_they_are() {
    for raw in [Vec::new(), b"x".to_vec(), text(100)] {
        for codec in Codec::ALL {
            let chunk = round_trip(codec, &raw);
            assert_eq!(chunk.header.codec, Codec::None);
        }
    }
}

#[test]
fn a_chunk_that_does_not_shrink_is_sent_raw() {
    let raw = random(256 << 10, 7);
    for codec in [Codec::Lz4, Codec::Zstd] {
        let chunk = round_trip(codec, &raw);
        assert_eq!(chunk.header.codec, Codec::None, "{codec} on random data");
        assert_eq!(chunk.data, raw);
    }
}

#[test]
fn a_stream_that_starts_like_an_image_is_sent_raw_throughout() {
    // A PNG signature, then bytes that would compress: the signature wins,
    // so no chunk of the stream is tried.
    let mut first = b"\x89PNG\r\n\x1a\n".to_vec();
    first.extend(text(64 << 10));
    let mut encoder = Encoder::new(Codec::Zstd);
    let chunks = [first, text(64 << 10)].map(|raw| encoder.encode(raw).expect("encodes"));
    assert!(chunks.iter().all(|chunk| chunk.header.codec == Codec::None));
}

#[test]
fn data_that_turns_compressible_gets_compressed_again() {
    let mut encoder = Encoder::new(Codec::Lz4);
    for seed in 0..8 {
        let chunk = encoder.encode(random(64 << 10, seed + 1)).expect("encodes");
        assert_eq!(chunk.header.codec, Codec::None);
    }
    // After backing off, the encoder still retries now and then.
    let compressed = (0..16)
        .map(|_| encoder.encode(text(64 << 10)).expect("encodes"))
        .filter(|chunk| chunk.header.codec == Codec::Lz4)
        .count();
    assert!(compressed >= 8, "only {compressed} of 16 text chunks compressed");
}

#[test]
fn a_chunk_over_the_limit_is_refused() {
    let error = Encoder::new(Codec::Lz4).encode(vec![0; CHUNK_MAX + 1]);
    assert!(matches!(error, Err(ChunkError::TooBig { len }) if len == CHUNK_MAX + 1));
}

#[test]
fn a_damaged_chunk_is_an_error_not_a_panic_or_garbage() {
    let raw = text(CHUNK_MAX);
    for codec in [Codec::Lz4, Codec::Zstd, Codec::None] {
        let chunk = Encoder::new(codec).encode(raw.clone()).expect("encodes");
        // Flip one byte at many places, and cut the chunk short.
        let mut damaged: Vec<Vec<u8>> = (0..64)
            .map(|step| {
                let mut data = chunk.data.clone();
                let at = step * data.len() / 64;
                data[at] ^= 0x5a;
                data
            })
            .collect();
        damaged.push(chunk.data[..chunk.data.len() / 2].to_vec());
        damaged.push(Vec::new());
        for data in damaged {
            match Decoder::new().decode(chunk.header, data) {
                Err(_) => {}
                Ok(bytes) => panic!("{codec}: damaged chunk decoded to {} bytes", bytes.len()),
            }
        }
    }
}

#[test]
fn a_header_that_lies_about_the_size_is_an_error() {
    let chunk = Encoder::new(Codec::Lz4).encode(text(64 << 10)).expect("encodes");
    let mut decoder = Decoder::new();
    let short = Header {
        raw_len: chunk.header.raw_len - 1,
        ..chunk.header
    };
    assert!(decoder.decode(short, chunk.data.clone()).is_err());
    let huge = Header {
        raw_len: u32::MAX,
        ..chunk.header
    };
    assert!(matches!(decoder.decode(huge, chunk.data.clone()), Err(ChunkError::SizeOutOfRange { .. })));
    let unchecked = Header {
        crc32: None,
        ..chunk.header
    };
    assert!(matches!(decoder.decode(unchecked, chunk.data), Err(ChunkError::NoChecksum { .. })));
}

#[test]
fn an_unknown_codec_number_is_not_a_codec() {
    assert_eq!(Codec::from_wire(9), None);
    assert_eq!(Codec::known(&[9, 2, -1, 0]), vec![Codec::Zstd, Codec::None]);
    for codec in Codec::ALL {
        assert_eq!(Codec::from_wire(codec.wire()), Some(codec));
    }
}

#[test]
fn a_peer_that_supports_only_none_still_gets_the_data() {
    // An old peer lists nothing, or only None: the sender picks None and
    // the chunks it sends need no decoder at all.
    let raw = text(3 * CHUNK_MAX / 2);
    for peer in [&[][..], &[Codec::None][..]] {
        let codec = choose(Preference::Zstd, Link::Local, peer);
        assert_eq!(codec, Codec::None);
        let mut encoder = Encoder::new(codec);
        let mut received = Vec::new();
        for piece in raw.chunks(CHUNK_MAX) {
            let chunk = encoder.encode(piece.to_vec()).expect("encodes");
            assert_eq!(chunk.header.codec, Codec::None);
            // What an old peer does: take the bytes as they are.
            received.extend_from_slice(&chunk.data);
        }
        assert_eq!(received, raw);
    }
    // And a chunk from an old peer, with no checksum, still decodes.
    let old = Header {
        codec: Codec::None,
        raw_len: 5,
        crc32: None,
    };
    assert_eq!(Decoder::new().decode(old, b"hello".to_vec()).expect("decodes"), b"hello");
}

/// One chunk on the wire: where it starts in the file, and the chunk.
struct Sent {
    offset: u64,
    chunk: Chunk,
}

fn send(file: &[u8], from: u64, codec: Codec, stop_after: Option<usize>) -> Vec<Sent> {
    let mut encoder = Encoder::new(codec);
    let mut sent = Vec::new();
    let mut offset = from;
    while (offset as usize) < file.len() && stop_after.is_none_or(|stop| sent.len() < stop) {
        let end = (offset as usize + CHUNK_MAX).min(file.len());
        let chunk = encoder.encode(file[offset as usize..end].to_vec()).expect("encodes");
        sent.push(Sent { offset, chunk });
        offset = end as u64;
    }
    sent
}

#[test]
fn a_transfer_resumed_mid_file_with_another_codec_is_byte_identical() {
    // Two chunks of text, one of random data, then a short one of text.
    let mut file = text(2 * CHUNK_MAX);
    file.extend(random(CHUNK_MAX, 3));
    file.extend(text(CHUNK_MAX / 3));

    for (before, after) in [
        (Codec::Lz4, Codec::Zstd),
        (Codec::Zstd, Codec::Lz4),
        (Codec::Lz4, Codec::None),
        (Codec::None, Codec::Zstd),
    ] {
        // The link drops after two chunks; the receiver kept what it
        // decoded and resumes from its offset (TR-4) with another codec.
        let mut received = Vec::new();
        let mut decoder = Decoder::new();
        for sent in send(&file, 0, before, Some(2)) {
            assert_eq!(sent.offset, received.len() as u64);
            received.extend(decoder.decode(sent.chunk.header, sent.chunk.data).expect("decodes"));
        }
        let resume_at = received.len() as u64;
        let mut decoder = Decoder::new();
        let mut codecs = Vec::new();
        for sent in send(&file, resume_at, after, None) {
            assert_eq!(sent.offset, received.len() as u64, "gapless after resuming");
            codecs.push(sent.chunk.header.codec);
            received.extend(decoder.decode(sent.chunk.header, sent.chunk.data).expect("decodes"));
        }
        assert_eq!(received, file, "{before} then {after}");
        if after != Codec::None {
            assert!(codecs.contains(&after), "{after} used after resuming: {codecs:?}");
            assert!(codecs.contains(&Codec::None), "the random chunk went raw: {codecs:?}");
        }
    }
}

#[test]
fn one_decoder_takes_chunks_of_any_codec_in_any_order() {
    let mut decoder = Decoder::new();
    let raw = text(200 << 10);
    for codec in [Codec::Zstd, Codec::Lz4, Codec::None, Codec::Zstd, Codec::Lz4] {
        let chunk = Encoder::new(codec).encode(raw.clone()).expect("encodes");
        assert_eq!(decoder.decode(chunk.header, chunk.data).expect("decodes"), raw);
    }
}
