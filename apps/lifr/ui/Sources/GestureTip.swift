import SwiftUI

/// A hint about a gesture that's easy to miss, shown in place until the
/// person closes it or uses the gesture, once per device.
package enum GestureTip: String, CaseIterable, Sendable {
    /// Swiping a task in a list.
    case swipeRow
    /// Dragging a task onto a day in the calendar.
    case calendarDrag
    /// The Mac's keys that open Quick Add from any app.
    case globalCapture

    /// Where this device remembers the tip was seen.
    package var storageKey: String { "lifrTipSeen-\(rawValue)" }
}

/// A tip as a row: what to try and how, with a button to close it.
package struct GestureTipCard: View {
    private let symbol: String
    private let title: LocalizedStringKey
    private let message: LocalizedStringKey
    private let close: () -> Void

    package init(
        symbol: String, title: LocalizedStringKey, message: LocalizedStringKey, close: @escaping () -> Void
    ) {
        self.symbol = symbol
        self.title = title
        self.message = message
        self.close = close
    }

    package var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: symbol)
                .font(.title3)
                .foregroundStyle(.tint)
                .frame(width: 28)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.subheadline.weight(.semibold))
                Text(message).font(.subheadline).foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
            Spacer(minLength: 0)
            Button(action: close) {
                Image(systemName: "xmark")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .padding(4)
                    .contentShape(.rect)
            }
            .buttonStyle(.borderless)
            .accessibilityLabel("Close Tip")
        }
        .padding(.vertical, 4)
    }
}
