import ComposableArchitecture
import Foundation
import LifrModelsClient

/// Fetches through a background session, so the system goes on downloading
/// while Lifr is suspended, or after it has quit, and wakes Lifr to take the
/// file. Each task asks for the rest of a file, from where its `.partial`
/// ends, and says so in its description; whatever process gets the finished
/// download appends it, even one that didn't start it.
final class BackgroundFetcher: ModelFetcher {
    /// The session's identifier, for SwiftUI's `.backgroundTask(.urlSession(_:))`.
    static let identifier = "\(Bundle.main.bundleIdentifier ?? "Lifr").models"

    private let session: URLSession
    private let events: Events

    /// - Parameter root: The folder the partial files are in. Tasks name
    ///   their file relative to it, since an app's container can move.
    init(root: URL, identifier: String = BackgroundFetcher.identifier) {
        events = Events(root: root)
        let configuration = URLSessionConfiguration.background(withIdentifier: identifier)
        configuration.sessionSendsLaunchEvents = true
        configuration.isDiscretionary = false
        configuration.httpAdditionalHeaders = ["User-Agent": "Lifr"]
        session = URLSession(configuration: configuration, delegate: events, delegateQueue: nil)
    }

    /// Returns once the system has handed over everything it downloaded
    /// while Lifr wasn't running.
    func caughtUp() async {
        await events.caughtUp()
    }

    func fetch(
        _ file: ModelFile, from url: URL, into partial: URL, received: @escaping @Sendable (Int64) -> Void
    ) async throws(ModelDownloadError) {
        var retried = false
        while true {
            let offset = Self.offset(of: partial, for: file)
            guard offset < file.size else { return }
            let job = BackgroundJob(
                partial: events.relative(partial), offset: offset, size: file.size, name: file.name)
            let (task, resumed) = await task(for: job, from: url)
            let result = await withTaskCancellationHandler {
                await events.wait(for: task, received: received)
            } onCancel: {
                task.cancel(byProducingResumeData: { _ in })
            }
            switch result {
            case .success:
                continue
            case .failure(let error) where resumed && !retried && error != .paused:
                // The saved resume data may be stale; ask afresh once.
                retried = true
            case .failure(let error):
                throw error
            }
        }
    }

    /// A task for `job`: one still running from before, one picking up its
    /// saved resume data, or a new request for the rest of the file.
    private func task(for job: BackgroundJob, from url: URL) async -> (URLSessionDownloadTask, resumed: Bool) {
        let running = await session.allTasks.compactMap { $0 as? URLSessionDownloadTask }
        if let task = running.first(where: {
            $0.state == .running && BackgroundJob.decode($0.taskDescription)?.partial == job.partial
        }) {
            return (task, false)
        }
        let partial = events.absolute(job.partial)
        let task: URLSessionDownloadTask
        let resumed: Bool
        if let record = ResumeRecord.take(for: partial), record.offset == job.offset {
            task = session.downloadTask(withResumeData: record.data)
            resumed = true
        } else {
            var request = URLRequest(url: url)
            if job.offset > 0 { request.setValue("bytes=\(job.offset)-", forHTTPHeaderField: "Range") }
            task = session.downloadTask(with: request)
            resumed = false
        }
        task.taskDescription = job.encoded
        task.countOfBytesClientExpectsToReceive = job.size - job.offset
        task.resume()
        return (task, resumed)
    }
}

/// What a background task is for, kept in its description.
struct BackgroundJob: Codable, Equatable {
    /// The `.partial` file, relative to the library's root.
    var partial: String
    /// Where the request starts in the file: the partial's size then.
    var offset: Int64
    /// The whole file's size.
    var size: Int64
    /// The file's name, for errors.
    var name: String

    var encoded: String? {
        (try? JSONEncoder().encode(self)).flatMap { String(data: $0, encoding: .utf8) }
    }

    static func decode(_ description: String?) -> BackgroundJob? {
        guard let description else { return nil }
        return try? JSONDecoder().decode(BackgroundJob.self, from: Data(description.utf8))
    }

