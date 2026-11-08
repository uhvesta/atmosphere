import ComposableArchitecture
import Darwin
import Foundation
import LifrAPI
import LifrCore
import LifrSessionClient
import LifrStoreClient
import XCTest
import os

@testable import LifrSessionLive

/// A server on a socket in a fresh folder, with the peer check, Keychain
/// and Touch ID played by the test.
final class Harness: Sendable {
    let folder: URL
    let server: SessionServer
    let store: StoreClient
    let grants: GrantStorage
    /// Who the next connection is, or why it's refused.
    let peer: OSAllocatedUnfairLock<Result<Peer, APIError>>
    /// Touch ID's answer, and every reason it was asked for.
    let touchID: OSAllocatedUnfairLock<(answer: Bool, reasons: [String])>
    let now = Date(timeIntervalSince1970: 1_800_000_000)

    static let claude = Peer(
        launcher: Launcher(identity: "com.anthropic.claude-code", team: "Q6L2SF6YDW", name: "Claude Code"), tty: nil)
    static let terminal = Peer(
        launcher: Launcher(identity: "com.apple.Terminal", team: nil, name: "Terminal", isTerminal: true), tty: 5)

    init(peer: Peer = Harness.claude, grants: [StoredGrant] = [], peers: PeerCheck? = nil) async throws {
        // Unix socket paths are short, so not the long temporary folder.
        folder = URL(fileURLWithPath: "/tmp/lifr-\(UUID().uuidString.prefix(8))", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        // The server saves from its own tasks, which don't inherit a test's
        // dependencies, so the store sets them for each change.
        let inner = StoreClient.inMemory(Workspace(), actor: .user(device: "Test Mac", surface: .mac))
        let now = now
        store = StoreClient(
            load: inner.load, snapshot: inner.snapshot,
            save: { operation in
                try await withDependencies {
                    $0.date.now = now
                    $0.uuid = .incrementing
                    $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
                } operation: {
                    try await inner.save(operation)
                }
            },
            updates: inner.updates, importSynced: inner.importSynced, archiveJournal: inner.archiveJournal,
            archivedJournal: inner.archivedJournal)
        _ = try await store.createAvenue(key: "HEALTH", name: "Health")
        self.grants = .inMemory(grants)
        let peer = OSAllocatedUnfairLock<Result<Peer, APIError>>(initialState: .success(peer))
        self.peer = peer
        let touchID = OSAllocatedUnfairLock(initialState: (answer: true, reasons: [String]()))
        self.touchID = touchID
        server = SessionServer(
            SessionServer.Environment(
                socket: folder.appendingPathComponent("lifr.sock"),
                disabledMarker: folder.appendingPathComponent("api-disabled"),
                peers: peers ?? PeerCheck { _ in peer.withLock { $0 } },
                grants: self.grants,
                authenticate: { reason in
                    touchID.withLock { state in
                        state.reasons.append(reason)
                        return state.answer
                    }
                },
                store: store, device: "Test Mac", now: { now },
                log: AccessLog(file: folder.appendingPathComponent("access.log")),
                cli: CLILink(helper: folder.appendingPathComponent("lifr"), directories: []),
                loginItem: LoginItem(isEnabled: { false }, setEnabled: { _ in })))
        await server.start()
    }

    deinit {
        try? FileManager.default.removeItem(at: folder)
    }

    var socket: URL { folder.appendingPathComponent("lifr.sock") }

    func connect() throws -> TestClient { try TestClient(path: socket.path) }

    /// A grant you approved for Claude Code.
    static func approved(_ scopes: Set<LifrAPI.Scope>, confirmWrites: Bool = false, team: String? = "Q6L2SF6YDW")
        -> StoredGrant
    {
        StoredGrant(
            identity: claude.launcher.identity, team: team,
            grant: Grant(
                client: "Claude Code", scopes: scopes, expiresAt: Date(timeIntervalSince1970: 1_900_000_000),
                confirmWrites: confirmWrites))
    }
}

/// The CLI's side of the socket, kept simple: send a line, read a line.
final class TestClient: @unchecked Sendable {
    private let socket: Int32
    private var buffer = Data()
    private var nextID = 1

    init(path: String) throws {
        socket = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: Array(path.utf8) + [0]) }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(socket, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else {
            Darwin.close(socket)
            throw POSIXError(.ECONNREFUSED)
        }
        var timeout = timeval(tv_sec: 10, tv_usec: 0)
        setsockopt(socket, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
    }

    deinit { Darwin.close(socket) }

    /// Sends `method` and waits for its response.
    func call(_ method: APIMethod, _ params: JSONValue = [:], surface: Actor.Surface = .cli) throws -> RPCResponse {
        try send(method, params, surface: surface)
        return try XCTUnwrap(receive())
    }

    func send(_ method: APIMethod, _ params: JSONValue = [:], surface: Actor.Surface = .cli) throws {
        let line = try Wire.line(
            RPCRequest(id: nextID, method: method.rawValue, params: params, surface: surface))
        nextID += 1
        _ = line.withUnsafeBytes { Darwin.send(socket, $0.baseAddress, $0.count, 0) }
    }

    /// The next response, or `nil` once the app closes the connection.
    func receive() throws -> RPCResponse? {
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            if let newline = buffer.firstIndex(of: UInt8(ascii: "\n")) {
                let line = buffer[buffer.startIndex..<newline]
                buffer.removeSubrange(buffer.startIndex...newline)
                return try Wire.decoder.decode(RPCResponse.self, from: Data(line))
            }
            let count = recv(socket, &chunk, chunk.count, 0)
            guard count > 0 else { return nil }
            buffer.append(contentsOf: chunk[0..<count])
        }
    }

    func hello() throws {
        let response = try call(.hello, try JSONValue(Hello.Params(client: "test")))
        XCTAssertNil(response.error)
    }

    func close() { shutdown(socket, SHUT_RDWR) }
}

/// Waits for the first event `match` picks out.
func next<T: Sendable>(
    _ events: AsyncStream<SessionEvent>, timeout: Duration = .seconds(10),
    _ match: @escaping @Sendable (SessionEvent) -> T?
) async throws -> T {
    try await withThrowingTaskGroup(of: T?.self) { group in
        group.addTask {
            for await event in events {
                if let found = match(event) { return found }
            }
            return nil
        }
        group.addTask {
            try await Task.sleep(for: timeout)
            return nil
        }
        let found = try await group.next() ?? nil
        group.cancelAll()
        return try XCTUnwrap(found, "No such event within \(timeout).")
    }
}

/// Runs blocking socket calls off Swift's thread pool, which the server
/// needs.
func background<T: Sendable>(_ body: @escaping @Sendable () throws -> T) async throws -> T {
    try await withCheckedThrowingContinuation { continuation in
        DispatchQueue.global().async { continuation.resume(with: Result { try body() }) }
    }
}
