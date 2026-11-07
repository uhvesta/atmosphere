#if os(macOS)
    import AppKit
    import ApplicationServices
    import ScreenCaptureKit
    import SnapshotTesting
    import SwiftUI
    import XCTest

    /// Walks a person's flow through a live Mac window: the screen stays up
    /// while the test changes it, and each step is checked against its
    /// references, the accessibility tree and an image, so a reviewer can
    /// follow what the person sees from step to step. Sheets and popovers
    /// are windows of their own on the Mac, so each step covers every
    /// window on screen, the main one first; sheets are drawn over it.
    ///
    /// It reads the tree the way VoiceOver does, and takes images from the
    /// window server, which draws the sidebar's material and other effects
    /// a view can't draw by itself. The process running the tests, such as
    /// the terminal, needs Accessibility and Screen Recording access in
    /// System Settings › Privacy & Security.
    @MainActor
    public final class FlowRecorder {
        private let window: NSWindow
        private let name: String
        /// The longest a step waits for the screen to settle before
        /// failing.
        private let timeout: Duration
        /// Whether the model's words are left out of the trees.
        private let redactsModelText: Bool
        private var count = 0
        private static var launched = false

        /// Hosts `view` in a window of `size`, the app's default window.
        /// `name` names the flow's references. A step waits at most
        /// `timeout` for the screen to settle. With `redactsModelText`, each
        /// run of what a real model wrote is one `‹model text›` line, since
        /// its words differ from one GPU to the next, and how long it
        /// thought is `‹time›`; a screen still showing
        /// "Thinking…" or an error doesn't match one that shows a reply.
        public init(
            _ name: String, _ view: some View, size: CGSize = CGSize(width: 1180, height: 760),
            timeout: Duration = .seconds(5), redactsModelText: Bool = false
        ) {
            self.name = name
            self.timeout = timeout
            self.redactsModelText = redactsModelText
            let app = NSApplication.shared
            app.setActivationPolicy(.accessory)
            // xctest never launches its app, and an app that hasn't
            // launched doesn't answer accessibility requests.
            if !FlowRecorder.launched {
                app.finishLaunching()
                FlowRecorder.launched = true
            }
            // The same look on every machine, whatever the person picked.
            app.appearance = NSAppearance(named: .aqua)
            window = NSWindow(
                contentRect: CGRect(origin: CGPoint(x: 80, y: 80), size: size),
                styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentViewController = NSHostingController(rootView: view)
            window.setContentSize(size)
            window.orderFrontRegardless()
            window.makeKey()
            MacAccessibilityTree.start()
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
                    "\(label) kept changing for \(timeout). If the screen is stuck working, that's the bug; if "
                        + "it's only slow, give FlowRecorder a longer timeout.", file: file, line: line)
                return
            }
            assertSnapshot(
                of: lines.joined(separator: "\n") + "\n", as: .lines, named: label, file: file,
                testName: testName, line: line)
            guard image else { return }
            let rendered: NSImage
            do {
                rendered = try await render()
            } catch {
                XCTFail("\(label) couldn't be captured: \(error)", file: file, line: line)
                return
            }
            assertSnapshot(
                of: rendered, as: .image(precision: 0.99, perceptualPrecision: 0.98), named: label, file: file,
                testName: testName, line: line)
        }

        /// Throws when flows can't run here: while the screen is locked,
        /// macOS hides the session's windows, from the accessibility tree
        /// too, so every step would fail. Call it from the test's `setUp`.
        public static func checkScreen() throws(ScreenError) {
            let session = CGSessionCopyCurrentDictionary() as? [String: Any]
            if session?["CGSSessionScreenIsLocked"] as? Bool == true { throw .locked }
        }

        public enum ScreenError: LocalizedError, CustomStringConvertible {
            case locked

            public var errorDescription: String? { description }

            public var description: String {
                "the screen is locked, and macOS hides a locked session's windows from the test. Unlock the Mac "
                    + "and run the test again."
            }
        }

        /// Ends the flow, closing its windows.
        public func finish() {
            for window in windows where window !== self.window { window.orderOut(nil) }
            window.close()
            window.contentViewController = nil
        }

        /// This flow's windows on screen: the main one, then its popovers
        /// and other windows, in the order they opened. A sheet is drawn
        /// over its window, so it's in the main one's image already.
        private var windows: [NSWindow] {
            [window]
                + NSApplication.shared.windows.filter { $0 !== window && $0.isVisible && $0.sheetParent == nil }
        }

        /// The windows stacked top to bottom, as the window server draws
        /// them.
        private func render() async throws -> NSImage {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            let shown = Dictionary(content.windows.map { ($0.windowID, $0) }, uniquingKeysWith: { first, _ in first })
            var images: [CGImage] = []
            for window in windows {
                guard let shared = shown[CGWindowID(window.windowNumber)] else { continue }
                let configuration = SCStreamConfiguration()
                configuration.width = Int(shared.frame.width * window.backingScaleFactor)
                configuration.height = Int(shared.frame.height * window.backingScaleFactor)
                configuration.showsCursor = false
                configuration.ignoreShadowsSingleWindow = true
                images.append(
                    try await SCScreenshotManager.captureImage(
                        contentFilter: SCContentFilter(desktopIndependentWindow: shared),
                        configuration: configuration))
            }
            guard !images.isEmpty else { throw CaptureError.noWindow }
            let gap = 32
            let width = images.map(\.width).max() ?? 0
            let height = images.map(\.height).reduce(0, +) + gap * (images.count - 1)
            guard
                let context = CGContext(
                    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                    space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(),
                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            else { throw CaptureError.noWindow }
            context.setFillColor(NSColor.gray.cgColor)
            context.fill(CGRect(x: 0, y: 0, width: width, height: height))
            var top = height
            for image in images {
                top -= image.height
                context.draw(image, in: CGRect(x: 0, y: top, width: image.width, height: image.height))
                top -= gap
            }
            guard let combined = context.makeImage() else { throw CaptureError.noWindow }
            return NSImage(cgImage: combined, size: CGSize(width: width / 2, height: height / 2))
        }

        private enum CaptureError: Error, CustomStringConvertible {
            case noWindow

            var description: String {
                "the window server shows none of its windows. Give the process running the tests, such as the "
                    + "terminal, Screen Recording access in System Settings › Privacy & Security."
            }
        }

        /// Waits until the screen stays the same for three looks in a row,
        /// and returns its accessibility tree; nil when it's still changing
        /// after `timeout`.
        private func settle() async -> [String]? {
            var last: [String] = []
            var unchanged = 0
            let clock = ContinuousClock()
            let deadline = clock.now + timeout
            while clock.now < deadline {
                try? await Task.sleep(for: .milliseconds(80))
                for window in windows { window.layoutIfNeeded() }
                // A process's requests to itself are answered on the thread
                // asking, and SwiftUI answers only on the main one.
                let lines = MacAccessibilityTree.lines(redactingModelText: redactsModelText)
                unchanged = lines == last ? unchanged + 1 : 0
                last = lines
                if unchanged == 3 {
                    // The tree settles before the last frames of a transition
                    // draw.
                    try? await Task.sleep(for: .milliseconds(250))
                    return last
                }
            }
            return nil
        }
    }

    /// The accessibility tree VoiceOver reads on the Mac, one element per
    /// line, indented under the element it's in: role, then title, label,
    /// value, help and identifier. Groups without a name of their own are
    /// left out, their children taking their place. Each window of the
    /// process, the main one first.
    @MainActor
    enum MacAccessibilityTree {
        nonisolated(unsafe) private static let app = AXUIElementCreateApplication(getpid())

        /// Containers that only matter for what's in them.
        private static let containers: Set<String> = [
            "AXGroup", "AXScrollArea", "AXSplitGroup", "AXLayoutArea", "AXUnknown", "AXColumn", "AXCell",
        ]
        /// Chrome every window has, the same each step.
        private static let skipped: Set<String> = ["AXScrollBar", "AXSplitter", "AXValueIndicator"]

        /// Asks SwiftUI for its accessibility tree, as an assistive app's
        /// first request does.
        static func start() {
            if !AXIsProcessTrusted() {
                print(
                    "warning: this process can't read accessibility trees. Give the terminal running the tests "
                        + "Accessibility access in System Settings › Privacy & Security.")
            }
            AXUIElementSetMessagingTimeout(app, 2)
            AXUIElementSetAttributeValue(app, "AXManualAccessibility" as CFString, kCFBooleanTrue)
        }

        /// What the app marks as written by the model. Its marker isn't
        /// shown.
        private static let modelText = modelTextIdentifier
        private static let redacted = "‹model text›"

        static func lines(redactingModelText redacts: Bool = false) -> [String] {
            // The main window first, then its sheets and popovers.
            let windows = (attribute(app, kAXWindowsAttribute) as? [AXUIElement] ?? [])
                .sorted { isMain($0) && !isMain($1) }
            var lines: [String] = []
            for window in windows { visit(window, depth: 0, redacts: redacts, into: &lines) }
            return lines
        }

        private static func isMain(_ window: AXUIElement) -> Bool {
            attribute(window, kAXMainAttribute) as? Bool == true
        }

        private static func visit(_ element: AXUIElement, depth: Int, redacts: Bool, into lines: inout [String]) {
            guard lines.count < 2_000 else { return }
            let role = attribute(element, kAXRoleAttribute) as? String ?? "AXUnknown"
            if skipped.contains(role) { return }
            if redacts, text(attribute(element, kAXIdentifierAttribute)) == modelText {
                // However many paragraphs or rows the model wrote, one line.
                let line = String(repeating: "  ", count: depth) + redacted
                if lines.last != line { lines.append(line) }
                return
            }
            var depth = depth
            if let line = describe(element, role: role) {
                lines.append(String(repeating: "  ", count: depth) + (redacts ? redactingTime(line) : line))
                depth += 1
            }
            for child in attribute(element, kAXChildrenAttribute) as? [AXUIElement] ?? [] {
                visit(child, depth: depth, redacts: redacts, into: &lines)
            }
        }

        /// `line` without how long a real model thought, which differs from
        /// one run to the next.
        private static func redactingTime(_ line: String) -> String {
            line.replacing(/Thoughts · [^"]*?(?= on the |")/, with: "Thoughts · ‹time›")
        }

        /// The element's line, or nil for an unnamed container.
        private static func describe(_ element: AXUIElement, role: String) -> String? {
            let title = text(attribute(element, kAXTitleAttribute))
            let label = text(attribute(element, kAXDescriptionAttribute))
            let value = text(attribute(element, kAXValueAttribute))
            if containers.contains(role), title == nil, label == nil, value == nil { return nil }
            var states = [role.replacingOccurrences(of: "AX", with: "")]
            if let subrole = attribute(element, kAXSubroleAttribute) as? String {
                states[0] += "/" + subrole.replacingOccurrences(of: "AX", with: "")
            }
            if attribute(element, kAXSelectedAttribute) as? Bool == true { states.append("selected") }
            if attribute(element, kAXEnabledAttribute) as? Bool == false { states.append("disabled") }
            var parts = ["[\(states.joined(separator: ", "))]", "\"\(title ?? label ?? "")\""]
            if let title, let label, title != label { parts.append("label: \"\(label)\"") }
            if let value { parts.append("value: \"\(value)\"") }
            if let help = text(attribute(element, kAXHelpAttribute)) { parts.append("help: \"\(help)\"") }
            if let identifier = text(attribute(element, kAXIdentifierAttribute)), identifier != modelText {
                parts.append("id: \(identifier)")
            }
            return parts.joined(separator: " ")
        }

        private static func attribute(_ element: AXUIElement, _ name: String) -> CFTypeRef? {
            var value: CFTypeRef?
            guard AXUIElementCopyAttributeValue(element, name as CFString, &value) == .success else { return nil }
            return value
        }

        private static func text(_ value: Any?) -> String? {
            let text: String
            switch value {
            case let string as String: text = string
            case let string as NSAttributedString: text = string.string
            case let number as NSNumber: text = number.stringValue
            default: return nil
            }
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            return trimmed.isEmpty ? nil : String(trimmed.prefix(200))
        }
    }
#endif
