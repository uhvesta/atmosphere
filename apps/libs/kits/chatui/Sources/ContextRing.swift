import InferenceKit
import SwiftUI

/// How much of the model's window the conversation took on its last turn,
/// as a circle that fills like a pie in the accent color, or an empty one
/// until a reply measures it: before the first one, and after the app
/// reopens a conversation, since the use isn't kept. A pie, not an arc, so it doesn't read as a
/// spinner.
public struct ContextRing: View {
    let use: ContextUse?

    public init(_ use: ContextUse?) {
        self.use = use
    }

    public var body: some View {
        ZStack {
            Circle()
                .stroke(.secondary, lineWidth: 1.5)
            Pie(fraction: use?.fraction ?? 0)
                .fill(.tint)
                .padding(3)
        }
        .frame(width: 18, height: 18)
        .accessibilityElement()
        .accessibilityLabel(Text("Context Window"))
        .accessibilityValue(Text(Self.description(of: use)))
    }

    /// What the ring shows, in words: how many tokens of how many.
    public static func description(of use: ContextUse?) -> String {
        guard let use else { return String(localized: "Shows after the next reply") }
        let percent = use.fraction.formatted(.percent.precision(.fractionLength(0)))
        return String(
            localized: "\(percent) used, \(use.used.formatted()) of \(use.window.formatted()) tokens")
    }
}

/// The ring as a button that opens to say how full the window is. A
/// button and a popover, not a menu, because a Mac menu's button draws
/// only text and images, which leaves the ring out.
public struct ContextButton: View {
    let use: ContextUse?
    @State private var showing = false

    public init(_ use: ContextUse?) {
        self.use = use
    }

    public var body: some View {
        Button {
            showing = true
        } label: {
            ContextRing(use)
        }
        .help(ContextRing.description(of: use))
        .popover(isPresented: $showing) {
            VStack(alignment: .leading, spacing: 4) {
                Text("Context Window").font(.headline)
                Text(ContextRing.description(of: use)).foregroundStyle(.secondary)
            }
            .padding()
            .presentationCompactAdaptation(.popover)
        }
    }
}

/// A slice of a circle from twelve o'clock, clockwise, `fraction` of the
/// way round, and at least a twelfth of it once anything is used, so a
/// little use reads as a slice rather than a clock hand.
private struct Pie: Shape {
    var fraction: Double

    var animatableData: Double {
        get { fraction }
        set { fraction = newValue }
    }

    func path(in rect: CGRect) -> Path {
        let share = min(max(fraction, 0), 1)
        guard share > 0 else { return Path() }
        let shown = max(share, 1.0 / 12)
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let radius = min(rect.width, rect.height) / 2
        var path = Path()
        path.move(to: center)
        path.addArc(
            center: center, radius: radius, startAngle: .degrees(-90),
            endAngle: .degrees(-90 + 360 * shown), clockwise: false)
        path.closeSubpath()
        return path
    }
}
