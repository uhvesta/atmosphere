"""A real model for the end-to-end test of the assistant: Gemma 4 E2B.

The same file as the app's catalog downloads, pinned to the same commit and
SHA-256: ModelCatalog.gemma4E2B in
apps/lifr/clients/models/Sources/Catalog.swift. The test checks the two
pins agree, so update both together. It's 2.6 GB, fetched only when the
test builds.
"""

GEMMA_4_E2B = struct(
    repo = "litert-community/gemma-4-E2B-it-litert-lm",
    commit = "c658da437244f072bf2f1cabc916b60d66d97663",
    path = "gemma-4-E2B-it.litertlm",
    sha256 = "181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c",
)

def _test_model_impl(rctx):
    model = GEMMA_4_E2B
    rctx.download(
        url = "https://huggingface.co/{}/resolve/{}/{}".format(model.repo, model.commit, model.path),
        output = model.path,
        sha256 = model.sha256,
    )
    rctx.file("BUILD.bazel", """\
exports_files(
    ["{}"],
    visibility = ["@@//apps/lifr/features/root:__pkg__"],
)
""".format(model.path))

test_model = repository_rule(
    implementation = _test_model_impl,
    doc = "Downloads Gemma 4 E2B as the app's catalog pins it.",
)
