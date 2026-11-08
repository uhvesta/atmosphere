/// Qwen3.5 in MLX's format at 4 bits, which MLX runs on the GPU of a Mac
/// or an iPhone, falling back to the same model in GGUF on llama.cpp. Each
/// is a folder: its config, tokenizer, chat template and weights, from
/// mlx-community. Their memory is estimated, not measured: the weights,
/// the window's keys and values at 16 bits (twice llama.cpp's 8), and
/// about 300 MB for MLX's buffers.
extension ModelCatalog {
    /// Alibaba's Qwen3.5 0.8B, the smallest that thinks when asked.
    public static let qwen35Tiny = qwen35(
        qwen35TinyGGUF, repo: "mlx-community/Qwen3.5-0.8B-4bit", commit: "da28692b5f139cb0ec58a356b437486b7dac7462",
        config: ModelFile(
            path: "config.json", size: 3_112,
            digest: .sha256("ba7770da23eae5ebd6827571f086e331956b33f4442a9e876fb4aa10969a6772")),
        template: smallTemplate,
        weights: [
            ModelFile(
                path: "model.safetensors", size: 625_229_487,
                digest: .sha256("f5a0d9dd3efa73510542a8023d610ff26be2b4b020d181cfc4bedaa1fcc5dd9e"))
        ],
        bytesPerToken: 6_528)

    /// Qwen3.5 2B.
    public static let qwen35Small = qwen35(
        qwen35SmallGGUF, repo: "mlx-community/Qwen3.5-2B-4bit", commit: "674aaa7240b91e8012fcad5d791b7dfe5ba90207",
        config: ModelFile(
            path: "config.json", size: 3_113,
            digest: .sha256("beb7fc5a6e0405fe332821cf1a8ef7b69bb390a8c8933171647de5579debf949")),
        template: smallTemplate,
        weights: [
            ModelFile(
                path: "model.safetensors", size: 1_722_271_785,
                digest: .sha256("713fe7e5d3c3965f7106b0d0ee17615f7869c23c8d327996df8c1196fbcf07d5"))
        ],
        bytesPerToken: 6_528)

    /// Qwen3.5 4B.
    public static let qwen35Medium = qwen35(
        qwen35MediumGGUF, repo: "mlx-community/Qwen3.5-4B-4bit", commit: "0e7ffd5c629ef7719d4cbc04069232580bfa9d9c",
        config: ModelFile(
            path: "config.json", size: 3_366,
            digest: .sha256("f3efc81b2ea8d96a45301037d3ccccbcccdef44a961845c87f286aaddbc6eaaa")),
        template: largeTemplate,
        weights: [
            ModelFile(
                path: "model.safetensors", size: 3_034_300_695,
                digest: .sha256("5fb9acd0246866381cf8c5c354c6db1019f6498eec4ccb4f5edcc71ffeacb2db"))
        ],
        bytesPerToken: 17_408)

    /// Qwen3.5 9B, which fits a Mac or an iPhone with 12 GB.
    public static let qwen35Large = qwen35(
        qwen35LargeGGUF, repo: "mlx-community/Qwen3.5-9B-4bit", commit: "8b2b98c00a6b4d291155e4890773ca8f769aee53",
        config: ModelFile(
            path: "config.json", size: 3_331,
            digest: .sha256("a96942cb6a8a1d3f1d17514d81a1925d04362a6a3233b389d13012211baaa9f8")),
        template: largeTemplate,
        weights: [
            ModelFile(
                path: "model-00001-of-00002.safetensors", size: 5_349_771_222,
                digest: .sha256("a68b87558c6ef43f74c2bd63ce7e9092ceddc3101f3def0030774bae5f42aadd")),
            ModelFile(
                path: "model-00002-of-00002.safetensors", size: 600_449_850,
                digest: .sha256("b0a770bf8469c7f3f18756a0e0283f1c1174344a83e059a4e483f6af4907352d")),
        ],
        bytesPerToken: 17_408)

    /// MLX's buffers, past the weights and the window.
    static let mlxOverhead: Int64 = 300_000_000

    /// The tokenizer every Qwen3.5 size shares.
    private static let tokenizer = [
        ModelFile(
            path: "tokenizer.json", size: 19_989_343,
            digest: .sha256("87a7830d63fcf43bf241c3c5242e96e62dd3fdc29224ca26fed8ea333db72de4")),
        ModelFile(
            path: "tokenizer_config.json", size: 1_139,
            digest: .sha256("e98f1901ac6f0adff67b1d540bfa0c36ac1a0cf59eb72ed78146ef89aafa1182")),
    ]

    /// The chat template of the 0.8B and 2B.
    private static let smallTemplate = ModelFile(
        path: "chat_template.jinja", size: 7_755,
        digest: .sha256("273d8e0e683b885071fb17e08d71e5f2a5ddfb5309756181681de4f5a1822d80"))

    /// The chat template of the 4B and 9B.
    private static let largeTemplate = ModelFile(
        path: "chat_template.jinja", size: 7_756,
        digest: .sha256("a4aee8afcf2e0711942cf848899be66016f8d14a889ff9ede07bca099c28f715"))

    /// `gguf`, running on MLX first, from `repo`'s files.
    private static func qwen35(
        _ gguf: ModelSpec, repo: String, commit: String, config: ModelFile, template: ModelFile,
        weights: [ModelFile], bytesPerToken: Int64
    ) -> ModelSpec {
        let files = [config] + tokenizer + [template] + weights
        let mlxBytesPerToken = bytesPerToken * 2
        let memory =
            weights.reduce(0) { $0 + $1.size } + Int64(gguf.contextTokens) * mlxBytesPerToken + mlxOverhead
        let mlx = ModelVariant(
            runtime: .mlx, quantization: "4bit", source: ModelSource(repo: repo, commit: commit, files: files),
            memoryBytes: memory, cpuMemoryBytes: memory, memoryBytesPerToken: mlxBytesPerToken,
            profile: gguf.profile)
        return ModelSpec(
            id: gguf.id, name: gguf.name, summary: gguf.summary, contextTokens: gguf.contextTokens,
            license: gguf.license, inputs: gguf.inputs, contextChoices: gguf.contextChoices,
            variants: [mlx] + gguf.variants)
    }
}
