import Foundation
import LifrCore
import LifrUI
import XCTest

/// How a repeat reads on screen, in the person's language and names, while
/// the stored form stays the same.
final class RecurrenceTitleTests: XCTestCase {
    private func recurrence(_ rule: Recurrence.Rule, every interval: Int = 1) -> Recurrence {
        Recurrence(rule: rule, interval: interval, seriesID: UUID(), index: 0)
    }

    func testWeekdaysAreNamedInTheLocalesLanguage() {
        let weekly = recurrence(.weekly(weekdays: [2, 5]), every: 2)

        let german = weekly.title(Locale(identifier: "de_DE"))
        let english = weekly.title(Locale(identifier: "en_US"))

        XCTAssertTrue(german.contains("Mo") && german.contains("Do"))
        XCTAssertTrue(english.contains("Mon") && english.contains("Thu"))
        XCTAssertTrue(english.contains("2"))
        XCTAssertEqual(weekly.description, "Every 2 weeks on Mon, Thu")
    }

    func testEveryRuleReadsWithItsNumbers() {
        let english = Locale(identifier: "en_US")
        let rules: [(Recurrence.Rule, [String])] = [
            (.daily, ["3"]),
            (.monthly(day: 15), ["3", "15"]),
            (.monthlyWeekday(ordinal: -1, weekday: 6), ["3", "Fri"]),
            // February 29 has a date to name.
            (.yearly(month: 2, day: 29), ["3", "Feb", "29"]),
            (.afterClosing(.week), ["3"]),
        ]
        for (rule, words) in rules {
            let title = recurrence(rule, every: 3).title(english)
            for word in words { XCTAssertTrue(title.contains(word), "\(title) lacks \(word)") }
            // Once is just "every", except after closing.
            if case .afterClosing = rule { continue }
            XCTAssertFalse(recurrence(rule).title(english).contains("1 "), recurrence(rule).title(english))
        }
    }
}
