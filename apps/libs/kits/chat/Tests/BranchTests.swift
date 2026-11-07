import ChatKit
import Foundation
import TestingKit

func branchTests() -> [TestCase] {
    [
        TestCase("given a message written after going back, then both branches are kept to go to") {
            var chat = chat(["one", "re one", "two", "re two"])
            chat.go(before: try chat.id("two"))
            chat.append(Message(id: UUID(), role: .person, text: "other", at: start + 600))
            try expectEqual(chat.messages.map(\.text), ["one", "re one", "other"])
            try expectEqual(chat.branches(at: try chat.id("other")).map(\.text), ["two", "other"])

            chat.go(to: try chat.id("re two"))
            try expectEqual(chat.messages.map(\.text), ["one", "re one", "two", "re two"])
        },
        TestCase("given replies that saved something, then checkpoints count them and going back names them") {
            var chat = chat(["one", "re one", "two", "re two"])
            let reply = try chat.id("re two")
            chat.saved[reply] = 3
            let points = chat.checkpoints { chat.saved[$0] ?? 0 }
            try expectEqual(points.map(\.changes), [0, 3])
            try expectEqual(points.map(\.isCurrent), [false, true])
            try expectEqual(
                chat.changedReplies(hiddenGoingBefore: try chat.id("two")) { chat.saved[$0] != nil }, [reply])
        },
        TestCase("given a fork, then it has the messages up to there and an empty context") {
            var chat = chat(["one", "re one", "two", "re two"])
            chat.saved[try chat.id("re one")] = 1
            let fork = try chat.fork(at: try chat.id("re one"), as: UUID(), now: start, name: "Fork").unwrap()
            try expectEqual(fork.messages.map(\.text), ["one", "re one"])
            try expectEqual(fork.saved, [:])
            try expectEqual(fork.title, "Fork")
            try expectEqual(chat.messages.count, 4)
        },
        TestCase("given a saved conversation, then its context reads back beside its own fields") {
            var chat = chat(["one", "re one"])
            chat.saved[try chat.id("re one")] = 2
            chat.rename("Plans")
            let read = try JSONDecoder().decode(Chat.self, from: JSONEncoder().encode(chat))
            try expectEqual(read, chat)
        },
        TestCase("given a conversation saved before branches and names, then it reads in one line, unnamed") {
            let json = """
                {"id":"\(UUID().uuidString)","createdAt":0,
                 "messages":[{"id":"\(UUID().uuidString)","role":"person","text":"Hi","at":0}]}
                """
            let read = try JSONDecoder().decode(Chat.self, from: Data(json.utf8))
            try expectEqual(read.title, "Hi")
            try expectEqual(read.saved, [:])
            try expect(!read.isSynced && read.parents.isEmpty)
        },
    ]
}
