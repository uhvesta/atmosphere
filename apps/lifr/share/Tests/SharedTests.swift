import Foundation
import LifrCore
import XCTest

@testable import LifrShare

final class SharedTests: XCTestCase {
    private let url = URL(string: "https://www.example.com/article")

    func testAPagesTitleNamesItAndTheLinkGoesInTheDescription() {
        let shared = Shared(title: "  How to sleep  ", text: nil, url: url)
        XCTAssertEqual(shared, Shared(title: "How to sleep", details: "https://www.example.com/article"))
    }

    func testWithoutATitleTheTextsFirstLineNamesIt() {
        let shared = Shared(title: nil, text: "Buy milk\nand eggs", url: nil)
        XCTAssertEqual(shared, Shared(title: "Buy milk", details: "Buy milk\nand eggs"))
    }

    func testALinkAloneIsNamedByItsSiteAndPath() {
        let shared = Shared(title: nil, text: nil, url: url)
        XCTAssertEqual(shared, Shared(title: "example.com/article", details: "https://www.example.com/article"))
    }

    func testSelectedTextIsKeptWithTheLink() {
        let shared = Shared(title: "How to sleep", text: "Keep the room cool.", url: url)
        XCTAssertEqual(shared.details, "https://www.example.com/article\n\nKeep the room cool.")
    }

    func testNothingSharedMakesAnEmptyTitle() {
        XCTAssertEqual(Shared(title: " ", text: "", url: nil).title, "")
    }

    func testAPlanInSharedTextIsKeptForLifrToReview() {
        let reply = """
            Here's your plan:

            ```toml
            [[task]]
            title = "Book the van"
            in = "HOME"
            ```
            """
        XCTAssertEqual(Shared(title: nil, text: reply, url: nil).plan, reply)
    }

    func testTextThatIsntAPlanHoldsNone() {
        XCTAssertNil(Shared(title: nil, text: "Buy [[milk]]\nand eggs", url: nil).plan)
        XCTAssertNil(Shared(title: "How to sleep", text: nil, url: url).plan)
    }

    func testATitleNamingADayIsDueThatDayWithoutTheWords() throws {
        let today = try CalendarDate("2026-11-11")
        let entry = SharedEntry("Call the vet tomorrow", today: today)
        XCTAssertEqual(entry.title, "Call the vet")
        XCTAssertEqual(entry.due, try CalendarDate("2026-11-12"))
    }

    func testWordsASharedTaskCantCarryStayInTheTitle() throws {
        let entry = SharedEntry("Call the vet 3pm !high", today: try CalendarDate("2026-11-11"))
        XCTAssertEqual(entry.title, "Call the vet 3pm !high")
        XCTAssertNil(entry.due)
    }

    func testADayKeptAsTextStaysInTheTitle() throws {
        let today = try CalendarDate("2026-11-11")
        let key = try XCTUnwrap(SharedEntry("Read Tomorrow", today: today).dueKey)
        let entry = SharedEntry("Read Tomorrow", today: today, ignoring: [key])
        XCTAssertEqual(entry.title, "Read Tomorrow")
        XCTAssertNil(entry.due)
    }
}
