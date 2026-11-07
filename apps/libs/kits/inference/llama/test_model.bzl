"""A real GGUF model for llama.cpp's smoke test: Qwen3.5 0.8B at Q4_K_M.

The same file as the app's catalog downloads, pinned to the same commit and
SHA-256: ModelCatalog.qwen35Tiny in
apps/libs/kits/inference/Sources/CatalogGGUF.swift. The test checks the two
pins agree, so update both together. It's 530 MB, fetched only when the
test builds.
"""

QWEN35_TINY = struct(
    repo = "unsloth/Qwen3.5-0.8B-GGUF",
    commit = "6ab461498e2023f6e3c1baea90a8f0fe38ab64d0",
    path = "Qwen3.5-0.8B-Q4_K_M.gguf",
    sha256 = "bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517",
)

def _gguf_test_model_impl(rctx):
    model = QWEN35_TINY
    rctx.download(
        url = "https://huggingface.co/{}/resolve/{}/{}".format(model.repo, model.commit, model.path),
        output = model.path,
        sha256 = model.sha256,
    )
    rctx.file("BUILD.bazel", """\
exports_files(
    ["{}"],
    visibility = ["@@//apps/libs/kits/inference/llama:__pkg__"],
)
""".format(model.path))

gguf_test_model = repository_rule(
    implementation = _gguf_test_model_impl,
    doc = "Downloads Qwen3.5 0.8B as the app's catalog pins it.",
)
