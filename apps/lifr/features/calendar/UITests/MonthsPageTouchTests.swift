import XCTest

/// Taps on the months page with a finger, not an accessibility action.
@MainActor
final class MonthsPageTouchTests: XCTestCase {
    private let app = XCUIApplication()

    override func setUp() async throws {
        continueAfterFailure = false
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
    }

    private func day(_ label: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", label)).firstMatch
    }

    /// Whether the calendar is on screen again, not just under the months page.
    private func calendarIsBack() -> Bool {
        let previous = app.buttons["Previous Month"]
        let hittable = NSPredicate(format: "hittable == true")
        let waiter = XCTWaiter.wait(for: [expectation(for: hittable, evaluatedWith: previous)], timeout: 5)
        return waiter == .completed
    }

    private func openMonths() {
        app.navigationBars.buttons["All Months"].tap()
        XCTAssertTrue(app.navigationBars["Months"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["Previous Month"].isHittable)
    }

    func testTappingADayInAnotherMonthGoesBackToTheCalendarOnIt() {
        openMonths()
        let december = day("Fri, Dec 4")
        if !december.isHittable { app.swipeUp() }
        XCTAssertTrue(december.isHittable)

        december.tap()

        XCTAssertTrue(calendarIsBack())
        XCTAssertTrue(day("Fri, Dec 4").isSelected)
    }

    func testTappingADayInTheOpenMonthPicksIt() {
        openMonths()
        let day = day("Fri, Nov 20")
        XCTAssertTrue(day.waitForExistence(timeout: 5))

        day.tap()

        XCTAssertTrue(calendarIsBack())
        XCTAssertTrue(self.day("Fri, Nov 20").isSelected)
        XCTAssertFalse(self.day("Tue, Nov 3").isSelected)
    }

    func testTappingAMonthsNameGoesBackToItsFirstDay() {
        openMonths()
        let january = app.buttons["January 2027"]
        for _ in 0..<6 where !january.isHittable { app.swipeUp() }

        january.tap()

        XCTAssertTrue(calendarIsBack())
        XCTAssertTrue(day("Fri, Jan 1").isSelected)
    }
}
