#!/bin/sh
# Manually prove that Swift builds hermetically on a macOS arm64 host, without
# Bazel. This is the go/no-go check that the Bazel toolchain in
# //tools/rules/swift is built on.
#
# It downloads (and checks the SHA-256 of) the swift.org toolchain, the Swift
# Static Linux SDK, the Apple macOS SDK package used by @llvm, and @llvm's
# prebuilt LLVM. Then it builds a Swift hello world for macOS arm64 and static
# Linux musl (arm64 and x86_64) inside a sandbox that denies reads of Xcode,
# the Command Line Tools, xcrun and /usr/bin/ld.
#
# Usage: tools/scripts/swift/prove_hermetic.sh [work-dir]
# The Linux binaries can be run with `limactl shell <vm> <binary>`.
set -eu

SWIFT_VERSION=6.4.2
SWIFT_PKG_URL="https://download.swift.org/swift-${SWIFT_VERSION}-release/xcode/swift-${SWIFT_VERSION}-RELEASE/swift-${SWIFT_VERSION}-RELEASE-osx.pkg"
SWIFT_PKG_SHA256=0a7d8489900d21dbf5f0187cb570e646a4c0226d0ddef2cbde8c7f54dfe79736
STATIC_SDK_URL="https://download.swift.org/swift-${SWIFT_VERSION}-release/static-sdk/swift-${SWIFT_VERSION}-RELEASE/swift-${SWIFT_VERSION}-RELEASE_static-linux-0.1.0.artifactbundle.tar.gz"
STATIC_SDK_SHA256=3786b179936d9cc9e266dd506bd1654176358d7b25288e243359f4d17cc4dc5a
# Same package and hash as osx.from_archive in @llvm's MODULE.bazel.
MACOS_SDK_URL=https://swcdn.apple.com/content/downloads/09/08/047-91568-A_Y1CFZWQCD4/4xekpyz43i26dbp4enxfro8eb1q7wiujh5/CLTools_macOSNMOS_SDK.pkg
MACOS_SDK_SHA256=5f044578cd78a3a9b9c965a42d56bad609ee5d252e1d4e6aa7c42fc3f35fee7b
MACOS_SDK_DIR=Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk

if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
  echo "This proof runs on a macOS arm64 host." >&2
  exit 1
fi

work=${1:-${TMPDIR:-/tmp}/swift-hermetic-proof}
mkdir -p "$work/dl" "$work/out"
work=$(cd "$work" && pwd)

fetch() {
  dest="$work/dl/$1"
  if [ ! -f "$dest" ]; then
    curl -fsSL -o "$dest.part" "$2"
    mv "$dest.part" "$dest"
  fi
  echo "$3  $dest" | shasum -a 256 -c - >/dev/null
}

fetch swift.pkg "$SWIFT_PKG_URL" "$SWIFT_PKG_SHA256"
fetch static-sdk.tar.gz "$STATIC_SDK_URL" "$STATIC_SDK_SHA256"
fetch macos-sdk.pkg "$MACOS_SDK_URL" "$MACOS_SDK_SHA256"

if [ ! -d "$work/swift" ]; then
  pkgutil --expand-full "$work/dl/swift.pkg" "$work/swift"
fi
if [ ! -d "$work/macos-sdk" ]; then
  pkgutil --expand-full "$work/dl/macos-sdk.pkg" "$work/macos-sdk"
fi
if [ ! -d "$work/static-sdk" ]; then
  mkdir "$work/static-sdk"
  tar -xzf "$work/dl/static-sdk.tar.gz" -C "$work/static-sdk"
fi

toolchain="$work/swift/swift-${SWIFT_VERSION}-RELEASE-osx-package.pkg/Payload"
sdk="$work/macos-sdk/Payload/$MACOS_SDK_DIR"
musl="$work/static-sdk/swift-${SWIFT_VERSION}-RELEASE_static-linux-0.1.0.artifactbundle/swift-${SWIFT_VERSION}-RELEASE_static-linux-0.1.0/swift-linux-musl/musl-1.2.5.sdk"

# swift.org's ld64.lld cannot link for macOS, so link with @llvm's ld64.lld
# (the same linker Bazel uses).
llvm_bin=$(cd "$(dirname "$0")/../../.." && bazel info output_base)/external/llvm++llvm_toolchain_minimal+llvm-toolchain-minimal-darwin-arm64/bin
if [ ! -x "$llvm_bin/ld64.lld" ]; then
  echo "Missing $llvm_bin/ld64.lld; run a C/C++ build first." >&2
  exit 1
fi

cat >"$work/out/noxcode.sb" <<'EOF'
(version 1)
(allow default)
(deny file-read*
  (subpath "/Applications/Xcode.app")
  (subpath "/Library/Developer")
  (literal "/usr/bin/xcrun")
  (literal "/usr/bin/ld"))
EOF

cat >"$work/out/hello.swift" <<'EOF'
import Foundation
#if os(macOS)
let os = "macOS"
#else
let os = "Linux"
#endif
#if arch(arm64)
let arch = "arm64"
#else
let arch = "x86_64"
#endif
print("hello from swift on \(os) \(arch), \(Date(timeIntervalSince1970: 0).timeIntervalSince1970)")
EOF

hermetic() {
  sandbox-exec -f "$work/out/noxcode.sb" \
    env -i PATH=/bin:/usr/bin HOME="$HOME" TMPDIR="${TMPDIR:-/tmp}" DEVELOPER_DIR=/nonexistent "$@"
}

hermetic "$toolchain/usr/bin/swiftc" \
  -sdk "$sdk" \
  -target arm64-apple-macos14.0 \
  -ld-path="$llvm_bin/ld64.lld" \
  "$work/out/hello.swift" -o "$work/out/hello_macos_arm64"

for arch in aarch64 x86_64; do
  hermetic "$toolchain/usr/bin/swiftc" \
    -target "$arch-swift-linux-musl" \
    -sdk "$musl/$arch" \
    -resource-dir "$musl/$arch/usr/lib/swift_static" \
    -static-executable -static-stdlib \
    -use-ld=lld \
    -Xclang-linker -resource-dir -Xclang-linker "$musl/$arch/usr/lib/swift/clang" \
    "$work/out/hello.swift" -o "$work/out/hello_linux_$arch"
done

file "$work/out/hello_macos_arm64" "$work/out/hello_linux_aarch64" "$work/out/hello_linux_x86_64"
"$work/out/hello_macos_arm64"
