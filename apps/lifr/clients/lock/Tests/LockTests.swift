import Foundation
import LifrLockClient
import XCTest

final class LockTests: XCTestCase {
    private let away = Date(timeIntervalSince1970: 1_793_700_000)

    func testAShortTripAwayDoesntLock() {
        XCTAssertFalse(AppLock.locks(away: away, now: away.addingTimeInterval(20)))
    }

    func testAMinuteAwayLocks() {
        XCTAssertTrue(AppLock.locks(away: away, now: away.addingTimeInterval(60)))
        XCTAssertTrue(AppLock.locks(away: away, now: away.addingTimeInterval(3600)))
    }

    func testNeverAwayDoesntLock() {
        XCTAssertFalse(AppLock.locks(away: nil, now: away))
    }

    func testTheAlwaysLockConfirmsOrFailsAsTold() async throws {
        try await LockClient.always().authenticate("Unlock Lifr")
        do {
            try await LockClient.always(failing: .noPasscode).authenticate("Unlock Lifr")
            XCTFail("Expected it to fail")
        } catch {
            XCTAssertEqual(error as? LockError, .noPasscode)
        }
    }

    func testAScriptedLockFailsInTurnThenConfirms() async throws {
        let lock = LockClient.scripted([.cancelled, nil])
        do {
            try await lock.authenticate("Unlock Lifr")
            XCTFail("Expected it to fail")
        } catch {
            XCTAssertEqual(error as? LockError, .cancelled)
        }
        try await lock.authenticate("Unlock Lifr")
        try await lock.authenticate("Unlock Lifr")
    }
}
