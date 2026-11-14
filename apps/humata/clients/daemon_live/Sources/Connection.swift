import Darwin
import Foundation
import HumataDaemonClient
import HumataProto
import SwiftProtobuf
import os

/// One connection to the daemon's socket: envelopes, each after its length
/// as a 4-byte big-endian integer, a hello each way, then requests matched to
/// their answers by id.
/// A streamed answer's items.
typealias Items = AsyncThrowingStream<Humata_V1_StreamItem, any Error>

final class Connection: Sendable {
    private enum Pending {
        case call(CheckedContinuation<Humata_V1_Response.OneOf_Kind, any Error>)
        case stream(
            response: CheckedContinuation<Humata_V1_Response.OneOf_Kind, any Error>?,
            items: Items.Continuation)
    }

    private struct State {
        var pending: [UInt64: Pending] = [:]
        var nextID: UInt64 = 1
        var closed = false
    }

    /// The largest envelope either side accepts, as humatad's MAX_FRAME.
    static let maxFrame = 1024 * 1024 * 1024

    private let descriptor: Int32
    private let state = OSAllocatedUnfairLock(initialState: State())
    /// Writes every frame after the hello, one at a time and in the order
    /// they were handed over, so no caller's thread waits for the socket:
    /// a large frame, such as a message with images, would hold it up.
    private let writer = DispatchQueue(label: "humata.daemon.connection.write", qos: .userInitiated)
    let peer: Humata_V1_Hello

    private init(descriptor: Int32, peer: Humata_V1_Hello) {
        self.descriptor = descriptor
        self.peer = peer
    }

    deinit { Darwin.close(descriptor) }

    /// Connects to `path` and says hello.
    static func open(path: String, programVersion: String) async throws -> Connection {
        try await withCheckedThrowingContinuation { continuation in
            DispatchQueue.global().async {
                continuation.resume(with: Result { try openBlocking(path: path, programVersion: programVersion) })
            }
        }
    }

