import InferenceKit
import SwiftUI

/// How much of the model's window the conversation took on its last turn,
/// as a ring that fills in the accent color, or an empty one before the
/// first reply.
public struct ContextRing: View {
    let use: ContextUse?

    public init(_ use: ContextUse?) {
        self.use = use
    }

    public var body: some View {
        ZStack {
            Circle()
                .stroke(.quaternary, lineWidth: 3)
            Circle()
                .trim(from: 0, to: use?.fraction ?? 0)
                .stroke(.tint, style: StrokeStyle(lineWidth: 3, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .frame(width: 18, height: 18)
        .accessibilityElement()
        .accessibilityLabel(Text("Context Window"))
        .accessibilityValue(Text(Self.description(of: use)))
    }

    /// What the ring shows, in words: how many tokens of how many.
    public static func description(of use: ContextUse?) -> String {
        guard let use else { return String(localized: "Nothing used yet") }
        let percent = use.fraction.formatted(.percent.precision(.fractionLength(0)))
        return String(
            localized: "\(percent) used, \(use.used.formatted()) of \(use.window.formatted()) tokens")
    }
}
