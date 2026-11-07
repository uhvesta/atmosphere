import ChatKit
import Foundation
import TestingKit

func compactionTests() -> [TestCase] {
    [
        TestCase("given a summary, then only the messages after it are left, less the last two to compact") {
            var chat = chat(["a", "b", "c", "d", "e", "f"])
            chat.summaries[try chat.id("b")] = ChatSummary(text: "Earlier", at: start)
            try expectEqual(chat.unsummarized.map(\.text), ["c", "d", "e", "f"])
            try expectEqual(chat.compactable.map(\.text), ["c", "d"])
        },
        TestCase("given a long answer, then the summary ends at a sentence within the limit") {
            try expectEqual(ChatSummary.text(from: "  One. Two three. Four", limit: 18), "One. Two three.")
            try expectEqual(ChatSummary.text(from: " ", limit: 18), nil)
        },
        TestCase("given an answer with a label and quotes, then the name is just the words") {
            try expectEqual(Chat.name(from: "Title: \"Weekend plans.\"\nMore"), "Weekend plans")
        },
        TestCase("given more than fits, then the newest messages are kept and the last is cut from the front") {
            let messages = chat(["aaaa", "bbbb", "cccccccc"]).messages
            try expectEqual(messages.newest(fitting: 12).map(\.text), ["bbbb", "cccccccc"])
            try expectEqual(messages.newest(fitting: 3).map(\.text), ["ccc"])
        },
    ]
}
