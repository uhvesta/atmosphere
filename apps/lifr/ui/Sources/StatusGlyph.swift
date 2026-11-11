import LifrCore
import SwiftUI

/// A task's status as a symbol. Decorative: rows read the status out in
/// their accessibility value.
package struct StatusGlyph: View {
    package var status: TaskStatus

    package init(_ status: TaskStatus) {
        self.status = status
    }

    package var body: some View {
        Image(systemName: symbol)
            .font(.title3)
            .foregroundStyle(color)
            .contentTransition(.symbolEffect(.replace))
            .animation(.default, value: status)
            .accessibilityHidden(true)
    }

    private var symbol: String {
        switch status {
        case .todo: "circle"
        case .inProgress: "circle.lefthalf.filled"
        case .procrastinated: "hourglass.circle"
        case .done: "checkmark.circle.fill"
        case .wontDo: "xmark.circle"
        }
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
