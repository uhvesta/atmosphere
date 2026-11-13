import Foundation
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
}
