#if os(iOS)
    import SnapshotTesting
    import SwiftUI
    import UIKit
    import XCTest

    /// Walks a person's flow through a live screen: the screen stays in one
    /// window while the test changes it, and each step is checked against its
    /// references, the accessibility tree and an image, so a reviewer can follow
    /// what the person sees from step to step.
    @MainActor
    package final class FlowRecorder {
        private let window: UIWindow
        private let name: String
        /// The longest a step waits for the screen to settle before checking it
        /// as it is.
        private let timeout: Duration
        private var count = 0

        /// Hosts `view` in a window the size of an iPhone 17. `name` names the
        /// flow's references. A step waits at most `timeout` for the screen to
        /// settle.
        package init(
            _ name: String, _ view: some View, size: CGSize = CGSize(width: 402, height: 874),
            timeout: Duration = .seconds(5)
        ) {
            self.timeout = timeout
            _ = AccessibilityTree.automation
            UIView.setAnimationsEnabled(false)
            self.name = name
            let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
            window = scene.map(UIWindow.init(windowScene:)) ?? UIWindow()
            window.frame = CGRect(origin: .zero, size: size)
            window.rootViewController = UIHostingController(rootView: view)
            window.makeKeyAndVisible()
        }

        /// Lets the screen settle after a change, then checks it as the next
        /// step, `<flow>-<number>-<step>`.
        package func step(
            _ step: String, image: Bool = true, file: StaticString = #filePath, testName: String = #function,
            line: UInt = #line
        ) async {
            count += 1
            let label = String(format: "%@-%02d-%@", name, count, step)
            guard let lines = await settle() else {
                XCTFail(
                    "\(label) kept changing for \(timeout). If the screen is stuck working, that's the bug; if it's "
                        + "only slow, give FlowRecorder a longer timeout.", file: file, line: line)
                return
            }
            assertSnapshot(
                of: lines.joined(separator: "\n") + "\n", as: .lines, named: label, file: file, testName: testName,
                line: line)
            guard image else { return }
            let bounds = window.bounds
            let rendered = UIGraphicsImageRenderer(bounds: bounds).image { _ in
                window.drawHierarchy(in: bounds, afterScreenUpdates: true)
            }
            assertSnapshot(
                of: rendered, as: .image(precision: 0.99, perceptualPrecision: 0.98), named: label, file: file,
                testName: testName, line: line)
        }

        /// Activates the `occurrence`th element labelled `label`, in reading
        /// order, as VoiceOver does with a double tap, so the step goes
        /// through the screen's own controls and bindings.
        package func tap(
            _ label: String, occurrence: Int = 0, file: StaticString = #filePath, line: UInt = #line
        ) {
            window.layoutIfNeeded()
            let matches = AccessibilityTree.readingOrder(AccessibilityTree.elements(in: window))
                .filter { $0.accessibilityLabel == label }
            guard matches.indices.contains(occurrence) else {
                XCTFail(
                    "There's no \"\(label)\" number \(occurrence + 1) on the screen; there are \(matches.count). "
                        + "Check the step before shows it.", file: file, line: line)
                return
            }
            if !matches[occurrence].accessibilityActivate() {
                XCTFail("\"\(label)\" didn't respond to a tap.", file: file, line: line)
            }
        }

        /// Ends the flow, taking the screen down.
        package func finish() {
            window.isHidden = true
            window.rootViewController = nil
        }

        /// Effects run on other tasks, and sheets present and toasts fade on
        /// later run loop turns: waits until the screen stays the same for three
        /// looks in a row, and returns its accessibility tree; nil when it's
        /// still changing after `timeout`.
        private func settle() async -> [String]? {
            var last: [String] = []
            var unchanged = 0
            let clock = ContinuousClock()
            let deadline = clock.now + timeout
            while clock.now < deadline {
                try? await Task.sleep(for: .milliseconds(80))
                window.layoutIfNeeded()
                let lines = AccessibilityTree.readingOrder(AccessibilityTree.elements(in: window))
                    .map(AccessibilityTree.describe)
                unchanged = lines == last ? unchanged + 1 : 0
                last = lines
                if unchanged == 3 {
                    // The tree settles before the last frames of a transition draw.
                    try? await Task.sleep(for: .milliseconds(250))
                    return last
                }
            }
            return nil
        }
    }
#endif
