"""libghostty-vt, Ghostty's terminal core as a C library, built from its pinned source.

`@libghostty_vt//:libghostty_vt` is the static library and headers that parse
and hold Humata 2's terminals (decision D10 in //apps/humata2:DECISIONS.md).
The repository downloads Zig and the libghostty-vt source tarball, each pinned
by SHA-256, fetches the source's own Zig packages (which its `build.zig.zon`
pins by hash), and builds the library once, when the repository is fetched:
for this Mac, and cross-compiled for aarch64 and x86_64 Linux against musl.
The `libghostty_vt` alias picks the one that matches the target platform.

Zig here is the macOS arm64 build, so the libraries build only on an Apple
silicon Mac; on any other host the target is marked incompatible.

To update, change `_VERSION` and `_SHA256` (`curl -sL <tarball> | shasum -a
256`, the tarball from
https://github.com/ghostty-org/ghostty/archive/refs/tags/vt-<version>.tar.gz),
and `_ZIG_*` if the new release needs another Zig (its `build.zig.zon` names
`minimum_zig_version`), then build and test //apps/humata2/terminal/vt/...
"""

_VERSION = "1.0.1"

_SHA256 = "a441bd6fd79505d5611aa54e90031d48a0625571cd40bfed7c03c8b956ea110c"

_ZIG_VERSION = "0.17.1"

_ZIG_SHA256 = "75f1bc7a1e9852f17ec9a0c1895f767c4ec56d46bc8a834920eece1d0359dd8d"

# Where each library goes in the repository, and the Zig target it is built
# for. None builds for this Mac.
_TARGETS = [
    ("macos_aarch64", None),
    ("macos_x86_64", "x86_64-macos"),
    ("linux_aarch64", "aarch64-linux-musl"),
    ("linux_x86_64", "x86_64-linux-musl"),
]

_BUILD = """\
load("@rules_cc//cc:defs.bzl", "cc_library")

licenses(["notice"])

exports_files(["LICENSE"], visibility = ["@@//apps/humata2:__subpackages__"])

[
    cc_library(
        name = "libghostty_vt_" + platform,
        srcs = [platform + "/lib/libghostty-vt.a"],
        hdrs = glob(["include/**/*.h"]),
        defines = ["GHOSTTY_STATIC"],
        includes = ["include"],
        linkstatic = True,
    )
    for platform in [
        "macos_aarch64",
        "macos_x86_64",
        "linux_aarch64",
        "linux_x86_64",
    ]
]

[
    config_setting(
        name = os + "_" + cpu,
        constraint_values = [
            "@platforms//cpu:" + cpu,
            "@platforms//os:" + os,
        ],
    )
    for os, cpu in [
        ("macos", "aarch64"),
        ("macos", "x86_64"),
        ("linux", "aarch64"),
        ("linux", "x86_64"),
    ]
]

alias(
    name = "libghostty_vt",
    actual = select(
        {
            ":macos_aarch64": ":libghostty_vt_macos_aarch64",
            ":macos_x86_64": ":libghostty_vt_macos_x86_64",
            ":linux_aarch64": ":libghostty_vt_linux_aarch64",
            ":linux_x86_64": ":libghostty_vt_linux_x86_64",
        },
        no_match_error = ("libghostty-vt is built for macOS and Linux on " +
                          "arm64 and x86_64 only. Build for one of those platforms, " +
                          "or add the target to _TARGETS in " +
                          "//apps/humata2/terminal/vt:libghostty_vt.bzl."),
    ),
    visibility = ["@@//apps/humata2:__subpackages__"],
)
"""

_STUB = """\
licenses(["notice"])

# The pinned Zig runs only on an Apple silicon Mac.
cc_library(
    name = "libghostty_vt",
    target_compatible_with = ["@platforms//:incompatible"],
    visibility = ["@@//apps/humata2:__subpackages__"],
)
"""

