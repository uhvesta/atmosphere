#if os(iOS)
    import SnapshotTesting
    import SwiftUI
    import UIKit

    extension Snapshotting where Value == UIViewController, Format == String {
        /// The accessibility tree VoiceOver would read, one element per line, in
        /// reading order: traits, label, value, hint and identifier.
        package static func accessibilityTree(size: CGSize = CGSize(width: 393, height: 852)) -> Snapshotting {
            SimplySnapshotting.lines.pullback { controller in
                _ = AccessibilityTree.automation
                let window = UIWindow(frame: CGRect(origin: .zero, size: size))
                window.rootViewController = controller
                window.isHidden = false
                controller.view.setNeedsLayout()
                controller.view.layoutIfNeeded()
                // SwiftUI builds its accessibility elements on the next run loop turn.
                RunLoop.main.run(until: Date().addingTimeInterval(0.05))
                let lines = AccessibilityTree.readingOrder(AccessibilityTree.elements(in: controller.view))
                    .map(AccessibilityTree.describe)
                window.isHidden = true
                return lines.joined(separator: "\n") + "\n"
            }
        }
    }

    extension Snapshotting where Value: SwiftUI.View, Format == String {
        /// The accessibility tree of a SwiftUI view, hosted at `size`.
        package static func accessibilityTree(size: CGSize = CGSize(width: 393, height: 852)) -> Snapshotting {
            Snapshotting<UIViewController, String>.accessibilityTree(size: size).pullback { view in
                UIHostingController(rootView: view)
            }
        }
    }

    enum AccessibilityTree {
        /// SwiftUI only builds accessibility elements while an assistive client is
        /// attached. Turning on accessibility automation, as XCUITest does, makes
        /// it build them in this process.
        static let automation: Void = {
            guard let library = dlopen("/usr/lib/libAccessibility.dylib", RTLD_NOW),
                let symbol = dlsym(library, "_AXSSetAutomationEnabled")
            else { return }
            typealias SetEnabled = @convention(c) (Int32) -> Void
            unsafeBitCast(symbol, to: SetEnabled.self)(1)
        }()

        /// Top to bottom, then leading to trailing, as VoiceOver reads them. The
        /// view hierarchy's order differs: a List puts its section headers after
        /// its rows.
        static func readingOrder(_ elements: [NSObject]) -> [NSObject] {
            elements.enumerated()
                .sorted { lhs, rhs in
                    let (left, right) = (lhs.element.accessibilityFrame, rhs.element.accessibilityFrame)
                    if left.minY.rounded() != right.minY.rounded() { return left.minY < right.minY }
                    if left.minX.rounded() != right.minX.rounded() { return left.minX < right.minX }
                    return lhs.offset < rhs.offset
                }
                .map(\.element)
        }

        static func elements(in object: NSObject) -> [NSObject] {
            if let view = object as? UIView, view.isHidden || view.alpha == 0 { return [] }
            if object.accessibilityElementsHidden { return [] }
            if object.isAccessibilityElement { return [object] }
            return children(of: object).flatMap(elements)
        }

        private static func children(of object: NSObject) -> [NSObject] {
            if let children = object.accessibilityElements as? [NSObject] { return children }
            let count = object.accessibilityElementCount()
            if count > 0, count != NSNotFound {
                return (0..<count).compactMap { object.accessibilityElement(at: $0) as? NSObject }
            }
            return (object as? UIView)?.subviews ?? []
        }

        static func describe(_ element: NSObject) -> String {
            var parts = [traits(element.accessibilityTraits)]
            parts.append(quoted(element.accessibilityLabel))
            if let value = element.accessibilityValue, !value.isEmpty { parts.append("value: \(quoted(value))") }
            if let hint = element.accessibilityHint, !hint.isEmpty { parts.append("hint: \(quoted(hint))") }
            if let identifiable = element as? UIAccessibilityIdentification,
                let identifier = identifiable.accessibilityIdentifier, !identifier.isEmpty
            {
                parts.append("id: \(identifier)")
            }
            return parts.joined(separator: " ")
        }

        private static func quoted(_ text: String?) -> String {
            "\"\(text ?? "")\""
        }

        private static let names: [(UIAccessibilityTraits, String)] = [
            (.button, "button"), (.link, "link"), (.header, "header"), (.searchField, "searchField"),
            (.image, "image"), (.selected, "selected"), (.notEnabled, "notEnabled"), (.adjustable, "adjustable"),
            (.toggleButton, "toggle"), (.tabBar, "tabBar"), (.summaryElement, "summary"),
            (.updatesFrequently, "updatesFrequently"), (.staticText, "text"),
        ]

        private static func traits(_ traits: UIAccessibilityTraits) -> String {
            let matched = names.filter { traits.contains($0.0) }.map(\.1)
            return "[\(matched.isEmpty ? "element" : matched.joined(separator: ", "))]"
        }
    }
#endif
