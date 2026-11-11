import Foundation
import KhodrDaemonClient
import KhodrDaemonLive
import XCTest

/// The real daemon through the app's client: a session on a remote host
/// (reached by running `khodrd host --stdio` directly, as SSH would), played,
/// watched and forked at an earlier event.
final class ForkFromSwiftTests: XCTestCase {
    private static let script = """
        {"rules": [
            {"when": "hello", "steps": [{"step": "say", "text": "hi there"}]},
            {"when": "", "steps": [{"step": "say", "text": "something else"}]}
        ]}
        """

    private var daemon: Process?
    private var scratch: URL?

    override func tearDown() {
        daemon?.terminate()
        daemon?.waitUntilExit()
        if let scratch { try? FileManager.default.removeItem(at: scratch) }
        super.tearDown()
    }

    func testASessionOnARemoteHostForksAtAnEarlierEvent() async throws {
        let client = try startDaemon()
        let work = try XCTUnwrap(scratch).appending(path: "work")
        try FileManager.default.createDirectory(at: work, withIntermediateDirectories: true)

        let session = try await client.start(
            StartRequest(
                backend: .scripted, workingDirectory: work.path, name: "first", host: "remote",
                backendConfig: Self.script))
        XCTAssertEqual(session.host, "remote")
        try await client.send(session.id, "hello", .enqueue)
        try await client.send(session.id, "what now", .enqueue)
        try await untilTurnsEnd(2, in: session.id, client: client)

        let history = try await client.history(session.id)
        XCTAssertEqual(
            Self.said(history),
            [.user("hello"), .assistant("hi there"), .user("what now"), .assistant("something else")])
        let second = try XCTUnwrap(history.first { $0.kind == .userMessage("what now") })

        let fork = try await client.fork(session.id, second.id, "fork")
        XCTAssertEqual(fork.parentID, session.id)
        XCTAssertEqual(fork.forkedBeforeEventID, second.id)
        XCTAssertEqual(fork.host, "remote")
        let forked = try await client.history(fork.id)
        XCTAssertEqual(Self.said(forked), [.user("hello"), .assistant("hi there")])

        // The fork goes its own way, and the original stays as it was.
        try await client.send(fork.id, "something different", .enqueue)
        try await untilTurnsEnd(2, in: fork.id, client: client)
        let went = try await client.history(fork.id)
        XCTAssertEqual(
            Self.said(went),
            [.user("hello"), .assistant("hi there"), .user("something different"), .assistant("something else")])
        let original = try await client.history(session.id)
        XCTAssertEqual(original, history)

        let sessions = try await client.sessions(nil)
        XCTAssertEqual(Set(sessions.map(\.id)), [session.id, fork.id])
    }

    func testANotRunningDaemonSaysSo() async throws {
        let client = DaemonClient.live(socket: "/tmp/khodr-no-such-daemon.sock")
        do {
            _ = try await client.sessions(nil)
            XCTFail("listed sessions with no daemon")
        } catch let error as DaemonError {
            guard case .notRunning = error else { return XCTFail("\(error)") }
        }
    }

    // MARK: - Helpers

    private struct NeverListened: Error {
        var socket: String
    }

    private enum Said: Equatable {
        case user(String)
        case assistant(String)
    }

    private static func said(_ events: [SessionEvent]) -> [Said] {
        events.compactMap {
            switch $0.kind {
            case .userMessage(let text): .user(text)
            case .assistantMessage(let text): .assistant(text)
            default: nil
            }
        }
    }

    private func startDaemon() throws -> DaemonClient {
        let runfiles = try XCTUnwrap(
            ProcessInfo.processInfo.environment["TEST_SRCDIR"], "Run this with bazel test, which says where khodrd is.")
        let khodrd = URL(fileURLWithPath: runfiles).appending(path: Binaries.khodrd).resolvingSymlinksInPath()
        // Socket paths are limited to about 100 bytes, so not under Bazel's
        // long test folder.
        let root = URL(fileURLWithPath: "/tmp").appending(path: "khodr-\(UUID().uuidString.prefix(8))")
        scratch = root
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let hosts = [
            "hosts": [
                "remote": [
                    "command": [khodrd.path, "host", "--stdio", "--state", root.appending(path: "remote").path]
                ]
            ]
        ]
        let hostsFile = root.appending(path: "hosts.json")
        try JSONSerialization.data(withJSONObject: hosts).write(to: hostsFile)
        let socket = root.appending(path: "d.sock").path

        let process = Process()
        process.executableURL = khodrd
        process.arguments = [
            "daemon", "--state", root.appending(path: "daemon").path, "--socket", socket, "--hosts", hostsFile.path,
        ]
        try process.run()
        daemon = process

        let deadline = Date().addingTimeInterval(20)
        while !FileManager.default.fileExists(atPath: socket) {
            guard Date() < deadline, process.isRunning else {
                throw NeverListened(socket: socket)
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
        return .live(socket: socket)
    }

    /// Watches until the session has ended `count` turns and is idle again.
    private func untilTurnsEnd(_ count: Int, in session: String, client: DaemonClient) async throws {
        try await withThrowingTaskGroup(of: Void.self) { group in
            group.addTask {
                var ended = 0
                for try await event in await client.watch(session) {
                    if case .turnEnded = event.kind { ended += 1 }
                    if ended >= count, event.kind == .idle { return }
                }
                XCTFail("the watch ended before \(count) turns did")
            }
            group.addTask {
                try await Task.sleep(for: .seconds(20))
                XCTFail("\(session) didn't end \(count) turns in time")
            }
            try await group.next()
            group.cancelAll()
        }
    }
}
