import ComposableArchitecture

#if canImport(UIKit)
    import UIKit
#elseif canImport(AppKit)
    import AppKit
#endif

/// Puts text on the clipboard.
package struct PasteboardClient: Sendable {
    package var copy: @Sendable @MainActor (String) -> Void

    package init(copy: @escaping @Sendable @MainActor (String) -> Void) {
        self.copy = copy
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
    }
    package static let testValue = PasteboardClient { _ in }
}

extension DependencyValues {
    package var pasteboard: PasteboardClient {
        get { self[PasteboardClient.self] }
        set { self[PasteboardClient.self] = newValue }
    }
}
