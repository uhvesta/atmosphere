import Foundation
import XCTest

@testable import LifrSpotlightClient

final class SpotlightChangeTests: XCTestCase {
    private let first = SpotlightItem(id: UUID(), title: "Book the van", detail: "HOME-0001 · Home", details: "")
    private let second = SpotlightItem(id: UUID(), title: "Pay rent", detail: "HOME-0002 · Home", details: "")

    func testTheFirstListReplacesEverything() {
        let change = SpotlightChange(from: nil, to: [first, second])
        XCTAssertEqual(change, SpotlightChange(replacesAll: true, removed: [], indexed: [first, second]))
    }

    func testAnEmptyListTakesEverythingOut() {
        let change = SpotlightChange(from: [first], to: [])
        XCTAssertEqual(change, SpotlightChange(replacesAll: true, removed: [], indexed: []))
    }

    func testARenameSendsOnlyThatTask() {
        var renamed = first
        renamed.title = "Book the bigger van"
        let change = SpotlightChange(from: [first, second], to: [renamed, second])
        XCTAssertEqual(change, SpotlightChange(replacesAll: false, removed: [], indexed: [renamed]))
    }

    func testAddingAndRemovingSendsOnlyThose() {
        let change = SpotlightChange(from: [first], to: [second])
        XCTAssertEqual(change, SpotlightChange(replacesAll: false, removed: [first.id], indexed: [second]))
    }
}
