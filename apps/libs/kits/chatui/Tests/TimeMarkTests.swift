import Foundation
import TestingKit

@testable import ChatUI

func timeMarkTests() -> [TestCase] {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "Europe/London") ?? .gmt
    let morning = Date(timeIntervalSince1970: 1_793_700_000)  // 2026-11-04, 09:20 in London
    return [
        TestCase("given the first message, then its day") {
            try expectEqual(TimeMark.mark(at: morning, after: nil, calendar: calendar), .day)
        },
        TestCase("given a message minutes after the last, then no mark") {
            try expectEqual(TimeMark.mark(at: morning + 59 * 60, after: morning, calendar: calendar), nil)
        },
        TestCase("given a message an hour or more after the last on the same day, then its time") {
            try expectEqual(TimeMark.mark(at: morning + 60 * 60, after: morning, calendar: calendar), .time)
        },
        TestCase("given a message on a later day, however soon, then its day") {
            let lateNight = calendar.date(bySettingHour: 23, minute: 59, second: 0, of: morning) ?? morning
            try expectEqual(TimeMark.mark(at: lateNight + 120, after: lateNight, calendar: calendar), .day)
        },
    ]
}
