import AppKit
import ChatUI
import SwiftUI
import WebKit

/// Why a diagram couldn't be drawn.
package enum DiagramError: Error, Equatable {
    /// The diagram's text has a mistake; `message` is mermaid's, which says
    /// where.
    case invalid(message: String)
    /// This build has no mermaid.js to draw with.
    case unavailable
    /// The web view that draws diagrams failed: `message` is WebKit's.
    case failed(message: String)
}

/// Draws mermaid diagrams as images, in one web view kept off screen that
/// runs the bundled mermaid.js with no network. Each image is kept by its
/// text, appearance and scale, so a diagram scrolled past and back, or in
/// two places, is drawn once.
@MainActor
package final class MermaidRenderer {
    /// Over the mermaid.js in the Resources of the bundle this is linked
    /// into: the app's, or a test bundle's.
    package static let shared = MermaidRenderer(resources: Bundle(for: MermaidRenderer.self).resourceURL)

    /// Mermaid blocks drawn as diagrams, for ChatUI's Markdown.
    package static let codeBlocks = CodeBlockRenderer { language, code in
        language.lowercased() == "mermaid" ? AnyView(MermaidDiagram(source: code)) : nil
    }

    /// The largest side of a drawing, in points, so a huge diagram can't
    /// take all the memory.
    static let largest = 4096.0
    /// The largest side of a drawing's bitmap, in pixels: a drawing at a
    /// scale, on a Retina screen, is cut down to it.
    static let largestPixels = 8192.0

    /// The width, in points, to snapshot a `width` by `height` drawing at
    /// to get it `scale` times its size, cut down so neither side of the
    /// bitmap passes `largestPixels` at `backing` pixels a point.
    static func snapshotWidth(width: Double, height: Double, scale: Double, backing: Double) -> Double {
        let side = max(width, height) * scale * max(backing, 1)
        return width * scale * (side > largestPixels ? largestPixels / side : 1)
    }

    private struct Key: Hashable {
        var source: String
        var dark: Bool
        var scale: Double
    }

    /// How long a draw may take before its page is given up on, so one
    /// that never answers doesn't hold up every diagram after it.
    private static let patience = Duration.seconds(10)

    private let resources: URL?
    private let cache = NSCache<NSString, NSImage>()
    private var drawing: [Key: Task<Result<NSImage, DiagramError>, Never>] = [:]
    /// The draw before the newest, which the newest waits for: they share
    /// the page.
    private var queue: Task<Void, Never>?
    private var page: Page?

    /// `resources` holds mermaid.html and mermaid.min.js.
    package init(resources: URL?) {
        self.resources = resources
        cache.countLimit = 200
        cache.totalCostLimit = 256 * 1024 * 1024
    }

    /// The image of `source` drawn before, if it was.
    package func cached(_ source: String, dark: Bool, scale: Double = 1) -> NSImage? {
        cache.object(forKey: Self.name(Key(source: source, dark: dark, scale: scale)))
    }

    /// `source` drawn, light or `dark`, at `scale` times its size.
    package func image(_ source: String, dark: Bool, scale: Double = 1) async throws(DiagramError) -> NSImage {
        let key = Key(source: source, dark: dark, scale: scale)
        if let image = cache.object(forKey: Self.name(key)) { return image }
        if let running = drawing[key] { return try await running.value.get() }
        let previous = queue
        let task = Task { @MainActor in
            await previous?.value
            return await self.draw(key)
        }
        drawing[key] = task
        queue = Task { _ = await task.value }
        let result = await task.value
        drawing[key] = nil
        if case .success(let image) = result {
            cache.setObject(image, forKey: Self.name(key), cost: Self.bytes(of: image))
        }
        return try result.get()
    }

    /// The memory `image`'s largest bitmap takes, at four bytes a pixel;
    /// by its size in points when none says how many pixels it has.
    private static func bytes(of image: NSImage) -> Int {
        let pixels = image.representations.map { $0.pixelsWide * $0.pixelsHigh }.max() ?? 0
        return (pixels > 0 ? pixels : Int(image.size.width * image.size.height)) * 4
    }

    private static func name(_ key: Key) -> NSString {
        "\(key.dark ? 1 : 0) \(key.scale)\n\(key.source)" as NSString
    }

    private func draw(_ key: Key) async -> Result<NSImage, DiagramError> {
        guard let resources, FileManager.default.fileExists(atPath: resources.appending(path: "mermaid.min.js").path)
        else { return .failure(.unavailable) }
        let page = self.page ?? Page(resources: resources)
        self.page = page
        let result = await Self.race(Self.patience) {
            await page.draw(key.source, dark: key.dark, scale: key.scale)
        }
        guard let result else {
            // Whatever it's stuck on, the next draw gets a page of its own.
            page.fail("The page that draws diagrams didn't answer in time.")
            if self.page === page { self.page = nil }
            return .failure(.failed(message: "The page that draws diagrams didn't answer in time."))
        }
        // A page that failed isn't asked again: the next draw makes a new one.
        if case .failure(.failed) = result, self.page === page { self.page = nil }
        return result
    }

    /// What `work` gives, or nil if it takes longer than `limit`: it's then
    /// left to end by itself, as WebKit's calls can't be stopped.
    private static func race(
        _ limit: Duration, _ work: @escaping @MainActor () async -> Result<NSImage, DiagramError>
    ) async -> Result<NSImage, DiagramError>? {
        let answer = Answer()
        return await withCheckedContinuation { continuation in
            answer.waiter = continuation
            let timer = Task { @MainActor in
                guard (try? await Task.sleep(for: limit)) != nil else { return }
                answer.give(nil)
            }
            Task { @MainActor in
                let result = await work()
                timer.cancel()
                answer.give(result)
            }
        }
    }

    /// Whoever waits for a raced draw, answered once.
    @MainActor
    private final class Answer {
        var waiter: CheckedContinuation<Result<NSImage, DiagramError>?, Never>?

        func give(_ result: Result<NSImage, DiagramError>?) {
            waiter?.resume(returning: result)
            waiter = nil
        }
    }
}

