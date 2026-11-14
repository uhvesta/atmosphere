import ChatUI
import CoreGraphics
import XCTest

@testable import HumataSession

/// The guesses at rows' heights the scroll bar goes by before rows are drawn.
final class RowEstimateTests: XCTestCase {
    private let words = Array(repeating: "the parser reads each token once and keeps where it ended", count: 6)
        .joined(separator: " ")

    private func height(
        _ item: ChatItem, width: CGFloat = 700, scale: TextScale = .actual
    ) -> CGFloat {
        let heights = RowEstimate.heights(
            lines: [ChatLine(item: 0, id: item.id)], items: [item], width: width, scale: scale)
        return heights.first ?? 0
    }

    func testMoreTextIsTaller() {
        let short = height(.reply(id: "r", text: "Done."))
        let long = height(.reply(id: "r", text: words))
        let longer = height(.reply(id: "r", text: words + "\n\n" + words + "\n\n" + words))
        XCTAssertGreaterThan(long, short)
        XCTAssertGreaterThan(longer, long)
        XCTAssertGreaterThan(height(.person(id: "p", text: words)), height(.person(id: "p", text: "hi")))
    }

    func testCodeIsTallerForEachLine() {
        let few = height(.reply(id: "r", text: "```swift\nlet one = 1\n```"))
        let code = String(repeating: "let one = 1\n", count: 30)
        let many = height(.reply(id: "r", text: "```swift\n" + code + "```"))
        XCTAssertGreaterThan(many, few)
    }

    func testAWiderChatIsShorter() {
        let reply = ChatItem.reply(id: "r", text: words)
        XCTAssertLessThan(height(reply, width: 1_200), height(reply, width: 400))
    }

    func testBiggerTextIsTaller() {
        let item = ChatItem.reply(id: "r", text: words)
        XCTAssertGreaterThan(height(item, scale: TextScale(percent: 150)), height(item, scale: .actual))
        let tool = ChatItem.tool(ToolCall(id: "t", tool: "Bash", subject: "ls"))
        XCTAssertGreaterThan(height(tool, scale: TextScale(percent: 150)), height(tool, scale: .actual))
    }

    func testClosedToolCallsAreOneHeightWhateverTheyRan() {
        let short = ChatItem.tool(ToolCall(id: "a", tool: "Bash", subject: "ls", output: "a", succeeded: true))
        let long = ChatItem.tool(
            ToolCall(
                id: "b", tool: "Read", subject: "/a/very/long/path/to/a/file.swift",
                output: String(repeating: "line\n", count: 5_000), succeeded: false))
        XCTAssertEqual(height(short), height(long))
        XCTAssertEqual(height(short, width: 300), height(short, width: 1_500))
    }

    func testAPartOfAReplyIsGuessedFromThatPartAlone() {
        let text = words + "\n\n" + words + "\n\n" + words
        let part = text.startIndex..<text.index(text.startIndex, offsetBy: 20)
        let lines = [ChatLine(item: 0, id: "r"), ChatLine(item: 0, part: part, id: "r/1")]
        let heights = RowEstimate.heights(
            lines: lines, items: [.reply(id: "r", text: text)], width: 700, scale: .actual)
        XCTAssertEqual(heights.count, 2)
        XCTAssertLessThan(heights[1], heights[0])
    }

    func testItRunsOffTheMainThread() async {
        let item = ChatItem.reply(id: "r", text: words)
        let lines = [ChatLine(item: 0, id: "r")]
        // Called with no await from a detached task: it takes no actor.
        let heights = await Task.detached {
            RowEstimate.heights(lines: lines, items: [item], width: 700, scale: .actual)
        }.value
        XCTAssertEqual(heights.count, 1)
        XCTAssertGreaterThan(heights[0], 0)
    }
}
