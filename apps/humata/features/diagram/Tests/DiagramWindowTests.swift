import AppKit
import XCTest

@testable import HumataDiagram

@MainActor
final class DiagramWindowTests: XCTestCase {
    func testFitShowsAllOfALargeDiagramAndEnlargesASmallOneOnlyAsFarAsItWasDrawn() {
        let space = CGSize(width: 548, height: 448)
        XCTAssertEqual(DiagramWindow.fit(CGSize(width: 1000, height: 200), in: space), 0.5, accuracy: 0.001)
        XCTAssertEqual(DiagramWindow.fit(CGSize(width: 250, height: 100), in: space), 2, accuracy: 0.001)
        XCTAssertEqual(DiagramWindow.fit(CGSize(width: 10, height: 10), in: space), DiagramWindow.scale)
    }

    func testARegionDraggedPastTheEdgeStopsAtIt() {
        let size = CGSize(width: 200, height: 100)
        let region = DiagramWindow.fraction(CGRect(x: 100, y: 50, width: 300, height: 300), of: size)
        XCTAssertEqual(region, CGRect(x: 0.5, y: 0.5, width: 0.5, height: 0.5))
    }

    func testTheRegionSentIsThatPartOfTheDrawing() throws {
        let image = NSImage(size: NSSize(width: 100, height: 50), flipped: false) { rect in
            NSColor.red.setFill()
            rect.fill()
            return true
        }
        let png = try XCTUnwrap(DiagramWindow.png(of: image, region: CGRect(x: 0, y: 0, width: 0.5, height: 0.5)))
        let part = try XCTUnwrap(NSBitmapImageRep(data: png))
        let whole = try XCTUnwrap(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
        XCTAssertEqual(part.pixelsWide, whole.width / 2)
        XCTAssertEqual(part.pixelsHigh, whole.height / 2)
    }
}