/// The page mermaid.js draws on.
@MainActor
private final class Page: NSObject, WKNavigationDelegate {
    private let web: WKWebView
    private let resources: URL
    private var loaded = false
    /// Why the page can draw nothing more: it didn't load, its process
    /// ended or it stopped answering. A page that failed stays failed.
    private(set) var failure: String?
    private var waiting: [CheckedContinuation<Bool, Never>] = []

    init(resources: URL) {
        let configuration = WKWebViewConfiguration()
        // Nothing it draws is kept on disk.
        configuration.websiteDataStore = .nonPersistent()
        web = WKWebView(frame: NSRect(x: 0, y: 0, width: 800, height: 600), configuration: configuration)
        self.resources = resources
        super.init()
        web.navigationDelegate = self
        web.setValue(false, forKey: "drawsBackground")
        web.loadFileURL(resources.appending(path: "mermaid.html"), allowingReadAccessTo: resources)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        guard failure == nil else { return }
        loaded = true
        for waiter in waiting { waiter.resume(returning: true) }
        waiting = []
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: any Error) {
        // A diagram's link, refused once the page is loaded, fails only itself.
        guard !loaded else { return }
        fail("The page that draws diagrams didn't load: \(error.localizedDescription)")
    }

    func webView(
        _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error
    ) {
        guard !loaded else { return }
        fail("The page that draws diagrams didn't load: \(error.localizedDescription)")
    }

    func webViewWebContentProcessDidTerminate(_ webView: WKWebView) {
        fail("The process that draws diagrams ended.")
    }

    /// Marks the page as done for, and tells the draws waiting on it.
    func fail(_ message: String) {
        if failure == nil { failure = message }
        for waiter in waiting { waiter.resume(returning: false) }
        waiting = []
    }

    /// Only the page itself: a diagram's link goes nowhere.
    func webView(
        _ webView: WKWebView, decidePolicyFor action: WKNavigationAction,
        decisionHandler: @escaping @MainActor (WKNavigationActionPolicy) -> Void
    ) {
        decisionHandler(action.request.url?.isFileURL == true && !loaded ? .allow : .cancel)
    }

    private func ready() async -> Bool {
        if failure != nil { return false }
        if loaded { return true }
        return await withCheckedContinuation { waiting.append($0) }
    }

    private static let script = """
        mermaid.initialize({
          startOnLoad: false, securityLevel: 'strict', theme: dark ? 'dark' : 'default',
          themeVariables: { fontFamily: '-apple-system, "Helvetica Neue", sans-serif' },
        });
        try { await mermaid.parse(source); } catch (error) { return { error: String(error.message || error) }; }
        const { svg } = await mermaid.render('diagram', source);
        const out = document.getElementById('out');
        out.innerHTML = svg;
        const drawn = out.firstElementChild;
        drawn.style.maxWidth = 'none';
        const box = drawn.getBoundingClientRect();
        const width = Math.min(Math.ceil(box.width), largest), height = Math.min(Math.ceil(box.height), largest);
        drawn.setAttribute('width', width);
        drawn.setAttribute('height', height);
        return { width, height };
        """

    func draw(_ source: String, dark: Bool, scale: Double) async -> Result<NSImage, DiagramError> {
        guard await ready() else {
            return .failure(.failed(message: failure ?? "The page that draws diagrams didn't load."))
        }
        let answer: Any?
        do {
            answer = try await web.callAsyncJavaScript(
                Self.script, arguments: ["source": source, "dark": dark, "largest": MermaidRenderer.largest],
                contentWorld: .page)
        } catch {
            // Only what the script threw is the diagram's mistake; anything
            // else is the page's, such as its process ending.
            let info = (error as NSError).userInfo
            guard let thrown = info["WKJavaScriptExceptionMessage"] as? String else {
                return .failure(.failed(message: failure ?? error.localizedDescription))
            }
            return .failure(.invalid(message: thrown))
        }
        guard let drawn = answer as? [String: Any] else {
            return .failure(.failed(message: "mermaid.js gave back nothing to show."))
        }
        if let error = drawn["error"] as? String { return .failure(.invalid(message: error)) }
        guard let width = drawn["width"] as? Double, let height = drawn["height"] as? Double, width > 0, height > 0
        else { return .failure(.invalid(message: "The diagram has nothing to draw.")) }
        web.frame = NSRect(x: 0, y: 0, width: width, height: height)
        let snapshot = WKSnapshotConfiguration()
        snapshot.rect = CGRect(x: 0, y: 0, width: width, height: height)
        snapshot.afterScreenUpdates = true
        // The snapshot is in points, each `backing` pixels; the screen with
        // the most is assumed, as the page is on none.
        let backing = web.window?.backingScaleFactor ?? NSScreen.screens.map(\.backingScaleFactor).max() ?? 2
        snapshot.snapshotWidth = NSNumber(
            value: MermaidRenderer.snapshotWidth(width: width, height: height, scale: scale, backing: backing))
        do {
            let image = try await web.takeSnapshot(configuration: snapshot)
            // Points, whatever the scale it was drawn at.
            image.size = NSSize(width: width, height: height)
            return .success(image)
        } catch {
            return .failure(.failed(message: error.localizedDescription))
        }
    }
}
