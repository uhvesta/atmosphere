"""A real MLX model for the smoke test: Qwen3.5 0.8B at 4 bits.

From mlx-community, pinned to one commit and each file's SHA-256. It's
630 MB, fetched only when the smoke test builds. The catalog pins the
same files, which the test checks.
"""

QWEN35_TINY_MLX = struct(
    repo = "mlx-community/Qwen3.5-0.8B-4bit",
    commit = "da28692b5f139cb0ec58a356b437486b7dac7462",
    directory = "Qwen3.5-0.8B-4bit",
    files = {
        "chat_template.jinja": "273d8e0e683b885071fb17e08d71e5f2a5ddfb5309756181681de4f5a1822d80",
        "config.json": "ba7770da23eae5ebd6827571f086e331956b33f4442a9e876fb4aa10969a6772",
        "model.safetensors": "f5a0d9dd3efa73510542a8023d610ff26be2b4b020d181cfc4bedaa1fcc5dd9e",
        "tokenizer.json": "87a7830d63fcf43bf241c3c5242e96e62dd3fdc29224ca26fed8ea333db72de4",
        "tokenizer_config.json": "e98f1901ac6f0adff67b1d540bfa0c36ac1a0cf59eb72ed78146ef89aafa1182",
    },
)

def _mlx_test_model_impl(rctx):
    model = QWEN35_TINY_MLX
    for path, sha256 in model.files.items():
        rctx.download(
            url = "https://huggingface.co/{}/resolve/{}/{}".format(model.repo, model.commit, path),
            output = model.directory + "/" + path,
            sha256 = sha256,
        )
    rctx.file("BUILD.bazel", """\
filegroup(
    name = "model",
    srcs = glob(["{}/**"]),
    visibility = ["@@//apps/libs/kits/inference/mlx:__pkg__"],
)

exports_files(
    ["{}/config.json"],
    visibility = ["@@//apps/libs/kits/inference/mlx:__pkg__"],
)
""".format(model.directory, model.directory))

mlx_test_model = repository_rule(
    implementation = _mlx_test_model_impl,
    doc = "Downloads Qwen3.5 0.8B in MLX's format, pinned by SHA-256.",
)
