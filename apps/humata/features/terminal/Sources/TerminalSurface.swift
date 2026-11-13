import AppKit
import SwiftUI

/// A terminal drawn by Ghostty, running `command`. A new `generation`
/// starts the command again on a new surface; `exited` is told once when a
/// surface's command ends, with its exit code if Ghostty knows it.
struct TerminalSurface: NSViewRepresentable {
    let command: TerminalCommand
    let generation: Int
    /// The find bar's state, which ⌘F in the terminal brings up.
    let search: TerminalSearch
    let exited: (Int32?) -> Void

    func makeNSView(context: Context) -> TerminalSurfaceHost { TerminalSurfaceHost() }

    func updateNSView(_ host: TerminalSurfaceHost, context: Context) {
        host.show(command, generation: generation, search: search, exited: exited)
    }

    static func dismantleNSView(_ host: TerminalSurfaceHost, coordinator: ()) {
        host.close()
    }
}

/// Holds the surface now drawing a terminal, replaced by another when the
/// command or generation changes, or a note when Ghostty couldn't start.
final class TerminalSurfaceHost: NSView {
    private var shown: (command: TerminalCommand, generation: Int)?
    private(set) var surface: GhosttySurfaceView?

    func show(
        _ command: TerminalCommand, generation: Int, search: TerminalSearch, exited: @escaping (Int32?) -> Void
    ) {
        if let shown, shown.command == command, shown.generation == generation {
            surface?.exited = exited
            surface?.search = search
            search.view = surface
            return
        }
        let hadKeys = surface.map { window?.firstResponder === $0 } ?? false
        let first = shown == nil
        close()
        shown = (command, generation)
        switch GhosttyRuntime.shared {
        case .success(let runtime):
            let view = GhosttySurfaceView(runtime: runtime, command: command)
            view.exited = exited
            // A new surface has no search going.
            search.ended()
            view.search = search
            search.view = view
            fill(with: view)
            surface = view
            if hadKeys || first { takeKeys() }
        case .failure(let error):
            let note = NSTextField(wrappingLabelWithString: error.localizedDescription)
            note.textColor = .secondaryLabelColor
            fill(with: note)
        }
    }

    /// Shown, as when its tab is picked, the terminal takes the keys.
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        takeKeys()
    }

    /// Gives the surface the keys, after SwiftUI has placed the view,
    /// unless the find bar is up: it keeps them then.
    private func takeKeys() {
        Task { @MainActor [weak self] in
            guard let self, let window, let surface, surface.search?.shown != true,
                window.firstResponder !== surface
            else { return }
            window.makeFirstResponder(surface)
        }
    }

    /// Ends the surface and its command, and shows nothing.
    func close() {
        surface?.search?.ended()
        surface?.close()
        surface = nil
        shown = nil
        subviews.forEach { $0.removeFromSuperview() }
    }

    private func fill(with view: NSView) {
        view.frame = bounds
        view.autoresizingMask = [.width, .height]
        addSubview(view)
    }
}
