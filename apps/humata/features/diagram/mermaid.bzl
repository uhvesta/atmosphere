"""mermaid.js, which draws the diagrams in Humata's Markdown.

`@mermaid_js//:mermaid.min.js` is the npm package's self-contained build,
pinned by version and the tarball's SHA-256, so every build draws with the
same one. It's bundled into the app and runs with no network.

To update, change `_VERSION` and `_SHA256` (`curl -sL <tarball> | shasum -a
256`, the tarball from `https://registry.npmjs.org/mermaid/latest`), then
run //apps/humata/features/diagram:tests.
"""

_VERSION = "12.3.0"

_SHA256 = "be0684e8d20bfc874df12c0f04927a5c482f57f2200eec54cca52a4583fd4040"

_BUILD = """\
licenses(["notice"])

exports_files(
    ["mermaid.min.js", "LICENSE"],
    visibility = ["@@//apps/humata/features/diagram:__pkg__"],
)
"""

def _mermaid_js_impl(repository_ctx):
    repository_ctx.download_and_extract(
        url = "https://registry.npmjs.org/mermaid/-/mermaid-{}.tgz".format(_VERSION),
        sha256 = _SHA256,
        stripPrefix = "package",
    )
    repository_ctx.symlink("dist/mermaid.min.js", "mermaid.min.js")
    repository_ctx.file("BUILD.bazel", _BUILD)

mermaid_js = repository_rule(
    implementation = _mermaid_js_impl,
    doc = "mermaid.js's self-contained build, pinned to one version.",
)
