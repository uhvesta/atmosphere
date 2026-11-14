import SwiftUI

/// A `mermaid` block drawn as its diagram, no wider than the text around
/// it. A spinner shows while it's drawn; a broken one shows its text and
/// what's wrong with it. Clicking it opens it large in its own window.
/// Scrolled away and back, it shows as it did, with no spinner between.
package struct MermaidDiagram: View {
    let source: String
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.openWindow) private var openWindow
    @State private var drawn: Drawn?

    private enum Drawn: Equatable {
        case image(NSImage)
        case failed(DiagramError)
    }

    package init(source: String) {
        self.source = source
    }

    private var dark: Bool { colorScheme == .dark }

    package var body: some View {
        // Drawn before, or found to have a mistake, it shows at once, with
        // no spinner: a row scrolled back into view has lost what it showed.
        let drawn =
            drawn ?? MermaidRenderer.shared.cached(source, dark: dark).map(Drawn.image)
            ?? MermaidRenderer.shared.cachedFailure(source, dark: dark).map(Drawn.failed)
        Group {
            switch drawn {
            case .image(let image):
                Button {
                    openWindow(value: DiagramWindowValue(source: source))
                } label: {
                    Image(nsImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: image.size.width)
                }
                .buttonStyle(.plain)
                .help("Open the diagram in its own window")
                .accessibilityLabel("Diagram")
                .accessibilityHint("Opens it large in its own window")
                .accessibilityIdentifier("diagram")
            case .failed(let error):
                Broken(source: source, error: error)
            case nil:
                ProgressView("Drawing the diagram…")
                    .controlSize(.small)
                    .frame(maxWidth: .infinity, minHeight: 60)
                    .accessibilityIdentifier("diagram.drawing")
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .task(id: "\(dark)\n\(source)") {
            do throws(DiagramError) {
                self.drawn = .image(try await MermaidRenderer.shared.image(source, dark: dark))
            } catch {
                self.drawn = .failed(error)
            }
        }
    }
}

/// A diagram that couldn't be drawn: its text, and why.
struct Broken: View {
    let source: String
    let error: DiagramError

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Label(Self.title(error), systemImage: "exclamationmark.triangle")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.orange)
            Text(Self.detail(error))
                .font(.caption.monospaced())
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            Text(source)
                .font(.callout.monospaced())
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .padding(10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
        }
        .accessibilityIdentifier("diagram.broken")
    }

    static func title(_ error: DiagramError) -> String {
        switch error {
        case .invalid: "This diagram has a mistake, so it shows as text"
        case .unavailable: "Diagrams can't be drawn in this build, so it shows as text"
        case .failed: "The diagram couldn't be drawn, so it shows as text"
        }
    }

    static func detail(_ error: DiagramError) -> String {
        switch error {
        case .invalid(let message):
            "\(message)\nAsk the agent to fix the diagram, or fix the block's text yourself."
        case .unavailable:
            "Humata's mermaid.js is missing from the app. Reinstall Humata to draw diagrams again."
        case .failed(let message):
            "\(message)\nScroll it out of view and back to try again; if it keeps failing, restart Humata."
        }
    }
}

/// Which diagram a window shows.
package struct DiagramWindowValue: Codable, Hashable {
    package var source: String

    package init(source: String) {
        self.source = source
    }
}
