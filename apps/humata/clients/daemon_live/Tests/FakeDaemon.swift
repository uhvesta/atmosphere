import Darwin
import Foundation
import HumataProto
import SwiftProtobuf

/// What went wrong in the stand-in daemon, so a test fails instead of
/// waiting for ever.
enum FakeDaemonError: Error {
    /// A socket call failed with this `errno`.
    case socket(call: String, errno: Int32)
    /// The client closed the connection, or sent nothing for `FakeDaemon.patience`.
    case nothingRead(wanted: Int)
    case notConnected
    case unexpected(Humata_V1_Envelope)
}

/// A stand-in for humatad on a Unix socket: it takes one connection, says
/// hello, and then reads and writes only when the test says so, which lets a
/// test leave the client's bytes unread.
final class FakeDaemon: @unchecked Sendable {
    /// How long a read waits for the client, in seconds.
    static let patience = 20

    let path: String
    private let listener: Int32
    /// The accepted connection. Only touched on `queue`.
    private var peer: Int32 = -1
    private let queue = DispatchQueue(label: "humata.fake-daemon")

    init() throws {
        // Socket paths are limited to about 100 bytes, so not under Bazel's
        // long test folder.
        path = "/tmp/humata-\(UUID().uuidString.prefix(8)).sock"
        listener = socket(AF_UNIX, SOCK_STREAM, 0)
        guard listener >= 0 else { throw FakeDaemonError.socket(call: "socket", errno: errno) }
        Self.setPatience(listener)
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes + [0]) }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else { throw FakeDaemonError.socket(call: "bind", errno: errno) }
        guard listen(listener, 4) == 0 else { throw FakeDaemonError.socket(call: "listen", errno: errno) }
    }

    deinit {
        if peer >= 0 { Darwin.close(peer) }
        Darwin.close(listener)
        unlink(path)
    }

    /// Takes the client's connection and trades hellos with it.
    func accept() async throws {
        try await onQueue { [self] in
            let accepted = Darwin.accept(listener, nil, nil)
            guard accepted >= 0 else { throw FakeDaemonError.socket(call: "accept", errno: errno) }
            Self.setPatience(accepted)
            var on: Int32 = 1
            setsockopt(accepted, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
            peer = accepted
            let theirs = try readBody(readLength())
            guard case .hello = theirs.body else { throw FakeDaemonError.unexpected(theirs) }
            var hello = Humata_V1_Hello()
            hello.protocolVersion = .current
            hello.role = .daemon
            hello.programVersion = "fake"
            var envelope = Humata_V1_Envelope()
            envelope.hello = hello
            try write(envelope)
        }
    }

    /// Reads only the next frame's length, leaving its body in the socket.
    func nextLength() async throws -> Int {
        try await onQueue { [self] in try readLength() }
    }

    /// Reads the body of a frame whose length `nextLength` returned.
    func body(_ length: Int) async throws -> Humata_V1_Envelope {
        try await onQueue { [self] in try readBody(length) }
    }

    /// Reads the next frame whole.
    func next() async throws -> Humata_V1_Envelope {
        try await onQueue { [self] in try readBody(readLength()) }
    }

    /// Answers the request `id`.
    func answer(_ id: UInt64, _ kind: Humata_V1_Response.OneOf_Kind) async throws {
        var response = Humata_V1_Response()
        response.kind = kind
        var envelope = Humata_V1_Envelope()
        envelope.id = id
        envelope.response = response
        let outgoing = envelope
        try await onQueue { [self] in try write(outgoing) }
    }

    /// Goes away: the connection closes and nothing listens any more.
    func stop() async throws {
        try await onQueue { [self] in
            if peer >= 0 { shutdown(peer, SHUT_RDWR) }
            shutdown(listener, SHUT_RDWR)
            unlink(path)
        }
    }

    private func onQueue<T: Sendable>(_ work: @escaping @Sendable () throws -> T) async throws -> T {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { continuation.resume(with: Result { try work() }) }
        }
    }

    private static func setPatience(_ descriptor: Int32) {
        var wait = timeval(tv_sec: patience, tv_usec: 0)
        setsockopt(descriptor, SOL_SOCKET, SO_RCVTIMEO, &wait, socklen_t(MemoryLayout<timeval>.size))
    }

    private func readLength() throws -> Int {
        try readExactly(4).reduce(0) { $0 << 8 | Int($1) }
    }

    private func readBody(_ length: Int) throws -> Humata_V1_Envelope {
        try Humata_V1_Envelope(serializedData: readExactly(length))
    }

    private func readExactly(_ count: Int) throws -> Data {
        guard peer >= 0 else { throw FakeDaemonError.notConnected }
        var data = Data(count: count)
        var offset = 0
        while offset < count {
            let read = data.withUnsafeMutableBytes { bytes -> Int in
                guard let base = bytes.baseAddress else { return 0 }
                return recv(peer, base + offset, count - offset, 0)
            }
            if read < 0 && errno == EINTR { continue }
            guard read > 0 else { throw FakeDaemonError.nothingRead(wanted: count - offset) }
            offset += read
        }
        return data
    }

    private func write(_ envelope: Humata_V1_Envelope) throws {
        guard peer >= 0 else { throw FakeDaemonError.notConnected }
        let body = try envelope.serializedData()
        var frame = Data()
        var length = UInt32(body.count).bigEndian
        withUnsafeBytes(of: &length) { frame.append(contentsOf: $0) }
        frame.append(body)
        try frame.withUnsafeBytes { bytes in
            guard let base = bytes.baseAddress else { return }
            var offset = 0
            while offset < bytes.count {
                let written = Darwin.send(peer, base + offset, bytes.count - offset, 0)
                if written < 0 && errno == EINTR { continue }
                guard written > 0 else { throw FakeDaemonError.socket(call: "send", errno: errno) }
                offset += written
            }
        }
    }
}
