import Foundation
import TestingKit

@testable import ChatUI

func markdownTests() -> [TestCase] {
    blockTests() + listTests() + tableTests() + inlineTests() + writingTests() + streamTests()
}

private func blockTests() -> [TestCase] {
    [
        TestCase("given lines without a blank between, then they're one paragraph") {
            try expectEqual(Markdown.blocks(in: "One\ntwo\n\nThree"), [.paragraph("One\ntwo"), .paragraph("Three")])
        },
        TestCase("given #s, then a heading of that level, without a closing sequence") {
            try expectEqual(
                Markdown.blocks(in: "# Plan\n## Week ##\n### Day\n#nospace"),
                [
                    .heading(level: 1, text: "Plan"), .heading(level: 2, text: "Week"),
                    .heading(level: 3, text: "Day"), .paragraph("#nospace"),
                ])
        },
        TestCase("given an underline of = or -, then the line above is a heading") {
            try expectEqual(
                Markdown.blocks(in: "Plan\n===\nWeek\n---"),
                [.heading(level: 1, text: "Plan"), .heading(level: 2, text: "Week")])
        },
        TestCase("given a fence, then code with its language, kept as written") {
            try expectEqual(
                Markdown.blocks(in: "Try:\n```swift\nlet a = 1\n\n  **b**\n```\nDone"),
                [.paragraph("Try:"), .code(language: "swift", code: "let a = 1\n\n  **b**"), .paragraph("Done")])
        },
        TestCase("given a tilde fence, then backticks inside don't close it") {
            try expectEqual(Markdown.blocks(in: "~~~\n```\n~~~"), [.code(language: nil, code: "```")])
        },
        TestCase("given ---, *** or _ _ _ after a blank line, then rules") {
            try expectEqual(Markdown.blocks(in: "A\n\n---\n***\n_ _ _"), [.paragraph("A"), .rule, .rule, .rule])
        },
        TestCase("given > lines, then a quote of the blocks in them, lazy lines too") {
            try expectEqual(
                Markdown.blocks(in: "> Water\nin the morning\n>\n> - often"),
                [
                    .quote([
                        .paragraph("Water\nin the morning"),
                        .list(MarkdownList(start: nil, items: [[.paragraph("often")]])),
                    ])
                ])
        },
        TestCase("given Windows line endings, then the same blocks") {
            try expectEqual(Markdown.blocks(in: "# A\r\nb\r\n"), [.heading(level: 1, text: "A"), .paragraph("b")])
        },
        TestCase("given nothing but space, then no blocks") {
            try expectEqual(Markdown.blocks(in: "  \n\n"), [])
        },
    ]
}

