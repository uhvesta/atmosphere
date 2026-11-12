import SwiftUI

extension EnvironmentValues {
    /// Tells the window how tall a bar along the bottom of the screen is, or
    /// 0 once it's gone, so the Undo toast sits above it.
    @Entry package var reportBottomBar: @MainActor (CGFloat) -> Void = { _ in }
}

extension View {
    /// Reports this bar's height to the window while it shows.
    package func reportsBottomBar() -> some View {
        modifier(ReportsBottomBar())
    }
}

private struct ReportsBottomBar: ViewModifier {
    @Environment(\.reportBottomBar) private var report

    func body(content: Content) -> some View {
        content
            .onGeometryChange(for: CGFloat.self) { geometry in
                geometry.size.height
            } action: { height in
                report(height)
            }
            .onDisappear { report(0) }
    }
}
