import XCTest

@testable import HumataReview

final class CodeQuoteTests: XCTestCase {
    func testCodeWithBackticksIsFencedSoItCantCloseEarly() {
        let code = "let s = \"```\""
        let quote = CodeQuote.text(path: nil, start: nil, end: nil, language: "swift", code: code)
        let lines = quote.components(separatedBy: "\n")
        XCTAssertEqual(lines.first, "````swift")
        XCTAssertEqual(lines.last, "````")
        XCTAssertTrue(quote.contains(code))
    }

    func testTheCommentComesAfterTheCode() throws {
        let quote = CodeQuote.text(path: "a.rs", start: 3, end: 4, language: "rs", code: "x", comment: "  why?  ")
        XCTAssertTrue(quote.hasPrefix("`a.rs`"))
        let fenceEnd = try XCTUnwrap(quote.range(of: "```\n\n"))
        XCTAssertEqual(quote[fenceEnd.upperBound...], "why?")
    }
}
