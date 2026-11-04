"""Download checked tool binaries for the detected macOS or Linux host."""

load("//bazel/platforms:host.bzl", "host_binary_key")

_VALE_VERSION = "3.23.0"
_RUMDL_VERSION = "0.2.77"
_SHFMT_VERSION = "3.14.1"
_BUILDIFIER_VERSION = "10.0.1"
_STARPLS_VERSION = "0.1.22"
_LIMA_VERSION = "2.2.1"

_VALE = {
    "darwin_amd64": ("macOS_64-bit", "416fdd3ba32e32dc71c87b479cb86757bb6437bcebc7a3e4a095e863b7ce583d"),
    "darwin_arm64": ("macOS_arm64", "b913574b2c83b541d8bc2d8938e53a58a2fb06bab15135efcc755afb074f4430"),
    "linux_amd64": ("Linux_64-bit", "cc35445a45186b8f0b01e11c01359694cf941e72cf6ab0fc44774f0e54c9d5fc"),
    "linux_arm64": ("Linux_arm64", "45720aadcb01401ac394641287a2c74e094045a89a450f5edcf98c8969df0884"),
}

_RUMDL = {
    "darwin_amd64": ("x86_64-apple-darwin", "0951dbddfa6a28274d202581f136884474fc7225349519f7ad0eb6038c4dc8ab"),
    "darwin_arm64": ("aarch64-apple-darwin", "52744729012b44514b3b4de1431839035d7018464899c5b09627204eebfb73bb"),
    "linux_amd64": ("x86_64-unknown-linux-musl", "0a516c0590dfd2f3cb0abfc42580464c7fadb9e631cbdffbcc5c2546ae885ad0"),
    "linux_arm64": ("aarch64-unknown-linux-musl", "cd7e0cd23697c4c0e915fb65b3566f71b4a2a0e90a392b193c418971e9ed399f"),
}

_SHFMT = {
    "darwin_amd64": ("darwin_amd64", "d33eee0da0f92835b3562e9767a05cee7e4eaeef47daa03bfd09da17b4b590a6"),
    "darwin_arm64": ("darwin_arm64", "b7c872db63553ccffc7253aba3ed7d4885a27d83f1ba567b1138c6315a5847e5"),
    "linux_amd64": ("linux_amd64", "76e77641faa025814b77f153b29796b8e6fa2fca03e0c76a691608b86c7ea7bf"),
    "linux_arm64": ("linux_arm64", "5f2db09dae91fca848f7adbdd014632e921a383863a2ad7e0450ad3aba0c6489"),
}

_BUILDIFIER = {
    "darwin_amd64": "1d02bb9148cadf2cbee330f9bd657352c765b52b68a03d970e10e47706bdc436",
    "darwin_arm64": "afb78f350319b59cc51d6add3a5f3ba68e63e5d88f68c5a9ea6328a07084d319",
    "linux_amd64": "e0ea28e2d639347724435ebafe0531fd764fbf20eec6a23000c81edd0d58e51d",
    "linux_arm64": "6d7aebd23aa85847a66d517bb6220d95f24a2752e62cce0f089145b680b539c7",
}

_STARPLS = {
    "darwin_amd64": ("darwin-amd64", "97967f041d950c1055664a8f1afc36b01c6793b2fb3af488fd20139720d32131"),
    "darwin_arm64": ("darwin-arm64", "675b7be4554e6c219b6774a6b814ec21061096e08ecdd8b8aeeaf3913eb20a4e"),
    "linux_amd64": ("linux-amd64", "7c661cdde0d1c026665086d07523d825671e29056276681616bb32d0273c5eab"),
    "linux_arm64": ("linux-aarch64", "55877ec4c3ff03e1d90d59c76f69a3a144b6c29688747c8ac4d77993e2eef1ad"),
}

_LIMA = {
    "darwin_amd64": ("Darwin-x86_64", "6b776e4bc41f358e6ee01fefcc78e2eb26f11635a0fcff632234af77a566c6c4"),
    "darwin_arm64": ("Darwin-arm64", "9e9eacce88f37e185c346bad73aa6136f738d8cdf8c3bb23cd42b071824bc66e"),
    "linux_amd64": ("Linux-x86_64", "b391ac7fcac2b2a5a1628107756463188ded4d18d3104a40a66a8bb6fec01bcb"),
    "linux_arm64": ("Linux-aarch64", "fbcf4d127936666aa2cf6013b663c250431f2a59394757b404070f8d6a160e10"),
}

