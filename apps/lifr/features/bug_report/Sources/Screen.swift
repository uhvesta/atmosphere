import Foundation

#if canImport(UIKit)
    import UIKit

    /// What's on screen, for a recording.
    enum Screen {
        /// The key window as a JPEG at 1x, or nil when there's none.
        @MainActor static func capture() -> Data? {
            let window = UIApplication.shared.connectedScenes
                .compactMap { $0 as? UIWindowScene }
                .flatMap(\.windows)
                .first(where: \.isKeyWindow)
            guard let window else { return nil }
            let format = UIGraphicsImageRendererFormat()
            format.scale = 1
            let image = UIGraphicsImageRenderer(bounds: window.bounds, format: format).image { _ in
                _ = window.drawHierarchy(in: window.bounds, afterScreenUpdates: false)
            }
            return image.jpegData(compressionQuality: 0.6)
        }
    }
#elseif canImport(AppKit)
    import AppKit

    /// What's on screen, for a recording.
    enum Screen {
        /// The key window's content as a JPEG, or nil when there's none.
        @MainActor static func capture() -> Data? {
            guard let view = NSApp.keyWindow?.contentView ?? NSApp.mainWindow?.contentView,
                let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds)
            else { return nil }
            view.cacheDisplay(in: view.bounds, to: bitmap)
            return bitmap.representation(using: .jpeg, properties: [.compressionFactor: 0.6])
        }
    }
#endif
