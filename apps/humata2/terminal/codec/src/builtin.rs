use crate::{Codec, Error, LZ4, Progress, ZLIB};
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress, Status};

pub(super) fn codecs() -> Vec<Box<dyn Codec>> {
    vec![Box::new(Lz4), Box::new(Zlib)]
}
struct Lz4;
impl Codec for Lz4 {
    fn id(&self) -> &str {
        LZ4
    }
    fn max_encoded_len(&self, raw_len: usize) -> Result<usize, Error> {
        // Guard the upstream bound calculation's multiplication as well as
        // the output allocation, on native and wasm32 targets alike.
        raw_len.checked_mul(110).ok_or(Error::SizeOverflow)?;
        Ok(lz4_flex::block::get_maximum_output_size(raw_len))
    }
    fn encode_into(&self, raw: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        lz4_flex::block::compress_into(raw, output).map_err(|source| Error::Lz4Encode { source })
    }
    fn decode_into(&self, encoded: &[u8], output: &mut [u8]) -> Result<Progress, Error> {
        let written = lz4_flex::block::decompress_into(encoded, output)
            .map_err(|source| Error::Lz4Decode { source })?;
        // The pinned checked/safe block decoder succeeds only after its
        // input cursor reaches the end of the supplied slice.
        Ok(Progress {
            consumed: encoded.len(),
            written,
        })
    }
}
struct Zlib;
impl Codec for Zlib {
    fn id(&self) -> &str {
        ZLIB
    }
    fn max_encoded_len(&self, raw_len: usize) -> Result<usize, Error> {
        raw_len
            .checked_mul(2)
            .and_then(|n| n.checked_add(64))
            .ok_or(Error::SizeOverflow)
    }
    fn encode_into(&self, raw: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let mut codec = Compress::new(Compression::fast(), true);
        loop {
            let before = (codec.total_in() as usize, codec.total_out() as usize);
            let status = codec
                .compress(
                    &raw[before.0..],
                    &mut output[before.1..],
                    FlushCompress::Finish,
                )
                .map_err(|source| Error::ZlibEncode { source })?;
            let after = (codec.total_in() as usize, codec.total_out() as usize);
            if status == Status::StreamEnd {
                if after.0 != raw.len() {
                    return Err(Error::Progress {
                        what: "encoded input",
                        expected: raw.len(),
                        actual: after.0,
                    });
                }
                return Ok(after.1);
            }
            if before == after || after.1 == output.len() {
                return Err(Error::Incomplete {
                    codec: ZLIB,
                    operation: "encode",
                    consumed: after.0,
                    written: after.1,
                });
            }
        }
    }
    fn decode_into(&self, encoded: &[u8], output: &mut [u8]) -> Result<Progress, Error> {
        let mut codec = Decompress::new(true);
        loop {
            let before = (codec.total_in() as usize, codec.total_out() as usize);
            let status = codec
                .decompress(
                    &encoded[before.0..],
                    &mut output[before.1..],
                    FlushDecompress::Finish,
                )
                .map_err(|source| Error::ZlibDecode { source })?;
            let after = (codec.total_in() as usize, codec.total_out() as usize);
            if status == Status::StreamEnd {
                return Ok(Progress {
                    consumed: after.0,
                    written: after.1,
                });
            }
            if before == after || after.0 == encoded.len() || after.1 == output.len() {
                return Err(Error::Incomplete {
                    codec: ZLIB,
                    operation: "decode",
                    consumed: after.0,
                    written: after.1,
                });
            }
        }
    }
}
