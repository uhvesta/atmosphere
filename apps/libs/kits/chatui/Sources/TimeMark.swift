import SwiftUI

/// When a message was sent, shown above it where the chat picks up again:
/// the day, for its first message and the first of each new day, or the
/// time, after a pause of an hour or more.
public enum TimeMark: Hashable, Sendable {
    case day
    case time

    /// The pause after which a message shows its time.
    static let pause: TimeInterval = 60 * 60

    /// The mark above a message sent `at`, after one sent `previous`, or
    /// nil for none.
    public static func mark(at date: Date, after previous: Date?, calendar: Calendar = .current) -> TimeMark? {
        guard let previous else { return .day }
        if !calendar.isDate(date, inSameDayAs: previous) { return .day }
        return date.timeIntervalSince(previous) >= pause ? .time : nil
    }
}

/// A time mark, centered and quiet, between messages.
public struct TimeMarkView: View {
    let date: Date
    let mark: TimeMark

    public init(_ date: Date, mark: TimeMark) {
        self.date = date
        self.mark = mark
    }

    public var body: some View {
        Group {
            switch mark {
            case .day: Text(date, format: .dateTime.weekday(.wide).day().month(.wide).hour().minute())
            case .time: Text(date, format: .dateTime.hour().minute())
            }
        }
        .font(.caption)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity)
        .accessibilityAddTraits(.isHeader)
    }
}
