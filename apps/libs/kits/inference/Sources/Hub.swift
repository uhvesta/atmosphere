import Foundation

/// The hosts the app downloads from: Hugging Face, and the CDN it sends files
/// from. Only https, with no credentials or port in the link.
public enum HostPolicy {
    public static func allows(_ url: URL) -> Bool {
        guard url.scheme?.lowercased() == "https", url.user() == nil, url.password() == nil, url.port == nil,
            let host = url.host()?.lowercased()
        else { return false }
        return isHub(host) || host.hasSuffix(".hf.co")
    }

    /// Hugging Face itself, where the API and the pages are.
    static func isHub(_ host: String) -> Bool {
        host == "huggingface.co" || host == "hf.co"
    }
}

/// What a pasted Hugging Face link points at.
public struct ModelLink: Hashable, Sendable {
    /// Such as litert-community/gemma-4-E2B-it-litert-lm.
    public var repo: String
    /// A branch, tag or commit, when the link has one.
    public var revision: String?
    /// One file in the repository, when the link is to a file.
    public var path: String?

    public init(repo: String, revision: String? = nil, path: String? = nil) {
        self.repo = repo
        self.revision = revision
        self.path = path
    }

    /// Reads a model page, a folder at a revision, or a file link, on
    /// huggingface.co or hf.co.
    public static func parse(_ text: String) throws(ModelResolveError) -> ModelLink {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let url = URL(string: trimmed), url.scheme != nil, let host = url.host(), !host.isEmpty else {
            throw .notALink(trimmed)
        }
        guard HostPolicy.allows(url), HostPolicy.isHub(host.lowercased()) else { throw .hostNotAllowed(host) }
        let parts = url.path(percentEncoded: false).split(separator: "/").map(String.init)
        let notModels: Set<String> = ["api", "datasets", "spaces", "docs", "models", "collections"]
        guard parts.count >= 2, !notModels.contains(parts[0]) else { throw .noRepository(trimmed) }
        let repo = "\(parts[0])/\(parts[1])"
        guard parts.count > 2 else { return ModelLink(repo: repo) }
        switch parts[2] {
        case "tree" where parts.count >= 4:
            return ModelLink(repo: repo, revision: parts[3])
        case "resolve" where parts.count >= 5, "blob" where parts.count >= 5:
            return ModelLink(repo: repo, revision: parts[3], path: parts[4...].joined(separator: "/"))
        default:
            throw .noRepository(trimmed)
        }
    }
}

/// What Hugging Face says about a repository at a revision.
public struct HubModelInfo: Decodable, Hashable, Sendable {
    /// The full commit SHA the revision is at.
    public var sha: String
    /// Whether it needs an account and accepting terms.
    public var isGated: Bool
    /// The license tag from its model card, such as apache-2.0.
    public var license: String?

    private enum CodingKeys: String, CodingKey { case sha, gated, cardData }
    private struct Card: Decodable { var license: String? }

    public init(sha: String, isGated: Bool, license: String?) {
        self.sha = sha
        self.isGated = isGated
        self.license = license
    }

    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        sha = try container.decode(String.self, forKey: .sha)
        // false, or how access is granted: "auto" or "manual".
        if let gated = try? container.decode(Bool.self, forKey: .gated) {
            isGated = gated
        } else {
            isGated = (try? container.decode(String.self, forKey: .gated)) != nil
        }
        license = try container.decodeIfPresent(Card.self, forKey: .cardData)?.license
    }
}

/// One entry of a repository's file list at a commit.
public struct HubTreeEntry: Decodable, Hashable, Sendable {
    public var type: String
    public var path: String
    public var size: Int64
    /// The git blob SHA-1.
    public var oid: String
    /// For a file kept in LFS: its SHA-256.
    public var lfs: LFS?

    public struct LFS: Decodable, Hashable, Sendable {
        public var oid: String
    }

    public init(path: String, size: Int64, oid: String, sha256: String?) {
        type = "file"
        self.path = path
        self.size = size
        self.oid = oid
        lfs = sha256.map(LFS.init)
    }

    /// The file, with the digest to check it against.
    public var file: ModelFile {
        ModelFile(path: path, size: size, digest: lfs.map { .sha256($0.oid.lowercased()) } ?? .gitBlobSHA1(oid))
    }
}

/// Turns what Hugging Face lists into a model to download.
public enum ModelResolution {
    /// The model at `link`: the one file a runtime can run, checked against
    /// `sha256` when the person gave one. A file in the catalog comes back
    /// as its catalog entry.
    public static func spec(
        _ link: ModelLink, info: HubModelInfo, tree: [HubTreeEntry], sha256 given: String?,
        catalog: [ModelSpec] = ModelCatalog.all
    ) throws(ModelResolveError) -> ModelSpec {
        guard !info.isGated else { throw .gated(repo: link.repo) }
        let expected = try given.map { text throws(ModelResolveError) in try checkedSHA256(text) }
        let files = tree.filter { $0.type == "file" }
        let entry: HubTreeEntry
        if let path = link.path {
            guard let found = files.first(where: { $0.path == path }) else {
                throw .hub(.notFound("\(path) in \(link.repo) at \(info.sha.prefix(7))"))
            }
            guard runnable(found.path) else { throw .noRunnableFile(repo: link.repo) }
            entry = found
        } else {
            let candidates = files.filter { runnable($0.path) }
            guard let only = candidates.first else { throw .noRunnableFile(repo: link.repo) }
            guard candidates.count == 1 else { throw .severalFiles(repo: link.repo, candidates.map(\.path)) }
            entry = only
        }
        var file = entry.file
        switch (file.digest, expected) {
        case (.sha256(let listed), let expected?) where listed != expected:
            throw .hashDisagrees(file: file.name, given: expected, listed: listed)
        case (.gitBlobSHA1, let expected?):
            file.digest = .sha256(expected)
        default:
            break
        }
        for known in catalog {
            let same = known.variants.first {
                $0.source?.repo == link.repo && $0.source?.files.map(\.path) == [file.path]
                    && $0.source?.files.first?.digest == file.digest
            }
            if let same { return known.running(same) }
        }
        let name = (file.name as NSString).deletingPathExtension
        return ModelSpec(
            id: ModelID(rawValue: "hf/\(link.repo)/\(file.path)"), name: name, summary: link.repo, runtime: .liteRT,
            source: ModelSource(repo: link.repo, commit: info.sha, files: [file]),
            contextTokens: ModelCatalog.defaultContextTokens, memoryBytes: 0, license: info.license ?? "")
    }

    /// Whether a runtime the app has can run the file.
    static func runnable(_ path: String) -> Bool {
        path.lowercased().hasSuffix(".litertlm")
    }

    /// `text` as a SHA-256 in lowercase hex.
    public static func checkedSHA256(_ text: String) throws(ModelResolveError) -> String {
        let hex = text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard hex.count == 64, hex.allSatisfy(\.isHexDigit) else { throw .badHash(text) }
        return hex
    }
}

extension ModelID {
    /// A folder name for the model's files: its ID, with anything but
    /// letters, digits, dots, dashes and underscores replaced.
    public var directoryName: String {
        let allowed = Set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-_")
        let name = String(rawValue.replacing("/", with: "--").map { allowed.contains($0) ? $0 : "_" })
        return name.hasPrefix(".") ? "_" + name.dropFirst() : name
    }
}
