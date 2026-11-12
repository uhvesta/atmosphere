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
    static let maxFrame = 64 * 1024 * 1024

    private let descriptor: Int32
    private let state = OSAllocatedUnfairLock(initialState: State())
    private let writing = OSAllocatedUnfairLock()
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
    func call(_ request: Humata_V1_Request.OneOf_Kind) async throws -> Humata_V1_Response.OneOf_Kind {
        let response = try await withCheckedThrowingContinuation { continuation in
            send(request, as: .call(continuation)) { continuation.resume(throwing: DaemonError.disconnected) }
        }
        return try Self.refused(response)
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

    /// Stops the stream `id`.
    private func cancel(_ id: UInt64) {
        let removed = state.withLock { $0.pending.removeValue(forKey: id) != nil }
        guard removed else { return }
        var envelope = Humata_V1_Envelope()
        envelope.id = id
        envelope.cancel = Humata_V1_Cancel()
        let cancel = envelope
        _ = writing.withLock { Self.write(cancel, to: descriptor) }
    }

    /// Ends the connection; everything waiting on it fails as disconnected.
    func close() {
        shutdown(descriptor, SHUT_RDWR)
    }

    @discardableResult
    private func send(_ request: Humata_V1_Request.OneOf_Kind, as pending: Pending, failed: () -> Void) -> UInt64? {
        let id: UInt64? = state.withLock { state in
            guard !state.closed else { return nil }
            let id = state.nextID
            state.nextID += 1
            state.pending[id] = pending
            return id
        }
        guard let id else {
            failed()
            return nil
        }
        var envelope = Humata_V1_Envelope()
        envelope.id = id
        var body = Humata_V1_Request()
        body.kind = request
        envelope.request = body
        let outgoing = envelope
        if !writing.withLock({ Self.write(outgoing, to: descriptor) }) {
            // The reader sees the hang-up and fails what is pending, this too.
            shutdown(descriptor, SHUT_RDWR)
        }
        return id
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
