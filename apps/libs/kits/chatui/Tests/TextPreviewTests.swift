import Foundation
import TestingKit

@testable import ChatUI

func textPreviewTests() -> [TestCase] {
    [
        TestCase("given a short text, then all of it shows, unboxed") {
            let preview = TextPreview("one\ntwo")
            try expectEqual(preview.shown, "one\ntwo")
            try expectEqual(preview.length, nil)
            try expect(!preview.isCut)
            try expect(!preview.isTall)
        },
        TestCase("given more lines than fit, then it's boxed but whole") {
            let text = Array(repeating: "line", count: 25).joined(separator: "\n")
            let preview = TextPreview(text)
            try expect(preview.isTall)
            try expect(!preview.isCut)
        },
        TestCase("given one long line, then it's boxed by its size") {
            let preview = TextPreview(String(repeating: "x", count: 2_001))
            try expect(preview.isTall)
            try expect(!TextPreview(String(repeating: "x", count: 2_000)).isTall)
        },
        TestCase("given a text past the limit, then its start shows with the whole size in bytes") {
            let preview = TextPreview(String(repeating: "ab", count: 50), limit: 10)
            try expectEqual(preview.shown, "ababababab")
            try expectEqual(preview.length, 100)
            try expect(preview.isCut)
        },
        TestCase("given a cut through a character, then it ends before that character") {
            // Each é takes two bytes, so ten bytes and one more cuts into the sixth.
            let preview = TextPreview(String(repeating: "é", count: 8), limit: 11)
            try expectEqual(preview.shown, String(repeating: "é", count: 5))
            try expectEqual(preview.length, 16)
        },
        TestCase("given a cut through an emoji, then no part of it shows") {
            let preview = TextPreview("ab👍🏽cd", limit: 4)
            try expectEqual(preview.shown, "ab")
            // Two letters, eight bytes of thumb and skin tone, two letters.
            try expectEqual(preview.length, 12)
        },
        TestCase("given text, then its lines count up to the limit") {
            try expectEqual(TextPreview.lines(in: "", atMost: 10), 1)
            try expectEqual(TextPreview.lines(in: "a\nb\nc", atMost: 10), 3)
            try expectEqual(TextPreview.lines(in: String(repeating: "\n", count: 1_000), atMost: 10), 10)
        },
        TestCase("given a message of more than twelve lines, then it's long") {
            try expect(PersonText.isLong(Array(repeating: "a", count: 13).joined(separator: "\n")))
            try expect(!PersonText.isLong(Array(repeating: "a", count: 12).joined(separator: "\n")))
        },
        TestCase("given a message past 700 characters, then it's long") {
            try expect(PersonText.isLong(String(repeating: "a", count: 701)))
            try expect(!PersonText.isLong(String(repeating: "a", count: 700)))
        },
    ]
}
