import HumataReview
import XCTest

final class HighlighterTests: XCTestCase {
    func testSwiftKeywordsStringsNumbersCommentsAndTypes() throws {
        let swift = try XCTUnwrap(Highlighter(path: "Sources/App.swift"))
        let tokens = swift.tokens(#"let name: String = "a \"b\"" + 42 // done"#)
        XCTAssertEqual(tokens.first, Highlighter.Token(.keyword, "let"))
        XCTAssertTrue(tokens.contains(Highlighter.Token(.type, "String")))
        XCTAssertTrue(tokens.contains(Highlighter.Token(.string, #""a \"b\"""#)))
        XCTAssertTrue(tokens.contains(Highlighter.Token(.number, "42")))
        XCTAssertEqual(tokens.last, Highlighter.Token(.comment, "// done"))
        XCTAssertEqual(tokens.map(\.text).joined(), #"let name: String = "a \"b\"" + 42 // done"#)
    }

    func testPythonCommentsStartWithAHash() throws {
        let python = try XCTUnwrap(Highlighter(path: "sum.py"))
        XCTAssertEqual(
            python.tokens("def add(a): # sum"),
            [
                Highlighter.Token(.keyword, "def"), Highlighter.Token(.plain, " add(a): "),
                Highlighter.Token(.comment, "# sum"),
            ])
    }

    func testAnUnclosedStringRunsToTheLinesEnd() throws {
        let rust = try XCTUnwrap(Highlighter(path: "lib.rs"))
        XCTAssertEqual(rust.tokens(#"x = "open"#).last, Highlighter.Token(.string, #""open"#))
    }

    func testAnUnknownLanguageIsntHighlighted() {
        XCTAssertNil(Highlighter(path: "notes.txt"))
    }
}

final class MarkdownBlockTests: XCTestCase {
    func testBlocksKeepTheirLines() {
        let blocks = MarkdownBlock.blocks([
            "# Title",
            "",
            "One paragraph",
            "over two lines.",
            "",
            "```swift",
            "let x = 1",
            "```",
            "Last.",
        ])
        XCTAssertEqual(
            blocks,
            [
                MarkdownBlock(kind: .heading(level: 1), startLine: 1, endLine: 1, text: "Title"),
                MarkdownBlock(kind: .paragraph, startLine: 3, endLine: 4, text: "One paragraph\nover two lines."),
                MarkdownBlock(kind: .code, startLine: 6, endLine: 8, text: "let x = 1"),
                MarkdownBlock(kind: .paragraph, startLine: 9, endLine: 9, text: "Last."),
            ])
    }

    func testAHashWithoutASpaceIsntAHeading() {
        XCTAssertEqual(MarkdownBlock.blocks(["#tag"]).map(\.kind), [.paragraph])
    }
}
