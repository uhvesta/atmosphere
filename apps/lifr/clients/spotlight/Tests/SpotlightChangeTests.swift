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
        XCTAssertEqual(change, SpotlightChange(replacesAll: false, removed: [first.identifier], indexed: [second]))
    }

    func testAProjectAndAnAvenueAreListedApartFromTasks() {
        let project = SpotlightItem(.project, id: first.id, title: "Move house", detail: "Project", details: "")
        let avenue = SpotlightItem(.avenue, id: UUID(), title: "Home", detail: "Avenue", details: "")
        let change = SpotlightChange(from: [first], to: [project, avenue])
        XCTAssertEqual(
            change, SpotlightChange(replacesAll: false, removed: [first.identifier], indexed: [project, avenue]))
    }

    func testAResultOpensWhatItNames() {
        for item in [first, SpotlightItem(.project, id: UUID(), title: "Move house", detail: "", details: "")] {
            let target = SpotlightItem.target(item.identifier)
            XCTAssertEqual(target?.kind, item.kind)
            XCTAssertEqual(target?.id, item.id)
        }
        XCTAssertNil(SpotlightItem.target("label/\(UUID().uuidString)"))
        XCTAssertNil(SpotlightItem.target("not an id"))
    }
}
