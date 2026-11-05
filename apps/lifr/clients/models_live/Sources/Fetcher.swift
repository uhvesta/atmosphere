import ComposableArchitecture
import Foundation
import LifrModelsClient

/// Brings a model file's bytes onto the device, after those already there.
protocol ModelFetcher: Sendable {
    /// Appends what's missing of `file`, from `url`, to `partial`, calling
    /// `received` with the bytes added as they come. Throws `.paused` when
    /// cancelled, keeping what came.
    func fetch(
        _ file: ModelFile, from url: URL, into partial: URL, received: @escaping @Sendable (Int64) -> Void
    ) async throws(ModelDownloadError)
}

extension ModelFetcher {
    /// Where `partial` stands: its size, after starting over if it's
    /// bigger than `file` can be.
    static func offset(of partial: URL, for file: ModelFile) -> Int64 {
        let size = FetchedFile.size(of: partial) ?? 0
        guard size > file.size else { return size }
        try? FileManager.default.removeItem(at: partial)
        return 0
    }
}

/// Files a fetch writes.
enum FetchedFile {
    static func size(of url: URL) -> Int64? {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path(percentEncoded: false)),
            let size = attributes[.size] as? NSNumber
        else { return nil }
        return size.int64Value
    }

    /// Opens `partial` to write at its end, creating it if it isn't there.
    static func appending(to partial: URL) throws(ModelDownloadError) -> FileHandle {
        do {
            if !FileManager.default.fileExists(atPath: partial.path(percentEncoded: false)) {
                guard FileManager.default.createFile(atPath: partial.path(percentEncoded: false), contents: nil) else {
                    throw CocoaError(.fileWriteUnknown)
                }
            }
            let handle = try FileHandle(forWritingTo: partial)
            try handle.seekToEnd()
            return handle
        } catch {
            throw .storage(.fileSystem(operation: "open", path: partial.path(), reason: "\(error)"))
        }
    }
}

/// Fetches a chunk at a time with Range requests on an ordinary session, so
/// a pause or a dropped connection loses at most one chunk. While Lifr is
/// suspended nothing comes; it goes on when Lifr does.
struct ChunkedFetcher: ModelFetcher {
    let hub: HubClient
    let chunkSize: Int64

    func fetch(
        _ file: ModelFile, from url: URL, into partial: URL, received: @escaping @Sendable (Int64) -> Void
    ) async throws(ModelDownloadError) {
        var offset = Self.offset(of: partial, for: file)
        let handle = try FetchedFile.appending(to: partial)
        defer { try? handle.close() }
        while offset < file.size {
            guard !Task.isCancelled else { throw .paused }
            let end = min(offset + chunkSize, file.size) - 1
            let (data, response) = try await hub.bytes(url, offset...end, what: file.name)
            let expected = end - offset + 1
            switch response.statusCode {
            case 206 where Int64(data.count) == expected:
                break
            case 200 where offset == 0 && Int64(data.count) == file.size:
                break
            case 200, 206:
                throw .wrongSize(file: file.name, expected: expected, received: Int64(data.count))
            default:
                throw .hub(.status(response.statusCode, file.name))
            }
            do {
                try handle.write(contentsOf: data)
            } catch {
                throw .storage(.fileSystem(operation: "write", path: partial.path(), reason: "\(error)"))
            }
            offset += Int64(data.count)
            received(Int64(data.count))
        }
    }
}
