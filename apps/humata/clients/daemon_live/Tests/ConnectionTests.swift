import Foundation
import HumataDaemonClient
import HumataDaemonLive
import HumataProto
import XCTest

/// The socket client against a stand-in daemon that reads and answers only
/// when told to: what a caller sees when its task is cancelled, and what
/// reaches the daemon, in which order, around a frame too large for the
/// socket to take at once.
final class ConnectionTests: XCTestCase {
    private struct TookTooLong: Error {}

    /// Well over what a Unix socket holds, so its write waits for the reader.
    private static let large = Data(count: 8 * 1024 * 1024)

    func testAReadCancelledBehindALargeMessageReturnsAtOnceAndIsNeverSent() async throws {
        let (daemon, client) = try await connected()

        let image = Self.image
        let big = Task { try await client.sendImages("s", "look", [image], .enqueue) }
        // The large frame has begun and can't finish: nothing reads it yet.
        let length = try await daemon.nextLength()

        let read = Task { try await client.sessions(nil) }
        read.cancel()
        let result = try await within { await read.result }
        XCTAssertThrowsError(try result.get()) { XCTAssertTrue($0 is CancellationError, "\($0)") }

        let after = Task { try await client.send("s", "after", .enqueue) }
        let first = try await daemon.body(length)
        XCTAssertEqual(Self.message(first)?.images.first?.data.count, Self.large.count)
        let second = try await daemon.next()
        XCTAssertEqual(Self.message(second)?.text, "after")

        try await daemon.answer(first.id, .messageSent(Humata_V1_MessageSent()))
        try await daemon.answer(second.id, .messageSent(Humata_V1_MessageSent()))
        try await within { try await big.value }
        try await within { try await after.value }
    }

    func testAMessageSentBehindALargeOneArrivesWholeAfterItAndIsAnsweredFirst() async throws {
        let (daemon, client) = try await connected()

        let image = Self.image
        let big = Task { try await client.sendImages("s", "look", [image], .enqueue) }
        let length = try await daemon.nextLength()
        let small = Task { try await client.send("s", "small", .enqueue) }

        let first = try await daemon.body(length)
        XCTAssertEqual(Self.message(first)?.text, "look")
        XCTAssertEqual(Self.message(first)?.images.first?.data.count, Self.large.count)
        let second = try await daemon.next()
        XCTAssertEqual(Self.message(second)?.text, "small")

        // Answers come back by request, not by order.
        try await daemon.answer(second.id, .messageSent(Humata_V1_MessageSent()))
        try await within { try await small.value }
        try await daemon.answer(first.id, .messageSent(Humata_V1_MessageSent()))
        try await within { try await big.value }
    }

    func testMessagesSentOneAfterAnotherArriveInThatOrder() async throws {
        let (daemon, client) = try await connected()
        let texts = (1...20).map { "message \($0)" }

        let sending = Task {
            for text in texts { try await client.send("s", text, .enqueue) }
        }
        var arrived: [String] = []
        for _ in texts {
            let envelope = try await daemon.next()
            arrived.append(Self.message(envelope)?.text ?? "")
            try await daemon.answer(envelope.id, .messageSent(Humata_V1_MessageSent()))
        }
        try await within { try await sending.value }
        XCTAssertEqual(arrived, texts)
    }

    func testACancelledReadsLateAnswerIsDroppedAndTheNextCallIsAnswered() async throws {
        let (daemon, client) = try await connected()

        let read = Task { try await client.sessions(nil) }
        // The daemon has it, so the cancel comes while its answer is awaited.
        let asked = try await daemon.next()
        read.cancel()
        let result = try await within { await read.result }
        XCTAssertThrowsError(try result.get()) { XCTAssertTrue($0 is CancellationError, "\($0)") }

        try await daemon.answer(asked.id, .sessions(Humata_V1_Sessions()))
        let send = Task { try await client.send("s", "still here", .enqueue) }
        let next = try await daemon.next()
        XCTAssertEqual(Self.message(next)?.text, "still here")
        try await daemon.answer(next.id, .messageSent(Humata_V1_MessageSent()))
        try await within { try await send.value }
    }

    func testAReadWhoseTaskIsNotCancelledGetsItsAnswer() async throws {
        let (daemon, client) = try await connected()

        let read = Task { try await client.sessions(nil) }
        let asked = try await daemon.next()
        try await daemon.answer(asked.id, .sessions(Humata_V1_Sessions()))
        let sessions = try await within { try await read.value }
        XCTAssertEqual(sessions, [])
    }

    func testAChangeWhoseTaskIsCancelledStillWaitsForItsAnswer() async throws {
        let (daemon, client) = try await connected()

        let send = Task { try await client.send("s", "do it", .enqueue) }
        let asked = try await daemon.next()
        send.cancel()
        try await daemon.answer(asked.id, .messageSent(Humata_V1_MessageSent()))
        // No error: the caller learns it was done.
        try await within { try await send.value }
    }

    func testAChangeCancelledBeforeItIsSentStillReachesTheDaemon() async throws {
        let (daemon, client) = try await connected()

        let send = Task {
            // Cancelled from the start, as an effect torn down mid-way is.
            withUnsafeCurrentTask { $0?.cancel() }
            try await client.send("s", "do it", .enqueue)
        }
        let asked = try await daemon.next()
        XCTAssertEqual(Self.message(asked)?.text, "do it")
        try await daemon.answer(asked.id, .messageSent(Humata_V1_MessageSent()))
        try await within { try await send.value }
    }

    func testACallAfterTheDaemonWentAwayFailsInsteadOfWaiting() async throws {
        let (daemon, client) = try await connected()
        try await daemon.stop()

        let send = Task { try await client.send("s", "anyone?", .enqueue) }
        let result = try await within { await send.result }
        XCTAssertThrowsError(try result.get()) { XCTAssertTrue($0 is DaemonError, "\($0)") }
    }

    // MARK: -

    private static var image: ImageAttachment { ImageAttachment(mediaType: "image/png", data: large) }

    private static func message(_ envelope: Humata_V1_Envelope) -> Humata_V1_SendMessage? {
        guard case .request(let request) = envelope.body, case .sendMessage(let message) = request.kind else {
            return nil
        }
        return message
    }

    /// A client connected to a stand-in daemon, after one message has gone
    /// there and back.
    private func connected() async throws -> (FakeDaemon, DaemonClient) {
        let daemon = try FakeDaemon()
        let client = DaemonClient.live(socket: daemon.path)
        let hello = Task { try await client.send("s", "hello", .enqueue) }
        try await daemon.accept()
        let first = try await daemon.next()
        try await daemon.answer(first.id, .messageSent(Humata_V1_MessageSent()))
        try await within { try await hello.value }
        return (daemon, client)
    }

    /// `work`'s result, or `TookTooLong` if it hasn't one in ten seconds:
    /// a call that should have returned and didn't fails the test here
    /// instead of holding it until the runner gives up.
    private func within<T: Sendable>(_ work: @escaping @Sendable () async throws -> T) async throws -> T {
        let (results, continuation) = AsyncThrowingStream<T, any Error>.makeStream()
        let working = Task {
            do {
                continuation.yield(try await work())
                continuation.finish()
            } catch {
                continuation.finish(throwing: error)
            }
        }
        let timer = Task {
            try? await Task.sleep(for: .seconds(10))
            continuation.finish(throwing: TookTooLong())
        }
        defer {
            working.cancel()
            timer.cancel()
        }
        for try await result in results { return result }
        throw TookTooLong()
    }
}
