import ComposableArchitecture

#if canImport(UIKit)
    import UIKit
#elseif canImport(AppKit)
    import AppKit
#endif

/// Puts text on the clipboard, and reads it back.
package struct PasteboardClient: Sendable {
    package var copy: @Sendable @MainActor (String) -> Void
    /// The text on the clipboard, if any.
    package var read: @Sendable @MainActor () -> String?

    package init(
        copy: @escaping @Sendable @MainActor (String) -> Void,
        read: @escaping @Sendable @MainActor () -> String? = { nil }
    ) {
        self.copy = copy
        self.read = read
    }
}

extension PasteboardClient: DependencyKey {
    package static let liveValue = PasteboardClient { text in
        #if canImport(UIKit)
            UIPasteboard.general.string = text
        #elseif canImport(AppKit)
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
        #endif
    } read: {
        #if canImport(UIKit)
            UIPasteboard.general.string
        #elseif canImport(AppKit)
            NSPasteboard.general.string(forType: .string)
        #else
            nil
        #endif
    }
    package static let testValue = PasteboardClient { _ in }
}

extension DependencyValues {
    package var pasteboard: PasteboardClient {
        get { self[PasteboardClient.self] }
        set { self[PasteboardClient.self] = newValue }
    }
}
