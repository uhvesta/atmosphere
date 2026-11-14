import XCTest

@testable import HumataSession

/// Which of a long chat's rows its list holds: the last of them, those
/// asked for above, and those a jump or an open edit names.
@MainActor
final class ChatTailTests: XCTestCase {
    /// A chat of `count` rows, each its own item, named `r0`, `r1` and on.
    private func lines(_ count: Int) -> [ChatLine] {
        (0..<count).map { ChatLine(item: $0, id: "r\($0)") }
    }

    func testAChatOpensOnItsLastRowsAndRowsAddedLeaveTheStartPut() {
        let short = ChatTail()
        XCTAssertEqual(short.start(in: lines(40), earlier: 0, showing: []), 0)

        let tail = ChatTail()
        let count = ChatTail.rows + 500
        let start = tail.start(in: lines(count), earlier: 0, showing: [])
        XCTAssertEqual(start, 500)
        // A turn's rows come: those shown still start where they did.
        XCTAssertEqual(tail.start(in: lines(count + 25), earlier: 0, showing: []), start)
        XCTAssertEqual(tail.start(in: lines(count + 900), earlier: 0, showing: [nil, nil]), start)
    }

    func testEarlierRowsAskedForComeAboveAndStopAtTheFirst() {
        let tail = ChatTail()
        let all = lines(ChatTail.rows + 500)
        XCTAssertEqual(tail.start(in: all, earlier: 0, showing: []), 500)
        XCTAssertEqual(tail.start(in: all, earlier: 200, showing: []), 300)
        // Asked for again with nothing new: nothing moves.
        XCTAssertEqual(tail.start(in: all, earlier: 200, showing: []), 300)
        XCTAssertEqual(tail.start(in: all, earlier: 450, showing: []), 50)
        // More asked for than there are: the chat's first row, and no further.
        XCTAssertEqual(tail.start(in: all, earlier: 5_000, showing: []), 0)
        XCTAssertEqual(tail.start(in: lines(all.count + 10), earlier: 5_000, showing: []), 0)
    }

    func testARowNamedAboveBringsTheRowsFromJustAboveItAndTheyStayOnceItIsNamedNoMore() {
        let tail = ChatTail()
        let all = lines(ChatTail.rows + 1_000)
        let opened = tail.start(in: all, earlier: 0, showing: [nil])
        XCTAssertEqual(opened, 1_000)

        let named = tail.start(in: all, earlier: 0, showing: ["r400"])
        // The row is held, with some above it so it isn't the first, and
        // not the whole chat with it.
        XCTAssertLessThan(named, 400)
        XCTAssertGreaterThan(named, 400 - ChatTail.rows)
        // The jump is over: what it brought stays, and rows added since
        // don't move it.
        XCTAssertEqual(tail.start(in: all, earlier: 0, showing: [nil]), named)
        XCTAssertEqual(tail.start(in: lines(all.count + 30), earlier: 0, showing: []), named)
        // Earlier rows asked for after it come above what it brought.
        XCTAssertEqual(tail.start(in: all, earlier: 100, showing: []), named - 100)
    }

    func testARowNamedAmongThoseShownOrNotInTheChatMovesNothing() {
        let tail = ChatTail()
        let all = lines(ChatTail.rows + 1_000)
        let opened = tail.start(in: all, earlier: 0, showing: [])
        XCTAssertEqual(tail.start(in: all, earlier: 0, showing: ["r1100"]), opened)
        XCTAssertEqual(tail.start(in: all, earlier: 0, showing: ["not a row"]), opened)
    }

    func testTwoRowsNamedAboveAreBothHeld() {
        let all = lines(ChatTail.rows + 1_000)
        for named in [["r700", "r200"], ["r200", "r700"]] {
            let tail = ChatTail()
            _ = tail.start(in: all, earlier: 0, showing: [])
            let start = tail.start(in: all, earlier: 0, showing: named)
            XCTAssertLessThanOrEqual(start, 200, "naming \(named) left one out")
            XCTAssertGreaterThan(start, 200 - ChatTail.rows, "naming \(named) brought far more than it named")
        }
    }

    func testAnEmptiedChatStartsOverAtItsLastRows() {
        let tail = ChatTail()
        let all = lines(ChatTail.rows + 1_000)
        _ = tail.start(in: all, earlier: 0, showing: [])
        XCTAssertLessThanOrEqual(tail.start(in: all, earlier: 600, showing: ["r100"]), 100)

        // Emptied, as before its history is read again: it opens at its
        // end as it first did, and what was asked for before counts no more.
        XCTAssertEqual(tail.start(in: [], earlier: 600, showing: ["r100"]), 0)
        XCTAssertEqual(tail.start(in: all, earlier: 600, showing: []), 1_000)
        XCTAssertEqual(tail.start(in: all, earlier: 700, showing: []), 900)
    }
}
