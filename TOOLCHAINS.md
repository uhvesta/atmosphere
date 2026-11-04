# Toolchains

Every compiler, SDK and linker comes from a pinned, hash-checked Bazel
download. The build never reads Xcode, the Command Line Tools, or the build
machine's compilers and libc. On macOS, `--config=no_xcode` proves this by
blocking `/Applications`, `/Library/Developer` and `xcrun` from the sandbox.

| Language | Source                                                                        | Pinned in                                                                    |
| -------- | ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| C/C++    | [`llvm`](https://github.com/hermeticbuild/hermetic-llvm) (clang, lld, libc++) | `MODULE.bazel`                                                               |
| Go       | `rules_go`, linking cgo through `@llvm`                                       | `go.mod`                                                                     |
| Rust     | `rules_rust`, linking through `@llvm`                                         | `Cargo.toml` (`rust-version`)                                                |
| Swift    | swift.org releases, `//bazel/toolchains/swift`                                     | `MODULE.bazel` (`swift.toolchain`), hashes in `bazel/toolchains/swift/*.json` |
| Zig      | [`rules_zig`](https://github.com/hermeticbuild/rules_zig)                     | `MODULE.bazel` (`zig.toolchain`)                                             |

`@llvm` is the only C/C++ toolchain. Swift, Rust, Go and Zig all link through
it, so one program can mix all of them (see `//examples/interop`).

## Platforms

Build for a platform with `--config=<name>`:

| Config             | Output                                                                           | Swift |
| ------------------ | -------------------------------------------------------------------------------- | ----- |
| `macos_arm64`      | macOS 14.0+ arm64                                                                | yes   |
| `linux_arm64`      | static musl, runs on any arm64 Linux                                             | yes   |
| `linux_x86_64`     | static musl, runs on any x86-64 Linux                                            | yes   |
| `linux_arm64_gnu`  | everything static except glibc (2.35+: Ubuntu 22.04+, Debian 12+)                | no    |
| `linux_x86_64_gnu` | everything static except glibc (2.35+: Ubuntu 22.04+, Debian 12+)                | no    |

Without `--config`, macOS hosts build for `macos_arm64` and Linux hosts for
static musl on the host CPU (`//bazel/platforms:linux_host`).

Any host builds every platform, except Swift for macOS, which needs a macOS
host. The Swift toolchains only pair Apple's SDK with Apple hardware.

## End-to-end check

`bazel run //tools/e2e` is the end-to-end check, for a macOS arm64 host. It
runs `bazel test --config=no_xcode //...`, builds every example for every
platform, then runs each binary: macOS binaries on this machine, arm64 Linux
binaries in Lima VMs (Ubuntu 24.04, Debian 13 and Alpine 3.23). x86-64 Linux
binaries are built but not run. It creates or starts the VMs from
`tools/e2e/lima/*.yaml` with a pinned `limactl`. `-- --skip-tests` skips the
tests, `-- --build-only` skips running, and
`-- --example swift --config linux_arm64` narrows the run.

## Swift

`MODULE.bazel` pins the release:

```starlark
swift = use_extension("//bazel/toolchains/swift:extensions.bzl", "swift")
swift.toolchain(version = "6.4.2")
use_repo(swift, "swift_toolchains")
register_toolchains("@swift_toolchains//:all")
```

The root module's `swift.toolchain` wins, so a module that depends on this one
picks its own release the same way. Every release in the catalog below also
gets toolchains, so `--@swift_toolchains//:version=6.3.3` builds with another
one. A release only downloads when a build uses it.

### Releases and SDKs

`bazel/toolchains/swift/swift_versions.json` lists every Swift release from 6.2 on
with the SHA-256 of each download. `bazel/toolchains/swift/macos_sdks.json` lists the macOS
15.x and 26.x Command Line Tools SDKs that Apple's software update catalog
still serves. Regenerate both with:

```sh
bazel run //tools/swift_catalog
```

The tool is a Rust CLI (`tools/swift_catalog`), so it doesn't
depend on the Swift toolchain it regenerates. It only downloads what's new;
`-- --help` lists its options. Then
`bazel test //bazel/toolchains/swift/tests:all_versions` builds the Swift example
with every release for macOS arm64 (and runs it) and both static Linux
targets. It's a manual target because it downloads every release.

Linux targets use the open-source
[Static Linux SDK](https://www.swift.org/documentation/articles/static-linux-getting-started.html)
(musl, the Swift runtime, and swift-foundation), so no Apple software goes
into a Linux binary. Apple's SDK is needed only to produce macOS binaries.

### macOS SDK selection

A swift.org compiler can't read SDK modules built by a newer compiler, so each
release can only use SDKs no newer than the one in the Xcode it shipped with.
For example, Swift 6.2 shipped with Xcode 26.0, so it can use the 15.x SDKs
but not 26.5. Without a choice, each release uses the newest SDK it can read.
`swift.macos_sdk(version = "15.5")` picks one for every release that can read
it. A release that no SDK fits still builds for Linux, but not for macOS.

The SDK is trimmed to `usr/lib/swift` plus the frameworks `import Foundation`
needs. Add frameworks with `swift.macos_sdk(frameworks = ["Network"])`.

A module can pin a release or SDK that isn't in the JSON files by passing the
hashes itself:
`swift.toolchain(version = "6.5.0", static_sdk_version = "0.1.0", sha256 = {"macos": "...", "linux_aarch64": "...", "linux_x86_64": "...", "static_sdk": "..."})`
or `swift.macos_sdk(urls = [...], sha256 = "...", strip_prefix = "...")`.

The rules_swift worker runs every compile through `/usr/bin/xcrun` on macOS.
`bazel/toolchains/swift/rules_swift_no_xcrun.patch` lets a toolchain opt out, and
`@swift_toolchains` does.

## Xcode apps

Apps that need the iOS SDK, SwiftUI and SwiftData, Apple's Swift macro
plugins or code signing are the one exception to the rule above: only Xcode
provides those. They build with Xcode's toolchains, on the platforms in
`//bazel/xcode`.

`--config=xcode` points `rules_apple`'s platform flags at `//bazel/xcode`'s
platforms, which carry the `//bazel/xcode` constraint. apple_support's C/C++
toolchain is registered for them before `@llvm`, and the `@swift_toolchains`
toolchains require `//bazel/xcode:hermetic`, so `rules_swift`'s Xcode
toolchain builds their Swift. Targets that need Xcode set
`target_compatible_with = ["//bazel/xcode"]`, so without the config, as with
`bazel build //...` and `--config=no_xcode`, Bazel skips them.

Lifr (`apps/lifr`) builds its iOS app, macOS app and `lifr` command this way,
with [`rules_apple`](https://github.com/bazelbuild/rules_apple):

```sh
bazel build --config=xcode //apps/lifr/...
bazel run //apps/lifr:xcodeproj   # generates apps/lifr/Lifr.xcodeproj
```

`--config=xcode` also builds the tools these builds run on the build machine
with Xcode: Swift macro plugins, which link with a flag `ld64.lld` rejects,
and `rules_xcodeproj`'s `rsync`, whose `lz4` and `xxhash` define the same
symbols.
Builds from the generated project use `--config=rules_xcodeproj`, which turns
on `--config=xcode`.

### Swift packages

Xcode apps can use Swift packages, through
[`rules_swift_package_manager`](https://github.com/cgrindel/rules_swift_package_manager).
`apps/lifr/Package.swift` lists them and `apps/lifr/Package.resolved` pins
them. After changing either, run `bazel run @swift_package//:resolve` and then
`bazel mod tidy`.

The module extension runs Xcode's `swift package`, but `MODULE.bazel.lock`
records its result, and the `@swiftpkg_*` repositories are only fetched by
targets that need Xcode. Hosts without Xcode, including Linux, never run it.
Each package compiles for its own minimum OS, as SwiftPM does, so build them
through a `rules_apple` target rather than with `--platforms` alone.

### Snapshot tests

Lifr's screens are checked with snapshot tests: `lifr_snapshot_test` in
`apps/lifr/testing/snapshot/defs.bzl`. They run on one pinned simulator,
iPhone 17 with iOS 26.2, so every machine renders the same pixels. The
references in `SnapshotTests/__Snapshots__` are copied into the test bundle.

```sh
bazel test --config=xcode //apps/lifr/...             # verify
bazel test --config=record_snapshots //apps/lifr/...  # re-record
```

Recording writes the new references into the source tree and fails on
purpose. Review them with `git diff`, then verify again. When a test fails,
it writes the new rendering under `bazel-testlogs/<package>/<test>/test.outputs`.

The PNG references are stored in Git LFS (see `.gitattributes`). Run
`git lfs install` once per clone. A reference that Git LFS left as a pointer
fails its test and says to run `git lfs pull`.

## Rust

`rules_rust` builds with the toolchain pinned by `rust-version` and links
through `@llvm`. Cargo build scripts get `@llvm`'s compiler and `ar` tool.
`bazel/toolchains/rust/rules_rust_libtool_ar.patch` keeps rules_rust from
dropping the archiver on macOS, where it is `llvm-libtool-darwin`. Without
the patch, cc-rs falls back to `/usr/bin/ar`, which needs the Command Line Tools.

## Zig

`MODULE.bazel` pins the release with `zig.toolchain(zig_version = "0.17.0")`.
The root module's first `zig.toolchain` wins. `//bazel/toolchains/zig` maps each
platform to a Zig target with the same libc and macOS minimum as `@llvm`.
