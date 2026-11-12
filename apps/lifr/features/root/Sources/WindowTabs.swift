#if os(macOS)
    import AppKit
    import SwiftUI

    /// Opens another window as a tab of the window in front.
    @MainActor
    enum WindowTabs {
        /// The window the next one to open joins, until it does.
        private static weak var host: NSWindow?

        static func open(_ place: Place, with openWindow: OpenWindowAction) {
            host = NSApp.keyWindow
            openWindow(id: WindowPlace.sceneID, value: WindowPlace(place))
        }

        fileprivate static func join(_ window: NSWindow) {
            guard let host, host !== window else { return }
            self.host = nil
            host.addTabbedWindow(window, ordered: .above)
            window.makeKeyAndOrderFront(nil)
        }
    }

    extension View {
        /// Joins the window in front as a tab, when New Tab opened this
        /// window.
        package func joinsTab() -> some View { background(TabJoiner()) }
    }

    private struct TabJoiner: NSViewRepresentable {
        func makeNSView(context: Context) -> NSView { JoiningView() }
        func updateNSView(_ nsView: NSView, context: Context) {}
    }

    private final class JoiningView: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window else { return }
            WindowTabs.join(window)
        }
    }
#endif