    /// Adds what came down for this job, in `download`, to `partial`.
    /// Throws, and starts the file over, if it doesn't fit where it goes.
    func land(_ download: URL, response: URLResponse?, at partial: URL) throws(ModelDownloadError) {
        guard let http = response as? HTTPURLResponse else { throw .hub(.unreadable(name)) }
        do {
            try HubClient.check(http, what: name)
        } catch {
            throw .hub(error)
        }
        let from: Int64
        switch http.statusCode {
        case 206:
            guard let start = Self.start(of: http) else { throw .hub(.unreadable(name)) }
            from = start
        case 200:
            from = 0
        default:
            throw .hub(.status(http.statusCode, name))
        }
        let have = FetchedFile.size(of: partial) ?? 0
        guard from == 0 || from == have else {
            // The partial changed since this was asked for; ask again.
            return
        }
        if from == 0 {
            try? FileManager.default.removeItem(at: partial)
            do {
                try FileManager.default.moveItem(at: download, to: partial)
            } catch {
                throw .storage(.fileSystem(operation: "move", path: partial.path(), reason: "\(error)"))
            }
        } else {
            try append(download, to: partial)
        }
        let landed = FetchedFile.size(of: partial) ?? 0
        guard landed == size else {
            try? FileManager.default.removeItem(at: partial)
            throw .wrongSize(file: name, expected: size, received: landed)
        }
    }

    private func append(_ download: URL, to partial: URL) throws(ModelDownloadError) {
        let handle = try FetchedFile.appending(to: partial)
        defer { try? handle.close() }
        do {
            let source = try FileHandle(forReadingFrom: download)
            defer { try? source.close() }
            try FetchedFile.forEachChunk(of: source) { try handle.write(contentsOf: $0) }
        } catch {
            throw .storage(.fileSystem(operation: "write", path: partial.path(), reason: "\(error)"))
        }
    }

    /// Where a partial response starts, from `Content-Range: bytes a-b/n`.
    static func start(of http: HTTPURLResponse) -> Int64? {
        guard let range = http.value(forHTTPHeaderField: "Content-Range"),
            range.lowercased().hasPrefix("bytes ")
        else { return nil }
        return range.dropFirst("bytes ".count).split(separator: "-").first.flatMap { Int64($0) }
    }
}

/// Resume data saved next to a partial file when its task stopped, with the
/// offset that task started at.
struct ResumeRecord: Codable {
    var offset: Int64
    var data: Data

    static func file(for partial: URL) -> URL {
        partial.deletingLastPathComponent().appending(path: partial.lastPathComponent + ".resume")
    }

    func save(for partial: URL) {
        try? JSONEncoder().encode(self).write(to: Self.file(for: partial), options: .atomic)
    }

    /// The record for `partial`, removing it.
    static func take(for partial: URL) -> ResumeRecord? {
        let file = file(for: partial)
        defer { try? FileManager.default.removeItem(at: file) }
        guard let data = try? Data(contentsOf: file) else { return nil }
        return try? JSONDecoder().decode(ResumeRecord.self, from: data)
    }
}

/// The session's delegate: lands downloads, reports progress and wakes
/// whoever waits for a task.
private final class Events: NSObject, URLSessionDownloadDelegate, Sendable {
    private struct State {
        var waiters: [Int: CheckedContinuation<Result<Void, ModelDownloadError>, Never>] = [:]
        var outcomes: [Int: Result<Void, ModelDownloadError>] = [:]
        var progress: [Int: @Sendable (Int64) -> Void] = [:]
        var written: [Int: Int64] = [:]
        var refused: [Int: String] = [:]
        var landed: [Int: ModelDownloadError?] = [:]
        var caughtUp: [CheckedContinuation<Void, Never>] = []
        /// Whether the system finished handing over events no one waited for.
        var finished = false
    }

    private let root: URL
    private let state = LockIsolated(State())

    init(root: URL) {
        self.root = root
    }

    func relative(_ url: URL) -> String {
        let base = root.standardizedFileURL.path(percentEncoded: false)
        let path = url.standardizedFileURL.path(percentEncoded: false)
        guard path.hasPrefix(base) else { return path }
        return String(path.dropFirst(base.count)).trimmingPrefix("/").description
    }

    func absolute(_ relative: String) -> URL {
        relative.hasPrefix("/")
            ? URL(filePath: relative) : root.appending(path: relative, directoryHint: .notDirectory)
    }

