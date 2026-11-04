import Darwin
import Foundation
import LifrAPI
import LifrCore

/// Sends requests to the Lifr app, one at a time.
package protocol Connection: AnyObject {
    /// The result of `method`, or the app's `APIError`.
    func call(_ method: APIMethod, _ params: JSONValue) throws(APIError) -> JSONValue
    /// Where the requests say they come from, for the journal.
    var surface: Actor.Surface { get set }
}

/// A connection over the app's socket. Opening one starts the app if it
/// isn't running and says hello.
package final class SocketConnection: Connection {
    private let socket: Int32
    private var buffer = Data()
    private var nextID = 1
    package var surface = Actor.Surface.cli

    /// How long to wait for the app to start.
    static let launchTimeout: TimeInterval = 10

    /// Connects as `client`, such as "lifr 0.1.0", starting the app if it
    /// isn't running.
    package static func open(client: String, home: URL = FileManager.default.homeDirectoryForCurrentUser)
        throws(APIError) -> SocketConnection
    {
        if FileManager.default.fileExists(atPath: LocalSocket.disabledMarker(home: home).path) {
            throw .apiDisabled
        }
        let path = LocalSocket.socket(home: home).path
        var socket = connect(to: path)
        if socket == nil {
            launchApp()
            let deadline = Date() + launchTimeout
            while socket == nil, Date() < deadline {
                usleep(200_000)
                socket = connect(to: path)
            }
        }
        guard let socket else { throw .appNotRunning }
        let connection = SocketConnection(socket: socket)
        // Compares protocol versions; the app refuses a mismatch.
        _ = try connection.call(.hello, try encode(Hello.Params(client: client)))
        return connection
    }

    private init(socket: Int32) {
        self.socket = socket
    }

    deinit { close(socket) }

    package func call(_ method: APIMethod, _ params: JSONValue) throws(APIError) -> JSONValue {
        let request = RPCRequest(id: nextID, method: method.rawValue, params: params, surface: surface)
        nextID += 1
        do {
            try send(Wire.line(request))
        } catch {
            throw closed
        }
        let response: RPCResponse
        do {
            response = try Wire.decoder.decode(RPCResponse.self, from: try receiveLine())
        } catch let error as APIError {
            throw error
        } catch {
            throw APIError(
                .internalError,
                "The Lifr app sent a response lifr can't read: \(error). Update Lifr so the CLI and app match.")
        }
        if let error = response.error { throw error.data }
        return response.result ?? .null
    }

    private var closed: APIError {
        APIError(
            .appNotRunning,
            "The Lifr app closed the connection, perhaps because it quit or the local API was turned off. "
                + "Open Lifr and retry.")
    }

    private func send(_ data: Data) throws {
        try data.withUnsafeBytes { (bytes: UnsafeRawBufferPointer) in
            guard let base = bytes.baseAddress else { return }
            var offset = 0
            while offset < bytes.count {
                let written = Darwin.send(socket, base + offset, bytes.count - offset, 0)
                guard written > 0 else { throw closed }
                offset += written
            }
        }
    }

    private func receiveLine() throws(APIError) -> Data {
        var chunk = [UInt8](repeating: 0, count: 64 * 1024)
        while true {
            if let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
                let line = buffer[buffer.startIndex..<newline]
                buffer.removeSubrange(buffer.startIndex...newline)
                return Data(line)
            }
            let count = recv(socket, &chunk, chunk.count, 0)
            guard count > 0 else { throw closed }
            buffer.append(contentsOf: chunk[0..<count])
        }
    }

    private static func connect(to path: String) -> Int32? {
        let socket = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard socket >= 0 else { return nil }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let capacity = MemoryLayout.size(ofValue: address.sun_path)
        guard path.utf8.count < capacity else {
            close(socket)
            return nil
        }
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            buffer.copyBytes(from: path.utf8)
            buffer[path.utf8.count] = 0
        }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(socket, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else {
            close(socket)
            return nil
        }
        return socket
    }

    /// Opens the app this `lifr` is bundled in, in the background, or Lifr
    /// by its bundle ID when this copy isn't inside an app.
    private static func launchApp() {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/open")
        if let app = bundlingApp() {
            process.arguments = ["-g", "-j", app.path]
        } else {
            process.arguments = ["-g", "-j", "-b", "com.avestabarzegar.lifr"]
        }
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try? process.run()
        process.waitUntilExit()
    }

    /// `Lifr.app` when this executable is its `Contents/Helpers/lifr`, after
    /// following the PATH link.
    private static func bundlingApp() -> URL? {
        guard let executable = Bundle.main.executableURL?.resolvingSymlinksInPath() else { return nil }
        let helpers = executable.deletingLastPathComponent()
        let app = helpers.deletingLastPathComponent().deletingLastPathComponent()
        guard helpers.lastPathComponent == "Helpers", app.pathExtension == "app" else { return nil }
        return app
    }

    private static func encode(_ value: some Encodable) throws(APIError) -> JSONValue {
        do {
            return try JSONValue(value)
        } catch {
            throw APIError(.internalError, "lifr couldn't encode its request: \(error). Report this.")
        }
    }
}
