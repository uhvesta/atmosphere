import AppKit
import SwiftUI

/// A terminal drawn by Ghostty, running `command`. A new `generation`
/// starts the command again on a new surface; `exited` is told once when a
/// surface's command ends, with its exit code if Ghostty knows it.
struct TerminalSurface: NSViewRepresentable {
    let command: TerminalCommand
    let generation: Int
    let exited: (Int32?) -> Void

    func makeNSView(context: Context) -> TerminalSurfaceHost { TerminalSurfaceHost() }

    func updateNSView(_ host: TerminalSurfaceHost, context: Context) {
        host.show(command, generation: generation, exited: exited)
    }

    static func dismantleNSView(_ host: TerminalSurfaceHost, coordinator: ()) {
        host.close()
    }
}

/// Holds the surface now drawing a terminal, replaced by another when the
/// command or generation changes, or a note when Ghostty couldn't start.
final class TerminalSurfaceHost: NSView {
    private var shown: (command: TerminalCommand, generation: Int)?
    private var surface: GhosttySurfaceView?

    func show(_ command: TerminalCommand, generation: Int, exited: @escaping (Int32?) -> Void) {
        if let shown, shown.command == command, shown.generation == generation {
            surface?.exited = exited
            return
        }
        let hadKeys = surface.map { window?.firstResponder === $0 } ?? false
        close()
        shown = (command, generation)
        switch GhosttyRuntime.shared {
        case .success(let runtime):
            let view = GhosttySurfaceView(runtime: runtime, command: command)
            view.exited = exited
            fill(with: view)
            surface = view
            if hadKeys { window?.makeFirstResponder(view) }
        case .failure(let error):
            let note = NSTextField(wrappingLabelWithString: error.localizedDescription)
            note.textColor = .secondaryLabelColor
            fill(with: note)
        }
    }

    /// Ends the surface and its command, and shows nothing.
    func close() {
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
