//! Release bench of each codec on 1 MiB chunks: throughput both ways and
//! the compression ratio, on prose, source code and random data (D65).
//!
//! `bazel run -c opt //apps/humata2/chunk:codecs_bench` reads its prose
//! (every .md) and code (every .rs) from apps/humata2 in the workspace;
//! under `bazel test` it falls back to built-in samples.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use humata2_chunk::{CHUNK_MAX, Codec, Decoder, Encoder};

/// Each measurement repeats until it has run this long.
const MEASURE_FOR: Duration = Duration::from_millis(400);

fn files(root: &Path, extension: &str, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let mut entries: Vec<PathBuf> = entries.filter_map(|entry| entry.ok().map(|entry| entry.path())).collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
        if name.starts_with('.') || name == "target" || name == "node_modules" || name.starts_with("bazel-") {
            continue;
        }
        if path.is_dir() {
            files(&path, extension, into);
        } else if path.extension().and_then(|found| found.to_str()) == Some(extension) {
            into.push(path);
        }
    }
}

/// Up to `max` bytes of every file with `extension` under the workspace's
/// apps/humata2, or `fallback` repeated when not run from a workspace.
fn corpus(extension: &str, max: usize, fallback: &[u8]) -> (Vec<u8>, String) {
    if let Some(workspace) = std::env::var_os("BUILD_WORKSPACE_DIRECTORY") {
        let root = PathBuf::from(workspace).join("apps/humata2");
        let mut found = Vec::new();
        files(&root, extension, &mut found);
        let mut bytes = Vec::new();
        for path in &found {
            if bytes.len() >= max {
                break;
            }
            if let Ok(mut file) = std::fs::File::open(path) {
                let _ = file.read_to_end(&mut bytes);
            }
        }
        bytes.truncate(max);
        if !bytes.is_empty() {
            return (bytes, format!("{} .{extension} files", found.len()));
        }
    }
    let bytes = fallback.iter().copied().cycle().take(max.min(4 << 20)).collect();
    (bytes, "built-in sample, repeated".to_owned())
}

fn random(len: usize) -> Vec<u8> {
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 24) as u8
        })
        .collect()
}

/// Runs `work` over every chunk until `MEASURE_FOR` has passed; returns
/// MB/s of raw bytes.
fn throughput(chunks: &[Vec<u8>], mut work: impl FnMut(usize) -> usize) -> f64 {
    let start = Instant::now();
    let mut bytes = 0usize;
    let mut sink = 0usize;
    while start.elapsed() < MEASURE_FOR {
        for (index, chunk) in chunks.iter().enumerate() {
            sink = sink.wrapping_add(work(index));
            bytes += chunk.len();
        }
    }
    std::hint::black_box(sink);
    bytes as f64 / start.elapsed().as_secs_f64() / 1e6
}

struct Row {
    name: &'static str,
    encode: f64,
    decode: f64,
    ratio: f64,
}

fn measure(
    name: &'static str,
    chunks: &[Vec<u8>],
    mut encode: impl FnMut(&[u8]) -> Vec<u8>,
    mut decode: impl FnMut(&[u8], usize) -> Vec<u8>,
) -> Row {
    let encoded: Vec<Vec<u8>> = chunks.iter().map(|chunk| encode(chunk)).collect();
    for (chunk, packed) in chunks.iter().zip(&encoded) {
        assert_eq!(&decode(packed, chunk.len()), chunk, "{name} round trip");
    }
    let raw: usize = chunks.iter().map(Vec::len).sum();
    let packed: usize = encoded.iter().map(Vec::len).sum();
    Row {
        name,
        encode: throughput(chunks, |index| encode(&chunks[index]).len()),
        decode: throughput(chunks, |index| decode(&encoded[index], chunks[index].len()).len()),
        ratio: raw as f64 / packed as f64,
    }
}

fn zstd_c(level: i32) -> impl FnMut(&[u8]) -> Vec<u8> {
    let mut compressor = zstd::bulk::Compressor::new(level).expect("a zstd compressor");
    move |raw| compressor.compress(raw).expect("zstd compresses")
}