_BUILD_HINT = ("Check the output above for the first error. If it is about a " +
               "missing SDK or linker, check that Xcode's command line tools are " +
               "installed (`xcode-select --install`) and build again.")

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
        _failed(what, result, _BUILD_HINT)

def _fetch(repository_ctx, what, arguments, environment, working_directory):
    """Runs a download, trying again when the network drops it."""
    result = None
    for _ in range(_FETCH_TRIES):
        result = _execute(repository_ctx, arguments, environment, working_directory)
        if result.return_code == 0:
            return
    _failed(what, result, _NETWORK_HINT.format(_FETCH_TRIES))

def _libghostty_vt_impl(repository_ctx):
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
        url = "https://github.com/ghostty-org/ghostty/archive/refs/tags/vt-{}.tar.gz".format(_VERSION),
        sha256 = _SHA256,
        stripPrefix = "ghostty-vt-{}".format(_VERSION),
        output = "src",
    )

    zig = str(repository_ctx.path("zig/zig"))
    src = str(repository_ctx.path("src"))
    environment = {
        "MACOSX_DEPLOYMENT_TARGET": "15.0",
        "ZIG_GLOBAL_CACHE_DIR": str(repository_ctx.path("zig-cache")),
        "ZIG_LOCAL_CACHE_DIR": str(repository_ctx.path("zig-cache-local")),
    }

    # Zig puts fetched packages in the source's own zig-pkg directory. The
    # fetch gets a cache of its own because, with Zig 0.17, a cache holding a
    # package's archive can crash the build ("reached unreachable code"), as
    # PACKAGING-LIBGHOSTTY.md in the source warns.
    _fetch(
        repository_ctx,
        "Fetching libghostty-vt's Zig packages",
        [zig, "build", "--fetch=all"],
        dict(environment, ZIG_GLOBAL_CACHE_DIR = str(repository_ctx.path("zig-fetch-cache"))),
        src,
    )
    for platform, target in _TARGETS:
        arguments = [
            zig,
            "build",
            "-Demit-lib-vt",
            "-Demit-xcframework=false",
            "-Doptimize=ReleaseFast",
            "-Dcpu=baseline",
            "--prefix",
            str(repository_ctx.path(platform)),
        ]
        if target:
            arguments.append("-Dtarget=" + target)
        _run(
            repository_ctx,
            "Building libghostty-vt for " + platform,
            arguments,
            environment,
            src,
        )

    # Keep the libraries, one copy of the headers (they are the same for
    # every target) and the license; drop the source, Zig and the caches so
    # the repository holds only what the build uses.
    _run(repository_ctx, "Keeping libghostty-vt's headers", ["mv", "macos_aarch64/include", "include"])
    for platform, _ in _TARGETS:
        # Rust's native link metadata can also emit -lghostty-vt. Keep the
        # searched directory static-only, including for unsandboxed manual
        # targets: an undeclared dylib/so otherwise silently wins that search.
        library_dir = repository_ctx.path(platform + "/lib")
        if not repository_ctx.path(platform + "/lib/libghostty-vt.a").exists:
            fail("The pinned Ghostty build omitted its static archive for " + platform + ". Inspect the build output; do not substitute a shared library.")
        for artifact in library_dir.readdir():
            if artifact.basename != "libghostty-vt.a":
                repository_ctx.delete(artifact)
        repository_ctx.delete(platform + "/include")
        repository_ctx.delete(platform + "/share")
    _run(repository_ctx, "Keeping Ghostty's license", ["cp", src + "/LICENSE", "LICENSE"])
    repository_ctx.delete("src")
    repository_ctx.delete("zig")
    repository_ctx.delete("zig-cache")
    repository_ctx.delete("zig-cache-local")
    repository_ctx.delete("zig-fetch-cache")
    repository_ctx.file("BUILD.bazel", _BUILD)

libghostty_vt = repository_rule(
    implementation = _libghostty_vt_impl,
    doc = "libghostty-vt built from its pinned source with a pinned Zig, for macOS and Linux.",
)
