"""Swift toolchain defaults. Release and SDK hashes live in JSON files.

swift_versions.json: every Swift 6.x release with the SHA-256 of the macOS
  .pkg, the two Ubuntu 22.04 tarballs, and the Static Linux SDK.
macos_sdks.json: the macOS 15.x and 26.x Command Line Tools SDK packages.

Regenerate both with `bazel run //tools/rules/swift:update_versions`.
"""

DEFAULT_SWIFT_VERSION = "6.4.2"

# Frameworks that `import Foundation` needs. Add more with
# swift.macos_sdk(frameworks = [...]).
MACOS_SDK_FRAMEWORKS = [
    "CFNetwork",
    "Combine",
    "CoreFoundation",
    "CoreServices",
    "DiskArbitration",
    "Foundation",
    "IOKit",
    "Security",
]

# Must match @llvm's default -mmacosx-version-min (--macos_minimum_os).
MACOS_MINIMUM_OS = "14.0"
