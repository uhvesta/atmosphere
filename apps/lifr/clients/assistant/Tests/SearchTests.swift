import Foundation
import LifrAssistantClient
import LifrCore
import XCTest

/// Finding conversations by what's in their names and messages, when they
/// were written, and words related to what's searched for.
final class SearchTests: XCTestCase {
    /// 2026-11-04 10:00 UTC, a Wednesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000 + 86_400)
    private let day: TimeInterval = 86_400

    private var calendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        return calendar
    }

    private func query(_ text: String) -> ConversationQuery {
        ConversationQuery(text, now: now, calendar: calendar)
    }

    private func date(_ text: String) throws -> Date {
        try XCTUnwrap(ISO8601DateFormatter().date(from: text + "T00:00:00Z"), text)
    }

    /// A conversation of `texts`, alternating the person and the
    /// assistant, each written a minute after the one before from `start`.
    private func conversation(_ texts: String..., at start: Date? = nil) -> Conversation {
        let start = start ?? now.addingTimeInterval(-3600)
        var conversation = Conversation(id: UUID(), createdAt: start)
        for (index, text) in texts.enumerated() {
            conversation.append(
                ChatMessage(
                    id: UUID(), role: index.isMultiple(of: 2) ? .person : .assistant, text: text,
                    at: start.addingTimeInterval(Double(index) * 60)))
        }
        return conversation
    }

    // MARK: - Reading the query

    func testAQueryWithoutATimeIsAllWords() {
        let plain = query("buy milk")

        XCTAssertEqual(plain.words, ["buy", "milk"])
        XCTAssertNil(plain.during)
    }

    func testDaysAreReadFromTodayYesterdayAndWeekdays() throws {
        XCTAssertEqual(query("today").during?.start, try date("2026-11-04"))
        XCTAssertEqual(query("milk yesterday").during?.start, try date("2026-11-03"))
        XCTAssertEqual(query("on Monday").during?.start, try date("2026-11-02"))
        XCTAssertEqual(query("wednesday").during?.start, try date("2026-11-04"), "Today, being Wednesday")
        XCTAssertEqual(query("last wednesday").during?.start, try date("2026-10-28"))
        XCTAssertEqual(query("3 days ago").during?.start, try date("2026-11-01"))
        XCTAssertEqual(query("on Monday").during?.duration, day)
    }

    func testWeeksMonthsAndYearsAreReadWhole() throws {
        let lastWeek = try XCTUnwrap(query("plumber last week").during)
        XCTAssertLessThan(lastWeek.start, try date("2026-11-01"))
        XCTAssertEqual(lastWeek.duration, 7 * day)
        XCTAssertEqual(query("this month").during?.start, try date("2026-11-01"))
        XCTAssertEqual(query("in October").during?.start, try date("2026-10-01"))
        XCTAssertEqual(query("december").during?.start, try date("2025-12-01"), "The last December before now")
        XCTAssertEqual(query("last year").during?.start, try date("2025-01-01"))
    }

    func testTheWordsAroundATimeAreLeftOut() {
        XCTAssertEqual(query("trip plans from last week").words, ["trip", "plans"])
        XCTAssertEqual(query("garden in October").words, ["garden"])
        XCTAssertEqual(query("I may call").words, ["I", "may", "call"], "May without “in” is a word")
        XCTAssertNil(query("I may call").during)
        XCTAssertNotNil(query("in may").during)
    }

    // MARK: - Matching

    func testAConversationMatchesItsNameIgnoringCaseAndAccents() {
        var named = conversation("Hello")
        named.name = "Café plans"

        XCTAssertNotNil(named.match(query("cafe")))
        XCTAssertNil(named.match(query("cafe"))?.message)
        XCTAssertNil(named.match(query("tea")))
        XCTAssertNil(named.match(query("  ")))
    }

    func testAConversationMatchesTheNewestMessageWithEveryWord() {
        let chat = conversation("Buy milk", "Added buy milk.", "And eggs", "Added eggs and milk.")

        XCTAssertEqual(chat.match(query("MILK"))?.message?.text, "Added eggs and milk.")
        XCTAssertEqual(chat.match(query("buy milk"))?.message?.text, "Added buy milk.")
        XCTAssertEqual(chat.match(query("milk buy"))?.message?.text, "Added buy milk.", "In any order")
        XCTAssertNil(chat.match(query("buy bread")))
    }

    func testAMessageOnABranchNotShownMatchesToo() throws {
        var chat = conversation("Plan the trip", "Sure.", "Book a hotel", "Booked.")
        let sure = try XCTUnwrap(chat.messages.first { $0.text == "Sure." })
        chat.go(to: sure.id)

        XCTAssertEqual(chat.match(query("hotel"))?.message?.text, "Book a hotel")
    }

    func testATimeFindsOnlyMessagesWrittenThen() throws {
        let old = conversation("Call the plumber", "Done.", at: now.addingTimeInterval(-10 * day))
        let recent = conversation("Call the plumber again", "Done.")

        XCTAssertNil(old.match(query("plumber today")))
        XCTAssertEqual(recent.match(query("plumber today"))?.message?.text, "Call the plumber again")
        XCTAssertEqual(recent.match(query("today"))?.message?.text, "Done.", "Only a time: the newest from then")
        XCTAssertNil(old.match(query("today")))
    }

    func testRelatedWordsFindWholeWordsOnlyWhenTheWordsTypedDont() throws {
        let shopping = conversation("Add a task to buy eggs and bread", "Added.")
        let other = conversation("Eggplant recipes", "Here are some.")

        let match = try XCTUnwrap(shopping.match(query("groceries"), related: ["egg", "supermarket"]))
        XCTAssertTrue(match.isRelated)
        XCTAssertEqual(match.message?.text, "Add a task to buy eggs and bread")
        XCTAssertNil(other.match(query("groceries"), related: ["egg"]), "Eggplant isn't an egg")
        XCTAssertNil(shopping.match(query("groceries")))
        let typed = try XCTUnwrap(shopping.match(query("bread"), related: ["egg"]))
        XCTAssertFalse(typed.isRelated)
    }

    func testRelatedPhrasesMatchTheirWordsTogether() {
        let bills = conversation("Remind me to pay the electricity bill", "Done.")

        XCTAssertNotNil(bills.match(query("utilities"), related: ["electricity bill"]))
        XCTAssertNil(bills.match(query("utilities"), related: ["water bill"]))
    }
}