private func listTests() -> [TestCase] {
    [
        TestCase("given bullets, then a list with an item each") {
            try expectEqual(
                Markdown.blocks(in: "- Soil\n- Seeds"),
                [.list(MarkdownList(start: nil, items: [[.paragraph("Soil")], [.paragraph("Seeds")]]))])
        },
        TestCase("given numbers, then a list from the first number") {
            try expectEqual(
                Markdown.blocks(in: "3. Dig\n4) Not this list\n4. Plant"),
                [
                    .list(MarkdownList(start: 3, items: [[.paragraph("Dig")]])),
                    .list(MarkdownList(start: 4, items: [[.paragraph("Not this list")]])),
                    .list(MarkdownList(start: 4, items: [[.paragraph("Plant")]])),
                ])
        },
        TestCase("given an item indented under another, then a list inside the item") {
            let nested = MarkdownList(start: nil, items: [[.paragraph("Tomatoes")], [.paragraph("Basil")]])
            try expectEqual(
                Markdown.blocks(in: "1. Plant\n   - Tomatoes\n   - Basil\n2. Water"),
                [.list(MarkdownList(start: 1, items: [[.paragraph("Plant"), .list(nested)], [.paragraph("Water")]]))])
        },
        TestCase("given a nested item indented less than its parent's text, then it's still nested") {
            let nested = MarkdownList(start: nil, items: [[.paragraph("Tomatoes")]])
            try expectEqual(
                Markdown.blocks(in: "1. Plant\n  - Tomatoes"),
                [.list(MarkdownList(start: 1, items: [[.paragraph("Plant"), .list(nested)]]))])
        },
        TestCase("given a list that runs into a paragraph, then the paragraph interrupts it") {
            try expectEqual(
                Markdown.blocks(in: "Steps:\n- Dig\n\nThen rest."),
                [
                    .paragraph("Steps:"), .list(MarkdownList(start: nil, items: [[.paragraph("Dig")]])),
                    .paragraph("Then rest."),
                ])
        },
        TestCase("given items with blank lines between, then one list") {
            try expectEqual(
                Markdown.blocks(in: "- A\n\n- B"),
                [.list(MarkdownList(start: nil, items: [[.paragraph("A")], [.paragraph("B")]]))])
        },
        TestCase("given bold at the start of a line, then a paragraph, not a list") {
            try expectEqual(Markdown.blocks(in: "**Soil** first"), [.paragraph("**Soil** first")])
        },
        TestCase("given code in an item, then the code's in the item") {
            try expectEqual(
                Markdown.blocks(in: "- Run:\n  ```\n  make\n  ```"),
                [.list(MarkdownList(start: nil, items: [[.paragraph("Run:"), .code(language: nil, code: "make")]]))])
        },
    ]
}

private func tableTests() -> [TestCase] {
    [
        TestCase("given a header and a delimiter row, then a table with its alignments") {
            try expectEqual(
                Markdown.blocks(in: "| Task | Day | Hours |\n|:--|:-:|--:|\n| Soil | Sat | 2 |\n| Plant | Sun |"),
                [
                    .table(
                        MarkdownTable(
                            alignments: [.leading, .center, .trailing], header: ["Task", "Day", "Hours"],
                            rows: [["Soil", "Sat", "2"], ["Plant", "Sun", ""]]))
                ])
        },
        TestCase("given pipes escaped or in code, then they stay in the cell") {
            try expectEqual(
                Markdown.blocks(in: "a | b\n--|--\n`x|y` | 1 \\| 2"),
                [
                    .table(
                        MarkdownTable(
                            alignments: [.leading, .leading], header: ["a", "b"], rows: [["`x|y`", "1 | 2"]]))
                ])
        },
        TestCase("given a delimiter row with fewer cells than the header, then a paragraph") {
            try expectEqual(Markdown.blocks(in: "| a | b |\n|---|"), [.paragraph("| a | b |\n|---|")])
        },
        TestCase("given a table right after a paragraph's line, then the paragraph ends before it") {
            try expectEqual(
                Markdown.blocks(in: "Here:\na | b\n-|-"),
                [
                    .paragraph("Here:"),
                    .table(MarkdownTable(alignments: [.leading, .leading], header: ["a", "b"], rows: [])),
                ])
        },
    ]
}

private func inlineTests() -> [TestCase] {
    [
        TestCase("given bold, italic, struck and code, then each run has its intent") {
            let string = Markdown.inline("**a** *b* ~~c~~ `d`")
            let intents = string.runs.compactMap(\.inlinePresentationIntent)
            try expect(intents.contains(.stronglyEmphasized))
            try expect(intents.contains(.emphasized))
            try expect(intents.contains(.strikethrough))
            try expect(intents.contains(.code))
            try expectEqual(String(string.characters), "a b c d")
        },
        TestCase("given web and mail links, then they link") {
            let string = Markdown.inline("[site](https://example.com) and [mail](mailto:a@example.com)")
            try expectEqual(
                string.runs.compactMap(\.link),
                [URL(string: "https://example.com"), URL(string: "mailto:a@example.com")].compactMap { $0 })
        },
        TestCase("given a link to another scheme, then its words show without a link") {
            let string = Markdown.inline("[run](javascript:alert(1)) [file](file:///etc/hosts)")
            try expect(string.runs.allSatisfy { $0.link == nil })
            try expect(String(string.characters).hasPrefix("run"))
        },
        TestCase("given line breaks in a paragraph, then they're kept") {
            try expectEqual(String(Markdown.inline("one\ntwo").characters), "one\ntwo")
        },
    ]
}

