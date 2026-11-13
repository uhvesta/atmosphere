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

Modelled on how boss (https://github.com/spinyfin/mono, Apache-2.0) pins and
builds GhosttyKit; its notice is in //apps/humata/app:THIRD_PARTY_NOTICES.md.
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

_METAL_HINT = ("Check the output above; if it names the Metal toolchain, install it " +
               "with `xcodebuild -downloadComponent MetalToolchain` and build again.")

_NETWORK_HINT = ("Zig couldn't download them, even after {} tries. Check that this Mac " +
                 "is online and can reach the hosts named above, then build again; " +
                 "packages fetched before are kept, so it picks up where it stopped.")

_FETCH_TRIES = 4

def _execute(repository_ctx, arguments, environment, working_directory):
    return repository_ctx.execute(
        arguments,
        environment = environment,
        quiet = False,
        timeout = 3600,
        working_directory = working_directory,
    )

def _failed(what, result, hint):
    fail("{} failed ({}). {}\n{}".format(what, result.return_code, hint, result.stderr[-4000:]))

def _run(repository_ctx, what, arguments, environment = {}, working_directory = "."):
    result = _execute(repository_ctx, arguments, environment, working_directory)
    if result.return_code != 0:
        _failed(what, result, _METAL_HINT)

def _fetch(repository_ctx, what, arguments, environment, working_directory):
    """Runs a download, trying again when the network drops it."""
    for _ in range(_FETCH_TRIES):
        result = _execute(repository_ctx, arguments, environment, working_directory)
        if result.return_code == 0:
            return
    _failed(what, result, _NETWORK_HINT.format(_FETCH_TRIES))

# On macOS Ghostty starts every command through `/usr/bin/login -flp`, to
# give the user's default shell a login session. Humata's surfaces always
# run their own command (`humata terminal attach`, whose shell is on the
# host), which wants none of that: login(1) is setuid, so it can't start in a
# sandbox, and it adds a login record and a `cd ~`. Without it, macOS runs
# commands as every other system does, with `/bin/sh -c`.
_LOGIN_WRAP = """\
    // hushlogin behavior.
    if (comptime builtin.target.os.tag.isDarwin()) darwin: {"""

_NO_LOGIN_WRAP = """\
    // hushlogin behavior. Humata: never, see ghostty_kit.bzl.
    if (false) darwin: {"""

def _run_commands_without_login(repository_ctx, path):
    source = repository_ctx.read(path)
    if source.count(_LOGIN_WRAP) != 1:
        fail(("Ghostty {}'s {} no longer has the login(1) wrap this repository " +
              "removes. Find where it now builds a command's argv on macOS, " +
              "update _LOGIN_WRAP to match, and check that terminals still start " +
              "without login(1).").format(_GHOSTTY_VERSION, path))
    repository_ctx.file(path, source.replace(_LOGIN_WRAP, _NO_LOGIN_WRAP), legacy_utf8 = False)

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
    _run_commands_without_login(repository_ctx, "src/src/termio/Exec.zig")

    zig = str(repository_ctx.path("zig/zig"))
    src = str(repository_ctx.path("src"))
    environment = {
        "MACOSX_DEPLOYMENT_TARGET": "15.0",
        "ZIG_GLOBAL_CACHE_DIR": str(repository_ctx.path("zig-cache")),
        "ZIG_LOCAL_CACHE_DIR": str(repository_ctx.path("zig-cache-local")),
    }
    _fetch(
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
