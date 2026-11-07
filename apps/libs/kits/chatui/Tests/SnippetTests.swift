import ChatUI
import TestingKit

func snippetTests() -> [TestCase] {
    let long = "We talked about the garden, the fence, the shed and the path before we got to the budget at last."
    return [
        TestCase("given a word near the start, then the whole text is the snippet") {
            try expectEqual(Snippet.around(["garden"], in: long), long)
        },
        TestCase("given a word far in, then the snippet starts at a word a little before it") {
            let snippet = Snippet.around(["BUDGET"], in: long)
            try expect(snippet.hasPrefix("…"))
            try expect(snippet.hasSuffix("the budget at last."))
            try expect(snippet.count < long.count)
            try expect(snippet.dropFirst().first?.isWhitespace == false)
        },
        TestCase("given words that aren't in it, then the whole text is the snippet") {
            try expectEqual(Snippet.around(["piano"], in: long), long)
        },
        TestCase("given two words, then the snippet goes from the earlier one") {
            let snippet = Snippet.around(["last", "shed"], in: long)
            try expect(snippet.contains("shed"))
        },
    ]
}
