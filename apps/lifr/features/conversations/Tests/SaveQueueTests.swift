import ComposableArchitecture
import Foundation
import LifrConversations
import XCTest

final class SaveQueueTests: XCTestCase {
    func testSavesOfAChatWaitingBehindOneRunningLeaveOnlyTheNewest() async throws {
        let queue = SaveQueue()
        let chat = UUID()
        let saved = LockIsolated<[Int]>([])
        let (started, start) = AsyncStream<Void>.makeStream()
        let (gate, open) = AsyncStream<Void>.makeStream()
        let first = queue.enqueue(saving: chat) {
            start.yield()
            for await _ in gate { break }
            saved.withValue { $0.append(1) }
        }
        for await _ in started { break }

        let second = queue.enqueue(saving: chat) { saved.withValue { $0.append(2) } }
        let third = queue.enqueue(saving: chat) { saved.withValue { $0.append(3) } }
        open.yield()
        try await first.value
        try await second.value
        try await third.value

        XCTAssertEqual(saved.value, [1, 3])
    }

    func testSavesOfOtherChatsAndOtherChangesAllRunInOrder() async throws {
        let queue = SaveQueue()
        let saved = LockIsolated<[Int]>([])
        let (started, start) = AsyncStream<Void>.makeStream()
        let (gate, open) = AsyncStream<Void>.makeStream()
        let first = queue.enqueue(saving: UUID()) {
            start.yield()
            for await _ in gate { break }
            saved.withValue { $0.append(1) }
        }
        for await _ in started { break }

        let other = queue.enqueue(saving: UUID()) { saved.withValue { $0.append(2) } }
        let delete = queue.enqueue { saved.withValue { $0.append(3) } }
        let again = queue.enqueue { saved.withValue { $0.append(4) } }
        open.yield()
        for task in [first, other, delete, again] { try await task.value }

        XCTAssertEqual(saved.value, [1, 2, 3, 4])
    }
}
