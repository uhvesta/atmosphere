import AppKit
import HumataAppFeature
import SwiftUI
import XCTest

@MainActor
final class AtLeastTests: XCTestCase {
    func testAtLeastIsItsLeastUnlessToldToAskItsView() {
        // Around a view that can't be under 900 points wide.
        let offered = CGSize(width: 400, height: 400)
        let least = NSHostingController(
            rootView: AtLeast(width: 360, height: 300) { Color.clear.frame(width: 900) })
        XCTAssertEqual(least.sizeThatFits(in: offered).width, 400, accuracy: 0.5, "what it's offered, unasked")

        let fitting = NSHostingController(
            rootView: AtLeast(width: 360, height: 300, fits: true) { Color.clear.frame(width: 900) })
        XCTAssertEqual(fitting.sizeThatFits(in: offered).width, 900, accuracy: 0.5, "what its view needs")

        // A view that fits in the least changes nothing.
        let narrow = NSHostingController(
            rootView: AtLeast(width: 360, height: 300, fits: true) { Color.clear.frame(width: 200) })
        XCTAssertEqual(narrow.sizeThatFits(in: offered).width, 400, accuracy: 0.5)
    }
}
