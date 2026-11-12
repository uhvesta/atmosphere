#if os(macOS)
    import Foundation
    import LifrRoot
    import PDFKit
    import XCTest

    /// Printing a list, through the same print operation as File › Print,
    /// saved to a PDF instead of shown in the panel.
    @MainActor
    final class ListPrinterTests: XCTestCase {
        func testAPrintedListHasItsTitleSectionsAndTasksWithTheirBoxes() throws {
            let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID()).pdf")
            defer { try? FileManager.default.removeItem(at: url) }
            let markdown = """
                ## Overdue
                - [ ] Renew passport (HOME-0001)
                ## Today
                - [x] Water the plants (HOME-0002)
                """

            XCTAssertTrue(ListPrinter.savePDF(title: "Today", markdown: markdown, to: url))

            let text = try XCTUnwrap(PDFDocument(url: url)?.string)
            for expected in ["Today", "Overdue", "☐ Renew passport (HOME-0001)", "☑ Water the plants (HOME-0002)"] {
                XCTAssertTrue(text.contains(expected), expected)
            }
            XCTAssertFalse(text.contains("- [ ]"))
            XCTAssertFalse(text.contains("##"))
        }
    }
#endif
