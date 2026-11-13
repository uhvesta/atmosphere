"""GhosttyKit, Ghostty's terminal library, built from its pinned source.

`@ghostty_kit//:GhosttyKit` is the static library and `ghostty.h` that draw
Humata's terminals. The repository downloads Zig and Ghostty's release
tarball, each pinned by SHA-256, fetches Ghostty's own Zig packages (which
its `build.zig.zon` pins by hash), and builds the library once, when the
repository is fetched. Building needs macOS with Xcode and its Metal
toolchain (`xcodebuild -downloadComponent MetalToolchain`); on any other
system the target is marked incompatible.

To update, change `_GHOSTTY_VERSION` and `_GHOSTTY_SHA256` (`curl -sL
<tarball> | shasum -a 256`, the tarball from
https://release.files.ghostty.org/<version>/ghostty-<version>.tar.gz), and
`_ZIG_*` if the new release needs another Zig, then build
//apps/humata/features/terminal:tests.
"""

_GHOSTTY_VERSION = "1.4.0"

_GHOSTTY_SHA256 = "cb188ac6c3a8278d5fd7bfef15d8c017104c87f638b5560eb38024a71ae0f005"

_ZIG_VERSION = "0.17.0"

_ZIG_SHA256 = "b607e9b9234790a008116ae5bdb71c6243b84b9fb42a53a9e70fde41c06c536a"

_BUILD = """\
load("@rules_apple//apple:apple.bzl", "apple_static_xcframework_import")

licenses(["notice"])

exports_files(["LICENSE"], visibility = ["@@//apps/humata:__subpackages__"])

apple_static_xcframework_import(
    name = "GhosttyKit",
    sdk_dylibs = ["c++"],
    sdk_frameworks = ["Carbon", "GameController"],
    visibility = ["@@//apps/humata/features/terminal:__pkg__"],
    xcframework_imports = glob(["GhosttyKit.xcframework/**"]),
)
"""

_STUB = """\
licenses(["notice"])

# Ghostty builds only on macOS with Xcode.
cc_library(
    name = "GhosttyKit",
    target_compatible_with = ["@platforms//:incompatible"],
    visibility = ["@@//apps/humata/features/terminal:__pkg__"],
)
"""

def _run(repository_ctx, what, arguments, environment = {}, working_directory = "."):
    result = repository_ctx.execute(
        arguments,
        environment = environment,
        quiet = False,
        timeout = 3600,
        working_directory = working_directory,
    )
    if result.return_code != 0:
        fail(("{} failed ({}). Check the output above; if it names the Metal " +
              "toolchain, install it with `xcodebuild -downloadComponent " +
              "MetalToolchain` and build again.\n{}").format(
            what,
            result.return_code,
            result.stderr[-4000:],
        ))

def _ghostty_kit_impl(repository_ctx):
    if repository_ctx.os.name != "mac os x" or repository_ctx.os.arch != "aarch64":
        repository_ctx.file("BUILD.bazel", _STUB)
        return

    repository_ctx.download_and_extract(
        url = "https://ziglang.org/download/{v}/zig-aarch64-macos-{v}.tar.xz".format(v = _ZIG_VERSION),
        sha256 = _ZIG_SHA256,
        stripPrefix = "zig-aarch64-macos-{}".format(_ZIG_VERSION),
        output = "zig",
    )
    repository_ctx.download_and_extract(
        url = "https://release.files.ghostty.org/{v}/ghostty-{v}.tar.gz".format(v = _GHOSTTY_VERSION),
        sha256 = _GHOSTTY_SHA256,
        stripPrefix = "ghostty-{}".format(_GHOSTTY_VERSION),
        output = "src",
    )

    zig = str(repository_ctx.path("zig/zig"))
    src = str(repository_ctx.path("src"))
    environment = {
        "MACOSX_DEPLOYMENT_TARGET": "15.0",
        "ZIG_GLOBAL_CACHE_DIR": str(repository_ctx.path("zig-cache")),
        "ZIG_LOCAL_CACHE_DIR": str(repository_ctx.path("zig-cache-local")),
    }
    _run(
        repository_ctx,
        "Fetching Ghostty's Zig packages",
        [zig, "build", "--fetch=all"],
        environment,
        src,
    )
    _run(
        repository_ctx,
        "Building GhosttyKit",
        [
            zig,
            "build",
            "-Doptimize=ReleaseFast",
            "-Dapp-runtime=none",
            "-Demit-macos-app=false",
            "-Demit-xcframework=true",
            "-Dxcframework-target=native",
            "-Di18n=false",
            "-Dsentry=false",
            "-Dversion-string=" + _GHOSTTY_VERSION,
        ],
        environment,
        src,
    )

    # Keep the library and the license; drop the source, Zig and the caches
    # (over a gigabyte) so the repository holds only what the build uses.
    _run(repository_ctx, "Keeping GhosttyKit", ["mv", src + "/macos/GhosttyKit.xcframework", "GhosttyKit.xcframework"])
    _run(repository_ctx, "Keeping Ghostty's license", ["cp", src + "/LICENSE", "LICENSE"])
    repository_ctx.delete("src")
    repository_ctx.delete("zig")
    repository_ctx.delete("zig-cache")
    repository_ctx.delete("zig-cache-local")
    repository_ctx.file("BUILD.bazel", _BUILD)

ghostty_kit = repository_rule(
    implementation = _ghostty_kit_impl,
    doc = "GhosttyKit built from Ghostty's pinned release with a pinned Zig.",
)
