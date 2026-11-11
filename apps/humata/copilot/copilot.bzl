"""The GitHub Copilot CLI, which Humata's Copilot backend drives.

`@copilot_cli_<platform>//:copilot` is the CLI's standalone release binary
for one platform, pinned by version and the tarball's SHA-256 so every build
and test drives the same CLI. `//apps/humata/copilot` picks the one for the
target platform. Each platform's repository is fetched only when it is used,
and each is about 200 MB.

To update, change `_VERSION`, then take the SHA-256s from the release's
SHA256SUMS.txt (`gh release download v<version> -R github/copilot-cli -p
SHA256SUMS.txt`). Then update `COPILOT_VERSION` in
apps/humata/daemon/agent/src/copilot.rs, check that the protocol version the
CLI reports in `connect` is still the one copilot/rpc.rs speaks, and run
//apps/humata/daemon/agent:copilot_test.
"""

_VERSION = "1.0.110-1"

PLATFORMS = {
    "darwin_arm64": ("darwin-arm64", "a2eaeca7db61ff33c730579cf27839341141f180e204b58ee7886153c3c4b9f3"),
    "darwin_x64": ("darwin-x64", "0a12adb6d354d25c1d8b65a7216e9a37925ca8288b0d5b65c2ce354ab4b14478"),
    "linux_arm64": ("linux-arm64", "1e9d6a7d8a4753a35438107914ae948eb51bf28b544a1d2da55ea94701a633d8"),
    "linux_x64": ("linux-x64", "25be6b688877d0b7593cacc231b3dd441090a99b46aa0c0ed791fb1f63bb4c53"),
}

_BUILD = """\
licenses(["restricted"])

exports_files(["copilot"], visibility = ["@@//apps/humata/copilot:__pkg__"])
"""

def _copilot_cli_impl(repository_ctx):
    asset, sha256 = PLATFORMS[repository_ctx.attr.platform]
    repository_ctx.download_and_extract(
        url = "https://github.com/github/copilot-cli/releases/download/v{version}/copilot-{asset}.tar.gz".format(
            version = _VERSION,
            asset = asset,
        ),
        sha256 = sha256,
    )
    repository_ctx.file("BUILD.bazel", _BUILD)

copilot_cli = repository_rule(
    implementation = _copilot_cli_impl,
    attrs = {
        "platform": attr.string(mandatory = True, values = PLATFORMS.keys()),
    },
    doc = "The Copilot CLI's release binary for one platform, pinned to one version.",
)
