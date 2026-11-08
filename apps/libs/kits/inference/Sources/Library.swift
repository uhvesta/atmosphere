import Foundation

/// A model in the library: one from the catalog, or one the person added by
/// its Hugging Face link.
public struct ModelID: Hashable, Codable, Sendable, RawRepresentable, ExpressibleByStringLiteral,
    CustomStringConvertible
{
    public var rawValue: String

    public init(rawValue: String) { self.rawValue = rawValue }
    public init(stringLiteral value: String) { rawValue = value }

    public var description: String { rawValue }

    /// Apple's on-device model, which needs no download.
    public static let apple: ModelID = "apple"
}

/// What runs a model: Apple's own, or the engine that loads its files.
public enum RuntimeKind: String, Hashable, Codable, Sendable, CaseIterable {
    /// Apple's on-device model, through Foundation Models.
    case apple
    /// Google's LiteRT-LM, for `.litertlm` files.
    case liteRT
    /// llama.cpp, for `.gguf` files.
    case llama
    /// MLX, for a folder of `.safetensors` in MLX's format.
    case mlx
    /// Core ML, built into the OS, for `.mlpackage` folders, such as a
    /// voice model's.
    case coreML

    /// Its name, for the person.
    public var name: String {
        switch self {
        case .apple: "Apple Intelligence"
        case .liteRT: "LiteRT-LM"
        case .llama: "llama.cpp"
        case .mlx: "MLX"
        case .coreML: "Core ML"
        }
    }

    /// Where it runs models. MLX runs only on the GPU: the CPU runs some
    /// models' layers, such as Qwen3.5's linear attention, step by step,
    /// hundreds of times slower.
    public var backends: [ModelBackend] {
        switch self {
        case .mlx: [.gpu]
        case .apple, .liteRT, .llama, .coreML: [.gpu, .cpu]
        }
    }
}

/// How to check a downloaded file.
public enum FileDigest: Hashable, Codable, Sendable {
    /// The SHA-256 of the file, in lowercase hex. Hugging Face lists it for
    /// every file kept in LFS, which model weights are.
    case sha256(String)
    /// The git blob SHA-1 of a small file Hugging Face keeps in git, in
    /// lowercase hex: SHA-1 of "blob <size>\0" and the bytes. The pinned
    /// commit vouches for it.
    case gitBlobSHA1(String)
}

/// One file of a model, in its repository.
public struct ModelFile: Hashable, Codable, Sendable {
    /// The path in the repository.
    public var path: String
    public var size: Int64
    public var digest: FileDigest

    public init(path: String, size: Int64, digest: FileDigest) {
        self.path = path
        self.size = size
        self.digest = digest
    }

    /// The file's name, without folders.
    public var name: String { path.split(separator: "/").last.map(String.init) ?? path }
}

/// Where a model's files come from: a Hugging Face repository at one commit,
/// so a later push can't change what's downloaded.
public struct ModelSource: Hashable, Codable, Sendable {
    /// Such as litert-community/gemma-4-E2B-it-litert-lm.
    public var repo: String
    /// The full commit SHA.
    public var commit: String
    public var files: [ModelFile]

    public init(repo: String, commit: String, files: [ModelFile]) {
        self.repo = repo
        self.commit = commit
        self.files = files
    }

    public var totalSize: Int64 { files.reduce(0) { $0 + $1.size } }

    /// Where Hugging Face serves `file` at this commit.
    public func url(of file: ModelFile) -> URL? {
        var components = URLComponents()
        components.scheme = "https"
        components.host = "huggingface.co"
        components.path = "/\(repo)/resolve/\(commit)/\(file.path)"
        return components.url
    }
}

/// One way to run a model: the engine, the files it loads, and what they
/// take. A model can have several, such as one file for each engine or
/// quantization, of which a device downloads one.
public struct ModelVariant: Hashable, Codable, Sendable {
    public var runtime: RuntimeKind
    /// How its weights are stored, such as Q4_K_M; empty when the files
    /// don't say.
    public var quantization: String
    /// The files to download; nil for Apple's model.
    public var source: ModelSource?
    /// As `ModelSpec` says of its own.
    public var memoryBytes: Int64
    public var cpuMemoryBytes: Int64
    public var memoryBytesPerToken: Int64
    public var profile: ModelProfile

    public init(
        runtime: RuntimeKind, quantization: String = "", source: ModelSource?, memoryBytes: Int64,
        cpuMemoryBytes: Int64 = 0, memoryBytesPerToken: Int64 = 0, profile: ModelProfile = .standard
    ) {
        self.runtime = runtime
        self.quantization = quantization
        self.source = source
        self.memoryBytes = memoryBytes
        self.cpuMemoryBytes = cpuMemoryBytes
        self.memoryBytesPerToken = memoryBytesPerToken
        self.profile = profile
    }
}

