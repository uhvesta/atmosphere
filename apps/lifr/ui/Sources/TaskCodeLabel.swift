import SwiftUI

/// A task's display code, such as HEALTH-7K2Q, in a fixed-width font so codes
/// line up.
package struct TaskCodeLabel: View {
    package var code: String

    package init(_ code: String) {
        self.code = code
    }

    package var body: some View {
        Text(code)
            .font(.caption.monospaced())
            .foregroundStyle(.secondary)
    }
}
