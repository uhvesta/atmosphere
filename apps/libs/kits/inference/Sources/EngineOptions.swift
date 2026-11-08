/// How an engine runs a model, as the person sets it to compare setups:
/// where it runs, its window, and the engine's own settings. A setting left
/// nil is the engine's usual one, which the app picked for that engine.
public struct EngineOptions: Hashable, Codable, Sendable {
    public var backend: ModelBackend
    /// The tokens it reads and writes at most.
    public var window: Int
    /// The CPU threads it uses; for the parts that run there on the GPU.
    public var cpuThreads: Int?
    /// How many of its layers run on the GPU, the rest on the CPU.
    public var gpuLayers: Int?
    /// The tokens of a prompt it reads in one go.
    public var batchTokens: Int?
    /// The tokens of each batch it works on at once on the device.
    public var microBatchTokens: Int?
    /// The precision of the keys and values it keeps for the tokens read.
    public var kvCache: KVCache?
    /// Whether attention runs as one fused step, which takes less memory.
    public var flashAttention: Bool?
    /// The precision of its activations on the GPU.
    public var gpuActivations: ModelProfile.Activations?

    public init(
        backend: ModelBackend, window: Int, cpuThreads: Int? = nil, gpuLayers: Int? = nil, batchTokens: Int? = nil,
        microBatchTokens: Int? = nil, kvCache: KVCache? = nil, flashAttention: Bool? = nil,
        gpuActivations: ModelProfile.Activations? = nil
    ) {
        self.backend = backend
        self.window = window
        self.cpuThreads = cpuThreads
        self.gpuLayers = gpuLayers
        self.batchTokens = batchTokens
        self.microBatchTokens = microBatchTokens
        self.kvCache = kvCache
        self.flashAttention = flashAttention
        self.gpuActivations = gpuActivations
    }

    /// The precision of the keys and values kept for each token read.
    public enum KVCache: String, Hashable, Codable, Sendable, CaseIterable {
        /// 16 bits: the most precise, and the most memory.
        case f16
        /// 8 bits: half the memory, and close to f16's answers.
        case q8 = "q8_0"
        /// 4 bits: a quarter, and answers that drift further.
        case q4 = "q4_0"
    }

    /// The settings that are the engine's own, besides where it runs and
    /// its window.
    public enum Setting: String, Hashable, Codable, Sendable, CaseIterable {
        case cpuThreads, gpuLayers, batchTokens, microBatchTokens, kvCache, flashAttention, gpuActivations
    }

    /// The settings it sets, of `Setting`.
    public var settings: Set<Setting> {
        var set: Set<Setting> = []
        if cpuThreads != nil { set.insert(.cpuThreads) }
        if gpuLayers != nil { set.insert(.gpuLayers) }
        if batchTokens != nil { set.insert(.batchTokens) }
        if microBatchTokens != nil { set.insert(.microBatchTokens) }
        if kvCache != nil { set.insert(.kvCache) }
        if flashAttention != nil { set.insert(.flashAttention) }
        if gpuActivations != nil { set.insert(.gpuActivations) }
        return set
    }

    /// Checks they're settings `runtime` has, in its range, at a window
    /// `spec` offers, and that they go together.
    public func validate(for spec: ModelSpec, on runtime: RuntimeKind) throws(EngineOptionsError) {
        guard runtime.backends.contains(backend) else { throw .backendUnavailable(backend, runtime) }
        let windows = spec.windows
        guard windows.contains(window) else { throw .windowUnavailable(window, offered: windows) }
        if let unsupported = Setting.allCases.first(where: { settings.contains($0) && !runtime.settings.contains($0) })
        {
            throw .unsupported(unsupported, runtime)
        }
        if let cpuThreads, cpuThreads < 1 { throw .outOfRange(.cpuThreads, cpuThreads) }
        if let gpuLayers, gpuLayers < 0 { throw .outOfRange(.gpuLayers, gpuLayers) }
        if let gpuLayers, gpuLayers > 0, backend == .cpu { throw .gpuLayersOnCPU }
        if let batchTokens, batchTokens < 1 { throw .outOfRange(.batchTokens, batchTokens) }
        if let microBatchTokens, microBatchTokens < 1 { throw .outOfRange(.microBatchTokens, microBatchTokens) }
        if let microBatchTokens, microBatchTokens > (batchTokens ?? EngineOptions.llamaBatchTokens) {
            throw .microBatchOverBatch(micro: microBatchTokens, batch: batchTokens ?? EngineOptions.llamaBatchTokens)
        }
        if let kvCache, kvCache != .f16, flashAttention == false { throw .quantizedCacheNeedsFlashAttention }
    }