    private static func openBlocking(path: String, programVersion: String) throws -> Connection {
        let descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw DaemonError.notRunning(socket: path, reason: errorText()) }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            Darwin.close(descriptor)
            throw DaemonError.notRunning(socket: path, reason: "the path is too long for a socket")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes + [0]) }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else {
            let reason = errorText()
            Darwin.close(descriptor)
            throw DaemonError.notRunning(socket: path, reason: reason)
        }
        var on: Int32 = 1
        setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))

        var hello = Humata_V1_Hello()
        hello.protocolVersion = .current
        hello.role = .client
        hello.programVersion = programVersion
        hello.clientKind = .app
        var envelope = Humata_V1_Envelope()
        envelope.hello = hello
        guard write(envelope, to: descriptor) else {
            Darwin.close(descriptor)
            throw DaemonError.disconnected
        }
        guard let answer = try read(from: descriptor) else {
            Darwin.close(descriptor)
            throw DaemonError.disconnected
        }
        guard case .hello(let peer) = answer.body else {
            Darwin.close(descriptor)
            throw DaemonError.unexpected("a \(String(describing: answer.body)) before its hello")
        }
        guard peer.protocolVersion == .current else {
            Darwin.close(descriptor)
            throw DaemonError.versionMismatch(daemon: peer.programVersion)
        }
        let connection = Connection(descriptor: descriptor, peer: peer)
        let thread = Thread { connection.readAnswers() }
        thread.name = "Humata daemon connection"
        thread.start()
        return connection
    }

    /// Whether the connection has closed; a new one is needed.
    var isClosed: Bool { state.withLock(\.closed) }

    /// Sends a request answered with one response. An error response throws
    /// `DaemonError.refused`.
    ///
    /// A request that only reads (`onlyReads`) is cut when the caller's task
    /// is cancelled: it throws `CancellationError` at once and its answer is
    /// dropped when it comes. One that changes something waits for its
    /// answer whatever happens to the task, so the caller always learns
    /// whether it was done.
    func call(_ request: Humata_V1_Request.OneOf_Kind) async throws -> Humata_V1_Response.OneOf_Kind {
        guard Self.onlyReads(request) else {
            let response = try await withCheckedThrowingContinuation { continuation in
                send(request, as: .call(continuation)) { continuation.resume(throwing: DaemonError.disconnected) }
            }
            return try Self.refused(response)
        }
        try Task.checkCancellation()
        // The request's id once it is sent, and whether the task was
        // cancelled: whichever of the two comes second cuts the call.
        typealias Answer = CheckedContinuation<Humata_V1_Response.OneOf_Kind, any Error>
        let cut = OSAllocatedUnfairLock(initialState: (id: UInt64?.none, cancelled: false))
        let response = try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { (continuation: Answer) in
                let sent = self.send(request, as: .call(continuation)) {
                    continuation.resume(throwing: DaemonError.disconnected)
                }
                guard let sent else { return }
                let cancelled = cut.withLock { state -> Bool in
                    state.id = sent
                    return state.cancelled
                }
                if cancelled { self.abandon(sent) }
            }
        } onCancel: {
            let sent = cut.withLock { state -> UInt64? in
                state.cancelled = true
                return state.id
            }
            if let sent { self.abandon(sent) }
        }
        return try Self.refused(response)
    }

    /// Whether a request changes nothing, so a caller that no longer wants
    /// its answer can stop waiting for it. A request not listed here is
    /// taken to change something.
    private static func onlyReads(_ request: Humata_V1_Request.OneOf_Kind) -> Bool {
        switch request {
        case .listProjects, .listSessions, .listImportable, .listModels, .listSkipped,
            .readHistory, .readImage, .getChatCommands, .listProposals,
            .readChanges, .listThreads,
            .listFiles, .readFile, .listFileVersions, .listFolder,
            .listItems, .readBoardLog, .listInbox, .listApprovals,
            .getStorage, .listSkills, .listPlugins, .getSessionDefaults,
            .listHosts, .listSshHosts, .listRecoveries, .listTerminals, .listForwards,
            .search, .listPullRequests, .getPullRefresh, .getAgentAccess,
            .listSavedRepositories, .listRepositoryOwners:
            true
        default:
            false
        }
    }

    /// Stops waiting for the call `id`: its caller gets `CancellationError`
    /// and its answer, if one still comes, is dropped. Nothing happens if it
    /// was already answered. The daemon isn't told, as it finishes a plain
    /// call's work either way.
    private func abandon(_ id: UInt64) {
        let waiting = state.withLock { state -> CheckedContinuation<Humata_V1_Response.OneOf_Kind, any Error>? in
            guard case .call(let answer) = state.pending[id] else { return nil }
            state.pending[id] = nil
            return answer
        }
        waiting?.resume(throwing: CancellationError())
    }

    /// Sends a request answered with a response and a stream of items, and
    /// returns the items. Ending the iteration tells the daemon to stop.
    func stream(_ request: Humata_V1_Request.OneOf_Kind) async throws -> Items {
        let (items, continuation) = Items.makeStream()
        let id = OSAllocatedUnfairLock<UInt64?>(initialState: nil)
        let response = try await withCheckedThrowingContinuation { answer in
            let sent = send(request, as: .stream(response: answer, items: continuation)) {
                answer.resume(throwing: DaemonError.disconnected)
            }
            id.withLock { $0 = sent }
        }
        _ = try Self.refused(response)
        if let id = id.withLock({ $0 }) {
            continuation.onTermination = { [weak self] _ in self?.cancel(id) }
        }
        return items
    }

    /// Stops the stream `id`. Its entry goes at once, so nothing more is
    /// delivered to it; telling the daemon is handed to the writer, since the
    /// write waits for whichever frame is being written, and the thread that
    /// cancels is often the main one.
    private func cancel(_ id: UInt64) {
        let removed = state.withLock { $0.pending.removeValue(forKey: id) != nil }
        guard removed else { return }
        var envelope = Humata_V1_Envelope()
        envelope.id = id
        envelope.cancel = Humata_V1_Cancel()
        let cancel = envelope
        // Holds the connection until the write is over, so its descriptor
        // is still open then.
        writer.async { [self] in
            _ = Self.write(cancel, to: descriptor)
        }
    }

    /// Ends the connection; everything waiting on it fails as disconnected.
    func close() {
        shutdown(descriptor, SHUT_RDWR)
    }

    /// Registers `pending` and hands the request to the writer, without
    /// waiting for the socket. Requests are written in the order of their
    /// ids, which is the order this was called in. On a closed connection
    /// nothing is sent and `failed` runs at once.
    @discardableResult
    private func send(_ request: Humata_V1_Request.OneOf_Kind, as pending: Pending, failed: () -> Void) -> UInt64? {
        let id: UInt64? = state.withLock { state in
            guard !state.closed else { return nil }
            let id = state.nextID
            state.nextID += 1
            state.pending[id] = pending
            // Handed over under the lock, so two sends can't swap places
            // between taking their ids and joining the queue.
            writer.async { [self] in writeRequest(request, id: id) }
            return id
        }
        if id == nil { failed() }
        return id
    }

    /// Writes the request `id`, on the writer's queue.
    private func writeRequest(_ request: Humata_V1_Request.OneOf_Kind, id: UInt64) {
        // Nothing waits for it any more: a read cut before its turn, or
        // the connection closed.
        guard state.withLock({ $0.pending[id] != nil }) else { return }
        var envelope = Humata_V1_Envelope()
        envelope.id = id
        var body = Humata_V1_Request()
        body.kind = request
        envelope.request = body
        if !Self.write(envelope, to: descriptor) {
            // The reader sees the hang-up and fails what is pending, this too.
            shutdown(descriptor, SHUT_RDWR)
        }
    }

    private static func refused(_ response: Humata_V1_Response.OneOf_Kind) throws -> Humata_V1_Response.OneOf_Kind {
        if case .error(let error) = response {
            throw DaemonError.refused(ErrorCode(error.code), message: error.message)
        }
        return response
    }

    private func readAnswers() {
        while let envelope = try? Self.read(from: descriptor) {
            route(envelope)
        }
        let pending = state.withLock { state in
            state.closed = true
            defer { state.pending = [:] }
            return Array(state.pending.values)
        }
        for waiting in pending {
            switch waiting {
            case .call(let answer):
                answer.resume(throwing: DaemonError.disconnected)
            case .stream(let response, let items):
                response?.resume(throwing: DaemonError.disconnected)
                items.finish(throwing: DaemonError.disconnected)
            }
        }
    }

    private func route(_ envelope: Humata_V1_Envelope) {
        let id = envelope.id
        switch envelope.body {
        case .response(let response):
            guard let kind = response.kind else { return }
            let waiting = state.withLock { state -> Pending? in
                guard let waiting = state.pending.removeValue(forKey: id) else { return nil }
                if case .stream(.some, let items) = waiting, !Self.isError(kind) {
                    state.pending[id] = .stream(response: nil, items: items)
                }
                return waiting
            }
            switch waiting {
            case .call(let answer):
                answer.resume(returning: kind)
            case .stream(.some(let answer), let items):
                answer.resume(returning: kind)
                if Self.isError(kind) { items.finish() }
            case .stream(.none, _), nil:
                break
            }
        case .item(let item):
            let items = state.withLock { state -> Items.Continuation? in
                if case .stream(_, let items) = state.pending[id] { items } else { nil }
            }
            items?.yield(item)
        case .end(let end):
            let waiting = state.withLock { $0.pending.removeValue(forKey: id) }
            if case .stream(_, let items) = waiting {
                if end.hasError {
                    items.finish(throwing: DaemonError.refused(ErrorCode(end.error.code), message: end.error.message))
                } else {
                    items.finish()
                }
            }
        default:
            break
        }
    }

    private static func isError(_ kind: Humata_V1_Response.OneOf_Kind) -> Bool {
        if case .error = kind { true } else { false }
    }

    private static func write(_ envelope: Humata_V1_Envelope, to descriptor: Int32) -> Bool {
        guard let body = try? envelope.serializedData(), body.count <= maxFrame else { return false }
        var frame = Data()
        var length = UInt32(body.count).bigEndian
        withUnsafeBytes(of: &length) { frame.append(contentsOf: $0) }
        frame.append(body)
        return frame.withUnsafeBytes { bytes in
            guard let base = bytes.baseAddress else { return true }
            var offset = 0
            while offset < bytes.count {
                let written = Darwin.send(descriptor, base + offset, bytes.count - offset, 0)
                if written < 0 && errno == EINTR { continue }
                guard written > 0 else { return false }
                offset += written
            }
            return true
        }
    }

    /// The next envelope, or nil when the connection closed between envelopes.
    private static func read(from descriptor: Int32) throws -> Humata_V1_Envelope? {
        guard let prefix = readExactly(4, from: descriptor) else { return nil }
        let length = prefix.reduce(0) { $0 << 8 | Int($1) }
        guard length <= maxFrame else {
            throw DaemonError.unexpected("a message of \(length) bytes, over the limit")
        }
        guard let body = readExactly(length, from: descriptor) else { throw DaemonError.disconnected }
        do {
            return try Humata_V1_Envelope(serializedData: body)
        } catch {
            throw DaemonError.unexpected("a message it couldn't decode")
        }
    }

    private static func readExactly(_ count: Int, from descriptor: Int32) -> Data? {
        var data = Data(count: count)
        var offset = 0
        while offset < count {
            let read = data.withUnsafeMutableBytes { bytes -> Int in
                guard let base = bytes.baseAddress else { return 0 }
                return recv(descriptor, base + offset, count - offset, 0)
            }
            if read < 0 && errno == EINTR { continue }
            guard read > 0 else { return nil }
            offset += read
        }
        return data
    }

    private static func errorText() -> String {
        String(cString: strerror(errno))
    }
}
