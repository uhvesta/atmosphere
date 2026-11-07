import ChatKit
import Foundation
import TestingKit

func searchTests() -> [TestCase] {
    let calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC") ?? .current
        return calendar
    }()
    return [
        TestCase("given a time in the query, then it's read as when and left out of the words") {
            let query = ConversationQuery("groceries last week", now: start, calendar: calendar)
            try expectEqual(query.words, ["groceries"])
            try expectEqual(query.during, calendar.dateInterval(of: .weekOfYear, for: start - 7 * 86_400))
        },
        TestCase("given words in an older message on another branch, then that message matches") {
            var chat = chat(["buy Crème fraîche", "ok", "two", "re two"])
            chat.go(before: try chat.id("buy Crème fraîche"))
            let match = try chat.match(ConversationQuery(words: ["creme"])).unwrap()
            try expectEqual(match.message?.text, "buy Crème fraîche")
            try expect(!match.isRelated)
        },
        TestCase("given only related words, then the match says so") {
            let chat = chat(["plan the gardens", "ok"])
            let match = try chat.match(ConversationQuery(words: ["plants"]), related: ["garden"]).unwrap()
            try expect(match.isRelated)
            try expectEqual(chat.match(ConversationQuery(words: ["plants"])), nil)
        },
    ]
}
