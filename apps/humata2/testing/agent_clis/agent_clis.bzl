"""Test-only agent CLIs for Humata 2's Linux VM rig.

`@h2_copilot_<platform>//:bin` is the GitHub Copilot CLI's standalone release
binary and `@h2_codex_<platform>//:bin` is OpenAI's Codex CLI, each pinned by
version and the release tarball's SHA-256 so every rig run drives the same
CLIs. `//apps/humata2/testing/agent_clis` names them per platform, and its
`install_in_vm` target copies the Linux ones into a Lima VM's
`~/.humata2/test-bin/`. Each repository is fetched only when something uses
it, and each is 100 to 300 MB. Everything here is `testonly` and visible only
inside apps/humata2.

This is separate from apps/humata/copilot/copilot.bzl, which pins an older
Copilot for Humata 1. Humata 2's conversation adapter needs Copilot 1.0.112-0
or newer (D619; ROAM proved 1.0.113-1).

To update Copilot, change `_COPILOT_VERSION`, then take the SHA-256s from the
release's SHA256SUMS.txt (`gh release download v<version> -R
github/copilot-cli -p SHA256SUMS.txt`): the lines for
`copilot-<asset>.tar.gz`, not the `github-copilot-*.tgz` ones. Then run
//apps/humata2/conversation:all against the new CLI in the VM rig.

To update Codex, pick the newest stable (not alpha) release of openai/codex at
least 3 days old, set `_CODEX_VERSION` to its number (tag `rust-v<version>`),
and take each SHA-256 from `gh release view rust-v<version> -R openai/codex
--json assets` (the `digest` of `codex-<triple>.tar.gz`; if a release has none,
download the asset and run `shasum -a 256` on it). The tarball holds one file,
`codex-<triple>`, which the rule renames to `codex`. Both rules fail with a
re-pin message if the expected member is missing.
"""

_COPILOT_VERSION = "1.0.113-1"

COPILOT_PLATFORMS = {
    "darwin_arm64": ("darwin-arm64", "619170fc5c6c4371ac72b788176d35c705b4b1b21af0be8a7f3fc92db0f88559"),
    "linux_arm64": ("linux-arm64", "05dbc3af7d70c774be5e16e8edcd851263b405da94db068417869cb7fb3169f4"),
    "linux_x64": ("linux-x64", "9ba5f8033642d7457623c17380559b66bdc136c4420a32683a3b87a91b4e8f29"),
}

_CODEX_VERSION = "0.169.0"

CODEX_PLATFORMS = {
    "darwin_arm64": ("aarch64-apple-darwin", "7f4c49c80bae1d0b4e347acfcdf11faa3475d87ae5e65a5eb1261d3c358145ee"),
    "linux_arm64": ("aarch64-unknown-linux-musl", "97b97d87eea075c5cb7709c7f6156a451622a7667953ecda40e0276637196f3f"),
    "linux_x64": ("x86_64-unknown-linux-musl", "9af4d751fce183faedea629437d5252dbf458eeb04130cf4f97be844621ea985"),
}

_BUILD = """\
package(default_testonly = True)

licenses(["{license}"])

filegroup(
    name = "bin",
    srcs = ["{name}"],
    visibility = ["@@//apps/humata2:__subpackages__"],
)
"""

def _check_member(repository_ctx, member, archive, version):
    if not repository_ctx.path(member).exists:
        fail((
            "The {archive} archive for version {version} has no '{member}' in it, so the layout changed. " +
            "Look at its members (tar -tzf), then fix the member name in agent_clis.bzl and re-pin the " +
            "version and SHA-256s as that file's docstring says."
        ).format(archive = archive, version = version, member = member))

def _copilot_impl(repository_ctx):
    asset, sha256 = COPILOT_PLATFORMS[repository_ctx.attr.platform]
    repository_ctx.download_and_extract(
        url = "https://github.com/github/copilot-cli/releases/download/v{version}/copilot-{asset}.tar.gz".format(
            version = _COPILOT_VERSION,
            asset = asset,
        ),
        sha256 = sha256,
    )
    _check_member(repository_ctx, "copilot", "copilot-{}.tar.gz".format(asset), _COPILOT_VERSION)
    repository_ctx.file("BUILD.bazel", _BUILD.format(name = "copilot", license = "restricted"))

def _codex_impl(repository_ctx):
    triple, sha256 = CODEX_PLATFORMS[repository_ctx.attr.platform]
    repository_ctx.download_and_extract(
        url = "https://github.com/openai/codex/releases/download/rust-v{version}/codex-{triple}.tar.gz".format(
            version = _CODEX_VERSION,
            triple = triple,
        ),
        sha256 = sha256,
    )
    _check_member(repository_ctx, "codex-" + triple, "codex-{}.tar.gz".format(triple), _CODEX_VERSION)
    repository_ctx.rename("codex-" + triple, "codex")
    repository_ctx.file("BUILD.bazel", _BUILD.format(name = "codex", license = "notice"))

h2_copilot_cli = repository_rule(
    implementation = _copilot_impl,
    attrs = {
        "platform": attr.string(mandatory = True, values = COPILOT_PLATFORMS.keys()),
    },
    doc = "The Copilot CLI's release binary for one platform, pinned to one version, test-only.",
)

h2_codex_cli = repository_rule(
    implementation = _codex_impl,
    attrs = {
        "platform": attr.string(mandatory = True, values = CODEX_PLATFORMS.keys()),
    },
    doc = "The Codex CLI's release binary for one platform, pinned to one version, test-only.",
)
