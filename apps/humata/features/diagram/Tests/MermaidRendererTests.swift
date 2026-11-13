import AppKit
import XCTest

@testable import HumataDiagram

@MainActor
final class MermaidRendererTests: XCTestCase {
    /// The test bundle's Resources, where mermaid.html and mermaid.min.js land.
    private var resources: URL? {
        Bundle(for: Self.self).resourceURL
    }

    func testADiagramIsDrawnAndKept() async throws {
        let renderer = MermaidRenderer(resources: resources)
        let source = "graph LR\n  A --> B"
        XCTAssertNil(renderer.cached(source, dark: false))

        let image = try await renderer.image(source, dark: false)

        XCTAssertGreaterThan(image.size.width, 0)
        XCTAssertGreaterThan(image.size.height, 0)
        XCTAssertNotNil(renderer.cached(source, dark: false))
        XCTAssertNil(renderer.cached(source, dark: true))
    }

    func testADiagramWithAMistakeSaysItsInvalid() async {
        let renderer = MermaidRenderer(resources: resources)
        do {
            _ = try await renderer.image("graph LR\n  A -->", dark: false)
            XCTFail("A broken diagram was drawn")
        } catch {
            guard case .invalid = error else { return XCTFail("Expected invalid, got \(error)") }
        }
    }

    func testTwoDiagramsAskedForAtOnceAreBothDrawn() async throws {
        let renderer = MermaidRenderer(resources: resources)
        async let first = renderer.image("graph TD\n  A --> B", dark: false)
        async let second = renderer.image("sequenceDiagram\n  A->>B: hi", dark: true)
        let (flow, sequence) = try await (first, second)
        XCTAssertGreaterThan(flow.size.width, 0)
        XCTAssertGreaterThan(sequence.size.width, 0)
    }

    /// The second draw is what a page that failed used to leave waiting
    /// for good, with every diagram after it.
    func testAPageThatCantLoadFailsEveryDrawInsteadOfHoldingThemUp() async throws {
        let broken = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: broken, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: broken) }
        // mermaid.js is there, but not the page that runs it.
        try Data().write(to: broken.appending(path: "mermaid.min.js"))
        let renderer = MermaidRenderer(resources: broken)
        for source in ["graph LR\n  A --> B", "graph LR\n  B --> C"] {
            do {
                _ = try await renderer.image(source, dark: false)
                XCTFail("Drew without the page")
            } catch {
                guard case .failed = error else { return XCTFail("Expected failed, got \(error)") }
            }
        }
    }

    func testWithoutMermaidNothingCanBeDrawn() async {
        let empty = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        let renderer = MermaidRenderer(resources: empty)
        do {
            _ = try await renderer.image("graph LR\n  A --> B", dark: false)
            XCTFail("Drew without mermaid.js")
        } catch {
            XCTAssertEqual(error, .unavailable)
        }
    }
}
