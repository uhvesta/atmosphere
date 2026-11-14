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

    func testTheWindowsLeastWidthMakesRoomForAnOpenSideChat() {
        let alone = WindowLeast(sideChat: nil)
        let beside = WindowLeast(sideChat: 440)
        let wider = WindowLeast(sideChat: 700)

        // The session keeps what it had alone, or more, and the window and
        // its column grow by at least the side chat's width.
        XCTAssertGreaterThanOrEqual(beside.session, alone.session)
        XCTAssertGreaterThanOrEqual(beside.detail, beside.session + 440)
        XCTAssertGreaterThan(beside.window, beside.detail)
        XCTAssertGreaterThan(beside.window, alone.window)
        XCTAssertEqual(wider.window - beside.window, 260, accuracy: 0.5)
        XCTAssertEqual(wider.session, beside.session)

        // A kept width outside what the side chat can be dragged to is held
        // to it, as the side chat holds it.
        XCTAssertEqual(WindowLeast(sideChat: 10_000).window, WindowLeast(sideChat: 900).window)
        XCTAssertEqual(WindowLeast(sideChat: 0).window, WindowLeast(sideChat: 360).window)
    }
}
