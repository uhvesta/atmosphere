/// The catalog's GGUF models, which llama.cpp runs, each at Q4_K_M: the
/// smallest of their makers' quantizations that keeps their answers close
/// to full precision. Their memory is estimated, not measured: the file,
/// the window's keys and values at 8 bits (the bytes per token below,
/// from each file's header), and about 300 MB for llama.cpp's buffers.
/// The planner uses it until the person measures the model on their
/// device.
extension ModelCatalog {
    static let gguf: [ModelSpec] = [
        qwen35Tiny, qwen35Small, qwen35Medium, qwen35Large, lfm3, lfm3VL2B, lfm25Thinking, lfm25MoE, granite42Small,
        granite42Large,
    ]

    /// Alibaba's Qwen3.5 0.8B in GGUF, where MLX can't run it. A quarter
    /// of its layers attend; the rest keep a fixed state.
    static let qwen35TinyGGUF = ggufModel(
        id: "qwen3.5-0.8b", name: "Qwen3.5 0.8B", summary: "Tiny and quick, thinks when asked",
        repo: "unsloth/Qwen3.5-0.8B-GGUF", commit: "6ab461498e2023f6e3c1baea90a8f0fe38ab64d0",
        file: ModelFile(
            path: "Qwen3.5-0.8B-Q4_K_M.gguf", size: 532_517_120,
            digest: .sha256("bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517")),
        bytesPerToken: 6_528, license: "Apache 2.0", profile: qwen35)

    /// Qwen3.5 2B in GGUF.
    static let qwen35SmallGGUF = ggufModel(
        id: "qwen3.5-2b", name: "Qwen3.5 2B", summary: "Small, thinks when asked",
        repo: "unsloth/Qwen3.5-2B-GGUF", commit: "f6d5376be1edb4d416d56da11e5397a961aca8ae",
        file: ModelFile(
            path: "Qwen3.5-2B-Q4_K_M.gguf", size: 1_280_835_840,
            digest: .sha256("aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223")),
        bytesPerToken: 6_528, license: "Apache 2.0", profile: qwen35)

    /// Qwen3.5 4B in GGUF.
    static let qwen35MediumGGUF = ggufModel(
        id: "qwen3.5-4b", name: "Qwen3.5 4B", summary: "Careful, thinks when asked",
        repo: "unsloth/Qwen3.5-4B-GGUF", commit: "e87f176479d0855a907a41277aca2f8ee7a09523",
        file: ModelFile(
            path: "Qwen3.5-4B-Q4_K_M.gguf", size: 2_740_937_888,
            digest: .sha256("00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4")),
        bytesPerToken: 17_408, license: "Apache 2.0", profile: qwen35)

    /// Qwen3.5 9B in GGUF.
    static let qwen35LargeGGUF = ggufModel(
        id: "qwen3.5-9b", name: "Qwen3.5 9B", summary: "The largest, for a Mac",
        repo: "unsloth/Qwen3.5-9B-GGUF", commit: "3885219b6810b007914f3a7950a8d1b469d598a5",
        file: ModelFile(
            path: "Qwen3.5-9B-Q4_K_M.gguf", size: 5_680_522_464,
            digest: .sha256("03b74727a860a56338e042c4420bb3f04b2fec5734175f4cb9fa853daf52b7e8")),
        bytesPerToken: 17_408, license: "Apache 2.0", profile: qwen35)

    /// Liquid AI's LFM3 1.7B. Most of its layers are convolutions, so its
    /// window takes little memory.
    public static let lfm3 = ggufModel(
        id: "lfm3-1.7b", name: "LFM3 1.7B", summary: "Quick on any device, thinks when asked",
        repo: "LiquidAI/LFM3-1.7B-GGUF", commit: "cd0c74c575b5c416f3011b5b4c298077f26ca8c2",
        file: ModelFile(
            path: "LFM3-1.7B-Q4_K_M.gguf", size: 1_099_627_808,
            digest: .sha256("3793e1e6e2266919d6e015287221b41100175a98eddaba92e111aed3b05c93c4")),
        bytesPerToken: 6_528, license: "LFM Open License 1.0", profile: lfm3Thinks)

    /// LFM3 VL 2B, LFM3 with a vision encoder, at 8 bits.
    public static let lfm3VL2B = ggufModel(
        id: "lfm3-vl-2b", name: "LFM3 VL 2B", summary: "Small, reads photos",
        repo: "LiquidAI/LFM3-VL-2B-GGUF", commit: "1add221a5ea6c047fcf1eaaf2d247939d7cf8849",
        file: ModelFile(
            path: "LFM3-VL-2B-Q4_K_M.gguf", size: 1_099_627_808,
            digest: .sha256("56dac2e192ff0d9ab8507f4f1aeb4aed0463449bbbefaac79497f041c95f21eb")),
        encoder: ModelFile(
            path: "mmproj-LFM3-VL-2B-Q8_0.gguf", size: 203_031_872,
            digest: .sha256("c5c5b58d544a18da919172d2d0368090ea1a4a351b6feb7f8a326a264a5918ab")),
        bytesPerToken: 6_528, license: "LFM Open License 1.0", profile: lfm3Thinks)

