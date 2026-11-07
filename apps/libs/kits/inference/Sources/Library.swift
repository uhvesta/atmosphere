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

/// What runs a model.
public enum RuntimeKind: String, Hashable, Codable, Sendable, CaseIterable {
    /// Apple's on-device model, through Foundation Models.
    case apple
    /// Google's LiteRT-LM, for `.litertlm` files.
    case liteRT
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

/// A model the app can run, and what it takes.
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

    public init(
        id: ModelID, name: String, summary: String, runtime: RuntimeKind, source: ModelSource?, contextTokens: Int,
        memoryBytes: Int64, cpuMemoryBytes: Int64 = 0, license: String, inputs: Set<ModelInput> = [],
        contextChoices: [Int] = [], memoryBytesPerToken: Int64 = 0, profile: ModelProfile = .standard
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
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, summary, runtime, source, contextTokens, memoryBytes, cpuMemoryBytes, license, inputs,
            contextChoices, memoryBytesPerToken, profile
    }

    /// Reads manifests written before models had inputs as text only;
    /// before they had a CPU figure or a choice of window as unknown; and
    /// before they had a profile with the standard one.
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

    public init(spec: ModelSpec, directory: URL, installedAt: Date, chosenWindow: Int? = nil) {
        self.spec = spec
        self.directory = directory
        self.installedAt = installedAt
        self.chosenWindow = chosenWindow
    }

    public var id: ModelID { spec.id }

    /// The tokens it reads and writes at most.
    public var window: Int { chosenWindow ?? spec.contextTokens }

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
