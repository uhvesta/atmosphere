import Foundation

/// The hosts Lifr downloads from: Hugging Face, and the CDN it sends files
/// from. Only https, with no credentials or port in the link.
package enum HostPolicy {
    package static func allows(_ url: URL) -> Bool {
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
package struct ModelLink: Hashable, Sendable {
    /// Such as litert-community/gemma-4-E2B-it-litert-lm.
    package var repo: String
    /// A branch, tag or commit, when the link has one.
    package var revision: String?
    /// One file in the repository, when the link is to a file.
    package var path: String?

    package init(repo: String, revision: String? = nil, path: String? = nil) {
        self.repo = repo
        self.revision = revision
        self.path = path
    }

    /// Reads a model page, a folder at a revision, or a file link, on
    /// huggingface.co or hf.co.
    package static func parse(_ text: String) throws(ModelResolveError) -> ModelLink {
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
package struct HubModelInfo: Decodable, Hashable, Sendable {
    /// The full commit SHA the revision is at.
    package var sha: String
    /// Whether it needs an account and accepting terms.
    package var isGated: Bool
    /// The license tag from its model card, such as apache-2.0.
    package var license: String?

    private enum CodingKeys: String, CodingKey { case sha, gated, cardData }
    private struct Card: Decodable { var license: String? }

    package init(sha: String, isGated: Bool, license: String?) {
        self.sha = sha
        self.isGated = isGated
        self.license = license
    }

    package init(from decoder: any Decoder) throws {
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
package struct HubTreeEntry: Decodable, Hashable, Sendable {
    package var type: String
    package var path: String
    package var size: Int64
    /// The git blob SHA-1.
    package var oid: String
    /// For a file kept in LFS: its SHA-256.
    package var lfs: LFS?

    package struct LFS: Decodable, Hashable, Sendable {
        package var oid: String
    }

    package init(path: String, size: Int64, oid: String, sha256: String?) {
        type = "file"
        self.path = path
        self.size = size
        self.oid = oid
        lfs = sha256.map(LFS.init)
    }

    /// The file, with the digest to check it against.
    package var file: ModelFile {
        ModelFile(path: path, size: size, digest: lfs.map { .sha256($0.oid.lowercased()) } ?? .gitBlobSHA1(oid))
    }
}

/// Turns what Hugging Face lists into a model to download.
package enum ModelResolution {
    /// The model at `link`: the one file a runtime can run, checked against
    /// `sha256` when the person gave one. A file in the catalog comes back
    /// as its catalog entry.
    package static func spec(
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
        if let known = catalog.first(where: {
            $0.source?.repo == link.repo && $0.source?.files.map(\.path) == [file.path]
                && $0.source?.files.first?.digest == file.digest
        }) {
            return known
        }
        let name = (file.name as NSString).deletingPathExtension
        return ModelSpec(
            id: ModelID(rawValue: "hf/\(link.repo)/\(file.path)"), name: name, summary: link.repo, runtime: .liteRT,
            source: ModelSource(repo: link.repo, commit: info.sha, files: [file]),
            contextTokens: ModelCatalog.defaultContextTokens, memoryBytes: 0, license: info.license ?? "")
    }

    /// Whether a runtime Lifr has can run the file.
    static func runnable(_ path: String) -> Bool {
        path.lowercased().hasSuffix(".litertlm")
    }

    /// `text` as a SHA-256 in lowercase hex.
    package static func checkedSHA256(_ text: String) throws(ModelResolveError) -> String {
        let hex = text.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        guard hex.count == 64, hex.allSatisfy(\.isHexDigit) else { throw .badHash(text) }
        return hex
    }
}

extension ModelID {
    /// A folder name for the model's files: its ID, with anything but
    /// letters, digits, dots, dashes and underscores replaced.
    package var directoryName: String {
        let allowed = Set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-_")
        let name = String(rawValue.replacing("/", with: "--").map { allowed.contains($0) ? $0 : "_" })
        return name.hasPrefix(".") ? "_" + name.dropFirst() : name
    }
}