def _lima_impl(ctx):
    ctx.download_and_extract(url = ctx.attr.url, sha256 = ctx.attr.sha256)

    # limactl finds its guest agents in ../share/lima, so keep the tree whole.
    ctx.file("BUILD.bazel", """\
exports_files(["bin/limactl"], visibility = {vis})

filegroup(
    name = "runtime",
    srcs = glob(["libexec/**", "share/**"]),
    visibility = {vis},
)
""".format(vis = repr(ctx.attr.export_visibility)))

_lima = repository_rule(
    implementation = _lima_impl,
    attrs = {
        "export_visibility": attr.string_list(mandatory = True),
        "sha256": attr.string(mandatory = True),
        "url": attr.string(mandatory = True),
    },
)

def _archive_tool_impl(ctx):
    ctx.download_and_extract(url = ctx.attr.url, sha256 = ctx.attr.sha256)
    ctx.file("BUILD.bazel", 'exports_files(["{}"], visibility = {})\n'.format(ctx.attr.binary, repr(ctx.attr.export_visibility)))

_archive_tool = repository_rule(
    implementation = _archive_tool_impl,
    attrs = {
        "binary": attr.string(mandatory = True),
        "export_visibility": attr.string_list(mandatory = True),
        "sha256": attr.string(mandatory = True),
        "url": attr.string(mandatory = True),
    },
)

def _binary_tool_impl(ctx):
    ctx.download(url = ctx.attr.url, output = ctx.attr.binary, sha256 = ctx.attr.sha256, executable = True)
    ctx.file("BUILD.bazel", 'exports_files(["{}"], visibility = {})\n'.format(ctx.attr.binary, repr(ctx.attr.export_visibility)))

_binary_tool = repository_rule(
    implementation = _binary_tool_impl,
    attrs = {
        "binary": attr.string(mandatory = True),
        "export_visibility": attr.string_list(mandatory = True),
        "sha256": attr.string(mandatory = True),
        "url": attr.string(mandatory = True),
    },
)

def _host_tools_impl(module_ctx):
    host_key = host_binary_key(module_ctx.os)
    vale_platform, vale_sha = _VALE[host_key]
    rumdl_platform, rumdl_sha = _RUMDL[host_key]
    shfmt_platform, shfmt_sha = _SHFMT[host_key]
    starpls_platform, starpls_sha = _STARPLS[host_key]
    lima_platform, lima_sha = _LIMA[host_key]

    _archive_tool(
        name = "vale",
        url = "https://github.com/vale-cli/vale/releases/download/v{0}/vale_{0}_{1}.tar.gz".format(_VALE_VERSION, vale_platform),
        sha256 = vale_sha,
        binary = "vale",
        export_visibility = ["@//bazel/lint:__pkg__"],
    )
    _archive_tool(
        name = "rumdl",
        url = "https://github.com/rvben/rumdl/releases/download/v{0}/rumdl-v{0}-{1}.tar.gz".format(_RUMDL_VERSION, rumdl_platform),
        sha256 = rumdl_sha,
        binary = "rumdl",
        export_visibility = ["@//bazel/format:__pkg__"],
    )
    _binary_tool(
        name = "shfmt",
        url = "https://github.com/mvdan/sh/releases/download/v{0}/shfmt_v{0}_{1}".format(_SHFMT_VERSION, shfmt_platform),
        sha256 = shfmt_sha,
        binary = "shfmt",
        export_visibility = ["@//bazel/format:__pkg__"],
    )
    _binary_tool(
        name = "buildifier",
        url = "https://github.com/bazelbuild/buildtools/releases/download/v{0}/buildifier-{1}".format(_BUILDIFIER_VERSION, host_key.replace("_", "-")),
        sha256 = _BUILDIFIER[host_key],
        binary = "buildifier",
        export_visibility = ["@//:__subpackages__"],
    )
    _binary_tool(
        name = "starpls",
        url = "https://github.com/withered-magic/starpls/releases/download/v{0}/starpls-{1}".format(_STARPLS_VERSION, starpls_platform),
        sha256 = starpls_sha,
        binary = "starpls",
        export_visibility = ["@//tools/ide:__pkg__"],
    )
    _lima(
        name = "lima",
        url = "https://github.com/lima-vm/lima/releases/download/v{0}/lima-{0}-{1}.tar.gz".format(_LIMA_VERSION, lima_platform),
        sha256 = lima_sha,
        export_visibility = ["@//tools/cross:__pkg__"],
    )
    return module_ctx.extension_metadata(reproducible = True)

host_tools = module_extension(
    implementation = _host_tools_impl,
    os_dependent = True,
    arch_dependent = True,
)
