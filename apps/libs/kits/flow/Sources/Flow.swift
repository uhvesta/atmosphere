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
    public final class FlowRecorder {
        private let window: UIWindow
        private let name: String
        /// The longest a step waits for the screen to settle before checking it
        /// as it is.
        private let timeout: Duration
        private var count = 0
        /// Whether each step also fails on what VoiceOver and Voice Control
        /// can't name.
        private let audits: Bool

        /// Hosts `view` in a window the size of an iPhone 17, in `style`, light
        /// or dark. `name` names the flow's references. A step waits at most
        /// `timeout` for the screen to settle. With `audits`, each step also
        /// fails on an element VoiceOver would read as nothing or a control
        /// Voice Control can't name.
        public init(
            _ name: String, _ view: some View, size: CGSize = CGSize(width: 402, height: 874),
            style: UIUserInterfaceStyle = .light, timeout: Duration = .seconds(5), audits: Bool = false
        ) {
            self.timeout = timeout
            self.audits = audits
            _ = AccessibilityTree.automation
            UIView.setAnimationsEnabled(false)
            self.name = name
            let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
            window = scene.map(UIWindow.init(windowScene:)) ?? UIWindow()
            window.overrideUserInterfaceStyle = style
            window.frame = CGRect(origin: .zero, size: size)
            window.rootViewController = UIHostingController(rootView: view)
            window.makeKeyAndVisible()
            // Draws the first screen now, so a change the test makes next is
            // always a change from it, as it is for a person, and not
            // sometimes the screen the app opens on.
            window.layoutIfNeeded()
        }

        /// Lets the screen settle after a change, then checks it as the next
        /// step, `<flow>-<number>-<step>`.
        public func step(
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
            if audits {
                for problem in AccessibilityTree.problems(in: window) {
                    XCTFail("\(label): \(problem)", file: file, line: line)
                }
            }
            guard image else { return }
            let rendered = await stillImage()
            assertSnapshot(
                of: rendered, as: .image(precision: 0.99, perceptualPrecision: 0.98), named: label, file: file,
                testName: testName, line: line)
        }

        /// Activates the `occurrence`th element labelled `label`, in reading
        /// order, as VoiceOver does with a double tap, so the step goes
        /// through the screen's own controls and bindings.
        public func tap(
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

        /// Scrolls the longest list on screen to its end, as a person would to
        /// see the bottom of a long form.
        public func scrollToEnd(file: StaticString = #filePath, line: UInt = #line) {
            window.layoutIfNeeded()
            let lists = Self.scrollViews(in: window).filter {
                !$0.isHidden && $0.alpha > 0 && $0.convert($0.bounds, to: window).intersects(window.bounds)
            }
            guard let list = lists.max(by: { $0.contentSize.height < $1.contentSize.height }) else {
                XCTFail("There's nothing on the screen that scrolls.", file: file, line: line)
                return
            }
            // A lazy list only guesses the height of rows it hasn't laid out,
            // so its end moves as scrolling lays them out: scroll until it
            // stays put.
            for _ in 0..<20 {
                let insets = list.adjustedContentInset
                let end = max(-insets.top, list.contentSize.height + insets.bottom - list.bounds.height)
                if list.contentOffset.y == end { return }
                list.setContentOffset(CGPoint(x: list.contentOffset.x, y: end), animated: false)
                list.layoutIfNeeded()
            }
        }

        /// Scrolls the list that holds the first element labelled `label` so
        /// the element is at the top of it, or as near as the list goes, as a
        /// person would to bring a row into view.
        public func scroll(to label: String, file: StaticString = #filePath, line: UInt = #line) {
            window.layoutIfNeeded()
            guard
                let element = AccessibilityTree.readingOrder(AccessibilityTree.elements(in: window))
                    .first(where: { $0.accessibilityLabel == label })
            else {
                XCTFail(
                    "There's no \"\(label)\" on the screen. Check the step before shows it.", file: file, line: line)
                return
            }
            let frame = element.accessibilityFrame
            let lists = Self.scrollViews(in: window).filter {
                !$0.isHidden && $0.alpha > 0 && $0.contentSize.height > $0.bounds.height
                    && $0.convert($0.bounds, to: nil).insetBy(dx: 0, dy: -$0.contentSize.height).contains(
                        CGPoint(x: frame.midX, y: frame.midY))
            }
            guard let list = lists.max(by: { $0.contentSize.height < $1.contentSize.height }) else {
                XCTFail("Nothing on the screen scrolls \"\(label)\".", file: file, line: line)
                return
            }
            let insets = list.adjustedContentInset
            let top = list.convert(list.bounds, to: nil).minY + insets.top
            let end = max(-insets.top, list.contentSize.height + insets.bottom - list.bounds.height)
            let offset = min(max(list.contentOffset.y + frame.minY - top, -insets.top), end)
            list.setContentOffset(CGPoint(x: list.contentOffset.x, y: offset), animated: false)
            list.layoutIfNeeded()
        }

        private static func scrollViews(in view: UIView) -> [UIScrollView] {
            let own = (view as? UIScrollView).map { [$0] } ?? []
            return own + view.subviews.flatMap(scrollViews)
        }

        /// Ends the flow, taking the screen down.
        public func finish() {
            window.isHidden = true
            window.rootViewController = nil
        }

        /// Effects run on other tasks, and sheets present and toasts fade on
        /// later run loop turns: waits until the screen stays the same for three
        /// looks in a row, its words and where each element sits, and returns
        /// its accessibility tree; nil when it's still changing after `timeout`.
        private func settle() async -> [String]? {
            var last: [String] = []
            var lastFrames: [CGRect] = []
            var unchanged = 0
            let clock = ContinuousClock()
            let deadline = clock.now + timeout
            while clock.now < deadline {
                try? await Task.sleep(for: .milliseconds(80))
                window.layoutIfNeeded()
                let elements = AccessibilityTree.readingOrder(AccessibilityTree.elements(in: window))
                let lines = elements.map(AccessibilityTree.describe)
                // A list still scrolling reads the same while it moves.
                let frames = elements.map(\.accessibilityFrame.integral)
                unchanged = lines == last && frames == lastFrames ? unchanged + 1 : 0
                last = lines
                lastFrames = frames
                // Two seconds still is settled, morphing or not, as a morph
                // left behind never ends.
                if unchanged >= 25 || (unchanged >= 3 && !Self.isMorphing(window)) {
                    return last
                }
            }
            return nil
        }

        /// Whether the system is still morphing its own chrome, as the tab
        /// bar does folding up beside a search field. The tree can hold still
        /// partway through, listing stand-ins for the parts on the move and
        /// the tabs the fold takes away, so a step waits for the fold to end.
        private static func isMorphing(_ view: UIView) -> Bool {
            if view.isHidden || view.alpha == 0 { return false }
            if NSStringFromClass(type(of: view)).contains("MorphAnimationContainer") { return true }
            return view.subviews.contains { isMorphing($0) }
        }

        /// The tree settles before the system's own transitions finish
        /// drawing, such as the tab bar folding up beside a search field:
        /// draws the screen until two looks in a row match, and returns the
        /// last look if it's still moving after `timeout`.
        /// The screen once two renders in a row match, as a PNG would keep
        /// it. The render itself can hold colours a PNG can't, which a
        /// recorded reference then never matches exactly.
        private func stillImage() async -> UIImage {
            let clock = ContinuousClock()
            let deadline = clock.now + timeout
            var last = render()
            var lastData = last.pngData()
            while clock.now < deadline {
                try? await Task.sleep(for: .milliseconds(100))
                let next = render()
                let nextData = next.pngData()
                if nextData == lastData {
                    return Self.stored(next, nextData)
                }
                last = next
                lastData = nextData
            }
            return Self.stored(last, lastData)
        }

        private static func stored(_ image: UIImage, _ data: Data?) -> UIImage {
            data.flatMap { UIImage(data: $0, scale: image.scale) } ?? image
        }

        private func render() -> UIImage {
            let bounds = window.bounds
            return UIGraphicsImageRenderer(bounds: bounds).image { _ in
                window.drawHierarchy(in: bounds, afterScreenUpdates: true)
            }
        }
    }
#endif
