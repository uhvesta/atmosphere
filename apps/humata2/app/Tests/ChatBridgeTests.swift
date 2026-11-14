import Foundation
import Humata2Core
import Humata2Models
import XCTest

/// The chat through the real core and its bindings, on a stand-in daemon
/// with a long history that answers in many small parts. Times reading the
/// history and streaming the reply, both as Swift ends up holding them.
@MainActor
final class ChatBridgeTests: XCTestCase {
    private static let rows = 5_000
    private static let deltas = 20_000
    private static let part = "token "

    private var daemon: Process?
    private var scratch: URL?

    override func tearDown() async throws {
        daemon?.terminate()
        daemon?.waitUntilExit()
        if let scratch { try? FileManager.default.removeItem(at: scratch) }
        try await super.tearDown()
    }

    func testALongChatAndALongReplyArriveWhole() async throws {
        let model = SessionsModel(socket: try startDaemon())
        XCTAssertNil(model.startError)
        let chat = model.chat

        let opening = ContinuousClock.now
        model.send(.select(id: "rehearsal"))
        try await until("the history is read") {
            chat.status.reading == nil && chat.rows.count == Self.rows
        }
        let opened = ContinuousClock.now - opening
        for index in [0, 1, Self.rows - 1] {
            XCTAssertEqual(chat.rows[index].id, "h\(index)")
            let text = "message \(index)"
            XCTAssertEqual(
                chat.rows[index].body,
                index % 2 == 0 ? .asked(text: text, images: 0) : .replied(text: text))
        }

        let sending = ContinuousClock.now
        var longest = Duration.zero
        var last = sending
        var seen = 0
        chat.send(.send(text: "go"))
        try await until("the reply has ended") {
            // How long the screen waited between two looks at a growing
            // reply: a stutter shows here.
            let now = ContinuousClock.now
            if let reply = chat.reply, reply.text.count != seen {
                seen = reply.text.count
                longest = max(longest, now - last)
                last = now
            }
            return chat.rows.count == Self.rows + 2 && chat.reply == nil && !chat.status.working
        }
        let replied = ContinuousClock.now - sending
        XCTAssertEqual(chat.rows[Self.rows].body, .asked(text: "go", images: 0))
        XCTAssertEqual(
            chat.rows[Self.rows + 1].body,
            .replied(text: String(repeating: Self.part, count: Self.deltas)))

        print(
            "bridge: \(Self.rows) rows read in \(opened); a reply of \(Self.deltas) parts in \(replied)"
                + " (\(replied / Self.deltas) each); longest gap while it grew \(longest)")
    }

    /// Lets the main actor run what the core sent until `done` holds.
    private func until(
        _ what: String, within limit: Duration = .seconds(60), _ done: () -> Bool
    ) async throws {
        let deadline = ContinuousClock.now + limit
        while !done() {
            guard ContinuousClock.now < deadline else {
                XCTFail("timed out waiting until \(what)")
                return
            }
            try await Task.sleep(for: .milliseconds(1))
        }
    }

    private func startDaemon() throws -> String {
        let runfiles = try XCTUnwrap(
            ProcessInfo.processInfo.environment["TEST_SRCDIR"],
            "Run this with bazel test, which says where the rehearsal is.")
        let rehearsal = URL(fileURLWithPath: runfiles).appending(path: Binaries.rehearsal)
            .resolvingSymlinksInPath()
        // Socket paths are limited to about 100 bytes, so not under Bazel's
        // deep temporary folder.
        let root = URL(fileURLWithPath: "/tmp").appending(path: "humata2-\(UUID().uuidString.prefix(8))")
        scratch = root
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let socket = root.appending(path: "d.sock").path

        let process = Process()
        process.executableURL = rehearsal
        process.arguments = ["--socket", socket, "--rows", "\(Self.rows)", "--deltas", "\(Self.deltas)"]
        try process.run()
        daemon = process
        let deadline = Date().addingTimeInterval(10)
        while !FileManager.default.fileExists(atPath: socket) {
            guard Date() < deadline, process.isRunning else {
                throw NeverListened(socket: socket)
            }
            Thread.sleep(forTimeInterval: 0.02)
        }
        return socket
    }

    private struct NeverListened: Error {
        var socket: String
    }
}
