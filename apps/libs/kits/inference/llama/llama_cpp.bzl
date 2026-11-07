"""llama.cpp, for running downloaded GGUF models on the GPU or the CPU.

`@llama_cpp//:llama` is the framework llama.cpp's own release builds for
Apple's platforms, with its C API as the Swift module `llama`: one build,
pinned by its tag and the zip's SHA-256, so every build of the app runs the
same llama.cpp. It has slices for iPhones and Macs, and none for the iOS
Simulator, which runs models on the Mac instead.

To update, change `_TAG` and `_COMMIT` to a newer release's, and take the
SHA-256s with `shasum -a 256` of its llama-<tag>-xcframework.zip and of
LICENSE at the commit. Check its release notes for API changes first:
llama.h changes often.
"""

_TAG = "b12189"
_COMMIT = "80d2c0c2c56e4a1bbf74be98f64b99927bf230bc"

_ZIP_SHA256 = "1b3d5fdbf0e852badf19a95cf21a247a1400f76ab89c7bb527f89909d298d440"
_LICENSE_SHA256 = "94f29bbed6a22c35b992c5c6ebf0e7c92f13b836b90f36f461c9cf2f0f1d010d"

_BUILD = """\
load("@rules_apple//apple:apple.bzl", "apple_dynamic_xcframework_import")

package(default_visibility = ["@@//apps/libs/kits/inference/llama:__subpackages__"])

licenses(["notice"])

exports_files(["LICENSE"])

apple_dynamic_xcframework_import(
    name = "llama",
    xcframework_imports = glob(
        ["llama.xcframework/**"],
        exclude = ["llama.xcframework/*/dSYMs/**"],
    ),
)
"""

def _llama_cpp_impl(repository_ctx):
    repository_ctx.download_and_extract(
        url = "https://github.com/ggml-org/llama.cpp/releases/download/{tag}/llama-{tag}-xcframework.zip".format(
            tag = _TAG,
        ),
        sha256 = _ZIP_SHA256,
        type = "zip",
        stripPrefix = "build-apple",
    )
    repository_ctx.download(
        url = "https://raw.githubusercontent.com/ggml-org/llama.cpp/{}/LICENSE".format(_COMMIT),
        output = "LICENSE",
        sha256 = _LICENSE_SHA256,
    )
    repository_ctx.file("BUILD.bazel", _BUILD)

llama_cpp = repository_rule(
    implementation = _llama_cpp_impl,
    doc = "llama.cpp's prebuilt framework for iOS and macOS, pinned to one release.",
)
