import Foundation

/// A model in the library: one from the catalog, or one the person added by
/// its Hugging Face link.
package struct ModelID: Hashable, Codable, Sendable, RawRepresentable, ExpressibleByStringLiteral,
    CustomStringConvertible
{
    package var rawValue: String

    package init(rawValue: String) { self.rawValue = rawValue }
    package init(stringLiteral value: String) { rawValue = value }

    package var description: String { rawValue }

    /// Apple's on-device model, which needs no download.
    package static let apple: ModelID = "apple"
}

/// What runs a model.
package enum RuntimeKind: String, Hashable, Codable, Sendable, CaseIterable {
    /// Apple's on-device model, through Foundation Models.
    case apple
    /// Google's LiteRT-LM, for `.litertlm` files.
    case liteRT
}

/// How to check a downloaded file.
package enum FileDigest: Hashable, Codable, Sendable {
    /// The SHA-256 of the file, in lowercase hex. Hugging Face lists it for
    /// every file kept in LFS, which model weights are.
    case sha256(String)
    /// The git blob SHA-1 of a small file Hugging Face keeps in git, in
    /// lowercase hex: SHA-1 of "blob <size>\0" and the bytes. The pinned
    /// commit vouches for it.
    case gitBlobSHA1(String)
}

/// One file of a model, in its repository.
package struct ModelFile: Hashable, Codable, Sendable {
    /// The path in the repository.
    package var path: String
    package var size: Int64
    package var digest: FileDigest

    package init(path: String, size: Int64, digest: FileDigest) {
        self.path = path
        self.size = size
        self.digest = digest
    }

    /// The file's name, without folders.
    package var name: String { path.split(separator: "/").last.map(String.init) ?? path }
}

/// Where a model's files come from: a Hugging Face repository at one commit,
/// so a later push can't change what's downloaded.
package struct ModelSource: Hashable, Codable, Sendable {
    /// Such as litert-community/gemma-4-E2B-it-litert-lm.
    package var repo: String
    /// The full commit SHA.
    package var commit: String
    package var files: [ModelFile]

    package init(repo: String, commit: String, files: [ModelFile]) {
        self.repo = repo
        self.commit = commit
        self.files = files
    }

    package var totalSize: Int64 { files.reduce(0) { $0 + $1.size } }

    /// Where Hugging Face serves `file` at this commit.
    package func url(of file: ModelFile) -> URL? {
        var components = URLComponents()
        components.scheme = "https"
        components.host = "huggingface.co"
        components.path = "/\(repo)/resolve/\(commit)/\(file.path)"
        return components.url
    }
}

/// A model Lifr can run, and what it takes.
package struct ModelSpec: Identifiable, Hashable, Codable, Sendable {
    package var id: ModelID
    /// Such as Gemma 4 E2B.
    package var name: String
    /// What it's good for, in a few words.
    package var summary: String
    package var runtime: RuntimeKind
    /// The files to download; nil for Apple's model.
    package var source: ModelSource?
    /// The most tokens Lifr gives it to read and write, before the memory
    /// planner's say.
    package var contextTokens: Int
    /// The memory it takes while answering, measured, at `contextTokens`
    /// or less. Zero when unknown.
    package var memoryBytes: Int64
    /// The license's name, such as Apache 2.0.
    package var license: String
    /// What it can read besides text.
    package var inputs: Set<ModelInput>

    package init(
        id: ModelID, name: String, summary: String, runtime: RuntimeKind, source: ModelSource?, contextTokens: Int,
        memoryBytes: Int64, license: String, inputs: Set<ModelInput> = []
    ) {
        self.id = id
        self.name = name
        self.summary = summary
        self.runtime = runtime
        self.source = source
        self.contextTokens = contextTokens
        self.memoryBytes = memoryBytes
        self.license = license
        self.inputs = inputs
    }

    private enum CodingKeys: String, CodingKey {
        case id, name, summary, runtime, source, contextTokens, memoryBytes, license, inputs
    }

    /// Reads manifests written before models had inputs as text only.
    package init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decode(ModelID.self, forKey: .id)
        name = try container.decode(String.self, forKey: .name)
        summary = try container.decode(String.self, forKey: .summary)
        runtime = try container.decode(RuntimeKind.self, forKey: .runtime)
        source = try container.decodeIfPresent(ModelSource.self, forKey: .source)
        contextTokens = try container.decode(Int.self, forKey: .contextTokens)
        memoryBytes = try container.decode(Int64.self, forKey: .memoryBytes)
        license = try container.decode(String.self, forKey: .license)
        inputs = try container.decodeIfPresent(Set<ModelInput>.self, forKey: .inputs) ?? []
    }

    /// The bytes to download; zero for Apple's model.
    package var downloadSize: Int64 { source?.totalSize ?? 0 }
}

/// A model whose files are on this device, checked.
package struct InstalledModel: Identifiable, Hashable, Codable, Sendable {
    package var spec: ModelSpec
    /// The folder with its files.
    package var directory: URL
    package var installedAt: Date

    package init(spec: ModelSpec, directory: URL, installedAt: Date) {
        self.spec = spec
        self.directory = directory
        self.installedAt = installedAt
    }

    package var id: ModelID { spec.id }

    /// Where `file` is.
    package func location(of file: ModelFile) -> URL {
        directory.appending(path: file.path, directoryHint: .notDirectory)
    }
}

/// What Lifr asks a model to do. The person can pick a model for each.
package enum ModelUse: String, Hashable, Codable, Sendable, CaseIterable {
    /// Conversations with the assistant.
    case assistant
    /// Writing skills from what the person describes.
    case skills
    /// Naming conversations.
    case titles
}

/// Where a download is.
package enum DownloadState: Hashable, Sendable {
    case downloading(received: Int64, total: Int64)
    /// Checking the files against their digests.
    case verifying
    /// Stopped; downloading again goes on from where it stopped.
    case paused(received: Int64, total: Int64)
    case failed(ModelDownloadError)

    /// How far along it is, from 0 to 1.
    package var fraction: Double {
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
package func spaceNeeded(toDownload remaining: Int64) -> Int64 {
    remaining + remaining / 10 + 1_000_000_000
}
