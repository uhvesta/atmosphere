import AppKit
import SwiftUI
import UniformTypeIdentifiers

/// The image a paste would bring: a text field takes only text, so ⌘V
/// with an image on the clipboard would otherwise do nothing.
package enum ClipboardImage: Equatable {
    /// Image files, as copied in the Finder.
    case files([URL])
    /// An image itself, as a screenshot to the clipboard is.
    case data(Data)

    /// What ⌘V brings as an image, given what the clipboard holds: image
    /// files ahead of all else, as their names are the only text with
    /// them; then an image, unless there is text, which the field pastes
    /// as it always does. Nil when the paste is the field's.
    package static func pasted(imageFiles: [URL], hasText: Bool, image: () -> Data?) -> ClipboardImage? {
        if !imageFiles.isEmpty { return .files(imageFiles) }
        guard !hasText else { return nil }
        return image().map { .data($0) }
    }

    /// The image on the clipboard itself, if there is one. Read only when
    /// asked for, as it can be large.
    @MainActor static var image: Data? {
        let pasteboard = NSPasteboard.general
        for type in [NSPasteboard.PasteboardType.png, .tiff] {
            if let data = pasteboard.data(forType: type) { return data }
        }
        return nil
    }

    /// What ⌘V brings as an image from the clipboard as it is now.
    @MainActor static var onClipboard: ClipboardImage? {
        let pasteboard = NSPasteboard.general
        let options: [NSPasteboard.ReadingOptionKey: Any] = [
            .urlReadingFileURLsOnly: true, .urlReadingContentsConformToTypes: [UTType.image.identifier],
        ]
        let files = pasteboard.readObjects(forClasses: [NSURL.self], options: options) as? [URL] ?? []
        let hasText = pasteboard.availableType(from: [.string]) != nil
        return pasted(imageFiles: files, hasText: hasText) { image }
    }
}

/// Behind the message's box: gives `paste` the clipboard's image when ⌘V
/// is pressed while typing in a field over it. The key is caught before
/// the Edit menu gets it, since the menu's Paste is off for a text field
/// when the clipboard has no text.
struct ImagePasteCatcher: NSViewRepresentable {
    let paste: (ClipboardImage) -> Void

    func makeNSView(context: Context) -> Catcher {
        let view = Catcher()
        view.paste = paste
        return view
    }

    func updateNSView(_ view: Catcher, context: Context) {
        view.paste = paste
    }

    final class Catcher: NSView {
        var paste: ((ClipboardImage) -> Void)?
        private var monitor: Any?

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let monitor {
                NSEvent.removeMonitor(monitor)
                self.monitor = nil
            }
            guard window != nil else { return }
            // Local monitors run on the main thread, before the menu's
            // shortcuts see the key.
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                let taken = MainActor.assumeIsolated { self?.take(event) ?? false }
                return taken ? nil : event
            }
        }

        /// Hands on the clipboard's image if `event` pastes into a field
        /// over this view and the clipboard has one. False to let the key
        /// go on as usual.
        private func take(_ event: NSEvent) -> Bool {
            guard isPaste(event), let image = ClipboardImage.onClipboard else { return false }
            paste?(image)
            return true
        }

        /// Whether `event` is ⌘V typed into a field that sits over this
        /// view, in its window.
        private func isPaste(_ event: NSEvent) -> Bool {
            guard let window, event.window === window,
                event.modifierFlags.intersection(.deviceIndependentFlagsMask) == .command,
                // With ⌘ held, a layout that isn't Latin still gives the
                // Latin letter here, as the menu's shortcut expects.
                event.characters?.lowercased() == "v" || event.charactersIgnoringModifiers?.lowercased() == "v",
                let typing = window.firstResponder as? NSView
            else { return false }
            return convert(bounds, to: nil).intersects(typing.convert(typing.bounds, to: nil))
        }
    }
}
