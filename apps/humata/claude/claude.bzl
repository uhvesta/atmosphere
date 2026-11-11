"""Claude Code, which Humata's Claude backend drives.

`@claude_cli_<platform>//:claude` is Claude Code's native release binary for
one platform, pinned by version and its SHA-256 so every build and test
drives the same CLI. `//apps/humata/claude` picks the one for the target
platform. Each platform's repository is fetched only when it is used, and
each is about 330 MB.

To update, change `_VERSION`, then take the checksums from the release's
manifest:

    curl -fsS https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases/<version>/manifest.json

Then update `CLAUDE_VERSION` in apps/humata/daemon/agent/src/claude.rs and
run //apps/humata/daemon/agent:claude_test, which checks the stream-json
protocol the backend speaks.
"""

_VERSION = "2.1.322"

_RELEASES = "https://storage.googleapis.com/claude-code-dist-86c565f3-f756-42ad-8dfa-d59b1c096819/claude-code-releases"

PLATFORMS = {
    "darwin_arm64": ("darwin-arm64", "a9d237949db816a0b7afd88a9a8e1df36b3a597a30f5a41d1c0539eb180a3450"),
    "darwin_x64": ("darwin-x64", "b513d15117cbf1bb05095dab41963fd394ff40de1fa99dcc6d7d1033bf292e9b"),
    "linux_arm64": ("linux-arm64", "f220ca50ea4f3222043ff83cc224a1262e0196e9897acc0871efa94235b0c1fd"),
    "linux_x64": ("linux-x64", "bfb3703ad5380242328459bc62ada4bde8ae7ba57c6e1eeadb9c00e08874c478"),
}

_BUILD = """\
licenses(["restricted"])

exports_files(["claude"], visibility = ["@@//apps/humata/claude:__pkg__"])
"""

def _claude_cli_impl(repository_ctx):
    platform, sha256 = PLATFORMS[repository_ctx.attr.platform]
    repository_ctx.download(
        url = "{releases}/{version}/{platform}/claude".format(
            releases = _RELEASES,
            version = _VERSION,
            platform = platform,
        ),
        output = "claude",
        sha256 = sha256,
        executable = True,
    )
    repository_ctx.file("BUILD.bazel", _BUILD)

claude_cli = repository_rule(
    implementation = _claude_cli_impl,
    attrs = {
        "platform": attr.string(mandatory = True, values = PLATFORMS.keys()),
    },
    doc = "Claude Code's native release binary for one platform, pinned to one version.",
)
