import Foundation

/// Why a Hugging Face link couldn't become a model to download.
package enum ModelResolveError: Error, Hashable, Sendable, CustomStringConvertible {
    /// Not a link Lifr can read.
    case notALink(String)
    /// A link to somewhere other than Hugging Face.
    case hostNotAllowed(String)
    /// The link doesn't name a repository, such as huggingface.co/org/name.
    case noRepository(String)
    /// The repository has no file a runtime Lifr has can run.
    case noRunnableFile(repo: String)
    /// The repository has several, and the link didn't say which.
    case severalFiles(repo: String, [String])
    /// The SHA-256 the person gave isn't what Hugging Face lists.
    case hashDisagrees(file: String, given: String, listed: String)
    /// The SHA-256 the person gave isn't 64 hex digits.
    case badHash(String)
    /// The repository needs an account and accepting its terms.
    case gated(repo: String)
    /// Hugging Face answered with an error.
    case hub(HubError)

    package var description: String {
        switch self {
        case .notALink(let text):
            "“\(text)” isn't a link. Paste the model's Hugging Face address, such as "
                + "https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm."
        case .hostNotAllowed(let host):
            "Lifr downloads models only from Hugging Face, and this link is on \(host). Find the model on "
                + "huggingface.co and paste that link."
        case .noRepository(let link):
            "\(link) doesn't name a model. Open the model's page on Hugging Face and paste its address, which "
                + "has the organization and the model's name."
        case .noRunnableFile(let repo):
            "\(repo) has no .litertlm file, which is the only kind Lifr can run. Look for a LiteRT-LM version of "
                + "the model, such as in litert-community."
        case .severalFiles(let repo, let files):
            "\(repo) has several model files (\(files.joined(separator: ", "))). Open the one you want on "
                + "Hugging Face and paste the file's link."
        case .hashDisagrees(let file, let given, let listed):
            "The SHA-256 you gave for \(file) (\(given)) isn't the one Hugging Face lists (\(listed)). Don't just "
                + "use the listed one: the file may have been replaced. Check the hash with whoever published it, "
                + "then paste the right one."
        case .badHash(let text):
            "“\(text)” isn't a SHA-256. It should be 64 characters, 0 to 9 and a to f. Copy it again, or leave it "
                + "empty to use the one Hugging Face lists."
        case .gated(let repo):
            "\(repo) asks for an account and accepting its terms, and Lifr can't download those models yet. Pick "
                + "a model that doesn't need one."
        case .hub(let error):
            error.description
        }
    }
}

/// What went wrong talking to Hugging Face.
package enum HubError: Error, Hashable, Sendable, CustomStringConvertible {
    /// No connection, or it dropped.
    case offline(String)
    /// 401 or 403.
    case notAllowed(String)
    /// 404.
    case notFound(String)
    /// 429.
    case rateLimited
    /// Another status.
    case status(Int, String)
    /// An answer Lifr couldn't read.
    case unreadable(String)
    /// Hugging Face sent the request on to a host that isn't Hugging Face.
    case redirectNotAllowed(String)

    package var description: String {
        switch self {
        case .offline(let reason):
            "Lifr couldn't reach Hugging Face (\(reason)). Check the connection and try again."
        case .notAllowed(let what):
            "Hugging Face won't share \(what): it doesn't exist, or it's private. Check the link; Lifr can only "
                + "download models anyone can."
        case .notFound(let what):
            "Hugging Face has no \(what). Check the link; the model may have been renamed or removed."
        case .rateLimited:
            "Hugging Face is limiting how often Lifr can ask. Wait a few minutes, then try again."
        case .status(let code, let what):
            "Hugging Face answered \(code) for \(what). Try again later."
        case .unreadable(let what):
            "Hugging Face's answer about \(what) wasn't what Lifr expected. Try again later; if it keeps "
                + "happening, Lifr needs an update."
        case .redirectNotAllowed(let host):
            "Hugging Face sent Lifr on to \(host), which Lifr doesn't download from. Try again later; if it keeps "
                + "happening, Lifr needs an update."
        }
    }
}

/// Why a download didn't finish.
package enum ModelDownloadError: Error, Hashable, Sendable, CustomStringConvertible {
    /// Hugging Face answered with an error, or the connection failed.
    case hub(HubError)
    /// Not enough free space for what's left to download.
    case notEnoughSpace(needed: Int64, available: Int64)
    /// A file's bytes don't match its digest. Lifr deleted it.
    case digestMismatch(file: String)
    /// A file's size isn't what the pinned commit says.
    case wrongSize(file: String, expected: Int64, received: Int64)
    /// The model has no files to download.
    case nothingToDownload
    /// The person paused it, or Lifr stopped it to delete the model.
    case paused
    /// Reading or writing the files failed.
    case storage(ModelStorageError)

    package var description: String {
        switch self {
        case .hub(let error):
            error.description
        case .notEnoughSpace(let needed, let available):
            "The download needs \(Self.bytes(needed)) free, with room to spare, and this device has "
                + "\(Self.bytes(available)). Free up space or remove another model, then download again."
        case .digestMismatch(let file):
            "\(file) didn't match its checksum, so Lifr deleted it. Download again, on a different network if you "
                + "can; if it keeps happening, the file on Hugging Face may have changed."
        case .wrongSize(let file, let expected, let received):
            "\(file) came to \(received) bytes instead of \(expected). Download again; Lifr starts that file over."
        case .nothingToDownload:
            "This model has no files to download. Remove it and add it again from its Hugging Face link."
        case .paused:
            "The download is paused. Download again to go on from where it stopped."
        case .storage(let error):
            error.description
        }
    }

    private static func bytes(_ count: Int64) -> String {
        ByteCountFormatter.string(fromByteCount: count, countStyle: .file)
    }
}

/// Why the models on disk couldn't be read or changed.
package enum ModelStorageError: Error, Hashable, Sendable, CustomStringConvertible {
    /// A file system call failed.
    case fileSystem(operation: String, path: String, reason: String)

    package var description: String {
        switch self {
        case .fileSystem(let operation, let path, let reason):
            "Lifr couldn't \(operation) \(path) (\(reason)). Restart Lifr and try again; if it keeps failing, "
                + "remove the model and download it again."
        }
    }
}
