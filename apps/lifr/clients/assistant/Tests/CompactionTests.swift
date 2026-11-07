import Foundation
import LifrAssistantClient
import LifrCore
import LifrModelsClient
import XCTest

final class CompactionTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func messages(_ texts: [String]) -> [ChatMessage] {
        texts.enumerated().map { index, text in
            ChatMessage(id: UUID(), role: index % 2 == 0 ? .person : .assistant, text: text, at: now)
        }
    }

    private func request(_ messages: [ChatMessage], summary: String? = nil) -> AssistantRequest {
        var request = AssistantRequest(messages: messages, workspace: Workspace(), now: now, timeZone: .gmt)
        request.summary = summary
        return request
    }

    private func conversation(_ count: Int) -> Conversation {
        var conversation = Conversation(id: UUID(), createdAt: now)
        for message in messages((0..<count).map { "Message \($0)" }) { conversation.append(message) }
        return conversation
    }

    func testALongConversationSaysHowManyMessagesItLeftOut() {
        let long = messages((0..<30).map { String(repeating: "Message \($0) ", count: 10) })

        XCTAssertGreaterThan(AssistantPrompt.leftOut(request(long), size: .full), 0)
        XCTAssertEqual(AssistantPrompt.leftOut(request(Array(long.suffix(2))), size: .full), 0)
    }

    func testTheSummaryIsSentAheadOfTheMessagesAfterIt() {
        let prompt = AssistantPrompt.prompt(request(messages(["Thanks", "Done"]), summary: "They planned the week."))

        XCTAssertTrue(prompt.contains("They planned the week."))
        let summary = prompt.range(of: "They planned the week.")
        let thanks = prompt.range(of: "Thanks")
        XCTAssertLessThan(try XCTUnwrap(summary).lowerBound, try XCTUnwrap(thanks).lowerBound)
    }

    func testTheSmallestSizeSendsNoSummary() {
        let prompt = AssistantPrompt.prompt(
            request(messages(["Thanks"]), summary: "They planned the week."), size: .smallest)

        XCTAssertFalse(prompt.contains("They planned the week."))
    }

    func testASummaryTakesRoomFromTheConversationItFollows() {
        let long = messages((0..<30).map { String(repeating: "Message \($0) ", count: 10) })
        let summary = String(repeating: "Earlier. ", count: 60)

        XCTAssertGreaterThan(
            AssistantPrompt.leftOut(request(long, summary: summary), size: .full),
            AssistantPrompt.leftOut(request(long), size: .full))
    }

    func testALongSummaryIsCutAtTheEndOfASentence() throws {
        let answer = String(repeating: "They planned the week together. ", count: 40)

        let summary = try XCTUnwrap(AssistantPrompt.summary(from: answer))

        XCTAssertLessThanOrEqual(summary.count, AssistantPrompt.summaryLimit)
        XCTAssertTrue(summary.hasSuffix("."))
        XCTAssertNil(AssistantPrompt.summary(from: "  \n"))
    }

    func testWhatsSummarizedFitsTheModelAndIncludesTheEarlierSummary() {
        let long = messages((0..<200).map { String(repeating: "Message \($0) ", count: 10) })
        let budget = AssistantPrompt.summaryInputBudget(for: .appleOnDevice)

        let prompt = AssistantPrompt.summaryPrompt(long, earlier: "They planned the week.", budget: budget)

        let instructions = AssistantPrompt.estimatedTokens(AssistantPrompt.summaryInstructions)
        XCTAssertLessThanOrEqual(
            AssistantPrompt.estimatedTokens(prompt) + instructions, ContextBudget.appleOnDevice.room)
        XCTAssertTrue(prompt.contains("They planned the week."))
        XCTAssertTrue(prompt.contains("Message 199"))
    }

    func testCompactingLeavesTheLatestExchangeAsWritten() throws {
        var conversation = conversation(6)

        XCTAssertEqual(conversation.compactable.map(\.text), (0..<4).map { "Message \($0)" })
        let through = try XCTUnwrap(conversation.compactable.last?.id)
        conversation.summaries[through] = ChatSummary(text: "Short", at: now)

        XCTAssertEqual(conversation.summary?.through, through)
        XCTAssertEqual(conversation.unsummarized.map(\.text), ["Message 4", "Message 5"])
        XCTAssertTrue(conversation.compactable.isEmpty)
    }

    func testASummaryDoesntApplyOnABranchFromBeforeIt() throws {
        var conversation = conversation(6)
        let through = try XCTUnwrap(conversation.compactable.last?.id)
        conversation.summaries[through] = ChatSummary(text: "Short", at: now)
        let second = conversation.messages[1].id

        conversation.go(to: second)

        XCTAssertNil(conversation.summary)
        XCTAssertEqual(conversation.unsummarized.count, 2)
    }

    func testAForkKeepsTheSummariesOfWhatItCopies() throws {
        var conversation = conversation(6)
        let through = try XCTUnwrap(conversation.compactable.last?.id)
        conversation.summaries[through] = ChatSummary(text: "Short", at: now)

        let fork = try XCTUnwrap(conversation.fork(at: conversation.messages[5].id, as: UUID(), now: now))
        let early = try XCTUnwrap(conversation.fork(at: conversation.messages[1].id, as: UUID(), now: now))

        XCTAssertEqual(fork.summary?.summary.text, "Short")
        XCTAssertTrue(early.summaries.isEmpty)
    }

    func testSummariesAreKeptAndAConversationSavedBeforeThemReadsWithNone() throws {
        var conversation = conversation(4)
        let through = try XCTUnwrap(conversation.compactable.last?.id)
        conversation.summaries[through] = ChatSummary(text: "Short", at: now)

        let decoded = try JSONDecoder().decode(Conversation.self, from: JSONEncoder().encode(conversation))
        XCTAssertEqual(decoded, conversation)

        var old = try XCTUnwrap(
            JSONSerialization.jsonObject(with: JSONEncoder().encode(conversation)) as? [String: Any])
        old["summaries"] = nil
        let earlier = try JSONDecoder().decode(Conversation.self, from: JSONSerialization.data(withJSONObject: old))
        XCTAssertTrue(earlier.summaries.isEmpty)
    }
}
