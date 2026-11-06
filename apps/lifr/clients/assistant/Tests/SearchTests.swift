import Foundation
import LifrAssistantClient
import LifrCore
import XCTest

/// Finding conversations by what's in their names and messages.
final class SearchTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_793_700_000)

    private func conversation(_ texts: String...) -> Conversation {
        var conversation = Conversation(id: UUID(), createdAt: start)
        for (index, text) in texts.enumerated() {
            conversation.append(
                ChatMessage(
                    id: UUID(), role: index.isMultiple(of: 2) ? .person : .assistant, text: text,
                    at: start.addingTimeInterval(Double(index))))
        }
        return conversation
    }

    func testAConversationMatchesItsNameIgnoringCaseAndAccents() {
        var named = conversation("Hello")
        named.name = "Café plans"

        XCTAssertNotNil(named.match("cafe"))
        XCTAssertNil(named.match("cafe")?.message)
        XCTAssertNil(named.match("tea"))
        XCTAssertNil(named.match("  "))
    }

    func testAConversationMatchesTheNewestMessageThatHasIt() throws {
        let chat = conversation("Buy milk", "Added buy milk.", "And eggs", "Added.")

        XCTAssertEqual(chat.match("MILK")?.message?.text, "Added buy milk.")
        XCTAssertEqual(chat.match("eggs")?.message?.text, "And eggs")
    }

    func testAMessageOnABranchNotShownMatchesToo() throws {
        var chat = conversation("Plan the trip", "Sure.", "Book a hotel", "Booked.")
        let sure = try XCTUnwrap(chat.messages.first { $0.text == "Sure." })
        chat.go(to: sure.id)
        chat.name = "Trip"

        XCTAssertEqual(chat.match("hotel")?.message?.text, "Book a hotel")
    }
}