/// A model the app can run, and what it takes. Its own fields describe the
/// variant it runs as; `variants` lists every one it has.
public struct ModelSpec: Identifiable, Hashable, Codable, Sendable {
    public var id: ModelID
    /// Such as Gemma 4 E2B.
    public var name: String
    /// What it's good for, in a few words.
    public var summary: String
    public var runtime: RuntimeKind
    /// The files to download; nil for Apple's model.
    public var source: ModelSource?
    /// The most tokens the app gives it to read and write, before the memory
    /// planner's say.
    public var contextTokens: Int
    /// The memory it takes while answering, measured, at `contextTokens`
    /// or less. Zero when unknown.
    public var memoryBytes: Int64
    /// The memory it takes on the CPU, which is less: there's no GPU copy
    /// of its weights. Zero when unknown.
    public var cpuMemoryBytes: Int64
    /// The license's name, such as Apache 2.0.
    public var license: String
    /// What it can read besides text.
    public var inputs: Set<ModelInput>
    /// The windows the person may pick from instead of `contextTokens`,
    /// smallest first; empty when it has only the one.
    public var contextChoices: [Int]
    /// The memory each token of window takes past `contextTokens`, for an
    /// estimate of a larger one. Zero when unknown.
    public var memoryBytesPerToken: Int64
    /// How it runs best.
    public var profile: ModelProfile
    /// Every way it can run, its own among them.
    public var variants: [ModelVariant]

    public init(
        id: ModelID, name: String, summary: String, runtime: RuntimeKind, source: ModelSource?, contextTokens: Int,
        memoryBytes: Int64, cpuMemoryBytes: Int64 = 0, license: String, inputs: Set<ModelInput> = [],
        contextChoices: [Int] = [], memoryBytesPerToken: Int64 = 0, profile: ModelProfile = .standard,
        quantization: String = ""
    ) {
        self.id = id
        self.name = name
        self.summary = summary
        self.runtime = runtime
        self.source = source
        self.contextTokens = contextTokens
        self.memoryBytes = memoryBytes
        self.cpuMemoryBytes = cpuMemoryBytes
        self.license = license
        self.inputs = inputs
        self.contextChoices = contextChoices
        self.memoryBytesPerToken = memoryBytesPerToken
        self.profile = profile
        variants = []
        variants = [variant(quantization: quantization)]
    }

    /// A model with several variants, running as the first of them.
    public init(
        id: ModelID, name: String, summary: String, contextTokens: Int, license: String,
        inputs: Set<ModelInput> = [], contextChoices: [Int] = [], variants: [ModelVariant]
    ) {
        let first = variants.first ?? ModelVariant(runtime: .apple, source: nil, memoryBytes: 0)
        self.init(
            id: id, name: name, summary: summary, runtime: first.runtime, source: first.source,
            contextTokens: contextTokens, memoryBytes: first.memoryBytes, cpuMemoryBytes: first.cpuMemoryBytes,
            license: license, inputs: inputs, contextChoices: contextChoices,
            memoryBytesPerToken: first.memoryBytesPerToken, profile: first.profile, quantization: first.quantization)
        if !variants.isEmpty { self.variants = variants }
    }

    /// The variant it runs as.
    public var variant: ModelVariant {
        let own = variant(quantization: "")
        return variants.first { $0.runtime == own.runtime && $0.source == own.source } ?? own
    }

    /// The same model, running as `variant`.
    public func running(_ variant: ModelVariant) -> ModelSpec {
        var spec = self
        spec.runtime = variant.runtime
        spec.source = variant.source
        spec.memoryBytes = variant.memoryBytes
        spec.cpuMemoryBytes = variant.cpuMemoryBytes
        spec.memoryBytesPerToken = variant.memoryBytesPerToken
        spec.profile = variant.profile
        return spec
    }

    private func variant(quantization: String) -> ModelVariant {
        ModelVariant(
            runtime: runtime, quantization: quantization, source: source, memoryBytes: memoryBytes,
            cpuMemoryBytes: cpuMemoryBytes, memoryBytesPerToken: memoryBytesPerToken, profile: profile)
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, summary, runtime, source, contextTokens, memoryBytes, cpuMemoryBytes, license, inputs,
            contextChoices, memoryBytesPerToken, profile, variants
    }

