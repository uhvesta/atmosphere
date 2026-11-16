# Bounded terminal compression

`terminal_codec` compresses opaque bytes independently of terminal parsing,
transport, authentication and disk policy. The default registry includes
`lz4-block-v1` and `zlib-v1`; the caller explicitly chooses the encoder.
There is no implicit codec fallback. Registry registration rejects existing
IDs and accepts caller-owned `Send + Sync` codec implementations.

`encode_preferred(&[LZ4, ZLIB], ...)` explicitly requests a preference order.
It tries the next codec only if actual compressed output exceeds the encoded
budget. Unknown codecs, plugin errors, allocation failures and malformed
codec progress are returned immediately. An empty preference list is an error.

`Limits::default()` permits 16 MiB decoded and 4 MiB encoded data. Callers
may supply their own trusted bounded limits, including a small archive
metadata allowance. Decoded claims and encoded length are checked before
decoded allocation. Codecs decode into a fixed supplied output slice and report
consumed/written counts, both of which must match exactly.

The versioned envelope requires codec ID, decoded/encoded lengths and CRC32
checksums of both representations. CRCs detect corruption; the caller owns
authentication. Zlib additionally requires its complete stream checksum.
Legacy Zlib payloads with a known decoded length can use
`decode_legacy_zlib`; an older archive without that length keeps its own
bounded legacy reader.

Serde carries the envelope for wire integration; callers bound incoming frames
before deserializing their encoded byte arrays. `Envelope::to_bytes` and
`from_bytes` provide a compact disk carrier: a 34-byte versioned header,
1–64 byte ASCII codec ID and compressed data, with little-endian 64-bit
lengths and 32-bit checksums. The encoded limit applies to compressed data;
callers must include the bounded carrier header in their file read cap.
Truncated/trailing carrier data is rejected before copying its payload.

The pinned `lz4_flex` block codec uses checked safe decoding. It needs a
worst-case encoder buffer even when a large checkpoint compresses below
the transport limit. That bounded temporary allocation is compacted before
returning the envelope, so queued data retains only its compressed bytes.
Compression implementations come from the pinned upstream libraries.

```sh
bazel test //apps/humata2/terminal/codec:codec_test --test_output=all
bazel test -c opt //apps/humata2/terminal/codec:dense_test \
  --test_arg=--nocapture --test_output=all
```

The manual dense target compares LZ4 and Zlib on an actual native Ghostty
checkpoint retaining approximately 10,000 colored Unicode rows. It checks
both codecs fit the 4 MiB encoded budget and reports encode/decode times;
there are no timing thresholds tied to a particular machine.

The verified native optimized run retained 9,596 rows in an 8,716,946-byte
checkpoint. Three samples produced these averages:

| Codec | Encoded bytes | Encode | Decode |
| --- | ---: | ---: | ---: |
| LZ4 | 4,154,587 | 29.047 ms | 7.851 ms |
| Zlib | 2,202,398 | 49.663 ms | 23.925 ms |

LZ4 fits this fixture's 4 MiB encoded limit with about 39 KiB remaining.
An explicitly configured `[LZ4, ZLIB]` preference retains Zlib's smaller
representation when a different workload exceeds that LZ4 budget.
