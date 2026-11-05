import Foundation
import XCTest

@testable import LifrBugReport

final class RedactorTests: XCTestCase {
    func testEachTextBecomesAPlaceholderTheSameTextTheSameOne() {
        var redactor = Redactor()

        let redacted = redactor.redact(#"Task(title: "Buy milk", notes: "", tag: "Buy milk", other: "Call Sam")"#)

        XCTAssertEqual(redacted, #"Task(title: "‹text 1›", notes: "", tag: "‹text 1›", other: "‹text 2›")"#)
    }

    func testEscapedQuotesAndRawAndMultilineTextAreReplacedWhole() {
        var redactor = Redactor()

        let redacted = redactor.redact(
            """
            a: "say \\"hi\\" now"
            b: #"raw "quoted" text"#
            c: \"\"\"
              line one
              line "two"
              \"\"\"
            """)

        XCTAssertFalse(redacted.contains("hi"))
        XCTAssertFalse(redacted.contains("quoted"))
        XCTAssertFalse(redacted.contains("line"))
        XCTAssertTrue(redacted.hasPrefix("a: "))
        XCTAssertTrue(redacted.contains("c: "))
    }

    func testAnUnfinishedTextIsDroppedToTheEndOfItsLine() {
        var redactor = Redactor()

        let redacted = redactor.redact("title: \"Buy mi\n… 40 more characters")

        XCTAssertFalse(redacted.contains("Buy"))
        XCTAssertTrue(redacted.hasSuffix("\n… 40 more characters"))
    }

    func testAReportWithoutPersonalTextKeepsWhatThePersonWroteAndDropsScreenshots() {
        let report = BugReport.sample(includesPersonal: false)

        let kept = Redactor.redact(report)

        XCTAssertEqual(kept.happened, report.happened)
        XCTAssertEqual(kept.expected, report.expected)
        XCTAssertEqual(kept.recording?.screenshots, [])
        XCTAssertEqual(kept.recording?.steps.count, report.recording?.steps.count)
        XCTAssertFalse(kept.text.contains("Buy milk"))
        XCTAssertFalse(kept.recording?.start.contains("Buy milk") ?? true)
    }

    func testAReportWithPersonalTextIsKeptAsItIs() {
        let report = BugReport.sample(includesPersonal: true)

        XCTAssertEqual(Redactor.redact(report), report)
    }
}

extension BugReport {
    static func sample(includesPersonal: Bool, id: UUID = UUID(0), at date: Date = Date(timeIntervalSince1970: 0))
        -> BugReport
    {
        BugReport(
            id: id, createdAt: date, happened: "The task vanished", expected: "It stays",
            includesPersonal: includesPersonal,
            device: DeviceInfo(
                model: "iPhone18,1", system: "iOS 27.0.0", appVersion: "1.0 (1)", language: "en-US",
                timeZone: "UTC", assistantModel: "gemma-4-e2b"),
            recording: Recording(
                startedAt: date, start: #"State(tasks: ["Buy milk"])"#,
                steps: [
                    Step(
                        offset: 1, action: #"Action.addTapped("Buy milk")"#,
                        changes: "- tasks: []\n+ tasks: [\"Buy milk\"]")
                ],
                screenshots: [Screenshot(offset: 1.4, afterSteps: 1, image: Data([1, 2, 3]))]))
    }
}
