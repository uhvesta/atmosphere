"""The same pinned terminal engine as libghostty_vt.bzl, for wasm32 browsers.

The static C-ABI archive is linked into the Rust/GPUI Wasm module so callbacks,
handles and memory belong to one module. The official-style standalone module is
also exported for JS-only experiments. No native PTY/SSH implementation runs here.
"""

_VERSION = "1.0.1"
_SHA256 = "a441bd6fd79505d5611aa54e90031d48a0625571cd40bfed7c03c8b956ea110c"
_ZIG_VERSION = "0.17.1"
_ZIG_SHA256 = "75f1bc7a1e9852f17ec9a0c1895f767c4ec56d46bc8a834920eece1d0359dd8d"

_BUILD = """\
load("@rules_cc//cc:defs.bzl", "cc_library")
licenses(["notice"])
cc_library(
    name = "libghostty_vt",
    srcs = ["wasm32/lib/libghostty-vt.a"],
    hdrs = glob(["include/**/*.h"]),
    includes = ["include"],
    defines = ["GHOSTTY_STATIC"],
    linkstatic = True,
    target_compatible_with = ["@platforms//cpu:wasm32"],
    visibility = ["@@//apps/humata2:__subpackages__"],
)
filegroup(
    name = "wasm_module",
    srcs = glob(["wasm32/bin/*.wasm"]),
    visibility = ["@@//apps/humata2:__subpackages__"],
)
exports_files(["LICENSE"], visibility = ["@@//apps/humata2:__subpackages__"])
"""

def _run(ctx, what, args, environment, directory):
    result = ctx.execute(args, environment = environment, working_directory = directory, quiet = False, timeout = 3600)
    if result.return_code:
        fail("{} failed ({}). Keep the pinned source and compiler versions together; inspect the compiler output before changing a pin.\n{}".format(what, result.return_code, result.stderr[-5000:]))

def _impl(ctx):
    if ctx.os.name != "mac os x" or ctx.os.arch != "aarch64":
        ctx.file("BUILD.bazel", 'load("@rules_cc//cc:defs.bzl", "cc_library")\ncc_library(name = "libghostty_vt", target_compatible_with = ["@platforms//:incompatible"], visibility = ["@@//apps/humata2:__subpackages__"])\nfilegroup(name = "wasm_module", srcs = [], target_compatible_with = ["@platforms//:incompatible"])\n')
        return
    ctx.download_and_extract(
        url = "https://ziglang.org/download/{v}/zig-aarch64-macos-{v}.tar.xz".format(v = _ZIG_VERSION),
        sha256 = _ZIG_SHA256,
        stripPrefix = "zig-aarch64-macos-" + _ZIG_VERSION,
        output = "zig",
    )
    ctx.download_and_extract(
        url = "https://github.com/ghostty-org/ghostty/archive/refs/tags/vt-{}.tar.gz".format(_VERSION),
        sha256 = _SHA256,
        stripPrefix = "ghostty-vt-" + _VERSION,
        output = "src",
    )
    zig = str(ctx.path("zig/zig"))
    source = str(ctx.path("src"))
    environment = {
        "ZIG_GLOBAL_CACHE_DIR": str(ctx.path("zig-cache")),
        "ZIG_LOCAL_CACHE_DIR": str(ctx.path("zig-cache-local")),
    }
    result = None
    for _ in range(4):
        result = ctx.execute([zig, "build", "--fetch=all"], environment = dict(environment, ZIG_GLOBAL_CACHE_DIR = str(ctx.path("zig-fetch-cache"))), working_directory = source, quiet = False, timeout = 3600)
        if result.return_code == 0:
            break
    if result.return_code:
        fail("Fetching pinned Ghostty Wasm dependencies failed after four attempts. Check connectivity and wait for rate limits before retrying; changing hashes is not a repair.\n" + result.stderr[-4000:])
    _run(ctx, "Building the freestanding Ghostty Wasm engine", [
        zig,
        "build",
        "-Demit-lib-vt",
        "-Demit-xcframework=false",
        "-Dtarget=wasm32-freestanding",
        "-Doptimize=ReleaseFast",
        "--prefix",
        str(ctx.path("wasm32")),
    ], environment, source)
    if not ctx.path("wasm32/lib/libghostty-vt.a").exists:
        fail("Ghostty's Wasm build did not produce its static C-ABI archive. Inspect the pinned build's artifacts; do not substitute a native library.")

    # Upstream's Wasm install omits headers; keep the same pinned source headers
    # for consumers and compile the ABI contract against the actual C layout.
    _run(ctx, "Keeping pinned Ghostty headers", ["cp", "-R", source + "/include", str(ctx.path("include"))], {}, ".")
    ctx.file("abi.c", ctx.read(ctx.attr.abi))
    _run(ctx, "Checking Ghostty's wasm32 C ABI", [zig, "cc", "-target", "wasm32-freestanding", "-std=c11", "-c", "-o", str(ctx.path("abi.o")), "-I" + str(ctx.path("include")), str(ctx.path("abi.c"))], environment, ".")
    ctx.file("LICENSE", ctx.read(ctx.path("src/LICENSE")))

    # Keep only the reproducible artifacts needed by consumers.
    for path in ["src", "zig", "zig-cache", "zig-cache-local", "zig-fetch-cache"]:
        ctx.delete(path)
    ctx.file("BUILD.bazel", _BUILD)

libghostty_vt_wasm = repository_rule(
    implementation = _impl,
    attrs = {"abi": attr.label(default = Label("//apps/humata2/terminal/vt:wasm_abi.c"), allow_single_file = True)},
    doc = "Pinned libghostty-vt wasm32 static archive and standalone module, built through Bazel.",
)