    /// The tokens llama.cpp reads in one go unless it's set.
    public static let llamaBatchTokens = 512

    /// How far from the most precise setup these are: the higher, the
    /// further its answers may drift. A full-precision cache and
    /// activations, as on a CPU, are the nearest.
    public var imprecision: Int {
        let cache: Int =
            switch kvCache {
            case .f16: 0
            case .q8, nil: 1
            case .q4: 2
            }
        let activations = backend == .cpu || gpuActivations == .full ? 0 : 1
        return cache + activations
    }
}

extension RuntimeKind {
    /// The settings its engine takes, of `EngineOptions.Setting`.
    public var settings: Set<EngineOptions.Setting> {
        switch self {
        case .llama: Set(EngineOptions.Setting.allCases).subtracting([.gpuActivations])
        case .liteRT: [.cpuThreads, .gpuActivations]
        case .mlx, .apple, .coreML: []
        }
    }
}

extension ModelSpec {
    /// Every window it runs at, smallest first.
    public var windows: [Int] { Array(Set(contextChoices + [contextTokens])).sorted() }
}

/// Why engine options can't be used as set.
public enum EngineOptionsError: Error, Hashable, Sendable, CustomStringConvertible {
    /// The engine doesn't run there, such as MLX on the CPU.
    case backendUnavailable(ModelBackend, RuntimeKind)
    /// The model doesn't run at that window.
    case windowUnavailable(Int, offered: [Int])
    /// The engine has no such setting.
    case unsupported(EngineOptions.Setting, RuntimeKind)
    /// A count under what the setting takes.
    case outOfRange(EngineOptions.Setting, Int)
    /// GPU layers asked of a run on the CPU only.
    case gpuLayersOnCPU
    /// A micro-batch larger than its batch.
    case microBatchOverBatch(micro: Int, batch: Int)
    /// A cache under 16 bits with fused attention off, which llama.cpp
    /// can't run.
    case quantizedCacheNeedsFlashAttention

    public var description: String {
        switch self {
        case .backendUnavailable(let backend, let runtime):
            "\(runtime.name) doesn't run on the \(backend == .gpu ? "GPU" : "CPU"). Pick the other one."
        case .windowUnavailable(let window, let offered):
            "The model doesn't run with \(window) tokens. Pick one of "
                + offered.map(String.init).joined(separator: ", ") + "."
        case .unsupported(let setting, let runtime):
            "\(runtime.name) has no \(setting.name) setting. Leave it at the engine's own."
        case .outOfRange(let setting, let value):
            "\(setting.name.capitalizedFirst) can't be \(value). Set it to \(setting == .gpuLayers ? 0 : 1) or more."
        case .gpuLayersOnCPU:
            "GPU layers run only on the GPU. Pick the GPU, or set no GPU layers."
        case .microBatchOverBatch(let micro, let batch):
            "The micro-batch (\(micro)) is larger than the batch (\(batch)). Make it the batch or smaller."
        case .quantizedCacheNeedsFlashAttention:
            "A cache under 16 bits needs flash attention. Turn flash attention on, or keep the cache at 16 bits."
        }
    }
}

extension String {
    fileprivate var capitalizedFirst: String { prefix(1).uppercased() + dropFirst() }
}

extension EngineOptions.Setting {
    /// What it's called on the screen.
    public var name: String {
        switch self {
        case .cpuThreads: "CPU threads"
        case .gpuLayers: "GPU layers"
        case .batchTokens: "batch"
        case .microBatchTokens: "micro-batch"
        case .kvCache: "KV cache"
        case .flashAttention: "flash attention"
        case .gpuActivations: "GPU precision"
        }
    }
}
