"""Google's LiteRT-LM, for running downloaded models on the GPU.

`@litert_lm//:LiteRTLM` is the Swift API, built from LiteRT-LM's own sources
over the prebuilt runtime Google publishes with each release: a framework
for iOS and a dylib for the Mac. They're the files LiteRT-LM's Package.swift
points SwiftPM at, with the same SHA-256s. rules_swift_package_manager can't
import the Mac one, an XCFramework of a bare dylib, so this fetches them
itself.

To update, change `_VERSION` and `_COMMIT` to the new tag, take the zips'
checksums from that tag's Package.swift, and hash each Swift file with
`shasum -a 256` from a checkout of the tag.
"""

_VERSION = "0.19.0"
_COMMIT = "83ce6067f2e2a3fb697556928f93936693ded971"

_RELEASE = "https://github.com/google-ai-edge/LiteRT-LM/releases/download/v{}/".format(_VERSION)

_FRAMEWORKS = {
    "CLiteRTLM.xcframework.zip": "b7b2450fc084058adc5185498f8b7810a9297e054a10cf55818b62e1d5f8d65d",
    "CLiteRTLM_mac.xcframework.zip": "1a9e283a99a119aefe21afda541a5308a7d88aa9cb12f58aa7974038e3369c7c",
}

# The Swift API: swift/ without its tests and the Foundation Models adapter,
# as in LiteRT-LM's Package.swift.
_SOURCES = {
    "LICENSE": "c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4",
    "swift/Benchmark.swift": "554a192f071b791f72a3f8599285b89a5538f32cc06d67a503e1a8bf9c4c6bb7",
    "swift/Config.swift": "8fb601890d23d957dba70ebce449ac9df5ff612c0d9474db1aa54b644debd52c",
    "swift/Conversation.swift": "ad4a051b2fd7780ab1bf8862dad6f2ebfd03e20fcd7287c54bd7eb3e24606617",
    "swift/EmbeddingEngine.swift": "de17d18e248b5b0d8a89ca70bef0fd39a81b5d05f4dc9732388d7cb35e1dc502",
    "swift/EmbeddingEngineConfig.swift": "541dd557b1a4ca189668dbcec2b83290e7f2b7fdec1dc93c0079ec65a2a97c38",
    "swift/Engine.swift": "6449e9b8c6b80655ef92abba8e940fc17b903bf5d69656d11d1d69b1026bb017",
    "swift/ExperimentalFlags.swift": "5a68f4662a0b8b9e6ff07bebce882f7f72ed9251193c0ceab3f03eb981d11d46",
    "swift/LiteRTLMError.swift": "86de95aaa32e753291a25429281ecb044bcb12741df076f9b6dd8b9aa5ea2bf0",
    "swift/Message.swift": "8564f90fbaa62b295473a26569cce1afef7e16a66365b53429d5a98578a36b19",
    "swift/ModelInfo.swift": "a203bab31b403df442888dcfc4385d6a97d85550e7cdddb7e5425032414a97c7",
    "swift/ResponseFormat.swift": "071705a8d9e9916a83b2197514d58bdbd3a30e643903a9e826a500fb67772067",
    "swift/Tool.swift": "d9698c6c7f8425cfa1f479cec5070fe94d9ca732b3e20b171e3dd90ca1545422",
    "swift/ToolManager.swift": "2199ee34981755f2cc36a2947efb0d65bcf873df0cdd8683e1c82cdff92e9e86",
}

_MAC = "CLiteRTLM_mac.xcframework/macos-arm64_x86_64"

_BUILD = """\
load("@rules_apple//apple:apple.bzl", "apple_dynamic_xcframework_import")
load("@rules_cc//cc:cc_import.bzl", "cc_import")
load("@rules_cc//cc:cc_library.bzl", "cc_library")
load("@rules_swift//swift:swift_interop_hint.bzl", "swift_interop_hint")
load("@rules_swift//swift:swift_library.bzl", "swift_library")

package(default_visibility = ["@@//apps/lifr:__subpackages__"])

licenses(["notice"])

exports_files(["LICENSE"])

swift_library(
    name = "LiteRTLM",
    srcs = glob(["swift/*.swift"]),
    # LiteRT-LM builds its Swift API in the Swift 5 language mode.
    copts = [
        "-swift-version",
        "5",
        "-suppress-warnings",
    ],
    module_name = "LiteRTLM",
    deps = select({{
        "@platforms//os:macos": [":CLiteRTLM_mac"],
        "//conditions:default": [":CLiteRTLM_ios"],
    }}),
)

apple_dynamic_xcframework_import(
    name = "CLiteRTLM_ios",
    xcframework_imports = glob(["CLiteRTLM.xcframework/**"]),
)

# The Mac's runtime is a dylib that loads from @rpath. The app copies it into
# Contents/Frameworks.
cc_import(
    name = "CLiteRTLM_mac_dylib",
    shared_library = "libCLiteRTLM_mac.dylib",
)

cc_library(
    name = "CLiteRTLM_mac",
    hdrs = glob(["{mac}/Headers/*.h"]),
    aspect_hints = [":CLiteRTLM_mac_module"],
    includes = ["{mac}/Headers"],
    deps = [":CLiteRTLM_mac_dylib"],
)

swift_interop_hint(
    name = "CLiteRTLM_mac_module",
    module_map = "{mac}/Headers/module.modulemap",
    module_name = "CLiteRTLM",
)
"""

def _litert_lm_impl(repository_ctx):
    for name, sha256 in _FRAMEWORKS.items():
        repository_ctx.download_and_extract(url = _RELEASE + name, sha256 = sha256, type = "zip")
    for path, sha256 in _SOURCES.items():
        repository_ctx.download(
            url = "https://raw.githubusercontent.com/google-ai-edge/LiteRT-LM/{}/{}".format(_COMMIT, path),
            output = path,
            sha256 = sha256,
        )

    # At the top, so the app copies the dylib itself and not its folders.
    repository_ctx.symlink(_MAC + "/libCLiteRTLM_mac.dylib", "libCLiteRTLM_mac.dylib")
    repository_ctx.file("BUILD.bazel", _BUILD.format(mac = _MAC))

litert_lm = repository_rule(
    implementation = _litert_lm_impl,
    doc = "LiteRT-LM's Swift API and prebuilt runtime, pinned to one release.",
)
