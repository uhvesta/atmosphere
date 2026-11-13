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

    private struct Key: Hashable {
        var source: String
        var dark: Bool
        var scale: Double
    }

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
        if case .success(let image) = result { cache.setObject(image, forKey: Self.name(key)) }
        return try result.get()
    }

    private static func name(_ key: Key) -> NSString {
        "\(key.dark ? 1 : 0) \(key.scale)\n\(key.source)" as NSString
    }

    private func draw(_ key: Key) async -> Result<NSImage, DiagramError> {
        guard let resources, FileManager.default.fileExists(atPath: resources.appending(path: "mermaid.min.js").path)
        else { return .failure(.unavailable) }
        let page = self.page ?? Page(resources: resources)
        self.page = page
        return await page.draw(key.source, dark: key.dark, scale: key.scale)
    }
}

/// The page mermaid.js draws on.
@MainActor
private final class Page: NSObject, WKNavigationDelegate {
    private let web: WKWebView
    private let resources: URL
    private var loaded = false
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
        loaded = true
        for waiter in waiting { waiter.resume(returning: true) }
        waiting = []
    }

    func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: any Error) {
        for waiter in waiting { waiter.resume(returning: false) }
        waiting = []
    }

    func webView(
        _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error
    ) {
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
            return .failure(.failed(message: "The page that draws diagrams didn't load."))
        }
        let answer: Any?
        do {
            answer = try await web.callAsyncJavaScript(
                Self.script, arguments: ["source": source, "dark": dark, "largest": MermaidRenderer.largest],
                contentWorld: .page)
        } catch {
            return .failure(.invalid(message: Self.message(of: error)))
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
        if scale != 1 { snapshot.snapshotWidth = NSNumber(value: width * scale) }
        do {
            let image = try await web.takeSnapshot(configuration: snapshot)
            // Points, whatever the scale it was drawn at.
            image.size = NSSize(width: width, height: height)
            return .success(image)
        } catch {
            return .failure(.failed(message: error.localizedDescription))
        }
    }

    /// What a script's error says, without WebKit's wrapping.
    private static func message(of error: any Error) -> String {
        let info = (error as NSError).userInfo
        return info["WKJavaScriptExceptionMessage"] as? String ?? error.localizedDescription
    }
}