fn zstd_c_decode() -> impl FnMut(&[u8], usize) -> Vec<u8> {
    let mut decompressor = zstd::bulk::Decompressor::new().expect("a zstd decompressor");
    move |packed, len| decompressor.decompress(packed, len).expect("zstd decompresses")
}

fn ruzstd_decode() -> impl FnMut(&[u8], usize) -> Vec<u8> {
    let mut decoder = ruzstd::decoding::FrameDecoder::new();
    move |packed, len| {
        let mut out = Vec::with_capacity(len);
        decoder.decode_all_to_vec(packed, &mut out).expect("ruzstd decodes");
        out
    }
}

fn rows(chunks: &[Vec<u8>]) -> Vec<Row> {
    let mut rows = vec![
        measure(
            "lz4_flex block",
            chunks,
            lz4_flex::block::compress,
            |packed, len| lz4_flex::block::decompress(packed, len).expect("lz4 decompresses"),
        ),
        measure("zstd (C) level 1", chunks, zstd_c(1), zstd_c_decode()),
        measure("zstd (C) level 3", chunks, zstd_c(3), zstd_c_decode()),
        measure("zstd (C) level 6", chunks, zstd_c(6), zstd_c_decode()),
        measure(
            "ruzstd Fastest",
            chunks,
            |raw| ruzstd::encoding::compress_to_vec(raw, ruzstd::encoding::CompressionLevel::Fastest),
            ruzstd_decode(),
        ),
        measure("zstd (C) 3, ruzstd decode", chunks, zstd_c(3), ruzstd_decode()),
    ];
    // The whole path a stream takes: the CRC, the skip rules and the
    // header, through this crate's Encoder and Decoder.
    for (name, codec) in [
        ("humata2_chunk lz4", Codec::Lz4),
        ("humata2_chunk zstd", Codec::Zstd),
        ("humata2_chunk none", Codec::None),
    ] {
        let mut encoder = Encoder::new(codec);
        let mut decoder = Decoder::new();
        let encoded: Vec<_> = chunks
            .iter()
            .map(|chunk| encoder.encode(chunk.clone()).expect("encodes"))
            .collect();
        let raw: usize = chunks.iter().map(Vec::len).sum();
        let packed: usize = encoded.iter().map(|chunk| chunk.data.len()).sum();
        let mut fresh = Encoder::new(codec);
        let encode = throughput(chunks, |at| {
            fresh.encode(chunks[at].clone()).expect("encodes").data.len()
        });
        let decode = throughput(chunks, |at| {
            let chunk = &encoded[at];
            decoder.decode(chunk.header, chunk.data.clone()).expect("decodes").len()
        });
        rows.push(Row {
            name,
            encode,
            decode,
            ratio: raw as f64 / packed as f64,
        });
    }
    rows
}

#[test]
fn codecs_bench() {
    let prose_sample: &[u8] = include_bytes!("../src/lib.rs");
    let code_sample: &[u8] = include_bytes!("../src/chunk.rs");
    let (prose, prose_from) = corpus("md", 8 << 20, prose_sample);
    let (code, code_from) = corpus("rs", 8 << 20, code_sample);
    let noise = random(8 << 20);
    for (label, data, from) in [
        ("prose", prose, prose_from),
        ("source code", code, code_from),
        ("random", noise, "xorshift".to_owned()),
    ] {
        let chunks: Vec<Vec<u8>> = data.chunks(CHUNK_MAX).map(<[u8]>::to_vec).collect();
        println!(
            "\n{label}: {:.1} MiB in {} chunks ({from})",
            data.len() as f64 / f64::from(1 << 20),
            chunks.len()
        );
        println!("{:<28} {:>12} {:>12} {:>8}", "codec", "encode MB/s", "decode MB/s", "ratio");
        for row in rows(&chunks) {
            println!("{:<28} {:>12.0} {:>12.0} {:>8.2}", row.name, row.encode, row.decode, row.ratio);
        }
    }
}