private func writingTests() -> [TestCase] {
    [
        TestCase("given ** not closed yet, then it's closed at the end") {
            try expectEqual(Markdown.closingOpenMarks("Start with **Buy so"), "Start with **Buy so**")
        },
        TestCase("given marks with nothing after them yet, then they're left out") {
            try expectEqual(Markdown.closingOpenMarks("Start with **"), "Start with ")
            try expectEqual(Markdown.closingOpenMarks("Run `"), "Run ")
        },
        TestCase("given marks that are closed, then the text is as written") {
            let text = "**a** *b* ~~c~~ `d*` snake_case_name"
            try expectEqual(Markdown.closingOpenMarks(text), text)
        },
        TestCase("given emphasis open inside other emphasis, then both close in order") {
            try expectEqual(Markdown.closingOpenMarks("**bold and *ital"), "**bold and *ital***")
        },
        TestCase("given code not closed, then it closes and marks in it don't count") {
            try expectEqual(Markdown.closingOpenMarks("Use `a**b"), "Use `a**b`")
        },
        TestCase("given a link being written, then only its words show") {
            try expectEqual(Markdown.closingOpenMarks("See [the gui"), "See the gui")
            try expectEqual(Markdown.closingOpenMarks("See [the guide](https://exa"), "See the guide")
            try expectEqual(Markdown.closingOpenMarks("See [the guide]"), "See the guide")
            try expectEqual(Markdown.closingOpenMarks("See [a](https://a.com)"), "See [a](https://a.com)")
        },
        TestCase("given a fence not closed yet, then the code so far, without a closing fence half written") {
            try expectEqual(
                Markdown.blocks(in: "```swift\nlet a = 1\n``"), [.code(language: "swift", code: "let a = 1")])
            try expectEqual(Markdown.blocks(in: "```"), [.code(language: nil, code: "")])
        },
        TestCase("given a list's last item being written, then its marks close") {
            let block = Markdown.inProgress(.list(MarkdownList(start: nil, items: [[.paragraph("**a")]])))
            try expectEqual(block, .list(MarkdownList(start: nil, items: [[.paragraph("**a**")]])))
        },
        TestCase("given a paragraph and a lone - being written under it, then it's still a paragraph") {
            try expectEqual(Markdown.blocks(in: "Soil\n-"), [.paragraph("Soil\n-")])
        },
    ]
}

private func streamTests() -> [TestCase] {
    let reply = """
        ## Weekend

        Start with **Buy soil**, then see [the guide](https://example.com).

        1. Dig
           - Deep
        2. Plant

        > Water early.

        ```swift
        let day = "Saturday"

        print(day)
        ```

        ---

        | Task | Day |
        |------|----:|
        | Soil | Sat |

        Done.
        """
    return [
        TestCase("given a reply arriving a character at a time, then each step reads as the whole text does") {
            var stream = MarkdownStream()
            var written = ""
            for char in reply {
                written.append(char)
                stream.update(written)
                try expect(
                    stream.blocks == Markdown.blocks(in: written),
                    "after \(written.count) characters, got \(stream.blocks), expected \(Markdown.blocks(in: written))")
            }
        },
        TestCase("given text that doesn't go on from the last, then it's read from the start") {
            var stream = MarkdownStream()
            stream.update("# A\n\nB\n\nC")
            stream.update("Other\n\n- x")
            try expectEqual(
                stream.blocks, [.paragraph("Other"), .list(MarkdownList(start: nil, items: [[.paragraph("x")]]))])
        },
    ]
}
