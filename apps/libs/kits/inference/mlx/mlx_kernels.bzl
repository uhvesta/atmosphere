"""MLX's Metal kernels, compiled for the app.

SwiftPM can't build Metal, so rules_swift_package_manager builds MLX's C++
and Swift but not its GPU kernels. This fetches the same mlx-swift commit
as apps/lifr/Package.resolved pins, pinned by SHA-256, and compiles the
kernels in Source/Cmlx/mlx-generated/metal into default.metallib, in a
bundle named mlx-swift_Cmlx: where MLX looks for it at runtime. The rest of
its kernels are compiled from source when first used.

To update, change `_COMMIT` to Package.resolved's mlx-swift revision and
`_SHA256` to `shasum -a 256` of its GitHub archive. The two must match, or
MLX's C++ runs kernels built from different source.
"""

_COMMIT = "19601207e9a0de51e03ee6ec0c3c5f3784275075"
_SHA256 = "434b0055e794bccfa209bce4a96a5e66beb226f56998fdb120b563d4d3b28b4c"

_BUILD = """\
load("@rules_apple//apple:resources.bzl", "apple_metal_library", "apple_resource_bundle")

package(default_visibility = ["@@//apps/libs/kits/inference/mlx:__subpackages__"])

licenses(["notice"])

exports_files(["LICENSE"])

_KERNELS = "Source/Cmlx/mlx-generated/metal/"

# MLX's own flags, from mlx/backend/metal/kernels/CMakeLists.txt.
apple_metal_library(
    name = "kernels",
    srcs = glob([_KERNELS + "**/*.metal"]),
    hdrs = glob([_KERNELS + "**/*.h"]),
    copts = [
        "-Wall",
        "-Wextra",
        "-fno-fast-math",
        "-Wno-c++17-extensions",
        "-Wno-c++20-extensions",
    ],
    out = "default.metallib",
)

apple_resource_bundle(
    name = "bundle",
    bundle_name = "mlx-swift_Cmlx",
    resources = [":kernels"],
)
"""

def _mlx_kernels_impl(repository_ctx):
    repository_ctx.download_and_extract(
        url = "https://github.com/ml-explore/mlx-swift/archive/{}.tar.gz".format(_COMMIT),
        sha256 = _SHA256,
        stripPrefix = "mlx-swift-" + _COMMIT,
    )
    repository_ctx.file("BUILD.bazel", _BUILD)

mlx_kernels = repository_rule(
    implementation = _mlx_kernels_impl,
    doc = "MLX's Metal kernels, from the mlx-swift commit Package.resolved pins.",
)
