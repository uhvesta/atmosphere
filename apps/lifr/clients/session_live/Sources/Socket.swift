#if os(macOS)
    import Darwin
    import Foundation
    import LifrSessionClient
    import os

    /// Listens on a Unix socket, mode 0600, until stopped.
    final class SocketListener: Sendable {
        private let path: URL
        private let source: any DispatchSourceRead

        /// Binds `path`, replacing a socket a previous run left behind.
        init(path: URL, accepted: @escaping @Sendable (Connection) -> Void) throws(SessionError) {
            self.path = path
            try? FileManager.default.createDirectory(
                at: path.deletingLastPathComponent(), withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700])
            unlink(path.path)
            let descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
            guard descriptor >= 0 else { throw Self.failure("can't create a socket") }
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            let bytes = Array(path.path.utf8)
            guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
                close(descriptor)
                throw .apiSwitchFailed(reason: "the socket path \(path.path) is too long")
            }
            withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes + [0]) }
            let bound = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            guard bound == 0, chmod(path.path, 0o600) == 0, listen(descriptor, 16) == 0 else {
                let error = Self.failure("can't listen at \(path.path)")
                close(descriptor)
                throw error
            }
            _ = fcntl(descriptor, F_SETFL, fcntl(descriptor, F_GETFL) | O_NONBLOCK)
            source = DispatchSource.makeReadSource(fileDescriptor: descriptor, queue: .global())
            source.setEventHandler {
                while true {
                    let client = accept(descriptor, nil, nil)
                    guard client >= 0 else { return }
                    _ = fcntl(client, F_SETFL, fcntl(client, F_GETFL) & ~O_NONBLOCK)
                    var on: Int32 = 1
                    setsockopt(client, SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
                    accepted(Connection(descriptor: client))
                }
            }
            source.setCancelHandler { close(descriptor) }
            source.resume()
        }

        /// Stops accepting and removes the socket. Open connections stay open.
        func stop() {
            source.cancel()
            unlink(path.path)
        }

        private static func failure(_ what: String) -> SessionError {
            .apiSwitchFailed(reason: "\(what) (\(String(cString: strerror(errno))))")
        }
    }

    /// One accepted connection, speaking one message per line.
    final class Connection: Sendable {
        let descriptor: Int32
        private let buffer = OSAllocatedUnfairLock(initialState: Data())

        /// Longer lines close the connection; no request comes near it.
        static let maxLine = 1 << 20

        init(descriptor: Int32) {
            self.descriptor = descriptor
        }

        deinit { Darwin.close(descriptor) }

        /// The next line, without its newline, or `nil` once the other end hangs
        /// up or sends too much.
        func readLine() async -> Data? {
            await withCheckedContinuation { continuation in
                DispatchQueue.global().async { continuation.resume(returning: self.blockingReadLine()) }
            }
        }

        /// Sends `data`, which ends with a newline. `false` when the other end
        /// is gone.
        @discardableResult
        func write(_ data: Data) -> Bool {
            data.withUnsafeBytes { bytes in
                var offset = 0
                while offset < bytes.count {
                    let written = send(descriptor, bytes.baseAddress! + offset, bytes.count - offset, 0)
                    guard written > 0 else { return false }
                    offset += written
                }
                return true
            }
        }

        /// Whether the other end hung up, without taking anything it sent.
        var hasHungUp: Bool {
            var byte: UInt8 = 0
            let count = recv(descriptor, &byte, 1, MSG_PEEK | MSG_DONTWAIT)
            return count == 0 || (count < 0 && errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR)
        }

        /// Ends the connection. A read waiting on it returns `nil`.
        func close() {
            shutdown(descriptor, SHUT_RDWR)
        }

        private func blockingReadLine() -> Data? {
            var chunk = [UInt8](repeating: 0, count: 64 * 1024)
            while true {
                let line: Data? = buffer.withLock { buffer in
                    guard let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) else { return nil }
                    let line = Data(buffer[buffer.startIndex..<newline])
                    buffer.removeSubrange(buffer.startIndex...newline)
                    return line
                }
                if let line { return line }
                let count = recv(descriptor, &chunk, chunk.count, 0)
                guard count > 0 else { return nil }
                let tooLong = buffer.withLock { buffer in
                    buffer.append(contentsOf: chunk[0..<count])
                    return buffer.count > Self.maxLine
                }
                if tooLong { return nil }
            }
        }
    }
#endif
