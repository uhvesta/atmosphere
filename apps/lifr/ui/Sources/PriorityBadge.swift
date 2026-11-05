import LifrCore
import SwiftUI

/// A small capsule with the priority's name. Shows nothing for no priority.
package struct PriorityBadge: View {
    package var priority: Priority
    @Environment(\.colorScheme) private var colorScheme

    package init(_ priority: Priority) {
        self.priority = priority
    }

    package var body: some View {
        if let title = priority.title {
            Text(title)
                .font(.caption2.weight(.semibold))
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .foregroundStyle(color)
                .background(color.opacity(0.15), in: Capsule())
        }
    }

    private var color: Color {
        switch priority {
        case .none, .low: .secondary
        // Yellow and orange text is too faint on a light background, so it
        // gets darker shades there that still read as the same colors.
        case .medium: colorScheme == .dark ? .yellow : Color(red: 0.55, green: 0.42, blue: 0)
        case .high: colorScheme == .dark ? .orange : Color(red: 0.78, green: 0.36, blue: 0)
        case .urgent: .red
        }
    }
}
