"""Pinned wasm-bindgen packaging tool, independent of Cargo execution."""
_VERSION = "0.2.127"
_RELEASES = {
    "darwin_arm64": ("aarch64-apple-darwin", "cd93e691eb5953ace5d8ffce52a20b024077a3dac3e2215b8224136b0efb7585"),
    "darwin_amd64": ("x86_64-apple-darwin", "81049c79f4e283e1725e6582a0528af1301a70ad23add1df1c4d042ec825263d"),
    "linux_arm64": ("aarch64-unknown-linux-musl", "699ddbb724ece16f912b73a98b5341a75f4b8c7886ce152d40dbe09f694e46c7"),
    "linux_amd64": ("x86_64-unknown-linux-musl", "61d4a7dc85acfa0d2354ccc0b8361928c7e52a746d17f28ebaa795ed3dc1614a"),
}

def _cli_impl(ctx):
    ctx.download_and_extract(
        url = ctx.attr.url,
        sha256 = ctx.attr.sha256,
        stripPrefix = ctx.attr.prefix,
    )
    ctx.file("BUILD.bazel", 'exports_files(["wasm-bindgen"], visibility = ["@@//apps/humata2/terminal/web:__pkg__"])\n')

_cli = repository_rule(
    implementation = _cli_impl,
    attrs = {"url": attr.string(), "sha256": attr.string(), "prefix": attr.string()},
)

def _tools_impl(ctx):
    os = "darwin" if "mac" in ctx.os.name else "linux"
    arch = "arm64" if ctx.os.arch in ["aarch64", "arm64"] else "amd64"
    triple, checksum = _RELEASES[os + "_" + arch]
    prefix = "wasm-bindgen-{}-{}".format(_VERSION, triple)
    _cli(name = "terminal_wasm_bindgen", url = "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/{}/{}.tar.gz".format(_VERSION, prefix), sha256 = checksum, prefix = prefix)
    return ctx.extension_metadata(reproducible = True)

wasm_tools = module_extension(implementation = _tools_impl, os_dependent = True, arch_dependent = True)
