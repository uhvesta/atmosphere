import Foundation
import LifrAPI
import LifrCore
import XCTest

/// `lifr add --read-dates` reads dates and priority from the title the
/// way Quick Add does; without it the title stays as typed.
final class AddDatesTests: XCTestCase {
    func testReadDatesTakesTheDueDateAndPriorityOutOfTheTitle() throws {
        let connection = try FixtureConnection()
        let run = connection.run("add", "--avenue", "HEALTH", "--read-dates", "Call", "mum", "tomorrow", "!high")
        XCTAssertEqual(run.status, 0, run.error)
        let sent = try XCTUnwrap(connection.requests.last)
        let tomorrow = CalendarDate(Date(), in: .current).adding(days: 1)
        XCTAssertEqual(sent["title"], .string("Call mum"))
        XCTAssertEqual(sent["dueDate"], .string(tomorrow.description))
        XCTAssertEqual(sent["priority"], .string("high"))
    }

    func testReadDatesKeepsFieldsGivenAsOptions() throws {
        let connection = try FixtureConnection()
        let run = connection.run("add", "--avenue", "HEALTH", "--due", "2026-12-01", "--read-dates", "Run", "tomorrow")
        XCTAssertEqual(run.status, 0, run.error)
        let sent = try XCTUnwrap(connection.requests.last)
        XCTAssertEqual(sent["title"], .string("Run"))
        XCTAssertEqual(sent["dueDate"], .string("2026-12-01"))
    }

    func testWithoutReadDatesTheTitleStaysAsTyped() throws {
        let connection = try FixtureConnection()
        let run = connection.run("add", "--avenue", "HEALTH", "Call", "mum", "tomorrow", "!high")
        XCTAssertEqual(run.status, 0, run.error)
        let sent = try XCTUnwrap(connection.requests.last)
        XCTAssertEqual(sent["title"], .string("Call mum tomorrow !high"))
        XCTAssertNil(sent["dueDate"])
        XCTAssertNil(sent["priority"])
    }
}
