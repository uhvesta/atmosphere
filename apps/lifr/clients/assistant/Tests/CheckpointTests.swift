import Foundation
import LifrAssistantClient
import LifrCore
import XCTest

/// Going back and forward in a conversation, writing a branch from a point
/// gone back to, and forking one.
final class CheckpointTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_793_700_000)
    private var count = 0

    /// A conversation of "one" → reply, "two" → reply, "three" → reply.
    private func threeTurns() -> Conversation {
        var conversation = Conversation(id: UUID(), createdAt: start)
        for text in ["one", "two", "three"] {
            conversation.append(message(.person, text))
            conversation.append(message(.assistant, "re " + text))
        }
        return conversation
    }

    private func message(_ role: ChatMessage.Role, _ text: String) -> ChatMessage {
        count += 1
        return ChatMessage(id: UUID(), role: role, text: text, at: start.addingTimeInterval(Double(count)))
    }

    private func id(_ text: String, in conversation: Conversation) throws -> UUID {
        try XCTUnwrap((conversation.messages + conversation.hidden).first { $0.text == text }?.id, text)
    }

    func testGoingBackShowsUpToThereAndForwardComesBack() throws {
        var conversation = threeTurns()
        XCTAssertNil(conversation.forward)

        try conversation.go(to: XCTUnwrap(conversation.back))
        XCTAssertEqual(conversation.messages.map(\.text), ["one", "re one", "two", "re two"])

        try conversation.go(to: XCTUnwrap(conversation.back))
        XCTAssertEqual(conversation.messages.map(\.text), ["one", "re one"])
        XCTAssertNil(conversation.back)

        try conversation.go(to: XCTUnwrap(conversation.forward))
        XCTAssertEqual(conversation.messages.map(\.text), ["one", "re one", "two", "re two"])

        try conversation.go(to: XCTUnwrap(conversation.newest))
        XCTAssertEqual(conversation.messages.map(\.text), ["one", "re one", "two", "re two", "three", "re three"])
        XCTAssertNil(conversation.newest)
    }

    func testWritingAfterGoingBackStartsABranchAndKeepsTheOther() throws {
        var conversation = threeTurns()
        try conversation.go(to: id("re one", in: conversation))
        conversation.append(message(.person, "two again"))
        conversation.append(message(.assistant, "re two again"))

        XCTAssertEqual(conversation.messages.map(\.text), ["one", "re one", "two again", "re two again"])
        XCTAssertNil(conversation.forward, "The new branch is the latest")
        let branches = try conversation.branches(at: id("two again", in: conversation))
        XCTAssertEqual(branches.map(\.text), ["two", "two again"])

        let first = try XCTUnwrap(branches.first)
        conversation.go(to: conversation.newest(through: first.id))
        XCTAssertEqual(conversation.messages.map(\.text), ["one", "re one", "two", "re two", "three", "re three"])
    }

    func testTheLogHasEveryPointOnEveryBranch() throws {
        var conversation = threeTurns()
        try conversation.go(to: id("re one", in: conversation))
        conversation.append(message(.person, "two again"))
        let reply = message(.assistant, "re two again")
        conversation.append(reply)
        conversation.changes[reply.id] = [UUID(), UUID()]

        let log = conversation.checkpoints
        XCTAssertEqual(log.map(\.message.text), ["one", "two", "three", "two again"])
        XCTAssertEqual(log.map(\.isShown), [true, false, false, true])
        XCTAssertEqual(log.filter(\.isCurrent).map(\.message.text), ["two again"])
        XCTAssertEqual(log.map(\.changes), [0, 0, 0, 2])
        XCTAssertEqual(log.last?.end, reply.id)
    }

    func testGoingBackPastChangesNamesTheRepliesThatMadeThem() throws {
        var conversation = threeTurns()
        let changed = try id("re three", in: conversation)
        conversation.changes[changed] = [UUID()]
        conversation.changes[try id("re one", in: conversation)] = [UUID()]

        XCTAssertEqual(try conversation.changedReplies(hiddenGoingTo: id("re two", in: conversation)), [changed])
        XCTAssertEqual(try conversation.changedReplies(hiddenGoingTo: id("re three", in: conversation)), [])
    }

    func testAForkHasTheMessagesUpToThereButNotTheUndoOrDraft() throws {
        var conversation = threeTurns()
        conversation.isSynced = true
        conversation.draft = .init(tasks: [.init(title: "buy milk", place: "HOME")])
        let changed = try id("re one", in: conversation)
        conversation.changes[changed] = [UUID()]
        let forkID = UUID()

        let fork = try XCTUnwrap(conversation.fork(at: id("re two", in: conversation), as: forkID, now: start))

        XCTAssertEqual(fork.id, forkID)
        XCTAssertEqual(fork.messages.map(\.text), ["one", "re one", "two", "re two"])
        XCTAssertNotEqual(fork.title, conversation.title)
        XCTAssertTrue(fork.isSynced)
        XCTAssertNil(fork.draft)
        XCTAssertTrue(fork.changes.isEmpty)
        XCTAssertEqual(conversation.messages.count, 6, "The conversation forked from stays as it was")
        XCTAssertNil(conversation.fork(at: UUID(), as: UUID(), now: start))
    }

    func testAConversationKeptBeforeBranchesGoesBack() throws {
        var conversation = threeTurns()
        conversation.parents = [:]
        conversation.latest = [:]
        let data = try JSONEncoder().encode(conversation)
        var kept = try JSONDecoder().decode(Conversation.self, from: data)

        try kept.go(to: id("re one", in: kept))
        XCTAssertEqual(kept.messages.map(\.text), ["one", "re one"])
        try kept.go(to: XCTUnwrap(kept.newest))
        XCTAssertEqual(kept.messages.count, 6)
    }
}
