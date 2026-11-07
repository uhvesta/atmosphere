#if os(iOS)
    import SnapshotTesting
    import SwiftUI
    import UIKit
    import XCTest

    /// The images every screen is checked in: the smallest and largest iPhones,
    /// light and dark, and the largest text size on the smallest one.
    public enum ScreenImage: String, CaseIterable, Sendable {
        case seLight = "se-light"
        case seDark = "se-dark"
        case proMaxLight = "pro-max-light"
        case proMaxDark = "pro-max-dark"
        case seLargestText = "se-largest-text"

        var config: ViewImageConfig {
            switch self {
            case .seLight, .seDark, .seLargestText: .iPhoneSe
            case .proMaxLight, .proMaxDark: .iPhone13ProMax
            }
        }

        var traits: UITraitCollection {
            UITraitCollection { traits in
                traits.userInterfaceStyle = self == .seDark || self == .proMaxDark ? .dark : .light
                if self == .seLargestText {
                    traits.preferredContentSizeCategory = .accessibilityExtraExtraExtraLarge
                }
            }
        }
    }

    /// Checks `view` against one reference per `ScreenImage`, named
    /// `<name>-<image>`, for each of `images`.
    @MainActor
    public func assertScreenImages(
        of view: @autoclosure () -> some View, named name: String, images: [ScreenImage] = ScreenImage.allCases,
        file: StaticString = #filePath, testName: String = #function, line: UInt = #line
    ) {
        for image in images {
            assertSnapshot(
                of: view(),
                as: .image(layout: .device(config: image.config), traits: image.traits),
                named: "\(name)-\(image.rawValue)", file: file, testName: testName, line: line)
        }
    }
#endif