    func wait(
        for task: URLSessionTask, received: @escaping @Sendable (Int64) -> Void
    ) async -> Result<Void, ModelDownloadError> {
        let id = task.taskIdentifier
        return await withCheckedContinuation { continuation in
            let done = state.withValue { state -> Result<Void, ModelDownloadError>? in
                if let outcome = state.outcomes.removeValue(forKey: id) { return outcome }
                state.waiters[id] = continuation
                state.progress[id] = received
                return nil
            }
            if let done { continuation.resume(returning: done) }
        }
    }

    func caughtUp() async {
        await withCheckedContinuation { continuation in
            let finished = state.withValue { state in
                defer { state.finished = false }
                if !state.finished { state.caughtUp.append(continuation) }
                return state.finished
            }
            if finished { continuation.resume() }
        }
    }

    // MARK: URLSessionDownloadDelegate

    func urlSession(
        _ session: URLSession, downloadTask: URLSessionDownloadTask, didResumeAtOffset fileOffset: Int64,
        expectedTotalBytes: Int64
    ) {
        report(downloadTask, written: fileOffset)
    }

    func urlSession(
        _ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64,
        totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64
    ) {
        report(downloadTask, written: totalBytesWritten)
    }

    func urlSession(
        _ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL
    ) {
        let id = downloadTask.taskIdentifier
        guard let job = BackgroundJob.decode(downloadTask.taskDescription) else { return }
        let error: ModelDownloadError?
        if let host = state.value.refused[id] {
            error = .hub(.redirectNotAllowed(host))
        } else {
            do throws(ModelDownloadError) {
                try job.land(location, response: downloadTask.response, at: absolute(job.partial))
                error = nil
            } catch let landing {
                error = landing
            }
        }
        state.withValue { $0.landed[id] = error }
    }

    func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void
    ) {
        guard let url = request.url, HostPolicy.allows(url) else {
            state.withValue { $0.refused[task.taskIdentifier] = request.url?.host() ?? "an unknown host" }
            completionHandler(nil)
            return
        }
        completionHandler(request)
    }

    func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: (any Error)?) {
        let id = task.taskIdentifier
        let job = BackgroundJob.decode(task.taskDescription)
        let outcome: Result<Void, ModelDownloadError>
        let (landed, refused) = state.withValue { state in
            (state.landed.removeValue(forKey: id), state.refused.removeValue(forKey: id))
        }
        if let refused {
            outcome = .failure(.hub(.redirectNotAllowed(refused)))
        } else if let error {
            let error = error as NSError
            if let job, let data = error.userInfo[NSURLSessionDownloadTaskResumeData] as? Data {
                ResumeRecord(offset: job.offset, data: data).save(for: absolute(job.partial))
            }
            outcome = .failure(
                error.domain == NSURLErrorDomain && error.code == NSURLErrorCancelled
                    ? .paused : .hub(.offline(error.localizedDescription)))
        } else if let landed {
            outcome = landed.map { .failure($0) } ?? .success(())
        } else {
            outcome = .failure(.hub(.unreadable(job?.name ?? "the model")))
        }
        let waiter = state.withValue { state in
            state.progress[id] = nil
            state.written[id] = nil
            if let waiter = state.waiters.removeValue(forKey: id) { return Optional(waiter) }
            state.outcomes[id] = outcome
            return nil
        }
        waiter?.resume(returning: outcome)
    }

    func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
        let waiting = state.withValue { state in
            defer { state.caughtUp = [] }
            state.finished = state.caughtUp.isEmpty
            return state.caughtUp
        }
        for continuation in waiting { continuation.resume() }
    }

    /// Tells the task's waiter about the bytes since the last report.
    private func report(_ task: URLSessionTask, written: Int64) {
        let id = task.taskIdentifier
        let report = state.withValue { state -> (@Sendable (Int64) -> Void, Int64)? in
            // Until someone waits, the bytes count from zero for them.
            guard let progress = state.progress[id] else { return nil }
            let added = written - (state.written[id] ?? 0)
            state.written[id] = written
            return (progress, added)
        }
        if let (progress, added) = report, added > 0 { progress(added) }
    }
}
