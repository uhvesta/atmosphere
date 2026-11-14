import LifrCore
import SwiftUI

/// A task's status as a symbol. Decorative: rows read the status out in
/// their accessibility value.
package struct StatusGlyph: View {
    package var status: TaskStatus
    package var font: Font

    package init(_ status: TaskStatus, font: Font = .title3) {
        self.status = status
        self.font = font
    }

    package var body: some View {
        Image(systemName: status.symbol)
            .font(font)
            .foregroundStyle(color)
            .contentTransition(.symbolEffect(.replace))
            .animation(.default, value: status)
            .accessibilityHidden(true)
    }

    private var color: Color {
        switch status {
        case .todo: .secondary
        case .inProgress: .blue
        case .procrastinated: .orange
        case .done: .green
        case .wontDo: .gray
        }
    }
}

extension TaskStatus {
    /// The symbol `StatusGlyph` shows.
    package var symbol: String {
        switch self {
        case .todo: "circle"
        case .inProgress: "circle.lefthalf.filled"
        case .procrastinated: "hourglass.circle"
        case .done: "checkmark.circle.fill"
        case .wontDo: "xmark.circle"
        }
    }
}
