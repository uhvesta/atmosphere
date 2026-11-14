import HumataDaemonClient
import XCTest

final class UnansweredSendsTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000)

    func testTheSameTextSentAgainSoonGoesWithTheSameID() {
        let sends = UnansweredSends()
        let first = sends.id(session: "s1", text: "continue", at: start)

        let again = sends.id(session: "s1", text: "continue", at: start + UnansweredSends.window / 2)

        XCTAssertEqual(again, first)
        // Other words, or the same to another session, are another message.
        XCTAssertNotEqual(sends.id(session: "s1", text: "stop", at: start + 1), first)
        XCTAssertNotEqual(sends.id(session: "s2", text: "continue", at: start + 1), first)
    }

    func testTheSameTextSentLaterIsANewMessage() {
        let sends = UnansweredSends()
        let first = sends.id(session: "s1", text: "continue", at: start)

        let later = sends.id(session: "s1", text: "continue", at: start + UnansweredSends.window + 1)

        XCTAssertNotEqual(later, first)
    }

    func testAnAnsweredSendForgetsTheSessionsEarlierOnes() {
        let sends = UnansweredSends()
        let first = sends.id(session: "s1", text: "continue", at: start)
        let other = sends.id(session: "s2", text: "continue", at: start)

        // Other words sent to the session were answered.
        _ = sends.id(session: "s1", text: "run the tests", at: start + 1)
        sends.answered(session: "s1")

        XCTAssertNotEqual(sends.id(session: "s1", text: "continue", at: start + 2), first)
        XCTAssertEqual(sends.id(session: "s2", text: "continue", at: start + 2), other)
    }
}
