import LifrCore
import SwiftUI

/// A small capsule with the priority's name. Shows nothing for no priority.
package struct PriorityBadge: View {
    package var priority: Priority

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
        case .medium: .yellow
        case .high: .orange
        case .urgent: .red
        }
    }
}