    /// LFM2.5 1.2B Thinking, which always thinks first; its reasoning
    /// shows only when the turn asks for it.
    public static let lfm25Thinking = ggufModel(
        id: "lfm2.5-1.2b-thinking", name: "LFM2.5 1.2B Thinking", summary: "Small, always thinks",
        repo: "LiquidAI/LFM2.5-1.2B-Thinking-GGUF", commit: "b71945805f7d4d87dabd73e26098d44bcad6c35f",
        file: ModelFile(
            path: "LFM2.5-1.2B-Thinking-Q4_K_M.gguf", size: 730_899_072,
            digest: .sha256("4a1e1824f71091f368137b6966094461019f6af3a5aadd96033bee59fc9eaeea")),
        bytesPerToken: 6_528, license: "LFM Open License 1.0", profile: lfmAlwaysThinks)

    /// LFM2.5 8B-A1B, a mixture of experts of which about 1B weights run
    /// for each token: as quick as a small model, with a large one's
    /// memory.
    public static let lfm25MoE = ggufModel(
        id: "lfm2.5-8b-a1b", name: "LFM2.5 8B-A1B", summary: "Large but quick, always thinks",
        repo: "LiquidAI/LFM2.5-8B-A1B-GGUF", commit: "49c14831707011e64d70b2ebd8462ba08d608434",
        file: ModelFile(
            path: "LFM2.5-8B-A1B-Q4_K_M.gguf", size: 5_155_564_768,
            digest: .sha256("4923ec14f06b968b74d663e5949867d2d9c3bf13a20b8be1a9f9af39989b2bb0")),
        bytesPerToken: 6_528, license: "LFM Open License 1.0", profile: lfmAlwaysThinks)

    /// IBM's Granite 4.2 3B. Every layer attends, so its window takes more
    /// memory than the others'.
    public static let granite42Small = ggufModel(
        id: "granite-4.2-3b", name: "Granite 4.2 3B", summary: "Steady with tools, thinks when asked",
        repo: "ibm-granite/granite-4.2-3b-GGUF", commit: "c40945d71cd90f249a56985e8155551a9188dc30",
        file: ModelFile(
            path: "granite-4.2-3b-Q4_K_M.gguf", size: 2_244_011_552,
            digest: .sha256("e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5")),
        bytesPerToken: 43_520, license: "Apache 2.0", profile: granite)

    /// Granite 4.2 8B.
    public static let granite42Large = ggufModel(
        id: "granite-4.2-8b", name: "Granite 4.2 8B", summary: "Larger and steadier, for a Mac",
        repo: "ibm-granite/granite-4.2-8b-GGUF", commit: "93f3f6a8938ee922b784cf4e5b4203cd3428df8f",
        file: ModelFile(
            path: "granite-4.2-8b-Q4_K_M.gguf", size: 5_347_917_952,
            digest: .sha256("16a9369d0805f80b7377d25d87f937a90c05dc04ad79173a52001e42c9aab311")),
        bytesPerToken: 87_040, license: "Apache 2.0", profile: granite)

    /// How Qwen3.5 runs: as Alibaba suggests for answers without thinking,
    /// and told to think, or not, by what starts its answer.
    static let qwen35 = ModelProfile(
        sampling: ModelProfile.Sampling(topK: 20, topP: 0.8, temperature: 0.7),
        thinking: ModelProfile.Thinking(on: "<think>\n", off: "<think>\n\n</think>\n\n", end: "</think>"))

    /// How LFM3 runs: cooler than most, as Liquid AI suggests.
    static let lfm3Thinks = ModelProfile(
        sampling: ModelProfile.Sampling(topK: 50, topP: 0.95, temperature: 0.3),
        thinking: ModelProfile.Thinking(on: "<think>", off: "<think></think>", end: "</think>"))

    /// How the LFM2.5 models that always think run.
    static let lfmAlwaysThinks = ModelProfile(
        sampling: ModelProfile.Sampling(topK: 50, topP: 0.95, temperature: 0.3),
        thinking: ModelProfile.Thinking(on: "<think>", off: nil, end: "</think>"))

    /// How Granite 4.2 runs.
    static let granite = ModelProfile(
        sampling: ModelProfile.Sampling(topK: 40, topP: 0.95, temperature: 0.6),
        thinking: ModelProfile.Thinking(on: "<think>\n", off: "<think></think>", end: "</think>"))

    /// The memory llama.cpp takes besides the weights and the window, for
    /// its buffers, estimated.
    static let llamaOverhead: Int64 = 300_000_000

    private static func ggufModel(
        id: ModelID, name: String, summary: String, repo: String, commit: String, file: ModelFile,
        encoder: ModelFile? = nil, bytesPerToken: Int64, license: String, profile: ModelProfile
    ) -> ModelSpec {
        let window = 8_192
        let files = [file] + (encoder.map { [$0] } ?? [])
        let weights = files.reduce(0) { $0 + $1.size }
        let memory = weights + Int64(window) * bytesPerToken + llamaOverhead
        return ModelSpec(
            id: id, name: name, summary: summary, runtime: .llama,
            source: ModelSource(repo: repo, commit: commit, files: files), contextTokens: window,
            memoryBytes: memory, cpuMemoryBytes: memory, license: license, inputs: encoder == nil ? [] : [.image],
            contextChoices: [4_096, 8_192, 16_384, 32_768], memoryBytesPerToken: bytesPerToken, profile: profile,
            quantization: "Q4_K_M")
    }
}
