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
| Swift    | swift.org releases, `//tools/rules/swift`                                     | `MODULE.bazel` (`swift.toolchain`), hashes in `tools/rules/swift/*.json` |
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
static musl on the host CPU (`//tools/platforms:linux_host`).

Any host builds every platform, except Swift for macOS, which needs a macOS
host. The Swift toolchains only pair Apple's SDK with Apple hardware.

## Testing cross-compiled binaries

`bazel run //tools/cross:matrix` builds every example for every platform,
then runs each binary: macOS binaries on this machine, Linux binaries in Lima
VMs (Ubuntu 24.04, Debian 13 and Alpine 3.23, with x86-64 through Rosetta).
It creates or starts the VMs from `tools/lima/*.yaml` with a pinned `limactl`.
The VMs need macOS on Apple silicon. Elsewhere, `-- --local` runs only the
binaries this machine runs natively (CI does this), and `-- --build-only`
skips running. `-- --example swift --config linux_arm64` narrows the run.

## Swift

`MODULE.bazel` pins the release:

```starlark
swift = use_extension("//tools/rules/swift:extensions.bzl", "swift")
swift.toolchain(version = "6.4.2")
use_repo(swift, "swift_toolchains")
register_toolchains("@swift_toolchains//:all")
```

The root module's `swift.toolchain` wins, so a module that depends on this one
picks its own release the same way. Every release in the catalog below also
gets toolchains, so `--@swift_toolchains//:version=6.3.3` builds with another
one. A release only downloads when a build uses it.

### Releases and SDKs

`tools/rules/swift/swift_versions.json` lists every Swift release from 6.2 on
with the SHA-256 of each download. `tools/rules/swift/macos_sdks.json` lists the macOS
15.x and 26.x Command Line Tools SDKs that Apple's software update catalog
still serves. Regenerate both with:

```sh
bazel run //tools/rules/swift:update_versions
```

The tool is a Rust CLI (`tools/rules/swift/update_versions`), so it doesn't
depend on the Swift toolchain it regenerates. It only downloads what's new;
`-- --help` lists its options. Then
`bazel test //tools/rules/swift/tests:all_versions` builds the Swift example
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
`tools/rules/swift/rules_swift_no_xcrun.patch` lets a toolchain opt out, and
`@swift_toolchains` does.

## Zig

`MODULE.bazel` pins the release with `zig.toolchain(zig_version = "0.17.0")`.
The root module's first `zig.toolchain` wins. `//tools/rules/zig` maps each
platform to a Zig target with the same libc and macOS minimum as `@llvm`.

## Mirrors

Download URLs are fixed. To fetch from a mirror, rewrite them with a
[downloader config](https://bazel.build/reference/command-line-reference#flag--downloader_config).
For example, put this in `bazel_downloader.cfg`:

```text
rewrite download.swift.org/(.*) mirror.example.com/swift/$1
rewrite ziglang.org/(.*) mirror.example.com/zig/$1
```

Then add `common --downloader_config=bazel_downloader.cfg` to `user.bazelrc`
or to your CI's `.bazelrc`. The hashes still apply, so a mirror can't change
what you build.