    /// Reads manifests written before models had inputs as text only;
    /// before they had a CPU figure or a choice of window as unknown; and
    /// before they had a profile with the standard one; and before models
    /// had variants as having only their own.
    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(ModelID.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        summary = try container.decode(String.self, forKey: .summary)
        runtime = try container.decode(RuntimeKind.self, forKey: .runtime)
        source = try container.decodeIfPresent(ModelSource.self, forKey: .source)
        contextTokens = try container.decode(Int.self, forKey: .contextTokens)
        memoryBytes = try container.decode(Int64.self, forKey: .memoryBytes)
        cpuMemoryBytes = try container.decodeIfPresent(Int64.self, forKey: .cpuMemoryBytes) ?? 0
        license = try container.decode(String.self, forKey: .license)
        inputs = try container.decodeIfPresent(Set<ModelInput>.self, forKey: .inputs) ?? []
        contextChoices = try container.decodeIfPresent([Int].self, forKey: .contextChoices) ?? []
        memoryBytesPerToken = try container.decodeIfPresent(Int64.self, forKey: .memoryBytesPerToken) ?? 0
        profile = try container.decodeIfPresent(ModelProfile.self, forKey: .profile) ?? .standard
        variants = []
        variants = try container.decodeIfPresent([ModelVariant].self, forKey: .variants) ?? [variant(quantization: "")]
    }

    /// The bytes to download; zero for Apple's model.
    public var downloadSize: Int64 { source?.totalSize ?? 0 }

    /// `window` as a choice to keep for `spec`: nil when it's the model's
    /// own window or one it doesn't offer.
    public static func choice(_ window: Int, of spec: ModelSpec) -> Int? {
        window != spec.contextTokens && spec.contextChoices.contains(window) ? window : nil
    }

    /// The memory it takes on the GPU with a window of `window` tokens:
    /// the measured figure, moved by what each token past `contextTokens`
    /// takes, and never under half the measured figure. Zero when unknown.
    public func memoryBytes(at window: Int) -> Int64 {
        guard memoryBytes > 0 else { return 0 }
        let more = Int64(window - contextTokens) * memoryBytesPerToken
        return max(memoryBytes / 2, memoryBytes + more)
    }
}

/// A model whose files are on this device, checked.
public struct InstalledModel: Identifiable, Hashable, Codable, Sendable {
    public var spec: ModelSpec
    /// The folder with its files.
    public var directory: URL
    public var installedAt: Date
    /// The window the person picked from `spec.contextChoices`, or nil for
    /// the model's own.
    public var chosenWindow: Int?
    /// Where the person picked for it to run on screen, or nil for the
    /// planner's pick.
    public var pickedBackend: ModelBackend?
    /// How fast it ran here, each time the person had it measured.
    public var measurements: [ModelMeasurement]
    /// How its engine runs it, as the person tuned it for the use it runs
    /// for; nil for the engine's usual settings at `chosenWindow`.
    public var options: EngineOptions?

    public init(
        spec: ModelSpec, directory: URL, installedAt: Date, chosenWindow: Int? = nil,
        pickedBackend: ModelBackend? = nil, measurements: [ModelMeasurement] = []
    ) {
        self.spec = spec
        self.directory = directory
        self.installedAt = installedAt
        self.chosenWindow = chosenWindow
        self.pickedBackend = pickedBackend
        self.measurements = measurements
    }

    public var id: ModelID { spec.id }

    /// The tokens it reads and writes at most.
    public var window: Int { options?.window ?? chosenWindow ?? spec.contextTokens }

    /// How it runs: its profile, with the CPU threads and GPU precision
    /// its options set.
    public var profile: ModelProfile {
        var profile = spec.profile
        if let threads = options?.cpuThreads { profile.cpuThreads = threads }
        if let activations = options?.gpuActivations { profile.gpuActivations = activations }
        return profile
    }

    /// Where `file` is.
    public func location(of file: ModelFile) -> URL {
        directory.appending(path: file.path, directoryHint: .notDirectory)
    }
}

/// What an app asks models to do, such as chatting or naming chats. The
/// person can pick a model for each; `ModelSelection` keeps what they
/// picked.
public protocol ModelUseCase: Hashable, Codable, Sendable, CaseIterable {
    /// The model this use runs on, given what the person picked: theirs for
    /// it, or else its own default, which may be another use's model.
    func model(in selection: [Self: ModelID]) -> ModelID
    /// The model this use runs on when the person hasn't picked one, if it
    /// doesn't follow another use's.
    var standard: ModelID? { get }
    /// What the person tunes a model for this use for: chatting, or quick
    /// tasks. Chat unless the use says.
    var purpose: TunePurpose { get }
}

/// Where a download is.
public enum DownloadState: Hashable, Sendable {
    case downloading(received: Int64, total: Int64)
    /// Checking the files against their digests.
    case verifying
    /// Stopped; downloading again goes on from where it stopped.
    case paused(received: Int64, total: Int64)
    case failed(ModelDownloadError)

    /// How far along it is, from 0 to 1.
    public var fraction: Double {
        switch self {
        case .downloading(let received, let total), .paused(let received, let total):
            total > 0 ? Double(received) / Double(total) : 0
        case .verifying: 1
        case .failed: 0
        }
    }
}

/// The bytes a download needs free: what's left to download, a tenth more
/// for the file system, and a gigabyte so the device isn't left full.
public func spaceNeeded(toDownload remaining: Int64) -> Int64 {
    remaining + remaining / 10 + 1_000_000_000
}
