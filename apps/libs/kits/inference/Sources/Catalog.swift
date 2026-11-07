/// The models the app offers without a link, each pinned to a commit and its
/// files' SHA-256, as Hugging Face listed them.
public enum ModelCatalog {
    public static let all: [ModelSpec] = [apple, gemma4E2B, gemma4E4B]

    /// The context the app gives a model it knows nothing about.
    public static let defaultContextTokens = 4_096

    /// Apple's on-device model.
    public static let apple = ModelSpec(
        id: .apple, name: "Apple Intelligence", summary: "Built in, nothing to download", runtime: .apple,
        source: nil, contextTokens: ContextBudget.appleOnDevice.window, memoryBytes: 0, license: "Apple")

    /// Google's Gemma 4 E2B, for the GPU through LiteRT-LM. Measured by
    /// Google at 1.45 GB on an iPhone 17 Pro, at 2,048 tokens, and 607 MB on
    /// its CPU; the CPU figure here leaves room for the app's 8,192 tokens.
    /// Its drafter stays off: on a Mac's GPU it wrote 12 to 28% faster for
    /// 260 MB more (`SpeedOnRealModelTests`), but it changed the answers,
    /// and with it the model drafted nothing for a new Avenue with tasks
    /// (`RequestsOnRealModelTests`).
    /// Each token of window past that takes about 150 KB, measured on a
    /// Mac's GPU at 16K and 32K (`ContextWindowOnRealModelTests`).
    public static let gemma4E2B = ModelSpec(
        id: "gemma-4-e2b", name: "Gemma 4 E2B", summary: "Small and quick", runtime: .liteRT,
        source: ModelSource(
            repo: "litert-community/gemma-4-E2B-it-litert-lm", commit: "c658da437244f072bf2f1cabc916b60d66d97663",
            files: [
                ModelFile(
                    path: "gemma-4-E2B-it.litertlm", size: 2_588_147_712,
                    digest: .sha256("181938105e0eefd105961417e8da75903eacda102c4fce9ce90f50b97139a63c"))
            ]), contextTokens: 8_192, memoryBytes: 1_450_000_000, cpuMemoryBytes: 1_000_000_000,
        license: "Apache 2.0", contextChoices: gemmaWindows, memoryBytesPerToken: 150_000,
        profile: gemma4)

    /// Google's Gemma 4 E4B, larger and slower than E2B, and the one that
    /// reads photos and listens to recordings. Measured by Google at 3.38 GB
    /// on an iPhone 17 Pro, at 2,048 tokens. Its window's memory isn't
    /// measured: twice E2B's per token, for its larger layers, so the
    /// planner errs towards the CPU. Its drafter stays off, as E2B's does.
    public static let gemma4E4B = ModelSpec(
        id: "gemma-4-e4b", name: "Gemma 4 E4B", summary: "Reads photos and recordings", runtime: .liteRT,
        source: ModelSource(
            repo: "litert-community/gemma-4-E4B-it-litert-lm", commit: "e6a3892a8c22cf87ff99583996149cf3cb32452b",
            files: [
                ModelFile(
                    path: "gemma-4-E4B-it.litertlm", size: 3_659_530_240,
                    digest: .sha256("0b2a8980ce155fd97673d8e820b4d29d9c7d99b8fa6806f425d969b145bd52e0"))
            ]), contextTokens: 8_192, memoryBytes: 3_380_000_000, license: "Apache 2.0", inputs: [.image, .audio],
        contextChoices: gemmaWindows, memoryBytesPerToken: 300_000, profile: gemma4)

    /// How Gemma 4 runs: on the GPU at full precision, as at half its
    /// answers garble digits once a prompt is past about 2,000 tokens,
    /// writing a date such as 2026-11-06 as 20026-111-06 (LiteRT-LM issue
    /// 2814). Its vision encoder runs where the model does and its audio
    /// encoder on the CPU, as Google runs them.
    static let gemma4 = ModelProfile(onScreen: .gpu, gpuActivations: .full, vision: .withModel, audio: .cpu)

    /// The windows Gemma 4 runs at in the app. It reads up to 128K tokens;
    /// past 32K the memory is more than an iPhone leaves an app.
    static let gemmaWindows = [4_096, 8_192, 16_384, 32_768]

    /// The catalog's entry for a model downloaded from it, by ID and
    /// commit: what it can read, and the memory it takes, may be newer than
    /// the manifest says.
    public static func entry(for spec: ModelSpec) -> ModelSpec? {
        all.first { $0.id == spec.id && $0.source?.commit == spec.source?.commit }
    }
}
