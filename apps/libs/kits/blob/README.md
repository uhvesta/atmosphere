# BlobKit

How an app writes the values it keeps: a short header that says how the
bytes are encoded and compressed, then the payload. Values are JSON
compressed with zstd. A value written before the header existed reads as
plain JSON, so files from older builds still open. It's plain Swift on
zstd's C library, so it builds with the hermetic toolchains too.

## Use it

```starlark
deps = ["//apps/libs/kits/blob"]
```

```swift
import BlobKit

let data = try Blob.encode(journal)               // JSON + zstd, with a header
let journal = try Blob.decode([Entry].self, from: data)
let format = try Blob.format(of: data)            // .current or .legacy
```

`Blob.encode` and `Blob.decode` throw `BlobError`. The error says whether
the bytes were damaged, came from a newer build, or don't match the type you
asked for.

## What it gives you

- `BlobFormat.current` is JSON with zstd, and `BlobFormat.legacy` is plain
  JSON with no header. `BlobFormat` has room for other encodings and codecs,
  and for a zstd dictionary, so you can change the format later without a
  migration.
- `//apps/libs/kits/blob:zstd` exposes zstd from the BCR as the Swift module
  `CZstd`. zstd's own module map isn't an input to sandboxed compiles, so
  the hermetic toolchains couldn't find it otherwise.

Test it with `bazel test //apps/libs/kits/blob:blob_test`.
